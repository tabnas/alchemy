// Shared test helpers. Cargo compiles this module into EVERY integration
// test binary, so an item only one binary uses is dead code in the
// others; the allow keeps that from being a warning rather than hiding
// anything real.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tabnas_alchemy::shared::{
    AbortFlag, CaptureSpec, CsvOptions, Duplicates, Fail, JoinOut, JsonOptions, Limits, Metrics,
    Renderers, RouteSink, Routers, ScanEmitter, ScanFinish, ScanOut, ScanStep, Sink, TableBinding,
    TableSink, TextOut,
};
use tabnas_alchemy::value::Val;
use tabnas_alchemy::{Program, Source};
use tabnas_support::{find_spec_dir, Failure, Value};

/// The routers and renderers of a compile-only test: compiling builds the
/// plan and keeps the two for the sinks, and asks nothing of them. The
/// real ones are tabnas-transduce's and tabnas-render's, which this crate
/// does not depend on: the tests that lower and run a program are in
/// tabnas-alchemy-cli, the composition root that depends on all three.
/// Every method panics.
pub struct NoStages;

const NO_STAGES: &str = "compile-only test: no stages";

impl Routers<Val> for NoStages {
    fn router(
        &self,
        _: Vec<CaptureSpec>,
        _: &Limits,
        _: Duplicates,
        _: Arc<Metrics>,
        _: Box<dyn RouteSink + Send>,
    ) -> Result<Box<dyn Sink + Send>, Fail> {
        panic!("{NO_STAGES}")
    }

    fn table_from_json(
        &self,
        _: TableBinding,
        _: &Limits,
        _: Duplicates,
        _: Arc<Metrics>,
        _: Box<dyn TableSink + Send>,
    ) -> Result<Box<dyn Sink + Send>, Fail> {
        panic!("{NO_STAGES}")
    }

    fn scan_emit(
        &self,
        _: Val,
        _: ScanStep<Val>,
        _: ScanFinish<Val>,
        _: ScanOut<Val>,
    ) -> Box<dyn ScanEmitter<Val>> {
        panic!("{NO_STAGES}")
    }

    fn guarded(
        &self,
        _: Box<dyn Sink + Send>,
        _: &Limits,
        _: AbortFlag,
        _: Arc<Metrics>,
    ) -> Box<dyn Sink + Send> {
        panic!("{NO_STAGES}")
    }
}

impl Renderers for NoStages {
    fn json(&self, _: Box<dyn TextOut + Send>, _: JsonOptions) -> Box<dyn Sink + Send> {
        panic!("{NO_STAGES}")
    }

    fn csv(
        &self,
        _: Box<dyn TextOut + Send>,
        _: CsvOptions,
    ) -> Result<Box<dyn TableSink + Send>, Fail> {
        panic!("{NO_STAGES}")
    }

    fn records_to_json(&self, _: Box<dyn Sink + Send>) -> Box<dyn TableSink + Send> {
        panic!("{NO_STAGES}")
    }

    fn join<'a>(&self, _: Box<dyn TextOut + Send + 'a>, _: &str) -> Box<dyn JoinOut + Send + 'a> {
        panic!("{NO_STAGES}")
    }

    fn replace_text<'a>(
        &self,
        _: Box<dyn TextOut + Send + 'a>,
        _: &str,
        _: &str,
    ) -> Box<dyn TextOut + Send + 'a> {
        panic!("{NO_STAGES}")
    }

    fn write_out(
        &self,
        _: Box<dyn Write + Send>,
        _: &Limits,
        _: Arc<Metrics>,
    ) -> Box<dyn TextOut + Send> {
        panic!("{NO_STAGES}")
    }
}

/// [`tabnas_alchemy::compile`] with [`NoStages`] routers and renderers,
/// for a test that reads what compiling decided and runs nothing.
pub fn compile(src: &str, file: &str) -> Result<Program, Fail> {
    tabnas_alchemy::compile(src, file, Arc::new(NoStages), Arc::new(NoStages))
}

/// [`tabnas_alchemy::compile_sources`] with [`NoStages`] routers and
/// renderers.
pub fn compile_sources(sources: &[Source<'_>]) -> Result<Program, Fail> {
    tabnas_alchemy::compile_sources(sources, Arc::new(NoStages), Arc::new(NoStages))
}

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
