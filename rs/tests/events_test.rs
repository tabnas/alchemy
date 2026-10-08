// `events`: the source's events as items a program reads one by one. The
// affine rule over `events`, which compiling decides, is here; the
// renderer of arbitrary nesting written over the events, and the plan
// report checked against a run, run on transduce's routers and render's
// renderers and are in tabnas-alchemy-cli's events_test.rs.

mod common;

use tabnas_alchemy::shared::Code;

use common::compile;

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
    // `json` takes back, but not table events for `csv` and not the
    // output; a stream of values is not events.
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
        ("def export [input] (events input)", "bad_output"),
    ] {
        let fail = compile(src, "bad.alc").unwrap_err();
        assert_eq!(fail.code, Code::DslTypeError, "{src}: {fail}");
        assert!(
            fail.message.starts_with(&format!("{finer}: ")),
            "{src}: {fail}"
        );
    }
}
