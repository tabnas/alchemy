// `events`: the source's events as items a program reads one by one. The
// affine rule over `events`, which compiling decides, is here; the
// renderer of arbitrary nesting written over the events, and the plan
// report checked against a run, run on transduce's routers and render's
// renderers and are in tabnas-dmog's events_test.rs.

mod common;

use tabnas_alchemy::shared::{Code, Fail, Limits, Metrics};
use tabnas_alchemy::{Input, Output};

use common::{compile, compile_with};

/// `events` consumes the input, so a program that reads it twice, or
/// captures it in a function, is refused as any affine stream is.
#[test]
fn events_over_the_input_is_affine() {
    let twice = "def export [input]\n  concat\n    join \"\" (map (fn [e] \"a\") (events input))\n    join \"\" (map (fn [e] \"b\") (events input))\n";
    let fail = compile(twice, "twice.alc").unwrap_err();
    assert_eq!(fail.code, Code::StreamReused, "{fail}");
    assert!(fail.message.starts_with("reused: "), "{fail}");
    assert_eq!((fail.row, fail.column), (Some(4), Some(39)));
    let with_json =
        "def export [input] (concat (json input) (join \"\" (map (fn [e] \"\") (events input))))";
    let fail = compile(with_json, "json.alc").unwrap_err();
    assert_eq!(fail.code, Code::StreamReused, "{fail}");
    assert!(fail.message.starts_with("reused: "), "{fail}");
    let captured = "def export [input]\n  concat-map (fn [x] (join \"\" (map (fn [e] \"\") (events input)))) [1]\n";
    let fail = compile(captured, "captured.alc").unwrap_err();
    assert_eq!(fail.code, Code::StreamReused, "{fail}");
    assert!(fail.message.starts_with("captured: "), "{fail}");
    // The stream `events` yields is a stream of items: events, which
    // `json` takes back, but not table events for `csv`; a stream of
    // values is not events. As the output it is a rewritten tree, handed
    // on as JSON events.
    let events = compile("def export [input] (events input)", "ok.alc").unwrap();
    assert_eq!(events.output(), Output::JsonEvents);
    for (src, finer) in [
        (
            "def export [input] (json (select (path each-index) input))",
            "protocol_mismatch",
        ),
        (
            "def export [input] (csv csv-options (events input))",
            "protocol_mismatch",
        ),
        (
            "def export [input] (events (select (path each-index) input))",
            "protocol_mismatch",
        ),
    ] {
        let fail = compile(src, "bad.alc").unwrap_err();
        assert_eq!(fail.code, Code::DslTypeError, "{src}: {fail}");
        assert!(
            fail.message.starts_with(&format!("{finer}: ")),
            "{src}: {fail}"
        );
    }
}

/// `parser-events`: a host that declares a parser-events input hands the
/// program the parser's rule passes, which `parser-events` reads as items
/// and nothing else reads. The stream is affine as `events`'s is, and a
/// transducer's outputs are a rewritten tree, handed on as JSON events.
/// Running one needs transduce's parser source and render's renderers,
/// so it is in tabnas-dmog.
#[test]
fn parser_events_over_a_declared_input() {
    let transducer = "def step [s pass]\n  match pass\n    case (close rule) (transition s [(scalar (get :name rule))])\n    case _ (transition s [])\n\ndef export [input]\n  as-events (scan-emit [] step (fn [s] []) (parser-events input))\n";
    let program = compile_with(transducer, "t.alc", Input::ParserEvents).unwrap();
    assert_eq!(program.input(), Input::ParserEvents);
    assert_eq!(program.output(), Output::JsonEvents);
    assert!(program
        .explain()
        .contains("Protocol:              ParserEvents/1 → Stream<ParserEvent>"));
    // Compiled over the default input, JSON events, it reads the wrong
    // protocol.
    let fail = compile(transducer, "t.alc").unwrap_err();
    assert!(fail.message.starts_with("protocol_mismatch: "), "{fail}");
    assert_eq!(
        compile("def export [input] input", "j.alc")
            .unwrap()
            .input(),
        Input::JsonEvents
    );

    let twice = "def export [input]\n  concat\n    join \"\" (map (fn [p] \"a\") (parser-events input))\n    join \"\" (map (fn [p] \"b\") (parser-events input))\n";
    let fail = compile_with(twice, "twice.alc", Input::ParserEvents).unwrap_err();
    assert_eq!(fail.code, Code::StreamReused, "{fail}");
    let captured = "def export [input]\n  concat-map (fn [x] (join \"\" (map (fn [p] \"\") (parser-events input)))) [1]\n";
    let fail = compile_with(captured, "captured.alc", Input::ParserEvents).unwrap_err();
    assert!(fail.message.starts_with("captured: "), "{fail}");

    // The host drives the sink its declared input names, and is told so
    // before anything runs when it does not.
    let limits = Limits::default();
    let refused = |result: Result<(), Fail>| {
        let fail = result.unwrap_err();
        assert_eq!(fail.code, Code::DslTypeError, "{fail}");
        assert!(fail.message.starts_with("protocol_mismatch: "), "{fail}");
    };
    refused(
        program
            .sink(Box::new(Vec::new()), None, &limits, Metrics::new())
            .map(drop),
    );
    let json = compile("def export [input] input", "j.alc").unwrap();
    refused(
        json.parser_sink(Box::new(Vec::new()), None, &limits, Metrics::new())
            .map(drop),
    );

    // A host that takes a transducer's events into its own sink drives
    // `parser_sink_into`, which holds the program to JSON events: one
    // compiled over JSON events, or answering a text or a table, is
    // refused before any stage is built. Running one is tabnas-dmog's.
    let into = |program: &tabnas_alchemy::Program| {
        program
            .parser_sink_into(Box::new(Vec::new()), &limits, Metrics::new())
            .map(drop)
    };
    refused(into(&json));
    let text = compile_with(
        "def export [input] (join \"\" (map (fn [p] \"a\") (parser-events input)))",
        "text.alc",
        Input::ParserEvents,
    )
    .unwrap();
    assert_eq!(text.output(), Output::Text);
    let fail = into(&text).unwrap_err();
    assert!(
        fail.message
            .starts_with("protocol_mismatch: the program's result is a text"),
        "{fail}"
    );
    let rows = compile_with(
        "def step [s pass]\n  match pass\n    case (close rule) (transition s [(scalar (get :name rule))])\n    case _ (transition s [])\n\ndef export [input]\n  table-from-json (record (entry :columns :infer) (entry :rows (path each-index))) (as-events (scan-emit [] step (fn [s] []) (parser-events input)))\n",
        "rows.alc",
        Input::ParserEvents,
    )
    .unwrap();
    assert_eq!(rows.output(), Output::TableRows);
    let fail = into(&rows).unwrap_err();
    assert!(
        fail.message
            .starts_with("protocol_mismatch: the program's result is a table"),
        "{fail}"
    );
}
