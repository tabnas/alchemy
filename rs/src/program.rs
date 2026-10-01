//! The public API a host embeds: [`compile`] a program (or
//! [`compile_sources`], several files linked into one), ask it what it
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
use tabnas_transduce::{AbortFlag, Code, Duplicates, Fail, Limits, Metrics, Selector, Sink};

use crate::ast::{Expr, Sources};
use crate::interp::{Runtime, MAX_PLAN_STEPS};
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
    sources: Sources,
    /// What the checker decided from `export`'s type.
    output: Output,
    native: bool,
    duplicates: Duplicates,
    /// The host's cancellation, handed to every sink's runtime.
    abort: AbortFlag,
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

/// Parse, desugar, resolve and check `src`, named `file` in diagnostics,
/// and build its plan: `export` applied to the input's plan, which
/// evaluates everything a stream does not defer, so a `fail` or a
/// `no_match` on that path is reported here, with its own code. The work
/// runs on a thread of [`crate::STACK_BYTES`], and building the plan is
/// bounded by [`MAX_PLAN_STEPS`] and [`crate::MAX_EVAL_DEPTH`].
pub fn compile(src: &str, file: &str) -> Result<Program, Fail> {
    compile_sources(&[Source::new(file, src)])
}

/// One of the sources a program is compiled from: the file it is named
/// by in diagnostics, its text, and the name its own `export` is linked
/// under when another source's `export` is the program's.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    pub file: &'a str,
    pub text: &'a str,
    /// When set, this source's `export` is defined under this name in the
    /// linked namespace, and every mention of `export` in this source
    /// names it, so that the program's `export`, in another source, can
    /// call it: a host composes a program's output into a format's render
    /// by linking the program as `program-export` and writing
    /// `def export [input] (yaml-render (program-export input))`. The
    /// source must define `export` (`no_export` otherwise) and must not
    /// mention the name already (`duplicate_def`), since renaming beside
    /// a binding of that name would change what the source means.
    pub export_as: Option<&'a str>,
}

impl<'a> Source<'a> {
    pub fn new(file: &'a str, text: &'a str) -> Source<'a> {
        Source {
            file,
            text,
            export_as: None,
        }
    }

    /// The same source with its `export` linked under `name`.
    pub fn export_as(self, name: &'a str) -> Source<'a> {
        Source {
            export_as: Some(name),
            ..self
        }
    }
}

/// Compile a program from several sources linked into one namespace: a
/// definition in any of them is in scope in all, `export` is defined in
/// one, and a name defined twice, in one file or across two, is
/// `duplicate_def`. Each source is named by its file, and the names are
/// distinct (`duplicate_file` otherwise). A failure in a program of
/// several sources carries the file its position is in, `Fail::file`,
/// displayed `(file:row:col)`, beside the row and column; a program of
/// one source is [`compile`], which positions without a file. This is
/// how a host links a format's parts (its lift and render: libraries of
/// definitions with no `export`) with the program that calls them.
///
/// A source whose `export` is not the program's is linked with
/// [`Source::export_as`]: its `export` is defined under that name
/// instead, so that the program's `export` can call it. This is how a
/// host composes a whole program's output into a format's render
/// (`def export [input] (yaml-render (program-export input))`, with the
/// user's program linked as `program-export`), in one plan under one
/// set of limits. The renamed source positions its failures in its own
/// file as any source does.
pub fn compile_sources(sources: &[Source<'_>]) -> Result<Program, Fail> {
    on_stack(|| {
        let mut named: Vec<(Arc<str>, Arc<str>)> = Vec::with_capacity(sources.len());
        for source in sources {
            if named.iter().any(|(file, _)| &**file == source.file) {
                return Err(Fail::new(
                    Code::DslTypeError,
                    format!(
                        "duplicate_file: {} is given twice; each source has its own name",
                        source.file
                    ),
                ));
            }
            named.push((Arc::from(source.file), Arc::from(source.text)));
        }
        if named.is_empty() {
            return Err(crate::check::no_export());
        }
        let linked = Sources::several(named);
        let mut forms = Vec::new();
        for source in sources {
            // The reader and the desugarer see one file at a time, so the
            // file is added here; the stages after them position through
            // `linked`.
            let in_file = |fail: Fail| {
                if linked.names_files() {
                    fail.in_file(source.file)
                } else {
                    fail
                }
            };
            let parsed = parse_file(source.text, source.file).map_err(in_file)?;
            let mut own = desugar::program(parsed, source.text).map_err(in_file)?;
            if let Some(name) = source.export_as {
                if !own.iter().any(|form| defines(form, "export")) {
                    return Err(in_file(Fail::new(
                        Code::DslTypeError,
                        format!(
                            "no_export: {} defines no export to link as {name}",
                            source.file
                        ),
                    )));
                }
                // Renaming every `export` is a consistent renaming only
                // while nothing in the source is already called `name`: a
                // parameter or a `let` of that name would capture a renamed
                // mention. The host chooses the name, so one in use is its
                // mistake, said at the first mention.
                if let Some(taken) = own.iter().find_map(|form| mentions(form, name)) {
                    return Err(in_file(
                        Fail::new(
                            Code::DslTypeError,
                            format!(
                                "duplicate_def: {} already names {name}, so its export cannot \
                                 be linked under it; link it under another name",
                                source.file
                            ),
                        )
                        .at(
                            linked.position(taken).0 as u64,
                            linked.position(taken).1 as u64,
                        ),
                    ));
                }
                for form in &mut own {
                    rename_symbol(form, "export", name);
                }
            }
            forms.extend(own);
        }
        let resolved = Arc::new(resolve(forms, &linked, &stdlib::outer)?);
        let checked = crate::check::program(&resolved, &linked)?;
        Program::build_here(
            resolved,
            linked,
            checked.output,
            true,
            Duplicates::Reject,
            AbortFlag::new(),
        )
    })
}

/// Whether `form` is a top-level `def` of `name`, as the desugarer leaves
/// one: `(def name value)`.
fn defines(form: &Expr, name: &str) -> bool {
    match form {
        Expr::List { items, .. } => {
            items.len() == 3 && items[0].symbol() == Some("def") && items[1].symbol() == Some(name)
        }
        _ => false,
    }
}

/// The span of the first symbol `name` in `form`, when it mentions one.
/// Recursive per level, which [`crate::MAX_NESTING`] bounds.
fn mentions<'f>(form: &'f Expr, name: &str) -> Option<&'f crate::ast::SourceSpan> {
    match form {
        Expr::Symbol { name: n, span } if n == name => Some(span),
        Expr::List { items, .. } | Expr::Vector { items, .. } => {
            items.iter().find_map(|item| mentions(item, name))
        }
        _ => None,
    }
}

/// Every symbol `from` in `form` becomes `to`: the name a `def` binds and
/// every mention of it alike, so the source means what it meant under
/// the new name, which [`compile_sources`] has checked is not in use.
/// Recursive per level, which [`crate::MAX_NESTING`] bounds.
fn rename_symbol(form: &mut Expr, from: &str, to: &str) {
    match form {
        Expr::Symbol { name, .. } if name == from => *name = to.to_string(),
        Expr::List { items, .. } | Expr::Vector { items, .. } => {
            for item in items {
                rename_symbol(item, from, to);
            }
        }
        _ => {}
    }
}

/// Run `work` on a thread of [`crate::STACK_BYTES`], so the checker's and
/// the evaluator's recursion has the room their bounds promise whatever
/// thread the caller is on.
fn on_stack<T: Send>(work: impl FnOnce() -> Result<T, Fail> + Send) -> Result<T, Fail> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("alchemy-compile".into())
            .stack_size(crate::STACK_BYTES)
            .spawn_scoped(scope, work)
            .map_err(|error| {
                Fail::new(
                    Code::ResourceLimitExceeded,
                    format!("no thread could be started to compile the program: {error}"),
                )
            })?;
        thread
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

impl Program {
    fn build(
        resolved: Arc<Resolved>,
        sources: Sources,
        output: Output,
        native: bool,
        duplicates: Duplicates,
        abort: AbortFlag,
    ) -> Result<Program, Fail> {
        on_stack(|| Program::build_here(resolved, sources, output, native, duplicates, abort))
    }

    fn build_here(
        resolved: Arc<Resolved>,
        sources: Sources,
        output: Output,
        native: bool,
        duplicates: Duplicates,
        abort: AbortFlag,
    ) -> Result<Program, Fail> {
        let result = Runtime::new(resolved.clone(), sources.clone())
            .with_native(native)
            .with_duplicates(duplicates)
            .with_fuel(Some(MAX_PLAN_STEPS))
            .with_abort(abort.clone())
            .export()?;
        Ok(Program {
            resolved,
            sources,
            output,
            native,
            duplicates,
            abort,
            result,
        })
    }

    /// The same program with the standard compositions run through the
    /// library's own definitions (`false`) or natively (`true`, the
    /// default). The differential test runs both.
    pub fn with_native(&self, native: bool) -> Result<Program, Fail> {
        Program::build(
            self.resolved.clone(),
            self.sources.clone(),
            self.output,
            native,
            self.duplicates,
            self.abort.clone(),
        )
    }

    /// The same program with another policy for repeated member names in
    /// captured values; the default rejects them.
    pub fn with_duplicates(&self, duplicates: Duplicates) -> Result<Program, Fail> {
        Program::build(
            self.resolved.clone(),
            self.sources.clone(),
            self.output,
            self.native,
            duplicates,
            self.abort.clone(),
        )
    }

    /// The same program with the host's cancellation: every sink it makes
    /// reads the flag as the program's functions run, so a timeout stops
    /// a long computation on one item with `ABORTED` rather than waiting
    /// for the item to finish. The source takes the same flag
    /// (`ParserSource::abort`) to stop between events.
    pub fn with_abort(&self, abort: AbortFlag) -> Program {
        Program {
            abort,
            ..self.clone()
        }
    }

    pub fn native(&self) -> bool {
        self.native
    }

    /// The policy for a member name repeated in a captured scope.
    pub fn duplicates(&self) -> Duplicates {
        self.duplicates
    }

    /// The file name the program was compiled under: the first, of
    /// several.
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

    /// The plan report of spec section 15.5: the chain of calls, the
    /// protocols, what is retained and under which limits, the ordering
    /// contract, the renderer, the guarantee and its qualification.
    pub fn explain(&self) -> String {
        crate::effects::explain(self)
    }

    /// The same report as one JSON object, for a host's `--explain`.
    pub fn explain_json(&self) -> serde_json::Value {
        crate::effects::explain_json(self)
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
            Runtime::new(self.resolved.clone(), self.sources.clone())
                .with_native(self.native)
                .with_duplicates(self.duplicates)
                .with_limits(limits)
                .with_abort(self.abort.clone()),
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
            | Plan::Events { source }
            | Plan::ScanEmit { source, .. }
            | Plan::Map { source, .. }
            | Plan::Filter { source, .. }
            | Plan::TableFromJson { source, .. }
            | Plan::Records { source }
            | Plan::CsvTable { source, .. }
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
            Plan::Concat { items, live } => match live.and_then(|i| items.get(i)) {
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
