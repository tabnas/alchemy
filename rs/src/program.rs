//! The public API a host embeds: [`compile`] a program, ask it what it
//! produces, and take a [`Sink`] to push the source's events into.
//!
//! aless runs a pipeline on its parse thread, so everything here is
//! `Send`: the program, the sink it builds, the values inside. The host
//! pushes `JsonEvent`s into the sink and one `End`; the sink writes the
//! output as it goes and flushes it at `End`. A failure comes back from
//! the event call that found it, as a transduce `Fail` whose
//! `committed_output` says whether bytes had already reached the writer.

use std::io::Write;
use std::sync::Arc;

use tabnas_render::{TextOut, WriteOut};
use tabnas_transduce::{Duplicates, Fail, Limits, Metrics, Selector, Sink};

use crate::interp::Runtime;
use crate::lower::{EventSink, Lowering, Out};
use crate::resolve::{resolve, Resolved};
use crate::value::{Plan, Protocol, Seq, Val};
use crate::{desugar, parse_file, stdlib};

pub use crate::lower::Renderer;

/// What a program's `export` produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// The program renders its own text.
    Text,
    /// `TableRows/1`: the host renders it (CSV by default, or JSON records).
    TableRows,
    /// `JsonEvents/1`: the host renders it as JSON.
    JsonEvents,
}

impl Output {
    pub fn as_str(self) -> &'static str {
        match self {
            Output::Text => "Text",
            Output::TableRows => "TableRows/1",
            Output::JsonEvents => "JsonEvents/1",
        }
    }
}

/// A compiled program: parsed, desugared, resolved and checked, with its
/// plan built. Cheap to clone; `Send + Sync`.
#[derive(Clone)]
pub struct Program {
    resolved: Arc<Resolved>,
    src: Arc<str>,
    /// What the checker decided from `export`'s type.
    output: Output,
    native: bool,
    duplicates: Duplicates,
    /// `export` applied to the input plan under the flags above.
    result: Val,
}

impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Program")
            .field("file", &self.resolved.file)
            .field("output", &self.output())
            .field("native", &self.native)
            .finish()
    }
}

/// Parse, desugar, resolve and check `src`, named `file` in diagnostics.
pub fn compile(src: &str, file: &str) -> Result<Program, Fail> {
    let forms = desugar::program(parse_file(src, file)?, src)?;
    let resolved = Arc::new(resolve(forms, src, file, &stdlib::outer)?);
    let checked = crate::check::program(&resolved, src)?;
    Program::build(resolved, src, checked.output, true, Duplicates::Reject)
}

impl Program {
    fn build(
        resolved: Arc<Resolved>,
        src: &str,
        output: Output,
        native: bool,
        duplicates: Duplicates,
    ) -> Result<Program, Fail> {
        let result = Runtime::new(resolved.clone(), src)
            .with_native(native)
            .with_duplicates(duplicates)
            .export()?;
        Ok(Program {
            resolved,
            src: Arc::from(src),
            output,
            native,
            duplicates,
            result,
        })
    }

    /// The same program with the standard compositions run through the
    /// library's own definitions (`false`) or natively (`true`, the
    /// default). The differential test runs both.
    pub fn with_native(&self, native: bool) -> Result<Program, Fail> {
        Program::build(
            self.resolved.clone(),
            &self.src,
            self.output,
            native,
            self.duplicates,
        )
    }

    /// The same program with another policy for repeated member names in
    /// captured values; the default rejects them.
    pub fn with_duplicates(&self, duplicates: Duplicates) -> Result<Program, Fail> {
        Program::build(
            self.resolved.clone(),
            &self.src,
            self.output,
            self.native,
            duplicates,
        )
    }

    pub fn native(&self) -> bool {
        self.native
    }

    /// The file name the program was compiled under.
    pub fn file(&self) -> &str {
        &self.resolved.file
    }

    pub fn resolved(&self) -> &Arc<Resolved> {
        &self.resolved
    }

    /// The plan `export` answered: a text or a stream.
    pub fn result(&self) -> &Val {
        &self.result
    }

    /// What the program produces, and so what the host renders: decided
    /// by the checker from `export`'s type.
    pub fn output(&self) -> Output {
        self.output
    }

    /// The protocol the built plan produces; the checker's `output` agrees
    /// with it, and a test holds the two together.
    pub fn plan_output(&self) -> Output {
        match &self.result {
            Val::Stream(plan) => match plan.protocol() {
                Protocol::JsonEvents => Output::JsonEvents,
                _ => Output::TableRows,
            },
            _ => Output::Text,
        }
    }

    /// The selector under which the source is read one row at a time,
    /// when the plan makes one known: the table binding's `:rows`, a
    /// `select`'s selector, or the one multi-location capture of a
    /// `route`. A host that streams a verified grammar prunes the parse
    /// under it.
    pub fn row_selector(&self) -> Option<&Selector> {
        match root_stage(self.plan()?) {
            Plan::TableFromJson {
                binding: Val::Record(fields),
                ..
            } => match fields.get("rows") {
                Some(Val::Selector(s)) => Some(&**s),
                _ => None,
            },
            Plan::Select { selector, .. } => Some(selector),
            Plan::Route { specs, .. } => {
                let multi: Vec<&Selector> = specs
                    .iter()
                    .map(|s| &s.selector)
                    .filter(|s| s.is_multi())
                    .collect();
                match (multi.as_slice(), specs.len()) {
                    ([one], _) => Some(one),
                    ([], 1) => Some(&specs[0].selector),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn plan(&self) -> Option<&Arc<Plan>> {
        match &self.result {
            Val::Stream(p) | Val::Text(p) => Some(p),
            _ => None,
        }
    }

    /// The sink for one run: the host pushes the source's events into it
    /// and one `End`, and the output reaches `out` through a coalescing
    /// writer with `limits.max_output_bytes` enforced and `output_bytes`
    /// counted in `metrics`. `render` chooses how a table or JSON-events
    /// result is rendered (CSV or JSON; `None` for the default); a
    /// program that renders its own text takes none (`render_of_text`).
    pub fn sink(
        &self,
        out: Box<dyn Write + Send>,
        render: Option<Renderer>,
        limits: &Limits,
        metrics: Arc<Metrics>,
    ) -> Result<Box<dyn Sink + Send>, Fail> {
        let out = WriteOut::new(out)
            .with_limits(limits)
            .with_metrics(metrics.clone());
        self.sink_out(Box::new(out), render, limits, metrics)
    }

    /// [`Program::sink`] over any text output: for a host that has its own
    /// writer stage, and for tests that read the text back.
    pub fn sink_out(
        &self,
        out: Box<dyn TextOut + Send>,
        render: Option<Renderer>,
        limits: &Limits,
        metrics: Arc<Metrics>,
    ) -> Result<EventSink, Fail> {
        let rt = Arc::new(
            Runtime::new(self.resolved.clone(), &self.src)
                .with_native(self.native)
                .with_duplicates(self.duplicates),
        );
        let out: Out = out;
        Lowering::new(rt, limits, metrics).sink(&self.result, out, render)
    }
}

/// The stage that reads the input directly: the plan whose source is
/// `Input`, or `Input` itself.
fn root_stage(plan: &Plan) -> &Plan {
    let mut here = plan;
    loop {
        let next: &Plan = match here {
            Plan::Input => return here,
            Plan::Route { source, .. }
            | Plan::Select { source, .. }
            | Plan::ScanEmit { source, .. }
            | Plan::Map { source, .. }
            | Plan::Filter { source, .. }
            | Plan::TableFromJson { source, .. }
            | Plan::Records { source }
            | Plan::Csv { source, .. }
            | Plan::Json { source } => {
                if matches!(**source, Plan::Input) {
                    return here;
                }
                source
            }
            Plan::ConcatMap {
                items: Seq::Stream(source),
                ..
            }
            | Plan::Join {
                items: Seq::Stream(source),
                ..
            } => source,
            Plan::Concat(items) => match items.iter().find(|v| v.is_live()) {
                Some(Val::Text(inner)) | Some(Val::Stream(inner)) => inner,
                _ => return here,
            },
            Plan::Replace {
                source: Val::Text(inner),
                ..
            } => inner,
            _ => return here,
        };
        here = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::tests::{PROGRAM, RECORDS};

    #[test]
    fn a_program_knows_its_output_and_row_selector() {
        let p = compile(PROGRAM, "t.alc").unwrap();
        assert_eq!(p.output(), Output::Text);
        assert_eq!(
            p.row_selector().unwrap().to_string(),
            ".response.payload.deep.records[*]"
        );
        assert!(p.native());
        let slow = p.with_native(false).unwrap();
        assert!(!slow.native());
        assert_eq!(
            slow.row_selector().map(ToString::to_string).as_deref(),
            Some(".response.payload.deep.records[*]")
        );
        let table = compile(&PROGRAM.replace("    csv csv-options\n", ""), "t.alc").unwrap();
        assert_eq!(table.output(), Output::TableRows);
        let echo = compile("def export [input] input", "t.alc").unwrap();
        assert_eq!(echo.output(), Output::JsonEvents);
        assert_eq!(echo.row_selector(), None);
        let select = compile(
            "def export [input] (join \",\" (select (path \"a\" each-index) input))",
            "t.alc",
        )
        .unwrap();
        assert_eq!(select.output(), Output::Text);
        assert_eq!(select.row_selector().unwrap().to_string(), ".a[*]");
    }

    #[test]
    fn the_sink_writes_through_a_write() {
        let p = compile(PROGRAM, "t.alc").unwrap();
        let buffer = crate::lower::tests::Shared::default();
        let limits = Limits::default();
        let mut sink = p
            .sink(Box::new(buffer.clone()), None, &limits, Metrics::new())
            .unwrap();
        let datum = tabnas_transduce::Datum::from_json(&serde_json::from_str(RECORDS).unwrap());
        tabnas_transduce::walk_datum(&datum, &mut sink).unwrap();
        sink.event(tabnas_transduce::JsonEvent::End).unwrap();
        let bytes = buffer.0.lock().unwrap().clone();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            crate::lower::tests::EXPECTED_CSV
        );
    }

    #[test]
    fn compile_reports_reader_resolver_and_export_failures() {
        assert_eq!(
            compile("(a b", "t.alc").unwrap_err().code,
            tabnas_transduce::Code::DslParseError
        );
        let f = compile("def export [input] (nope input)", "t.alc").unwrap_err();
        assert!(f.message.starts_with("unknown_name: "), "{f}");
        let f = compile("def x 1", "t.alc").unwrap_err();
        assert!(f.message.starts_with("no_export: "), "{f}");
    }
}
