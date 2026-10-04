// The tests that lower a program and run it through transduce's routers
// and render's renderers, moved here from the unit tests of src/lower.rs,
// src/program.rs and src/effects.rs: a unit test in src/ cannot take the
// real routers and renderers, which are built on this crate's library, of
// which a unit test is a second copy. Each module is named for the source
// file its tests were in.

mod common;

use std::sync::{Arc, Mutex};

use tabnas_alchemy::ast::Sources;
use tabnas_alchemy::interp::Runtime;
use tabnas_alchemy::lower::{Lowering, Out, Renderer};
use tabnas_alchemy::resolve::resolve;
use tabnas_alchemy::shared::{Fail, Limits, Metrics};
use tabnas_alchemy::{desugar, parse_file, stdlib};
use tabnas_transduce::{ParserSource, Prune, SourceMode};

use common::{compile, renderers, routers};

/// The spec's worked example, byte for byte as aless's fixture has it.
const RECORDS: &str = r#"{"response":{"metadata":{"fields":[{"title":"Identifier","path":["id"]},{"title":"Full name","path":["person","name"]},{"title":"Balance","path":["account","balance"]}]},"payload":{"deep":{"records":[{"id":123,"person":{"name":"Alice"},"account":{"balance":50.25}},{"account":{"balance":72},"person":{"name":"Bob"},"id":456}]}}}}"#;

const EXPECTED_CSV: &str =
    "\"Identifier\",\"Full name\",\"Balance\"\r\n\"123\",\"Alice\",\"50.25\"\r\n\"456\",\"Bob\",\"72\"\r\n";

/// The spec's program: sections 12.1 and 13.4.
const PROGRAM: &str = "def column-from-meta [source]\n  record\n    entry :label (get \"title\" source)\n    entry :source\n      as-path\n        get \"path\" source\n\ndef api-binding\n  record\n    entry :columns\n      path \"response\" \"metadata\" \"fields\"\n    entry :rows\n      path \"response\" \"payload\" \"deep\" \"records\" each-index\n    entry :column column-from-meta\n\ndef api-table [input]\n  table-from-json api-binding input\n\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n";

fn runtime(src: &str, native: bool) -> Arc<Runtime> {
    let forms = desugar::program(parse_file(src, "t.alc").unwrap(), src).unwrap();
    let sources = Sources::one("t.alc", src);
    let resolved = resolve(forms, &sources, &stdlib::outer).unwrap();
    Arc::new(Runtime::new(Arc::new(resolved), sources).with_native(native))
}

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

/// Run `src` over the JSON `input` through the json grammar's
/// incremental events, lowered through transduce's routers and render's
/// renderers; the output bytes, or the failure.
fn run(src: &str, input: &str, native: bool, render: Option<Renderer>) -> Result<String, Fail> {
    let rt = runtime(src, native);
    let result = rt.export()?;
    let limits = Limits::default();
    let metrics = Metrics::new();
    let buffer = Shared::default();
    let out: Out = Box::new(tabnas_render::WriteOut::new(buffer.clone()));
    let sink = Lowering::new(rt, &limits, metrics.clone(), routers(), renderers())
        .sink(&result, out, render)?;
    let (outcome, _) = ParserSource::new(tabnas_json::make(), input)
        .grammar("json")
        .mode(SourceMode::Incremental {
            prune: Prune::Never,
        })
        .limits(limits)
        .metrics(metrics)
        .run_owned(sink);
    outcome?;
    let bytes = buffer.0.lock().unwrap().clone();
    Ok(String::from_utf8(bytes).expect("utf-8 output"))
}

mod lower {
    use super::*;
    use tabnas_alchemy::shared::Code;

    #[test]
    fn the_worked_example_through_the_interpreted_library() {
        assert_eq!(run(PROGRAM, RECORDS, false, None).unwrap(), EXPECTED_CSV);
    }

    #[test]
    fn the_worked_example_through_the_native_path() {
        assert_eq!(run(PROGRAM, RECORDS, true, None).unwrap(), EXPECTED_CSV);
    }

    #[test]
    fn json_echoes_the_input_with_its_lexemes() {
        let echo = "def export [input] (json input)";
        assert_eq!(
            run(echo, RECORDS, true, None).unwrap(),
            format!("{RECORDS}\n")
        );
        assert_eq!(
            run(echo, "[1.50, 1e2]", false, None).unwrap(),
            "[1.50,1e2]\n"
        );
    }

    #[test]
    fn select_map_and_concat_map_stream_items() {
        let src = "def export [input]\n  pipe input\n    select (path \"a\" each-index)\n    map (fn [x] (get :n x))\n    concat-map (fn [n] (concat (scalar-text csv-options n) \";\"))";
        assert_eq!(
            run(src, r#"{"a":[{"n":1},{"n":2.50}],"b":3}"#, false, None).unwrap(),
            "1;2.50;"
        );
        let joined = "def export [input]\n  join \",\"\n    map (fn [x] (get :n x)) (select (path \"a\" each-index) input)";
        assert_eq!(
            run(
                joined,
                r#"{"a":[{"n":"x"},{"n":""},{"n":"y"}]}"#,
                false,
                None
            )
            .unwrap(),
            "x,,y"
        );
        let framed = "def export [input]\n  concat\n    \"[\"\n    join \",\" (select (path each-index) input)\n    \"]\"\n    (text \"!\")";
        assert_eq!(run(framed, r#"["a","b"]"#, false, None).unwrap(), "[a,b]!");
        assert_eq!(run(framed, "[]", false, None).unwrap(), "[]!");
        let filtered = "def export [input]\n  join \"|\"\n    filter (fn [x] (get :keep x)) (select (path each-index) input)";
        assert_eq!(
            run(
                "def export [input]\n  concat-map (fn [x] (get :v x))\n    filter (fn [x] (get :keep x)) (select (path each-index) input)",
                r#"[{"keep":true,"v":"a"},{"keep":false,"v":"b"},{"keep":true,"v":"c"}]"#,
                false,
                None
            )
            .unwrap(),
            "ac"
        );
        let _ = filtered;
    }

    #[test]
    fn replace_text_over_a_live_text_crosses_fragments() {
        let src = "def export [input]\n  replace-text \"ab\" \"X\"\n    concat-map (fn [s] s) (select (path each-index) input)";
        assert_eq!(
            run(src, r#"["a","b","zab","a"]"#, false, None).unwrap(),
            "XzXa"
        );
    }

    #[test]
    fn a_finite_text_result_is_written_at_the_end() {
        assert_eq!(
            run("def export [input] \"done\"", "1", false, None).unwrap(),
            "done"
        );
    }

    #[test]
    fn a_stream_result_is_rendered_by_the_host() {
        let table = "def export [input] (table-from-json api-binding input)\n".to_string()
            + &PROGRAM.replace(
                "def export [input]\n  pipe input\n    api-table\n    csv csv-options\n",
                "",
            );
        assert_eq!(run(&table, RECORDS, true, None).unwrap(), EXPECTED_CSV);
        assert_eq!(
            run(&table, RECORDS, false, Some(Renderer::Csv)).unwrap(),
            EXPECTED_CSV
        );
        let json = r#"[{"Identifier":123,"Full name":"Alice","Balance":50.25},{"Identifier":456,"Full name":"Bob","Balance":72}]"#;
        assert_eq!(
            run(&table, RECORDS, true, Some(Renderer::Json)).unwrap(),
            format!("{json}\n")
        );
        assert_eq!(
            run(&table, RECORDS, false, Some(Renderer::Json)).unwrap(),
            format!("{json}\n")
        );
        let echo = "def export [input] input";
        assert_eq!(run(echo, "[1]", true, None).unwrap(), "[1]\n");
        let f = run(echo, "[1]", true, Some(Renderer::Csv)).unwrap_err();
        assert!(f.message.starts_with("protocol_mismatch: "), "{f}");
        let f = run(
            "def export [input] (json input)",
            "1",
            true,
            Some(Renderer::Json),
        )
        .unwrap_err();
        assert!(f.message.starts_with("render_of_text: "), "{f}");
    }

    #[test]
    fn records_of_a_table_round_trips_through_json() {
        let src = PROGRAM.replace("    csv csv-options\n", "    records\n    json\n");
        let json = r#"[{"Identifier":123,"Full name":"Alice","Balance":50.25},{"Identifier":456,"Full name":"Bob","Balance":72}]"#;
        assert_eq!(run(&src, RECORDS, true, None).unwrap(), format!("{json}\n"));
        assert_eq!(
            run(&src, RECORDS, false, None).unwrap(),
            format!("{json}\n")
        );
    }

    #[test]
    fn protocol_mismatches_are_named() {
        let f = run(
            "def export [input] (csv csv-options input)",
            "1",
            true,
            None,
        )
        .unwrap_err();
        assert_eq!(f.code, Code::DslTypeError);
        assert!(f.message.starts_with("protocol_mismatch: "), "{f}");
        let f = run(
            "def export [input] (json (select (path each-index) input))",
            "[1]",
            true,
            None,
        )
        .unwrap_err();
        assert!(f.message.starts_with("protocol_mismatch: "), "{f}");
        let f = run(
            "def export [input] (concat-map (fn [x] x) input)",
            "[1]",
            true,
            None,
        )
        .unwrap_err();
        assert!(f.message.starts_with("protocol_mismatch: "), "{f}");
    }
}

mod program {
    use super::*;
    use tabnas_alchemy::shared::Sink;

    #[test]
    fn the_sink_writes_through_a_write() {
        let p = compile(PROGRAM, "t.alc").unwrap();
        let buffer = Shared::default();
        let limits = Limits::default();
        let mut sink = p
            .sink(Box::new(buffer.clone()), None, &limits, Metrics::new())
            .unwrap();
        let datum = tabnas_transduce::Datum::from_json(&serde_json::from_str(RECORDS).unwrap());
        tabnas_transduce::walk_datum(&datum, &mut sink).unwrap();
        sink.event(tabnas_transduce::JsonEvent::End).unwrap();
        let bytes = buffer.0.lock().unwrap().clone();
        assert_eq!(String::from_utf8(bytes).unwrap(), EXPECTED_CSV);
    }
}

mod effects {
    use super::*;
    use serde_json::Value as Json;
    use tabnas_alchemy::effects::explain_json;

    /// The CSV renderer is reported with the dialect it is built with: the
    /// program's own options when its `csv` runs natively, the defaults when
    /// the host renders a table; and what it reports is what it writes.
    #[test]
    fn a_custom_csv_dialect_is_reported_as_it_runs() {
        let lf = PROGRAM.replace("    csv csv-options\n", "    csv lf\n")
            + "\ndef lf (record (entry :delimiter \";\") (entry :newline \"\\n\") (entry :header false) (entry :null-text \"NULL\") (entry :missing \"-\"))\n";
        let program = compile(&lf, "lf.alc").unwrap();
        assert!(program.native());
        let r = &explain_json(&program)["renderer"];
        assert_eq!(r["name"], "csv");
        assert_eq!(r["host"], false);
        assert_eq!(r["newline"], "\n");
        assert_eq!(r["header"], false);
        assert_eq!(r["delimiter"], ";");
        assert_eq!(r["null_text"], "NULL");
        assert_eq!(r["missing"], "text");
        assert_eq!(r["missing_text"], "-");
        assert_eq!(
            run(&lf, RECORDS, true, None).unwrap(),
            "\"123\";\"Alice\";\"50.25\"\n\"456\";\"Bob\";\"72\"\n"
        );
        // The null and missing texts reach the output as reported: a
        // record with a null name and no balance.
        let sparse = RECORDS.replace(
            r#"{"account":{"balance":72},"person":{"name":"Bob"},"id":456}"#,
            r#"{"person":{"name":null},"id":456}"#,
        );
        assert_ne!(sparse, RECORDS);
        assert_eq!(
            run(&lf, &sparse, true, None).unwrap(),
            "\"123\";\"Alice\";\"50.25\"\n\"456\";\"NULL\";\"-\"\n"
        );
        // The default dialect, and the host's renderer, report the defaults.
        let default = compile(PROGRAM, "export.alc").unwrap();
        let host = compile(&PROGRAM.replace("    csv csv-options\n", ""), "host.alc").unwrap();
        for (name, program, is_host) in [("default", default, false), ("host", host, true)] {
            let r = &explain_json(&program)["renderer"];
            assert_eq!(r["name"], "csv", "{name}");
            assert_eq!(r["host"], is_host, "{name}");
            assert_eq!(r["delimiter"], ",", "{name}");
            assert_eq!(r["newline"], "\r\n", "{name}");
            assert_eq!(r["header"], true, "{name}");
            assert_eq!(r["null_text"], "", "{name}");
            assert_eq!(r["missing"], "error", "{name}");
            assert_eq!(r["missing_text"], Json::Null, "{name}");
        }
    }
}
