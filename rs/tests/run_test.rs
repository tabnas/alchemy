// The public API end to end: the spec's worked example (section 5) run
// through the spec's program (sections 12.1 and 13.4), natively and
// interpreted, byte for byte; the streaming behaviours of the spec's
// section 19.5 that apply to a run over one document; the `json` echo
// of every fixture against the walk's own rendering; and `records` of a
// table round-tripping through JSON.

#[path = "../../../transduce/rs/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tabnas_alchemy::{compile, Output, Program, Renderer};
use tabnas_render::{JsonOptions, JsonRenderer, StringOut, WriteOut};
use tabnas_transduce::{
    replay, Code, Fail, Limits, Metrics, OwnedJsonEvent, ParserSource, Prune, SourceMode,
};

/// The spec's worked example: aless's `tests/fixtures/records.json`,
/// byte for byte (329 bytes; the metadata before the rows; Bob's members
/// in another order; `50.25` and `72` as written).
const RECORDS: &str = r#"{"response":{"metadata":{"fields":[{"title":"Identifier","path":["id"]},{"title":"Full name","path":["person","name"]},{"title":"Balance","path":["account","balance"]}]},"payload":{"deep":{"records":[{"id":123,"person":{"name":"Alice"},"account":{"balance":50.25}},{"account":{"balance":72},"person":{"name":"Bob"},"id":456}]}}}}"#;

const EXPECTED_CSV: &str =
    "\"Identifier\",\"Full name\",\"Balance\"\r\n\"123\",\"Alice\",\"50.25\"\r\n\"456\",\"Bob\",\"72\"\r\n";

/// The spec's program (sections 12.1 and 13.4).
const PROGRAM: &str = "def column-from-meta [source]
  record
    entry :label (get \"title\" source)
    entry :source
      as-path
        get \"path\" source

def api-binding
  record
    entry :columns
      path \"response\" \"metadata\" \"fields\"
    entry :rows
      path \"response\" \"payload\" \"deep\" \"records\" each-index
    entry :column column-from-meta

def api-table [input]
  table-from-json api-binding input

def export [input]
  pipe input
    api-table
    csv csv-options
";

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What a host does: parse `text` with the json grammar incrementally,
/// pruned under the program's row selector when it has one, push the
/// events into the program's sink, and mark a failure as leaving partial
/// output when `output_bytes` says bytes reached the writer.
fn drive(
    program: &Program,
    text: &str,
    render: Option<Renderer>,
    limits: &Limits,
) -> (Result<(), Fail>, String) {
    let metrics = Metrics::new();
    let buffer = Shared::default();
    let sink = match program.sink(Box::new(buffer.clone()), render, limits, metrics.clone()) {
        Ok(sink) => sink,
        Err(f) => return (Err(f), String::new()),
    };
    let prune = match program.row_selector() {
        Some(selector) => Prune::Under(selector.clone()),
        None => Prune::Never,
    };
    let (outcome, _) = ParserSource::new(tabnas_json::make(), text)
        .grammar("json")
        .mode(SourceMode::Incremental { prune })
        .limits(limits.clone())
        .metrics(metrics.clone())
        .run_owned(sink);
    let outcome = outcome.map(|_| ()).map_err(|f| {
        if Metrics::get(&metrics.output_bytes) > 0 && !f.committed_output {
            f.committed()
        } else {
            f
        }
    });
    let bytes = buffer.0.lock().unwrap().clone();
    (outcome, String::from_utf8(bytes).expect("utf-8 output"))
}

fn ok(program: &Program, text: &str, render: Option<Renderer>) -> String {
    let (outcome, out) = drive(program, text, render, &Limits::default());
    outcome.unwrap_or_else(|f| panic!("{f}"));
    out
}

fn err(program: &Program, text: &str, limits: &Limits) -> (Fail, String) {
    let (outcome, out) = drive(program, text, None, limits);
    (outcome.expect_err("the run fails"), out)
}

fn both() -> [Program; 2] {
    let native = compile(PROGRAM, "export.alc").expect("the spec's program compiles");
    let interpreted = native.with_native(false).unwrap();
    [native, interpreted]
}

/// Acceptance 3: the worked example prints the spec's bytes, both ways.
#[test]
fn the_worked_example_prints_the_spec_csv_both_ways() {
    for program in both() {
        assert_eq!(program.output(), Output::Text);
        assert_eq!(
            program.row_selector().unwrap().to_string(),
            ".response.payload.deep.records[*]"
        );
        assert_eq!(
            ok(&program, RECORDS, None),
            EXPECTED_CSV,
            "native={}",
            program.native()
        );
    }
}

fn shaped(records: &str) -> String {
    let meta = support::METADATA;
    format!(
        r#"{{"response":{{"metadata":{meta},"payload":{{"deep":{{"records":[{records}]}}}}}}}}"#
    )
}

/// Spec 19.5: metadata after rows is rejected under the metadata-first
/// policy, before any row is retained, and no CSV is written.
#[test]
fn metadata_after_rows_is_an_input_order_violation() {
    let meta = support::METADATA;
    let doc = format!(
        r#"{{"response":{{"payload":{{"deep":{{"records":[{}]}}}},"metadata":{meta}}}}}"#,
        support::record(1)
    );
    for program in both() {
        let (fail, out) = err(&program, &doc, &Limits::default());
        assert_eq!(
            fail.code,
            Code::InputOrderViolation,
            "native={}",
            program.native()
        );
        assert!(!fail.committed_output);
        assert_eq!(out, "");
    }
}

/// Spec 19.5: cells follow the schema's order whatever the row's.
#[test]
fn cells_follow_schema_order_not_member_order() {
    let doc = shaped(r#"{"account":{"balance":1},"person":{"name":"z"},"id":2}"#);
    for program in both() {
        assert_eq!(
            ok(&program, &doc, None),
            "\"Identifier\",\"Full name\",\"Balance\"\r\n\"2\",\"z\",\"1\"\r\n"
        );
    }
}

/// Spec 19.5: no matching rows is a valid empty table: the header alone.
#[test]
fn no_matching_rows_prints_the_header_only() {
    let meta = support::METADATA;
    for doc in [
        shaped(""),
        format!(r#"{{"response":{{"metadata":{meta}}}}}"#),
    ] {
        for program in both() {
            assert_eq!(
                ok(&program, &doc, None),
                "\"Identifier\",\"Full name\",\"Balance\"\r\n",
                "native={}",
                program.native()
            );
        }
    }
}

/// Spec 19.5: invalid trailing input fails the run after rows were
/// exported; with enough rows to pass the writer's budget, the failure
/// says the output is partial, and what was committed is whole records.
#[test]
fn invalid_trailing_input_fails_after_rows_were_exported() {
    let whole = support::records_json(2000);
    let doc = format!("{whole} x");
    for program in both() {
        let full = ok(&program, &whole, None);
        let (fail, out) = err(&program, &doc, &Limits::default());
        assert_eq!(fail.code, Code::InputInvalid, "native={}", program.native());
        assert!(fail.committed_output, "native={}", program.native());
        assert!(out.starts_with("\"Identifier\",\"Full name\",\"Balance\"\r\n"));
        assert!(out.len() > 32 * 1024);
        // The writer coalesces by fragment, never holding a row back to
        // end on a record boundary (spec 17.4): what was committed is a
        // prefix of the whole output, and may end inside a record.
        assert!(full.starts_with(&out), "native={}", program.native());
        assert!(out.len() < full.len());
    }
    // A small document: the rows were buffered, not committed, so nothing
    // reached the writer and the failure says so.
    let (fail, out) = err(&both()[0], &format!("{RECORDS} x"), &Limits::default());
    assert_eq!(fail.code, Code::InputInvalid);
    assert!(!fail.committed_output);
    assert_eq!(out, "");
}

/// Spec 19.5: a very large selected row fails clearly under the limit
/// that bounds it. The native transducer materializes rows under
/// `max_record_bytes`; the library's generic `capture` is bounded by
/// `max_capture_bytes`, which is the one it names.
#[test]
fn a_very_large_selected_row_names_the_limit() {
    let big = format!(
        r#"{{"id":1,"person":{{"name":"{}"}},"account":{{"balance":2}}}}"#,
        "x".repeat(4096)
    );
    let doc = shaped(&format!("{},{big}", support::record(0)));
    let limits = Limits {
        max_record_bytes: 1024,
        max_capture_bytes: 1024,
        ..Limits::default()
    };
    let [native, interpreted] = both();
    let (fail, _) = err(&native, &doc, &limits);
    assert_eq!(fail.code, Code::ResourceLimitExceeded);
    assert_eq!(fail.limit.as_ref().unwrap().name, "max_record_bytes");
    assert_eq!(
        fail.path.as_deref(),
        Some(".response.payload.deep.records[1]")
    );
    let (fail, _) = err(&interpreted, &doc, &limits);
    assert_eq!(fail.code, Code::ResourceLimitExceeded);
    assert_eq!(fail.limit.as_ref().unwrap().name, "max_capture_bytes");
    assert_eq!(
        fail.path.as_deref(),
        Some(".response.payload.deep.records[1]")
    );
}

/// A missing cell under the standard options is `MISSING_VALUE`, and a
/// null is the empty string, both ways.
#[test]
fn missing_and_null_cells_follow_the_options() {
    for program in both() {
        let (fail, _) = err(
            &program,
            &shaped(r#"{"id":1,"person":{},"account":{"balance":2}}"#),
            &Limits::default(),
        );
        assert_eq!(fail.code, Code::MissingValue, "native={}", program.native());
        assert_eq!(
            ok(
                &program,
                &shaped(r#"{"id":null,"person":{"name":"n"},"account":{"balance":null}}"#),
                None
            ),
            "\"Identifier\",\"Full name\",\"Balance\"\r\n\"\",\"n\",\"\"\r\n"
        );
    }
}

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../transduce/rs/tests/fixtures");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("transduce's fixtures are beside this checkout")
        .map(|e| e.expect("an entry").path())
        .collect();
    paths.sort();
    paths
}

fn grammar_for(path: &Path) -> Option<fn() -> tabnas::Tabnas> {
    match path.extension().and_then(|e| e.to_str())? {
        "json" => Some(tabnas_json::make),
        "jsonl" => Some(tabnas_jsonl::make),
        "yaml" => Some(tabnas_yaml::make),
        "csv" | "tsv" => Some(tabnas_csv::make),
        _ => None,
    }
}

/// `json input` echoes every fixture as the JSON renderer renders the
/// walk's events: the plan passes the events through untouched.
#[test]
fn json_echo_of_every_fixture_equals_the_walks_rendering() {
    let echo = compile("def export [input] (json input)", "echo.alc").unwrap();
    assert_eq!(echo.output(), Output::Text);
    let identity = compile("def export [input] input", "id.alc").unwrap();
    assert_eq!(identity.output(), Output::JsonEvents);
    let mut compared = 0;
    for path in fixtures() {
        let Some(make) = grammar_for(&path) else {
            continue;
        };
        let text = std::fs::read_to_string(&path).unwrap();
        let (outcome, events) =
            ParserSource::new(make(), &text).run_owned(Vec::<OwnedJsonEvent>::new());
        if outcome.is_err() {
            continue;
        }
        let mut reference = JsonRenderer::new(
            StringOut::new(),
            JsonOptions {
                indent: None,
                trailing_newline: true,
            },
        );
        if replay(&events, &mut reference).is_err() {
            // A document the renderer refuses (a non-finite number, say)
            // is refused the same way through the program; not compared.
            continue;
        }
        let expected = reference.into_inner().into_string();
        for program in [&echo, &identity] {
            let buffer = Shared::default();
            let mut sink = program
                .sink(
                    Box::new(buffer.clone()),
                    None,
                    &Limits::default(),
                    Metrics::new(),
                )
                .unwrap();
            replay(&events, &mut sink).unwrap_or_else(|f| panic!("{}: {f}", path.display()));
            let bytes = buffer.0.lock().unwrap().clone();
            assert_eq!(
                String::from_utf8(bytes).unwrap(),
                expected,
                "{}",
                path.display()
            );
        }
        compared += 1;
    }
    assert!(compared > 10, "{compared} fixtures compared");
}

/// `records` of a table is JSON events: rendered as JSON they read back
/// as the rows, keyed by label, with the lexemes kept.
#[test]
fn records_of_a_table_round_trips() {
    let program = compile(
        &PROGRAM.replace("    csv csv-options\n", "    records\n    json\n"),
        "records.alc",
    )
    .unwrap();
    assert_eq!(program.output(), Output::Text);
    let expected = r#"[{"Identifier":123,"Full name":"Alice","Balance":50.25},{"Identifier":456,"Full name":"Bob","Balance":72}]"#;
    for program in [program.clone(), program.with_native(false).unwrap()] {
        let out = ok(&program, RECORDS, None);
        assert_eq!(out, format!("{expected}\n"));
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[1]["Full name"], "Bob");
        assert_eq!(parsed[0]["Balance"], 50.25);
    }
    // The table result rendered by the host as JSON is the same document,
    // and as CSV the spec's bytes.
    let table = compile(&PROGRAM.replace("    csv csv-options\n", ""), "table.alc").unwrap();
    assert_eq!(table.output(), Output::TableRows);
    assert_eq!(
        ok(&table, RECORDS, Some(Renderer::Json)),
        format!("{expected}\n")
    );
    assert_eq!(ok(&table, RECORDS, Some(Renderer::Csv)), EXPECTED_CSV);
    assert_eq!(ok(&table, RECORDS, None), EXPECTED_CSV);
    // A renderer for a program that renders its own text is refused.
    let (fail, _) = drive(&program, RECORDS, Some(Renderer::Json), &Limits::default());
    let fail = fail.unwrap_err();
    assert_eq!(fail.code, Code::DslTypeError);
    assert!(fail.message.starts_with("render_of_text: "), "{fail}");
}

/// The output limit is the writer's: a run that would exceed it fails
/// with the limit named, and nothing past it is written.
#[test]
fn the_output_limit_is_enforced_by_the_writer() {
    let limits = Limits {
        max_output_bytes: Some(200),
        ..Limits::default()
    };
    let (fail, _) = err(&both()[0], &support::records_json(50), &limits);
    assert_eq!(fail.code, Code::ResourceLimitExceeded);
    assert_eq!(fail.limit.as_ref().unwrap().name, "max_output_bytes");
}

/// `sink_out` takes any text output: a writer with no budget commits
/// every fragment as it is written.
#[test]
fn sink_out_takes_the_hosts_own_text_output() {
    let program = compile(PROGRAM, "export.alc").unwrap();
    let buffer = Shared::default();
    let out = WriteOut::new(buffer.clone()).with_budget(0);
    let mut sink = program
        .sink_out(Box::new(out), None, &Limits::default(), Metrics::new())
        .unwrap();
    let (outcome, events) =
        ParserSource::new(tabnas_json::make(), RECORDS).run_owned(Vec::<OwnedJsonEvent>::new());
    outcome.unwrap();
    replay(&events, &mut sink).unwrap();
    assert_eq!(
        String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap(),
        EXPECTED_CSV
    );
}
