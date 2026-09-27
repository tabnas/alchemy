// Shared test helpers. Cargo compiles this module into EVERY integration
// test binary, so an item only one binary uses is dead code in the
// others; the allow keeps that from being a warning rather than hiding
// anything real.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use tabnas_support::{find_spec_dir, Failure, Value};
use tabnas_transduce::Fail;

/// The repository root: the parent of `rs/`.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rs/ has a parent")
        .to_path_buf()
}

/// The shared `test/spec` directory, found by walking up from the crate
/// rather than by counting `..` hops.
pub fn spec_dir() -> PathBuf {
    find_spec_dir(Some(Path::new(env!("CARGO_MANIFEST_DIR"))))
        .expect("a test/spec directory above rs/")
}

/// The code a `Fail` from this crate pins: the reader and the desugarer
/// both lead the message with it (`bad_dedent: ...`, `empty_step: ...`),
/// so `ERROR:<code>` in a fixture names that word. A message without the
/// convention pins nothing but the transduce code.
pub fn fail_code(fail: &Fail) -> String {
    fail.message.split_once(": ").map_or_else(
        || fail.code.as_str().to_string(),
        |(code, _)| code.to_string(),
    )
}

/// A failure as the shared runner sees it: the code, the position when
/// there is one, and the whole failure for the report.
pub fn to_failure(fail: &Fail) -> Failure {
    let mut failure = Failure::new(fail_code(fail)).with_message(fail.to_string());
    if let (Some(row), Some(col)) = (fail.row, fail.column) {
        failure = failure.at(row as usize, col as usize);
    }
    failure
}

/// Canonical text as the fixture data model: the expected column is a
/// JSON string of it.
pub fn text_value(text: String) -> Value {
    Value::String(text)
}
