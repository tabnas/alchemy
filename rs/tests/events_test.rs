// `events`: the source's events as items a program reads one by one, and
// the operators a renderer over them needs (`push`, `pop`, `top`, `count`,
// `quoted`, `repeat`). The proof is a renderer of arbitrary nesting, which
// nothing before `events` could express: a tiny YAML-like block form
// written by a `scan-emit` whose state is the stack of open containers,
// one keyword marker each, asserted byte for byte; then the affine rule
// over `events`, and the plan report.

use std::sync::{Arc, Mutex};

use tabnas_alchemy::{compile, Output, Program};
use tabnas_transduce::{Code, Fail, Limits, Metrics, ParserSource, Prune, SourceMode};

/// A YAML-like block renderer over the events. The state is the stack of
/// open containers, one marker each: `:object-open` and `:array-open` for
/// a container with nothing written yet (its opening line is held until
/// its first child or its end, so an empty one writes as `{}` or `[]` on
/// the key's line), `:object` for an object waiting for a key, `:member`
/// for one whose key is written and waits for the value, `:array` for an
/// array waiting for an item. A mapping under a key goes on the next
/// lines two spaces in; a sequence item is `- `; a mapping in a sequence
/// has its first key on the dash's line; a sequence in a sequence starts
/// on the next line, two spaces in; a root scalar stands alone. Strings
/// and keys go through `quoted`; a number is told from a string by
/// `kind` and written by its lexeme.
const RENDER: &str = r#"def indent [ctx]
  repeat (count ctx) "  "

def replace-top [marker s]
  push marker (pop s)

; A value completed: an object waiting for its member's value waits for
; its next key.
def done [s]
  match (count s)
    case 0 s
    case _
      match (top s)
        case :member (replace-top :object s)
        case _ s

; What comes before a value whose parent is the top of ctx: a space on
; the key's line, or a dash, for a scalar or an empty container; a line
; break for a mapping or a sequence under a key; a dash and the first key
; for a mapping in a sequence; a dash alone for a sequence in a sequence.
def lead [kind ctx]
  match (count ctx)
    case 0 ""
    case _
      match (top ctx)
        case :member
          match kind
            case :mapping (concat "\n" (indent ctx))
            case :sequence "\n"
            case _ " "
        case :array
          match kind
            case :sequence (concat (indent (pop ctx)) "-\n")
            case _ (concat (indent (pop ctx)) "- ")

; A held array is opened as a sequence by its first item.
def opened [s]
  match (count s)
    case 0 s
    case _
      match (top s)
        case :array-open (replace-top :array s)
        case _ s

def opening [s]
  match (count s)
    case 0 ""
    case _
      match (top s)
        case :array-open (lead :sequence (pop s))
        case _ ""

def yaml-scalar [value]
  match (kind value)
    case :null "null"
    case :boolean
      match value
        case true "true"
        case false "false"
    case :number (scalar-text csv-options value)
    case _ (quoted value)

def step [s event]
  match event
    case object-start
      transition (push :object-open (opened s)) [(opening s)]
    case array-start
      transition (push :array-open (opened s)) [(opening s)]
    case (key name)
      match (top s)
        case :object-open
          transition (replace-top :member s) [(lead :mapping (pop s)) (quoted name) ":"]
        case :object
          transition (replace-top :member s) [(indent (pop s)) (quoted name) ":"]
    case (scalar value)
      let [s2 (opened s)]
        transition (done s2) [(opening s) (lead :scalar s2) (yaml-scalar value) "\n"]
    case object-end
      match (top s)
        case :object-open
          transition (done (pop s)) [(lead :scalar (pop s)) "{}\n"]
        case _
          transition (done (pop s)) []
    case array-end
      match (top s)
        case :array-open
          transition (done (pop s)) [(lead :scalar (pop s)) "[]\n"]
        case _
          transition (done (pop s)) []

def finish [s]
  match (count s)
    case 0 []
    case _ (fail "the events ended inside a container")

def export [input]
  join ""
    scan-emit [] step finish (events input)
"#;

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

/// Parse `text` with the json grammar incrementally and push its events
/// through the program's sink: the output, or the failure with what had
/// been written.
fn drive(program: &Program, text: &str, limits: &Limits) -> (Result<(), Fail>, String) {
    let metrics = Metrics::new();
    let buffer = Shared::default();
    let sink = match program.sink(Box::new(buffer.clone()), None, limits, metrics.clone()) {
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
    let bytes = buffer.0.lock().unwrap().clone();
    (
        outcome.map(|_| ()),
        String::from_utf8(bytes).expect("utf-8 output"),
    )
}

fn render(program: &Program, text: &str) -> String {
    let (outcome, out) = drive(program, text, &Limits::default());
    outcome.unwrap_or_else(|f| panic!("{text}: {f}"));
    out
}

/// The renderer writes a nested document, with an empty object and an
/// empty array, strings that need escapes, and a scalar root, byte for
/// byte as the block form has them.
#[test]
fn events_render_a_nested_document_as_a_yaml_like_block() {
    let program = compile(RENDER, "render.alc").unwrap();
    assert_eq!(program.output(), Output::Text);
    // Every event is needed, so nothing is pruned.
    assert!(program.row_selector().is_none());
    let nested =
        r#"{"a":"x","b":["y",null,{"c":"q\"x\n","d":{}}],"e":[],"f":{"g":[[]]},"n":1.5e3,"z":-0}"#;
    assert_eq!(
        render(&program, nested),
        "\"a\": \"x\"\n\
         \"b\":\n\
         \x20 - \"y\"\n\
         \x20 - null\n\
         \x20 - \"c\": \"q\\\"x\\n\"\n\
         \x20   \"d\": {}\n\
         \"e\": []\n\
         \"f\":\n\
         \x20 \"g\":\n\
         \x20   - []\n\
         \"n\": 1.5e3\n\
         \"z\": -0\n"
    );
    // A number is told from a string by `kind` and written by its lexeme;
    // a string that spells a number stays quoted.
    assert_eq!(render(&program, "42"), "42\n");
    assert_eq!(render(&program, r#"["42",42]"#), "- \"42\"\n- 42\n");
    // A scalar root stands alone; its escapes are JSON's, with DEL and
    // the C1 controls escaped too, and everything else as itself.
    assert_eq!(
        render(
            &program,
            r#""tab\there \u0001 \u007f \u0085 caf\u00e9 \\ \"""#
        ),
        "\"tab\\there \\u0001 \\u007f \\u0085 café \\\\ \\\"\"\n"
    );
    assert_eq!(render(&program, "true"), "true\n");
    assert_eq!(render(&program, "null"), "null\n");
    // Empty containers at the root, and a sequence in a sequence.
    assert_eq!(render(&program, "{}"), "{}\n");
    assert_eq!(render(&program, "[]"), "[]\n");
    assert_eq!(
        render(&program, r#"[["a"],[],{"k":[false]}]"#),
        "-\n\
         \x20 - \"a\"\n\
         - []\n\
         - \"k\":\n\
         \x20   - false\n"
    );
    // A key the source repeats is delivered as it arrives, twice: nothing
    // is mapped by key between events.
    assert_eq!(
        render(&program, r#"{"a":"1","a":"2"}"#),
        "\"a\": \"1\"\n\"a\": \"2\"\n"
    );
    // The stack is what the stage retains: a document nested deeper than
    // `max_depth` is refused by the source before the state could grow to
    // it, and the failure names the limit.
    let deep = format!("{}1{}", "[".repeat(40), "]".repeat(40));
    let shallow = Limits {
        max_depth: 16,
        ..Limits::default()
    };
    let (outcome, _) = drive(&program, &deep, &shallow);
    let fail = outcome.unwrap_err();
    assert_eq!(fail.code, Code::ResourceLimitExceeded, "{fail}");
    assert_eq!(fail.limit.as_ref().unwrap().name, "max_depth");
}

/// A program that writes each key on its own line, in the explicit form
/// past YAML's 1024-character limit on an implicit key (`length` and
/// `compare`), and each number in YAML's spellings, the non-finite ones
/// included (`number-class`), as a YAML render does.
const KEY_FORMS: &str = r#"def key-line [name]
  match (compare (length (quoted name)) 1024)
    case :greater ["? " (quoted name) "\n"]
    case _ [(quoted name) ":\n"]

def number-line [n]
  match (number-class n)
    case :finite [(scalar-text csv-options n) "\n"]
    case :infinity [".inf\n"]
    case :negative-infinity ["-.inf\n"]
    case :nan [".nan\n"]

def step [s event]
  match event
    case (key name) (transition s (key-line name))
    case (scalar v)
      match (kind v)
        case :number (transition s (number-line v))
        case _ (transition s [])
    case _ (transition s [])

def finish [s] []

def export [input]
  join ""
    scan-emit [] step finish (events input)
"#;

/// Past 1024 characters, quotes included, a key takes the explicit form;
/// at 1024 it is still implicit.
#[test]
fn a_key_past_the_implicit_limit_takes_the_explicit_form() {
    let program = compile(KEY_FORMS, "keys.alc").unwrap();
    let long = "k".repeat(1023);
    assert_eq!(
        render(&program, &format!(r#"{{"{}":1}}"#, &long[..1022])),
        format!("\"{}\":\n1\n", &long[..1022])
    );
    assert_eq!(
        render(&program, &format!(r#"{{"{long}":1}}"#)),
        format!("? \"{long}\"\n1\n")
    );
}

/// YAML's non-finite numbers arrive as numbers with no lexeme, and a
/// program writes them in YAML's spellings, where `scalar-text` would
/// refuse them as JSON and CSV must; a quoted `'.inf'` is a string.
#[test]
fn the_non_finite_numbers_are_written_in_yamls_spellings() {
    let program = compile(KEY_FORMS, "keys.alc").unwrap();
    let metrics = Metrics::new();
    let buffer = Shared::default();
    let sink = program
        .sink(Box::new(buffer.clone()), None, &Limits::default(), metrics)
        .unwrap();
    let (outcome, _) = ParserSource::new(
        tabnas_yaml::make(),
        "- .inf\n- -.inf\n- .nan\n- 1.5\n- '.inf'\n",
    )
    .run_owned(sink);
    outcome.unwrap();
    let out = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    assert_eq!(out, ".inf\n-.inf\n.nan\n1.5\n");
}

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
    // The stream `events` yields is a stream of items: it is not JSON
    // events for `json`, not table events for `csv`, and not the output.
    for (src, finer) in [
        (
            "def export [input] (json (events input))",
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

/// The report names the stage: every event delivered as an item, nothing
/// retained by it, and the `scan-emit` after it conditional on its state.
#[test]
fn explain_reports_an_events_pipeline() {
    let program = compile(RENDER, "render.alc").unwrap();
    let text = program.explain();
    assert!(
        text.starts_with("export: events → scan-emit → join\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "Protocol:              JsonEvents/1 → Stream<Event> → Stream<Value> → Text\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("Selection:             none; every event is delivered as an item\n"),
        "{text}"
    );
    assert!(
        text.contains("Duplicate members:     preserved; the events are copied as they arrive, not mapped by key\n"),
        "{text}"
    );
    assert!(
        text.contains("Retained state:        what the step returns, no deeper than max_depth, capped at max_metadata_bytes\n"),
        "{text}"
    );
    assert!(
        text.contains("Output order:          source order\n"),
        "{text}"
    );
    let j = program.explain_json();
    assert_eq!(
        j["chain"],
        serde_json::json!(["events", "scan-emit", "join"])
    );
    assert_eq!(
        j["protocol"],
        serde_json::json!(["JsonEvents/1", "Stream<Event>", "Stream<Value>", "Text"])
    );
    assert_eq!(j["confidence"], "conditional");
    assert_eq!(j["readiness"], "event");
    assert_eq!(j["retention"].as_array().unwrap().len(), 1);
    assert_eq!(j["retention"][0]["scope"], "state");
    assert_eq!(j["renderer"]["name"], "text");
    assert_eq!(j["output"], "Text");
    // A map over the events alone retains nothing at all.
    let keys = compile(
        "def export [input]\n  join \"\"\n    map\n      fn [event]\n        match event\n          case (key name) (concat (quoted name) \"\\n\")\n          case _ \"\"\n      events input\n",
        "keys.alc",
    )
    .unwrap();
    let j = keys.explain_json();
    assert_eq!(j["confidence"], "proven");
    assert_eq!(j["retention"], serde_json::json!([]));
    assert_eq!(
        render(&keys, r#"{"a":1,"b":{"c":[2,{"d":3}]},"a":4}"#),
        "\"a\"\n\"b\"\n\"c\"\n\"d\"\n\"a\"\n"
    );
}
