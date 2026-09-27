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
    let buffer = Shared::default();
    let mut sink = program.sink(
        Box::new(buffer.clone()),
        render,
        &Limits::default(),
        Metrics::new(),
    )?;
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
    let mut documents: Vec<(String, fn() -> Tabnas, String)> = Vec::new();
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

/// The two paths differ, knowingly, in two places the standard shapes
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
    // A numeric title: the native binding requires a string label, as
    // transduce's own `column_from_meta` does; the library text renders
    // the number's lexeme as the header cell.
    let program = compile(PROGRAM, "export.alc").unwrap();
    let doc = format!(
        r#"{{"response":{{"metadata":{{"fields":[{{"title":42,"path":["id"]}}]}},"payload":{{"deep":{{"records":[{}]}}}}}}}}"#,
        support::record(1)
    );
    let numeric = events(tabnas_json::make, &doc).unwrap();
    assert_eq!(
        run(&program, &numeric, None).unwrap_err().code,
        Code::InputInvalid
    );
    assert_eq!(
        run(&program.with_native(false).unwrap(), &numeric, None).unwrap(),
        "\"42\"\r\n\"1\"\r\n"
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
