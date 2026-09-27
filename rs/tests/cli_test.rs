// The `alchemy` binary, run the way a script runs it: the three commands,
// standard input as `-`, the statuses, and that nothing but the answer
// reaches standard output.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alchemy"))
}

fn run(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(binary())
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the alchemy binary starts");
    if let Some(text) = stdin {
        // A command refused before standard input is read may have exited
        // already, closing the pipe: that is the behaviour under test, not
        // a failure to write.
        if let Err(error) = child
            .stdin
            .take()
            .expect("a piped stdin")
            .write_all(text.as_bytes())
        {
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
        }
    }
    child.wait_with_output().expect("the binary finishes")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf-8 output")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf-8 output")
}

fn program_file(name: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("alchemy-cli-{}-{name}", std::process::id()));
    std::fs::write(&path, text).expect("the program file is written");
    path
}

const EXPORT: &str =
    "def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options\n";

#[test]
fn canon_prints_the_canonical_form() {
    let file = program_file("canon.alc", EXPORT);
    let output = run(&["canon", file.to_str().expect("a utf-8 path")], None);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "(def export [input] (pipe input (table-from-json api-binding) (csv csv-options)))\n"
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn format_prints_the_layout_form_and_reads_standard_input() {
    let output = run(
        &["format", "-"],
        Some("(def export [input] (pipe input (table-from-json api-binding) (csv csv-options)))"),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), EXPORT);
}

#[test]
fn check_is_silent_on_success() {
    let output = run(&["check", "-"], Some(EXPORT));
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

#[test]
fn check_reports_a_failure_as_json_on_stderr_with_status_2() {
    let output = run(&["check", "-"], Some("pipe x\n  f\n  []\n"));
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    let fail: serde_json::Value =
        serde_json::from_str(stderr(&output).trim()).expect("one JSON object on stderr");
    assert_eq!(fail["code"], "DSL_PARSE_ERROR");
    assert!(
        fail["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("empty_step: ")),
        "{fail}"
    );
    assert_eq!(fail["row"], 3);
    assert_eq!(fail["col"], 3);
    assert_eq!(fail["output"], "none");
}

#[test]
fn a_reader_error_carries_the_engine_code_and_position() {
    let output = run(&["canon", "-"], Some("a\n  b\n c\n"));
    assert_eq!(output.status.code(), Some(2));
    let fail: serde_json::Value =
        serde_json::from_str(stderr(&output).trim()).expect("one JSON object on stderr");
    assert_eq!(fail["code"], "DSL_PARSE_ERROR");
    assert!(
        fail["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("bad_dedent: ")),
        "{fail}"
    );
    assert_eq!(
        (fail["row"].as_u64(), fail["col"].as_u64()),
        (Some(3), Some(2))
    );
}

/// The reader bounds nesting, so a program nested far past the bound is a
/// `too_deep` failure with status 2 from every command, where it used to
/// take the process down with a stack overflow (status -6, nothing on
/// standard error but the runtime's abort).
#[test]
fn a_program_nested_beyond_the_bound_is_a_failure_not_an_abort() {
    let depth = 100_000;
    let parens = format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
    let mut indented = String::new();
    for level in 0..600 {
        indented.push_str(&"  ".repeat(level));
        indented.push_str("x\n");
    }
    // Flat to the reader, one level per step to the desugarer.
    let pipe = format!("pipe x{}\n", " f".repeat(depth));
    // The failure names the opener, the line or the form that passed the
    // bound: the 256th paren, the line that would open the 256th level,
    // the pipe.
    for (command, program, row) in [
        ("canon", &parens, 1),
        ("format", &parens, 1),
        ("check", &parens, 1),
        ("canon", &indented, 257),
        ("check", &pipe, 1),
    ] {
        let output = run(&[command, "-"], Some(program));
        assert_eq!(
            output.status.code(),
            Some(2),
            "{command}: {}",
            stderr(&output)
        );
        assert_eq!(stdout(&output), "", "{command}");
        let fail: serde_json::Value =
            serde_json::from_str(stderr(&output).trim()).unwrap_or_else(|_| {
                panic!("{command}: one JSON object on stderr: {}", stderr(&output))
            });
        assert_eq!(fail["code"], "DSL_PARSE_ERROR", "{command}");
        assert!(
            fail["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("too_deep: ")),
            "{command}: {fail}"
        );
        assert_eq!(fail["row"], row, "{command}: {fail}");
    }
}

#[test]
fn usage_errors_and_unreadable_files_exit_2() {
    let output = run(&[], None);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).starts_with("usage:"));
    assert_eq!(stdout(&output), "");

    let output = run(&["explain", "-"], Some("x"));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("INPUT_INVALID"),
        "{}",
        stderr(&output)
    );

    // An unknown command is refused before standard input is read, so
    // what it holds does not matter: unparsable input is not a parse
    // error here.
    let output = run(&["bogus", "-"], Some("("));
    assert_eq!(output.status.code(), Some(2));
    let fail: serde_json::Value =
        serde_json::from_str(stderr(&output).trim()).expect("one JSON object on stderr");
    assert_eq!(fail["code"], "INPUT_INVALID");
    assert_eq!(stdout(&output), "");

    let output = run(&["canon", "/nonexistent/program.alc"], None);
    assert_eq!(output.status.code(), Some(2));
    let fail: serde_json::Value =
        serde_json::from_str(stderr(&output).trim()).expect("one JSON object on stderr");
    assert_eq!(fail["code"], "INPUT_INVALID");
    assert_eq!(stdout(&output), "");
}
