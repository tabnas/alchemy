// The differential test: the standard library's own text, interpreted,
// against the native compositions the runtime substitutes for it. Over
// every transduce fixture a grammar this crate's tests can read, and the
// generated documents transduce's tests and benches share, the spec's
// worked-example program produces the same bytes both ways, or fails with
// the same code. The library text is the reference; the native path is
// the optimization, and this is what makes it one.

#[path = "../../../transduce/rs/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tabnas::Tabnas;
use tabnas_alchemy::{compile, Program, Renderer};
use tabnas_transduce::{
    replay, Code, Fail, JsonEvent, Limits, Metrics, OwnedJsonEvent, ParserSource,
};

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

/// A writer the test keeps a handle on after the sink took it.
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

/// The grammar for a fixture, by extension, among those this crate's
/// tests take; `None` skips the fixture.
fn grammar_for(path: &Path) -> Option<fn() -> Tabnas> {
    match path.extension().and_then(|e| e.to_str())? {
        "json" => Some(tabnas_json::make),
        "jsonl" => Some(tabnas_jsonl::make),
        "yaml" => Some(tabnas_yaml::make),
        "csv" | "tsv" => Some(tabnas_csv::make),
        _ => None,
    }
}

/// The events of one document, recorded once so both paths replay the
/// same stream (the walk of the grammar's value, sound for every grammar).
fn events(make: fn() -> Tabnas, text: &str) -> Result<Vec<OwnedJsonEvent>, Fail> {
    let (outcome, events) = ParserSource::new(make(), text).run_owned(Vec::new());
    outcome.map(|_| events)
}

/// Replay `events` through `program`; the output, or the failure.
fn run(
    program: &Program,
    events: &[OwnedJsonEvent],
    render: Option<Renderer>,
) -> Result<String, Fail> {
    run_under(program, events, render, &Limits::default(), Metrics::new())
}

/// [`run`] under the host's `limits`, counting in `metrics`.
fn run_under(
    program: &Program,
    events: &[OwnedJsonEvent],
    render: Option<Renderer>,
    limits: &Limits,
    metrics: Arc<Metrics>,
) -> Result<String, Fail> {
    let buffer = Shared::default();
    let mut sink = program.sink(Box::new(buffer.clone()), render, limits, metrics)?;
    replay(events, &mut sink)?;
    let bytes = buffer.0.lock().unwrap().clone();
    Ok(String::from_utf8(bytes).expect("utf-8 output"))
}

/// One document, both ways: the same bytes, or the same code.
fn differential(name: &str, program: &Program, events: &[OwnedJsonEvent]) -> Result<(), String> {
    let interpreted = program
        .with_native(false)
        .expect("the program compiles without the fast paths");
    assert!(program.native() && !interpreted.native());
    match (run(program, events, None), run(&interpreted, events, None)) {
        (Ok(a), Ok(b)) if a == b => Ok(()),
        (Ok(a), Ok(b)) => Err(format!(
            "{name}: the bytes differ\n  native:      {a:?}\n  interpreted: {b:?}"
        )),
        (Err(a), Err(b)) if a.code == b.code => Ok(()),
        (Err(a), Err(b)) => Err(format!(
            "{name}: the codes differ\n  native:      {a}\n  interpreted: {b}"
        )),
        (Ok(a), Err(b)) => Err(format!(
            "{name}: native produced {a:?}, interpreted failed: {b}"
        )),
        (Err(a), Ok(b)) => Err(format!(
            "{name}: interpreted produced {b:?}, native failed: {a}"
        )),
    }
}

/// A document to compare: its name, the grammar that reads it, its text.
type Document = (String, fn() -> Tabnas, String);

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../transduce/rs/tests/fixtures")
}

#[test]
fn the_library_loads() {
    let lib = tabnas_alchemy::stdlib::stdlib();
    assert!(lib.get("table-from-json").is_some());
    assert!(lib.get("csv").is_some());
}

/// Every transduce fixture a grammar here reads, plus the generated
/// documents in every shape: identical bytes or identical codes.
#[test]
fn interpreted_and_native_agree_on_every_fixture_and_generated_document() {
    let program = compile(PROGRAM, "export.alc").expect("the spec's program compiles");
    let mut documents: Vec<Document> = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("transduce's fixtures are beside this checkout")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let Some(make) = grammar_for(&path) else {
            continue;
        };
        let text = std::fs::read_to_string(&path).expect("a fixture reads");
        documents.push((path.display().to_string(), make, text));
    }
    for n in [0, 1, 2, 3, 50] {
        documents.push((
            format!("records_json({n})"),
            tabnas_json::make,
            support::records_json(n),
        ));
    }
    documents.push((
        "records_yaml(3)".into(),
        tabnas_yaml::make,
        support::records_yaml(3),
    ));
    documents.push((
        "records_jsonl(3)".into(),
        tabnas_jsonl::make,
        support::records_jsonl(3),
    ));
    documents.push((
        "records_csv(3)".into(),
        tabnas_csv::make,
        support::records_csv(3),
    ));
    // Cells of every kind, in the worked example's shape: null, missing
    // members (a failure under the standard options), nested containers,
    // booleans, quotes, delimiters, line breaks and non-ASCII text.
    let meta = support::METADATA;
    let shaped = |records: &str| {
        format!(
            r#"{{"response":{{"metadata":{meta},"payload":{{"deep":{{"records":[{records}]}}}}}}}}"#
        )
    };
    for (name, records) in [
        (
            "nulls",
            r#"{"id":null,"person":{"name":null},"account":{"balance":null}}"#,
        ),
        ("missing", r#"{"id":1,"person":{},"account":{"balance":2}}"#),
        (
            "containers",
            r#"{"id":[1,2.50],"person":{"name":{"first":"A","last":"B"}},"account":{"balance":{}}}"#,
        ),
        (
            "booleans",
            r#"{"id":true,"person":{"name":false},"account":{"balance":0}}"#,
        ),
        (
            "quotes",
            r#"{"id":"say \"hi\"","person":{"name":"a,b"},"account":{"balance":"line\r\nbreak"}}"#,
        ),
        (
            "unicode",
            r#"{"id":"caf\u00e9","person":{"name":"日本"},"account":{"balance":"\ud83d\ude00"}}"#,
        ),
        (
            "lexemes",
            r#"{"id":1e2,"person":{"name":"x"},"account":{"balance":-0.0}}"#,
        ),
        (
            "big",
            r#"{"id":12345678901234567890123,"person":{"name":"x"},"account":{"balance":1E+2}}"#,
        ),
        (
            "two rows out of order",
            r#"{"account":{"balance":1},"id":2,"person":{"name":"z"}},{"id":3,"person":{"name":"y"},"account":{"balance":4}}"#,
        ),
    ] {
        documents.push((name.into(), tabnas_json::make, shaped(records)));
    }
    // The order contract and the absent shapes.
    documents.push((
        "metadata after rows".into(),
        tabnas_json::make,
        format!(
            r#"{{"response":{{"payload":{{"deep":{{"records":[{}]}}}},"metadata":{meta}}}}}"#,
            support::record(1)
        ),
    ));
    documents.push((
        "no metadata".into(),
        tabnas_json::make,
        format!(
            r#"{{"response":{{"payload":{{"deep":{{"records":[{}]}}}}}}}}"#,
            support::record(1)
        ),
    ));
    documents.push((
        "no records".into(),
        tabnas_json::make,
        format!(r#"{{"response":{{"metadata":{meta}}}}}"#),
    ));
    documents.push((
        "metadata not an array".into(),
        tabnas_json::make,
        r#"{"response":{"metadata":{"fields":{}}}}"#.into(),
    ));
    documents.push((
        "descriptor without title".into(),
        tabnas_json::make,
        r#"{"response":{"metadata":{"fields":[{"path":["a"]}]}}}"#.into(),
    ));
    documents.push((
        "descriptor with a bad segment".into(),
        tabnas_json::make,
        r#"{"response":{"metadata":{"fields":[{"title":"a","path":[true]}]}}}"#.into(),
    ));
    documents.push((
        "row that is not an object".into(),
        tabnas_json::make,
        shaped("1"),
    ));

    let total = documents.len();
    let mut failures = Vec::new();
    let mut skipped = 0;
    for (i, (name, make, text)) in documents.iter().enumerate() {
        if i % 10 == 0 {
            println!("differential: {i} of {total} ({}%)", i * 100 / total);
        }
        let events = match events(*make, text) {
            Ok(events) => events,
            Err(_) => {
                // The grammar refused the document: nothing reached a
                // program, so there is nothing to compare.
                skipped += 1;
                continue;
            }
        };
        if let Err(report) = differential(name, &program, &events) {
            failures.push(report);
        }
    }
    println!("differential: {total} of {total} (100%), {skipped} unreadable");
    assert!(total - skipped > 20, "enough documents were read");
    assert!(
        failures.is_empty(),
        "{} document(s) differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The same, with the JSON renderer over the table both ways (`records`
/// then `json`), and with a program whose options record changes the
/// dialect the native renderer takes.
#[test]
fn interpreted_and_native_agree_on_other_renderings() {
    let records = PROGRAM.replace("    csv csv-options\n", "    records\n    json\n");
    let program = compile(&records, "records.alc").unwrap();
    let dialect = PROGRAM.replace(
        "    csv csv-options\n",
        "    csv options\n\ndef options\n  record\n    entry :delimiter \";\"\n    entry :newline \"\\n\"\n    entry :header false\n    entry :null-text \"NULL\"\n    entry :missing \"-\"\n",
    );
    let dialect = compile(&dialect, "dialect.alc").unwrap();
    let meta = support::METADATA;
    let docs = [
        support::records_json(3),
        format!(
            r#"{{"response":{{"metadata":{meta},"payload":{{"deep":{{"records":[{{"id":null,"person":{{}},"account":{{"balance":"a;b"}}}}]}}}}}}}}"#
        ),
    ];
    for (i, doc) in docs.iter().enumerate() {
        let events = events(tabnas_json::make, doc).unwrap();
        differential(&format!("records/json {i}"), &program, &events).unwrap();
        differential(&format!("dialect {i}"), &dialect, &events).unwrap();
    }
    let events = events(tabnas_json::make, &docs[1]).unwrap();
    assert_eq!(
        run(&dialect, &events, None).unwrap(),
        "\"NULL\";\"-\";\"a;b\"\n"
    );
}

/// The spec's worked example, byte for byte as aless's fixture has it.
const RECORDS: &str = r#"{"response":{"metadata":{"fields":[{"title":"Identifier","path":["id"]},{"title":"Full name","path":["person","name"]},{"title":"Balance","path":["account","balance"]}]},"payload":{"deep":{"records":[{"id":123,"person":{"name":"Alice"},"account":{"balance":50.25}},{"account":{"balance":72},"person":{"name":"Bob"},"id":456}]}}}}"#;

const EXPECTED_CSV: &str =
    "\"Identifier\",\"Full name\",\"Balance\"\r\n\"123\",\"Alice\",\"50.25\"\r\n\"456\",\"Bob\",\"72\"\r\n";

/// Both ways, one outcome: the same bytes, or the same code and the same
/// limit named; `expected` is that outcome.
fn agree(
    name: &str,
    program: &Program,
    events: &[OwnedJsonEvent],
    limits: &Limits,
    expected: Result<&str, Code>,
) {
    let interpreted = program.with_native(false).unwrap();
    let native_rows = Metrics::new();
    let interpreted_rows = Metrics::new();
    let a = run_under(program, events, None, limits, native_rows.clone());
    let b = run_under(&interpreted, events, None, limits, interpreted_rows.clone());
    match (&a, &b, expected) {
        (Ok(a), Ok(b), Ok(want)) => {
            assert_eq!(a, want, "{name}: native");
            assert_eq!(b, want, "{name}: interpreted");
            assert_eq!(
                Metrics::get(&native_rows.rows),
                Metrics::get(&interpreted_rows.rows),
                "{name}: rows counted"
            );
        }
        (Err(a), Err(b), Err(code)) => {
            assert_eq!(
                (a.code, b.code),
                (code, code),
                "{name}:\n  native: {a}\n  interpreted: {b}"
            );
            assert_eq!(
                a.limit.as_ref().map(|l| l.name),
                b.limit.as_ref().map(|l| l.name),
                "{name}: the limit named"
            );
        }
        _ => panic!("{name}: expected {expected:?}\n  native: {a:?}\n  interpreted: {b:?}"),
    }
}

/// The library's `csv` validates the table events it renders as the
/// native renderer does (spec 13.2: the protocol validator is not
/// omitted), so a program that produces its own table events, or a
/// document or an options record the renderer refuses, fails with the
/// same code both ways, and a sound one prints the same bytes.
#[test]
fn the_library_csv_validates_what_the_renderer_validates() {
    let items = events(
        tabnas_json::make,
        r#"{"items":[{"n":"a","v":1},{"n":"b\"q","v":-2},{"n":"c","v":null}],"tail":"t"}"#,
    )
    .unwrap();
    let limits = Limits::default();
    let label = |l: &str| format!("(record (entry :label \"{l}\"))");
    let table = |step: &str, finish: &str| {
        format!(
            "def export [input] (csv csv-options (scan-emit no-schema {step} {finish} (select (path \"items\" each-index) input)))"
        )
    };
    // One schema on the first item, a row per item: the sound shape.
    let first = |schema: &str, row: &str| {
        format!(
            "(fn [s x] (transition (ready []) (if (is-ready s) [(row {row})] [(schema {schema}) (row {row})])))"
        )
    };
    let n = label("N");
    let v = label("V");
    let cases: Vec<(&str, String, Result<&str, Code>)> = vec![
        (
            "sound",
            table(&first(&format!("[{n} {v}]"), "[(get :n x) (get :v x)]"), "(fn [s] [table-end])"),
            Ok("\"N\",\"V\"\r\n\"a\",\"1\"\r\n\"b\"\"q\",\"-2\"\r\n\"c\",\"\"\r\n"),
        ),
        (
            "rows without a schema",
            table("(fn [s x] (transition s [(row [(get :n x)])]))", "(fn [s] [table-end])"),
            Err(Code::ProtocolOrderError),
        ),
        (
            "a schema per item",
            table(&format!("(fn [s x] (transition s [(schema [{n}])]))"), "(fn [s] [table-end])"),
            Err(Code::ProtocolOrderError),
        ),
        (
            "a row wider than the schema",
            table(&first(&format!("[{n}]"), "[(get :n x) (get :v x)]"), "(fn [s] [table-end])"),
            Err(Code::ProtocolOrderError),
        ),
        (
            "no table-end",
            table(&first(&format!("[{n}]"), "[(get :n x)]"), "(fn [s] [])"),
            Err(Code::ProtocolOrderError),
        ),
        (
            "two table-ends",
            table(&first(&format!("[{n}]"), "[(get :n x)]"), "(fn [s] [table-end table-end])"),
            Err(Code::ProtocolOrderError),
        ),
        (
            "an item that is not a table event",
            "def export [input] (csv csv-options (map (fn [x] (get :n x)) (select (path \"items\" each-index) input)))".to_string(),
            Err(Code::ProtocolOrderError),
        ),
        (
            "no items at all",
            "def export [input] (csv csv-options (scan-emit no-schema (fn [s x] (transition s [x])) (fn [s] []) (select (path \"none\" each-index) input)))".to_string(),
            Err(Code::ProtocolOrderError),
        ),
        (
            "a schema of no columns",
            table(&first("[]", "[]"), "(fn [s] [table-end])"),
            Err(Code::TargetValueUnrepresentable),
        ),
        (
            "a column without a label",
            table(&first("[(record (entry :x \"N\"))]", "[(get :n x)]"), "(fn [s] [table-end])"),
            Err(Code::MissingValue),
        ),
        (
            "a label that is a record",
            table(&first("[(record (entry :label (record)))]", "[(get :n x)]"), "(fn [s] [table-end])"),
            Err(Code::InputInvalid),
        ),
    ];
    for (name, src, expected) in cases {
        let program = compile(&src, "table.alc").unwrap_or_else(|f| panic!("{name}: {f}"));
        agree(name, &program, &items, &limits, expected);
    }

    // The worked example over a document whose metadata is empty: a table
    // of no columns has no CSV form, whichever path renders it.
    let program = compile(PROGRAM, "export.alc").unwrap();
    let empty = events(
        tabnas_json::make,
        r#"{"response":{"metadata":{"fields":[]},"payload":{"deep":{"records":[{"id":1}]}}}}"#,
    )
    .unwrap();
    agree(
        "no columns",
        &program,
        &empty,
        &limits,
        Err(Code::TargetValueUnrepresentable),
    );

    // A delimiter no CSV reader could take is refused before anything
    // runs, natively (the renderer) and interpreted (`csv-table`).
    let records = events(tabnas_json::make, RECORDS).unwrap();
    for delimiter in ["\\\"", "\\n", "\\r"] {
        let src = PROGRAM.replace(
            "    csv csv-options\n",
            &format!("    csv (record (entry :delimiter \"{delimiter}\") (entry :newline \"\\r\\n\") (entry :header true) (entry :null-text \"\") (entry :missing :error))\n"),
        );
        let program = compile(&src, "delimiter.alc").unwrap();
        agree(
            &format!("delimiter {delimiter}"),
            &program,
            &records,
            &limits,
            Err(Code::TargetValueUnrepresentable),
        );
    }

    // A column function that answers something other than a record fails
    // as `get` does, both ways.
    let keyword = compile(
        "def b (record (entry :columns (path \"m\")) (entry :rows (path \"r\" each-index)) (entry :column (fn [d] :oops)))\ndef export [input] (csv csv-options (table-from-json b input))",
        "keyword.alc",
    )
    .unwrap();
    let doc = events(
        tabnas_json::make,
        r#"{"m":[{"title":"t","path":["a"]}],"r":[{"a":"x"}]}"#,
    )
    .unwrap();
    agree(
        "a column that is a keyword",
        &keyword,
        &doc,
        &limits,
        Err(Code::DslTypeError),
    );
}

/// The library's table holds the scopes it captures to the limits the
/// native table does (the metadata under `max_metadata_bytes`, each row
/// under `max_record_bytes`, at most `max_columns` columns), so under the
/// host's own limits both ways fail alike or print alike, and count the
/// same rows.
#[test]
fn the_limits_hold_alike_both_ways() {
    let program = compile(PROGRAM, "export.alc").unwrap();
    let records = events(tabnas_json::make, RECORDS).unwrap();
    let cases: Vec<(&str, Limits, Result<&str, Code>)> = vec![
        ("defaults", Limits::default(), Ok(EXPECTED_CSV)),
        (
            "max_record_bytes",
            Limits {
                max_record_bytes: 100,
                ..Limits::default()
            },
            Err(Code::ResourceLimitExceeded),
        ),
        (
            "max_metadata_bytes",
            Limits {
                max_metadata_bytes: 50,
                ..Limits::default()
            },
            Err(Code::ResourceLimitExceeded),
        ),
        (
            "max_columns",
            Limits {
                max_columns: 2,
                ..Limits::default()
            },
            Err(Code::ResourceLimitExceeded),
        ),
        (
            "max_capture_bytes decides neither table",
            Limits {
                max_capture_bytes: 100,
                ..Limits::default()
            },
            Ok(EXPECTED_CSV),
        ),
    ];
    for (name, limits, expected) in cases {
        agree(name, &program, &records, &limits, expected);
    }
    // A cell that is a vector is its compact JSON text, one scalar of the
    // output, held to max_scalar_bytes both ways.
    let containers = events(
        tabnas_json::make,
        &RECORDS.replace(r#""id":123"#, r#""id":[1,2,3,4,5,6]"#),
    )
    .unwrap();
    agree(
        "a container cell under max_scalar_bytes",
        &program,
        &containers,
        &Limits {
            max_scalar_bytes: 12,
            ..Limits::default()
        },
        Err(Code::ResourceLimitExceeded),
    );
    agree(
        "a container cell within it",
        &program,
        &containers,
        &Limits { max_scalar_bytes: 13, ..Limits::default() },
        Ok("\"Identifier\",\"Full name\",\"Balance\"\r\n\"[1,2,3,4,5,6]\",\"Alice\",\"50.25\"\r\n\"456\",\"Bob\",\"72\"\r\n"),
    );
    let metrics = Metrics::new();
    run_under(
        &program.with_native(false).unwrap(),
        &records,
        None,
        &Limits::default(),
        metrics.clone(),
    )
    .unwrap();
    assert_eq!(Metrics::get(&metrics.rows), 2);
}

/// A program over an inferred binding (`:columns :infer`): a root array
/// of rows, written as CSV with a missing cell as an empty field.
const INFERRED: &str = "def options
  record
    entry :delimiter \",\"
    entry :newline \"\\r\\n\"
    entry :header true
    entry :null-text \"\"
    entry :missing \"\"

def rows-binding
  record
    entry :columns :infer
    entry :rows (path each-index)

def export [input]
  pipe input
    table-from-json rows-binding
    csv options
";

/// The inferred binding: the columns are the first row's keys, in its
/// order, each reading its own key. Natively it is transduce's
/// `Schema::Infer`; the library's text infers them with `keys`. Both ways
/// write the same bytes, fail with the same code, and hold the same
/// limits, over every shape a first row and a later row can take.
#[test]
fn an_inferred_binding_agrees_both_ways() {
    let program = compile(INFERRED, "inferred.alc").expect("the inferred program compiles");
    let json = |text: &str| events(tabnas_json::make, text).unwrap();
    let ok = |name: &str, text: &str, want: &str| {
        agree(name, &program, &json(text), &Limits::default(), Ok(want))
    };
    let err = |name: &str, text: &str, code: Code| {
        agree(name, &program, &json(text), &Limits::default(), Err(code))
    };
    ok(
        "the first row's keys, in its order",
        r#"[{"b":1,"a":"x"},{"a":"y","b":2}]"#,
        "\"b\",\"a\"\r\n\"1\",\"x\"\r\n\"2\",\"y\"\r\n",
    );
    ok(
        "a key a later row lacks is a missing cell",
        r#"[{"a":1,"b":2},{"a":3}]"#,
        "\"a\",\"b\"\r\n\"1\",\"2\"\r\n\"3\",\"\"\r\n",
    );
    ok(
        "a key only a later row has is no column",
        r#"[{"a":1},{"c":3,"a":2}]"#,
        "\"a\"\r\n\"1\"\r\n\"2\"\r\n",
    );
    ok(
        "cells of every kind",
        r#"[{"s":"q\"x","n":1.5,"t":true,"z":null,"o":{"k":[1,2]}}]"#,
        "\"s\",\"n\",\"t\",\"z\",\"o\"\r\n\"q\"\"x\",\"1.5\",\"true\",\"\",\"{\"\"k\"\":[1,2]}\"\r\n",
    );
    ok(
        "a key that spells an index names a member",
        r#"[{"0":"zero","1":"one"}]"#,
        "\"0\",\"1\"\r\n\"zero\",\"one\"\r\n",
    );
    ok(
        "a later row that is not an object has missing cells",
        r#"[{"a":1},2]"#,
        "\"a\"\r\n\"1\"\r\n\"\"\r\n",
    );
    err(
        "a first row that is a scalar",
        r#"[1,{"a":1}]"#,
        Code::InputInvalid,
    );
    err(
        "a first row that is an array",
        r#"[[1],{"a":1}]"#,
        Code::InputInvalid,
    );
    // A table of no columns, from no rows or from an empty first row, is
    // one the CSV renderer refuses, as it refuses any.
    err("no rows", "[]", Code::TargetValueUnrepresentable);
    err(
        "an empty first row",
        r#"[{},{"a":1}]"#,
        Code::TargetValueUnrepresentable,
    );
    let three = json(r#"[{"a":1,"b":2,"c":3}]"#);
    agree(
        "max_columns holds the inferred columns",
        &program,
        &three,
        &Limits {
            max_columns: 2,
            ..Limits::default()
        },
        Err(Code::ResourceLimitExceeded),
    );
    agree(
        "max_record_bytes holds each row",
        &program,
        &three,
        &Limits {
            max_record_bytes: 8,
            ..Limits::default()
        },
        Err(Code::ResourceLimitExceeded),
    );
    // The worked example's documents, bound by inference from their rows.
    let api = INFERRED.replace(
        "(path each-index)",
        "(path \"response\" \"payload\" \"deep\" \"records\" each-index)",
    );
    let api = compile(&api, "inferred-api.alc").expect("the api-shaped program compiles");
    let mut failures = Vec::new();
    for n in [0, 1, 2, 3, 50] {
        let events = events(tabnas_json::make, &support::records_json(n)).unwrap();
        if let Err(report) = differential(&format!("records_json({n})"), &api, &events) {
            failures.push(report);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Each row counts once in `metrics.rows`, by the last table stage it
/// passes: the adapter to a renderer, a `csv-table` whose rows reach no
/// later table stage, or the native table when it hands its rows straight
/// to a renderer. A `map`, a `filter` or a `scan-emit` between two table
/// stages hands the count on, so a row a filter drops is never counted,
/// and rows that only ever become a text of items are not rows of any
/// table; natively and interpreted alike, with the same bytes.
#[test]
fn each_row_counts_once_across_composed_table_stages() {
    let tail = |t: &str| PROGRAM.replace("    csv csv-options\n", t);
    let identity = "    map (fn [e] e)\n";
    let drop_rows = "    filter (fn [e] (match e (case (row cells) false) (case _ true)))\n";
    let pass_scan = "    scan-emit null (fn [s e] (transition s [e])) (fn [s] []) \n";
    let as_text = "    map (fn [e] \"x\")\n    join \",\"\n";
    let cases: Vec<(&str, String, Option<Renderer>, u64)> = vec![
        ("csv", tail("    csv csv-options\n"), None, 2),
        (
            "csv-table as the result",
            tail("    csv-table csv-options\n"),
            None,
            2,
        ),
        (
            "csv-table as the result, as json",
            tail("    csv-table csv-options\n"),
            Some(Renderer::Json),
            2,
        ),
        (
            "csv over csv-table",
            tail("    csv-table csv-options\n    csv csv-options\n"),
            None,
            2,
        ),
        (
            "records over csv-table",
            tail("    csv-table csv-options\n    records\n    json\n"),
            None,
            2,
        ),
        (
            "csv-table twice",
            tail("    csv-table csv-options\n    csv-table csv-options\n    csv csv-options\n"),
            None,
            2,
        ),
        (
            "the library csv over the native table",
            tail("    csv opts\n\ndef opts\n  record\n    entry :delimiter \"||\"\n    entry :newline \"\\r\\n\"\n    entry :header true\n    entry :null-text \"\"\n    entry :missing :error\n"),
            None,
            2,
        ),
        (
            "a map, then the program's csv",
            tail(&format!("{identity}    csv csv-options\n")),
            None,
            2,
        ),
        (
            "a map, csv-table, csv",
            tail(&format!("{identity}    csv-table csv-options\n    csv csv-options\n")),
            None,
            2,
        ),
        (
            "csv-table, a map, csv",
            tail(&format!("    csv-table csv-options\n{identity}    csv csv-options\n")),
            None,
            2,
        ),
        (
            "a map, the host's renderer",
            tail(identity),
            Some(Renderer::Csv),
            2,
        ),
        (
            "a map, the host's json",
            tail(identity),
            Some(Renderer::Json),
            2,
        ),
        (
            "a filter that keeps every row, the host's renderer",
            tail("    filter (fn [e] true)\n"),
            Some(Renderer::Csv),
            2,
        ),
        (
            "a scan that passes each event on, then csv",
            tail(&format!("{pass_scan}    csv csv-options\n")),
            None,
            2,
        ),
        (
            "a filter that drops every row, then csv",
            tail(&format!("{drop_rows}    csv csv-options\n")),
            None,
            0,
        ),
        (
            "a filter that drops every row, the host's renderer",
            tail(drop_rows),
            Some(Renderer::Csv),
            0,
        ),
        (
            "the table's events as a text: no table stage",
            tail(as_text),
            None,
            0,
        ),
        (
            "csv-table's events as a text",
            tail(&format!("    csv-table csv-options\n{as_text}")),
            None,
            2,
        ),
    ];
    let records = events(tabnas_json::make, RECORDS).unwrap();
    for (name, src, render, rows) in cases {
        let program = compile(&src, "rows.alc").unwrap();
        let mut outputs = Vec::new();
        for native in [true, false] {
            let program = program.with_native(native).unwrap();
            let metrics = Metrics::new();
            let out = run_under(
                &program,
                &records,
                render,
                &Limits::default(),
                metrics.clone(),
            )
            .unwrap_or_else(|f| panic!("{name} (native: {native}): {f}"));
            assert_eq!(
                Metrics::get(&metrics.rows),
                rows,
                "{name} (native: {native})"
            );
            outputs.push(out);
        }
        assert_eq!(outputs[0], outputs[1], "{name}: the bytes differ");
    }
}

/// The two paths differ, knowingly, in one place the standard shapes
/// never reach; pinned so a change to either is seen.
#[test]
fn the_known_differences_are_pinned() {
    // Metadata selected twice: the native transducer calls it an order
    // violation (a table has one schema); the library text says `fail`,
    // which is INPUT_INVALID.
    let twice = PROGRAM.replace(
        "      path \"response\" \"metadata\" \"fields\"\n",
        "      path \"response\" \"metadata\" each-index\n",
    );
    let program = compile(&twice, "twice.alc").unwrap();
    let doc = format!(
        r#"{{"response":{{"metadata":[{},{}],"payload":{{"deep":{{"records":[{}]}}}}}}}}"#,
        r#"[{"title":"a","path":["id"]}]"#,
        r#"[{"title":"b","path":["id"]}]"#,
        support::record(1)
    );
    let twice = events(tabnas_json::make, &doc).unwrap();
    assert_eq!(
        run(&program, &twice, None).unwrap_err().code,
        Code::InputOrderViolation
    );
    assert_eq!(
        run(&program.with_native(false).unwrap(), &twice, None)
            .unwrap_err()
            .code,
        Code::InputInvalid
    );
    // A numeric title was a second difference; one label policy now
    // serves every table (a string as it is, a number by its lexeme, a
    // boolean by its name), so both ways render the lexeme.
    let program = compile(PROGRAM, "export.alc").unwrap();
    let doc = format!(
        r#"{{"response":{{"metadata":{{"fields":[{{"title":42,"path":["id"]}}]}},"payload":{{"deep":{{"records":[{}]}}}}}}}}"#,
        support::record(1)
    );
    let numeric = events(tabnas_json::make, &doc).unwrap();
    agree(
        "a numeric title",
        &program,
        &numeric,
        &Limits::default(),
        Ok("\"42\"\r\n\"1\"\r\n"),
    );
}

/// The events end with `End`, as every source promises; a recording that
/// does not is a source defect, and the sink says so rather than dropping
/// the output silently: nothing is flushed.
#[test]
fn a_stream_without_end_writes_nothing() {
    let program = compile(PROGRAM, "export.alc").unwrap();
    let mut events = events(tabnas_json::make, &support::records_json(2)).unwrap();
    assert_eq!(events.pop(), Some(OwnedJsonEvent::End));
    for native in [true, false] {
        let program = program.with_native(native).unwrap();
        let buffer = Shared::default();
        let mut sink = program
            .sink(
                Box::new(buffer.clone()),
                None,
                &Limits::default(),
                Metrics::new(),
            )
            .unwrap();
        replay(&events, &mut sink).unwrap();
        assert!(buffer.0.lock().unwrap().is_empty(), "native={native}");
        // And the End then completes it.
        sink.event(JsonEvent::End).unwrap();
        assert!(!buffer.0.lock().unwrap().is_empty(), "native={native}");
    }
}
