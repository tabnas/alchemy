//! The effect summary and the plan report (spec sections 15.1, 15.2 and
//! 15.5; design brief 4.3).
//!
//! The summary is computed from the plan the program built, not from its
//! text: the plan knows which stages run (the native table transducer or
//! a `route` and a `scan-emit`, a renderer or the text algebra) and what
//! each retains, and the registry's effect metadata says what each
//! operator promises. Live retention requirements are summed rather than
//! maxed (15.2): the standard table holds its metadata and one row at the
//! same time, and the report says both.
//!
//! [`explain`] prints the report in the layout of 15.5: the entry's chain
//! of calls, then labelled lines (source reads, protocol chain, selection,
//! what is retained, output order, the ordering contract and how it is
//! enforced, the renderer's profile, external storage), then the
//! guarantee and its qualification. [`explain_json`] is the same
//! information as one object, for a host's `--explain`.

use serde_json::{json, Value as Json};
use tabnas_render::{CsvOptions, Newline};
use tabnas_transduce::{Duplicates, Selector};

use crate::ast::Expr;
use crate::program::{Output, Program};
use crate::value::{Plan, Seq, Val};

/// What a stage keeps alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionScope {
    /// The column metadata, until the end.
    Metadata,
    /// One selected row at a time.
    Record,
    /// One selected scope, materialized whole.
    Subtree,
    /// The state a `scan-emit` step returns.
    State,
}

impl RetentionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RetentionScope::Metadata => "metadata",
            RetentionScope::Record => "record",
            RetentionScope::Subtree => "subtree",
            RetentionScope::State => "state",
        }
    }
}

/// One live retention requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retention {
    pub scope: RetentionScope,
    /// The label the report prints it under.
    pub label: &'static str,
    pub reason: String,
    /// The selector whose matches are retained, when one applies.
    pub selector: Option<Selector>,
    /// The `Limits` field that caps it, when the runtime caps it.
    pub limit: Option<&'static str>,
}

/// When output is ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// After each source event.
    Event,
    /// After each row or item.
    Record,
    /// Only at the end.
    ScopeEnd,
}

impl Readiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Readiness::Event => "event",
            Readiness::Record => "record",
            Readiness::ScopeEnd => "scope-end",
        }
    }
}

/// An ordering the plan relies on, and who enforces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderConstraint {
    pub before: String,
    pub after: String,
    /// `static` when the plan cannot violate it; `runtime` when the run
    /// checks it and fails.
    pub enforcement: &'static str,
}

/// How sure the analysis is of its guarantee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// Every stage is a native with fixed retention under the limits.
    Proven,
    /// A stage's retention is the program's (a `scan-emit` state).
    Conditional,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Proven => "proven",
            Confidence::Conditional => "conditional",
        }
    }
}

/// The renderer that writes the text, when one does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererProfile {
    /// The CSV renderer, always quoted, in the dialect it is built with:
    /// the program's own options for its `csv`, the defaults (CRLF, a
    /// header) when the host chose it for a table result (`host`).
    Csv { host: bool, options: CsvOptions },
    /// The JSON renderer: compact, one document, a trailing newline.
    Json { host: bool },
    /// The program's own text algebra.
    Text,
}

/// The effect summary of spec 15.2 with the report's other facts.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectSummary {
    /// The definition the host applies.
    pub entry: String,
    /// Whether the result is a text that never reaches the input: the
    /// source is read and validated, and nothing of it is used.
    pub finite: bool,
    /// The calls on the data-last spine of the entry's body, innermost
    /// first: `api-table → csv`.
    pub chain: Vec<String>,
    pub passes: u8,
    pub protocol: Vec<String>,
    pub selection: String,
    /// What happens to a member name repeated in an object of the source
    /// (spec 18.2): rejected (or resolved by the policy) where a scope is
    /// captured, preserved where the events are copied as they arrive.
    pub duplicates: String,
    pub retention: Vec<Retention>,
    pub readiness: Readiness,
    pub output_order: String,
    pub order_constraints: Vec<OrderConstraint>,
    pub renderer: RendererProfile,
    pub external_storage: &'static str,
    pub confidence: Confidence,
    pub guarantee: String,
    pub qualification: Vec<String>,
    pub output: Output,
}

/// The stages of a plan from the input outward.
fn stages(plan: &Plan) -> Vec<&Plan> {
    let mut out = Vec::new();
    let mut here = plan;
    loop {
        out.push(here);
        let next: &Plan = match here {
            Plan::Input | Plan::Lit(_) => break,
            Plan::Route { source, .. }
            | Plan::Select { source, .. }
            | Plan::ScanEmit { source, .. }
            | Plan::Map { source, .. }
            | Plan::Filter { source, .. }
            | Plan::TableFromJson { source, .. }
            | Plan::Records { source }
            | Plan::CsvTable { source, .. }
            | Plan::Csv { source, .. }
            | Plan::Json { source } => source,
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
                _ => break,
            },
            Plan::Replace {
                source: Val::Text(inner),
                ..
            } => inner,
            Plan::ConcatMap { .. } | Plan::Join { .. } | Plan::Replace { .. } => break,
        };
        here = next;
    }
    out.reverse();
    out
}

/// The protocol a stage produces, as the report names it.
fn protocol_of(stage: &Plan) -> &'static str {
    match stage {
        Plan::Input | Plan::Records { .. } => "JsonEvents/1",
        Plan::TableFromJson { .. } | Plan::CsvTable { .. } => "TableRows/1",
        Plan::Route { .. } => "Stream<Selected>",
        Plan::Select { .. } => "Stream<Value>",
        Plan::ScanEmit { .. } | Plan::Map { .. } | Plan::Filter { .. } => "Stream<Value>",
        _ => "Text",
    }
}

/// The calls on the data-last spine of an `export` body, innermost
/// first, ending where the spine reaches the parameter or a special form.
fn chain_of(body: &Expr) -> Vec<String> {
    let mut chain = Vec::new();
    let mut here = body;
    while let Expr::List { items, .. } = here {
        let Some(head) = items.first().and_then(Expr::symbol) else {
            break;
        };
        if matches!(head, "fn" | "let" | "if" | "match" | "def") || items.len() < 2 {
            break;
        }
        chain.push(head.to_string());
        here = items.last().expect("a list with at least two items");
    }
    chain.reverse();
    chain
}

/// Whether the plan produces table events, natively or as tagged values
/// the host's adapter reads.
fn tabular(stages: &[&Plan], output: Output) -> bool {
    stages.iter().any(|s| {
        matches!(
            s,
            Plan::TableFromJson { .. }
                | Plan::CsvTable { .. }
                | Plan::Csv { .. }
                | Plan::Records { .. }
        )
    }) || output == Output::TableRows
}

/// The duplicate-member line: what the plan does with a member name an
/// object of the source repeats.
fn duplicates_of(stages: &[&Plan], policy: Duplicates, finite: bool) -> String {
    if finite {
        return "not examined; the input is only validated".to_string();
    }
    let captured = stages.iter().any(|s| {
        matches!(
            s,
            Plan::Route { .. } | Plan::Select { .. } | Plan::TableFromJson { .. }
        )
    });
    if captured {
        match policy {
            Duplicates::Reject => "rejected in captured scopes (DUPLICATE_MEMBER)",
            Duplicates::LastWins => "the last value wins in captured scopes",
            Duplicates::FirstWins => "the first value wins in captured scopes",
        }
        .to_string()
    } else {
        "preserved; the events are copied as they arrive, not mapped by key".to_string()
    }
}

/// The summary of a compiled program.
pub fn summarize(program: &Program) -> EffectSummary {
    let output = program.output();
    let plan: Option<&Plan> = match program.result() {
        Val::Stream(p) | Val::Text(p) => Some(p),
        _ => None,
    };
    // A result that never reaches the input (a string, or a text of the
    // program's own) is written at the end; the source is only validated.
    let finite = !plan.is_some_and(Plan::is_live);
    let stages: Vec<&Plan> = if finite {
        Vec::new()
    } else {
        plan.map(stages).unwrap_or_default()
    };
    let chain = program
        .resolved()
        .get("export")
        .and_then(|def| match &*def.value {
            Expr::List { items, .. } if items.len() == 3 => Some(chain_of(&items[2])),
            _ => None,
        })
        .unwrap_or_default();

    if finite {
        return EffectSummary {
            entry: "export".to_string(),
            finite,
            chain,
            passes: 1,
            protocol: vec!["Text".to_string()],
            selection: "none; the input is read and validated, and nothing of it is used"
                .to_string(),
            duplicates: duplicates_of(&stages, program.duplicates(), finite),
            retention: Vec::new(),
            readiness: Readiness::ScopeEnd,
            output_order: "the program's own text, written once the input has validated"
                .to_string(),
            order_constraints: Vec::new(),
            renderer: RendererProfile::Text,
            external_storage: "disabled",
            confidence: Confidence::Proven,
            guarantee: "Memory is independent of the document's size under the configured\n  depth and scalar/key limits: nothing of the input is kept. The text\n  is the program's own, written under the output limit.".to_string(),
            qualification: vec![
                "The text is written only after the whole input has validated; an\n  invalid input writes nothing.".to_string(),
            ],
            output,
        };
    }
    let mut protocol: Vec<String> = vec!["JsonEvents/1".to_string()];
    for stage in stages.iter().skip(1) {
        let p = protocol_of(stage);
        if protocol.last().map(String::as_str) != Some(p) {
            protocol.push(p.to_string());
        }
    }
    let renderer = match (stages.last(), output) {
        // The fast path builds `Plan::Csv` only for options that map onto
        // the renderer's dialect, and lowering builds the renderer with them.
        (Some(Plan::Csv { options, .. }), _) => RendererProfile::Csv {
            host: false,
            options: crate::lower::csv_options(options).unwrap_or_default(),
        },
        (Some(Plan::Json { .. }), _) => RendererProfile::Json { host: false },
        (_, Output::TableRows) => {
            protocol.push("Text".to_string());
            // The host's renderer, as lowering builds it.
            RendererProfile::Csv {
                host: true,
                options: CsvOptions::default(),
            }
        }
        (_, Output::JsonEvents) => {
            protocol.push("Text".to_string());
            RendererProfile::Json { host: true }
        }
        _ => RendererProfile::Text,
    };

    let mut selection = "none; every event passes through".to_string();
    let mut retention = Vec::new();
    let mut order_constraints = Vec::new();
    let mut readiness = Readiness::Event;
    let mut confidence = Confidence::Proven;
    let mut output_order = "source order".to_string();
    for stage in &stages {
        match stage {
            Plan::TableFromJson { binding, .. } => {
                let selector = |key: &str| match binding.field(key) {
                    Some(Val::Selector(s)) => Some((*s).clone()),
                    _ => None,
                };
                selection = "shared prefix matcher, two capture routes".to_string();
                readiness = Readiness::Record;
                output_order = "schema first; cells in schema order".to_string();
                retention.push(Retention {
                    scope: RetentionScope::Metadata,
                    label: "Retained metadata:",
                    reason: "the column descriptors, bound once they complete".to_string(),
                    selector: selector("columns"),
                    limit: Some("max_metadata_bytes"),
                });
                retention.push(Retention {
                    scope: RetentionScope::Record,
                    label: "Row capture:",
                    reason: "one row at a time, projected into schema order and released"
                        .to_string(),
                    selector: selector("rows"),
                    limit: Some("max_record_bytes"),
                });
                order_constraints.push(OrderConstraint {
                    before: "metadata completes".to_string(),
                    after: "first row begins".to_string(),
                    enforcement: "runtime",
                });
            }
            Plan::Route { specs, .. } => {
                selection = format!(
                    "shared prefix matcher, {} capture route{}",
                    count(specs.len()),
                    if specs.len() == 1 { "" } else { "s" }
                );
                readiness = Readiness::Record;
                output_order = "selection order: completed matches, in source order".to_string();
                for spec in specs {
                    let multi = spec.selector.is_multi();
                    retention.push(Retention {
                        scope: if multi {
                            RetentionScope::Record
                        } else {
                            RetentionScope::Subtree
                        },
                        label: if multi { "Row capture:" } else { "Capture:" },
                        reason: format!(
                            "{} one selected scope, materialized whole and released after delivery",
                            if multi { "one at a time," } else { "" }
                        )
                        .trim_start()
                        .to_string(),
                        selector: Some(spec.selector.clone()),
                        limit: Some(spec.budget.as_ref().map_or("max_capture_bytes", |b| b.name)),
                    });
                }
            }
            Plan::Select { selector, .. } => {
                selection = "shared prefix matcher, one capture route".to_string();
                readiness = Readiness::Record;
                output_order = "selection order: completed matches, in source order".to_string();
                retention.push(Retention {
                    scope: RetentionScope::Record,
                    label: "Row capture:",
                    reason: "one selected value at a time, released after delivery".to_string(),
                    selector: Some(selector.clone()),
                    limit: Some("max_capture_bytes"),
                });
            }
            Plan::ScanEmit { .. } => {
                confidence = Confidence::Conditional;
                retention.push(Retention {
                    scope: RetentionScope::State,
                    label: "Retained state:",
                    reason: "what the step returns, no deeper than max_depth".to_string(),
                    selector: None,
                    limit: Some("max_metadata_bytes"),
                });
                order_constraints.push(OrderConstraint {
                    before: "each item".to_string(),
                    after: "its outputs".to_string(),
                    enforcement: "static",
                });
            }
            _ => {}
        }
    }
    if tabular(&stages, output) && !matches!(stages.first(), Some(Plan::TableFromJson { .. })) {
        // Table events reach a renderer: it validates the protocol.
        if !order_constraints.iter().any(|c| c.before == "schema") {
            order_constraints.push(OrderConstraint {
                before: "schema".to_string(),
                after: "rows, then one end".to_string(),
                enforcement: "runtime",
            });
        }
    }
    if let RendererProfile::Csv { .. } = renderer {
        if output_order == "source order" {
            output_order = "schema first; cells in schema order".to_string();
        }
    }

    let guarantee = if retention.is_empty() {
        "Memory is independent of the document's size under the configured\n  depth, scalar/key and output limits: nothing is retained beyond the\n  renderer's nesting stack.".to_string()
    } else if confidence == Confidence::Proven {
        "Memory is independent of the number of rows under the configured\n  depth, metadata, record, scalar/key and output limits.".to_string()
    } else {
        "Memory is independent of the number of rows under the configured\n  depth, capture, metadata, scalar/key and output limits: the state the\n  step returns is capped at max_metadata_bytes. What one step computes\n  is the program's, bounded by the host's abort flag.".to_string()
    };
    let mut qualification = Vec::new();
    if order_constraints
        .iter()
        .any(|c| c.before.starts_with("metadata"))
    {
        qualification
            .push("Valid JSON that violates the metadata-first contract is rejected.".to_string());
    }
    qualification
        .push("A later error can occur after earlier output has been written.".to_string());

    let duplicates = duplicates_of(&stages, program.duplicates(), finite);
    EffectSummary {
        entry: "export".to_string(),
        finite,
        chain,
        passes: 1,
        protocol,
        selection,
        duplicates,
        retention,
        readiness,
        output_order,
        order_constraints,
        renderer,
        external_storage: "disabled",
        confidence,
        guarantee,
        qualification,
        output,
    }
}

fn count(n: usize) -> String {
    match n {
        0 => "no".to_string(),
        1 => "one".to_string(),
        2 => "two".to_string(),
        3 => "three".to_string(),
        n => n.to_string(),
    }
}

impl EffectSummary {
    /// The report, in the layout of spec 15.5.
    pub fn text(&self) -> String {
        let mut lines: Vec<(String, String)> = vec![
            ("Source reads:".into(), self.passes.to_string()),
            ("Protocol:".into(), self.protocol.join(" → ")),
            ("Selection:".into(), self.selection.clone()),
            ("Duplicate members:".into(), self.duplicates.clone()),
        ];
        for r in &self.retention {
            let mut value = String::new();
            if let Some(s) = &r.selector {
                if r.scope == RetentionScope::Record {
                    value.push_str("one ");
                }
                value.push_str(&s.to_string());
            } else {
                value.push_str(&r.reason);
            }
            if let Some(limit) = r.limit {
                value.push_str(&format!(", capped at {limit}"));
            }
            lines.push((r.label.to_string(), value));
        }
        lines.push(("Output order:".into(), self.output_order.clone()));
        let (contract, verification) = match self.order_constraints.first() {
            Some(c) => (
                format!("{} before {}", c.before, c.after),
                self.order_constraints
                    .iter()
                    .map(|c| c.enforcement)
                    .find(|e| *e == "runtime")
                    .unwrap_or("static")
                    .to_string(),
            ),
            None => ("none".to_string(), "static".to_string()),
        };
        lines.push(("Ordering contract:".into(), contract));
        lines.push(("Contract verification:".into(), verification));
        match &self.renderer {
            RendererProfile::Csv { host, .. } => lines.push((
                "CSV quoting:".into(),
                if *host {
                    "always (the host's renderer)".to_string()
                } else {
                    "always".to_string()
                },
            )),
            RendererProfile::Json { host } => lines.push((
                "JSON profile:".into(),
                if *host {
                    "compact, one document, trailing newline (the host's renderer)".to_string()
                } else {
                    "compact, one document, trailing newline".to_string()
                },
            )),
            RendererProfile::Text => {}
        }
        lines.push((
            "External storage:".into(),
            self.external_storage.to_string(),
        ));

        let mut out = String::new();
        out.push_str(&self.entry);
        out.push_str(": ");
        out.push_str(match (self.chain.is_empty(), self.finite) {
            (true, true) => "a text of its own",
            (true, false) => "input",
            (false, _) => "",
        });
        out.push_str(&self.chain.join(" → "));
        out.push_str("\n\n");
        for (label, value) in &lines {
            out.push_str(&format!("{label:<23}{value}\n"));
        }
        out.push_str("\nGuarantee:\n  ");
        out.push_str(&self.guarantee);
        out.push_str("\n\nQualification:\n");
        for q in &self.qualification {
            out.push_str("  ");
            out.push_str(q);
            out.push('\n');
        }
        out
    }

    /// The same facts as one JSON object.
    pub fn json(&self) -> Json {
        let renderer = match &self.renderer {
            RendererProfile::Csv { host, options } => json!({
                "name": "csv",
                "quoting": "always",
                "delimiter": options.delimiter.to_string(),
                "newline": match options.newline {
                    Newline::CrLf => "\r\n",
                    Newline::Lf => "\n",
                },
                "header": options.header,
                "host": host,
            }),
            RendererProfile::Json { host } => {
                json!({"name": "json", "indent": null, "trailing_newline": true, "host": host})
            }
            RendererProfile::Text => json!({"name": "text"}),
        };
        json!({
            "entry": self.entry,
            "finite": self.finite,
            "chain": self.chain,
            "output": self.output.as_str(),
            "passes": self.passes,
            "protocol": self.protocol,
            "selection": self.selection,
            "duplicates": self.duplicates,
            "retention": self.retention.iter().map(|r| json!({
                "scope": r.scope.as_str(),
                "reason": r.reason,
                "selector": r.selector.as_ref().map(ToString::to_string),
                "limit": r.limit,
            })).collect::<Vec<_>>(),
            "readiness": self.readiness.as_str(),
            "output_order": self.output_order,
            "order_constraints": self.order_constraints.iter().map(|c| json!({
                "before": c.before, "after": c.after, "enforcement": c.enforcement,
            })).collect::<Vec<_>>(),
            "renderer": renderer,
            "external_storage": self.external_storage,
            "confidence": self.confidence.as_str(),
            "guarantee": self.guarantee.replace("\n  ", " "),
            "qualification": self.qualification.iter().map(|q| q.replace("\n  ", " ")).collect::<Vec<_>>(),
        })
    }
}

/// The report of spec 15.5 for a compiled program.
pub fn explain(program: &Program) -> String {
    summarize(program).text()
}

/// The report as one JSON object.
pub fn explain_json(program: &Program) -> Json {
    summarize(program).json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile;
    use crate::lower::tests::PROGRAM;

    #[test]
    fn the_worked_example_reports_in_the_spec_layout() {
        let program = compile(PROGRAM, "export.alc").unwrap();
        let text = explain(&program);
        assert_eq!(
            text,
            "export: api-table → csv\n\
             \n\
             Source reads:          1\n\
             Protocol:              JsonEvents/1 → TableRows/1 → Text\n\
             Selection:             shared prefix matcher, two capture routes\n\
             Duplicate members:     rejected in captured scopes (DUPLICATE_MEMBER)\n\
             Retained metadata:     .response.metadata.fields, capped at max_metadata_bytes\n\
             Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n\
             Output order:          schema first; cells in schema order\n\
             Ordering contract:     metadata completes before first row begins\n\
             Contract verification: runtime\n\
             CSV quoting:           always\n\
             External storage:      disabled\n\
             \n\
             Guarantee:\n\
             \x20 Memory is independent of the number of rows under the configured\n\
             \x20 depth, metadata, record, scalar/key and output limits.\n\
             \n\
             Qualification:\n\
             \x20 Valid JSON that violates the metadata-first contract is rejected.\n\
             \x20 A later error can occur after earlier output has been written.\n"
        );
        let j = explain_json(&program);
        assert_eq!(j["chain"], json!(["api-table", "csv"]));
        assert_eq!(
            j["protocol"],
            json!(["JsonEvents/1", "TableRows/1", "Text"])
        );
        assert_eq!(j["retention"][0]["scope"], "metadata");
        assert_eq!(j["retention"][1]["limit"], "max_record_bytes");
        assert_eq!(j["confidence"], "proven");
        assert_eq!(j["readiness"], "record");
        assert_eq!(j["renderer"]["name"], "csv");
        assert_eq!(j["output"], "Text");
        assert_eq!(j["order_constraints"][0]["enforcement"], "runtime");
    }

    #[test]
    fn the_interpreted_library_reports_its_route_and_state() {
        let program = compile(PROGRAM, "export.alc")
            .unwrap()
            .with_native(false)
            .unwrap();
        let s = summarize(&program);
        // The library's `csv` validates the table events it renders
        // (`csv-table`), so the chain passes through TableRows/1 as the
        // native one does.
        assert_eq!(
            s.protocol,
            [
                "JsonEvents/1",
                "Stream<Selected>",
                "Stream<Value>",
                "TableRows/1",
                "Text"
            ]
        );
        assert_eq!(s.selection, "shared prefix matcher, two capture routes");
        assert_eq!(s.confidence, Confidence::Conditional);
        assert_eq!(
            s.retention.iter().map(|r| r.scope).collect::<Vec<_>>(),
            [
                RetentionScope::Subtree,
                RetentionScope::Record,
                RetentionScope::State
            ]
        );
        let text = s.text();
        // The library's captures name the limits the native table holds
        // the same scopes to, and the state is capped.
        assert!(
            text.contains(
                "Capture:               .response.metadata.fields, capped at max_metadata_bytes\n"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n"
            ),
            "{text}"
        );
        assert!(text.contains("Retained state:        what the step returns, no deeper than max_depth, capped at max_metadata_bytes\n"), "{text}");
        assert!(text.contains("Contract verification: runtime\n"), "{text}");
        assert!(
            text.contains("Ordering contract:     each item before its outputs\n"),
            "{text}"
        );
    }

    #[test]
    fn the_json_echo_and_a_host_rendered_table() {
        let echo = compile("def export [input] (json input)", "echo.alc").unwrap();
        assert_eq!(
            explain(&echo),
            "export: json\n\
             \n\
             Source reads:          1\n\
             Protocol:              JsonEvents/1 → Text\n\
             Selection:             none; every event passes through\n\
             Duplicate members:     preserved; the events are copied as they arrive, not mapped by key\n\
             Output order:          source order\n\
             Ordering contract:     none\n\
             Contract verification: static\n\
             JSON profile:          compact, one document, trailing newline\n\
             External storage:      disabled\n\
             \n\
             Guarantee:\n\
             \x20 Memory is independent of the document's size under the configured\n\
             \x20 depth, scalar/key and output limits: nothing is retained beyond the\n\
             \x20 renderer's nesting stack.\n\
             \n\
             Qualification:\n\
             \x20 A later error can occur after earlier output has been written.\n"
        );
        let identity = compile("def export [input] input", "id.alc").unwrap();
        let text = explain(&identity);
        assert!(text.starts_with("export: input\n"), "{text}");
        assert!(text.contains("JSON profile:          compact, one document, trailing newline (the host's renderer)\n"), "{text}");
        let table = compile(&PROGRAM.replace("    csv csv-options\n", ""), "table.alc").unwrap();
        let text = explain(&table);
        assert!(text.starts_with("export: api-table\n"), "{text}");
        assert!(
            text.contains("Protocol:              JsonEvents/1 → TableRows/1 → Text\n"),
            "{text}"
        );
        assert!(
            text.contains("CSV quoting:           always (the host's renderer)\n"),
            "{text}"
        );
        assert_eq!(explain_json(&table)["renderer"]["host"], true);
    }

    #[test]
    fn a_text_of_its_own_reads_the_input_only_to_validate_it() {
        let done = compile("def export [input] \"done\"", "done.alc").unwrap();
        assert_eq!(
            explain(&done),
            "export: a text of its own\n\
             \n\
             Source reads:          1\n\
             Protocol:              Text\n\
             Selection:             none; the input is read and validated, and nothing of it is used\n\
             Duplicate members:     not examined; the input is only validated\n\
             Output order:          the program's own text, written once the input has validated\n\
             Ordering contract:     none\n\
             Contract verification: static\n\
             External storage:      disabled\n\
             \n\
             Guarantee:\n\
             \x20 Memory is independent of the document's size under the configured\n\
             \x20 depth and scalar/key limits: nothing of the input is kept. The text\n\
             \x20 is the program's own, written under the output limit.\n\
             \n\
             Qualification:\n\
             \x20 The text is written only after the whole input has validated; an\n\
             \x20 invalid input writes nothing.\n"
        );
        let j = explain_json(&done);
        assert_eq!(j["finite"], true);
        assert_eq!(j["protocol"], json!(["Text"]));
        assert_eq!(j["readiness"], "scope-end");
        assert_eq!(
            j["qualification"],
            json!(["The text is written only after the whole input has validated; an invalid input writes nothing."])
        );
        // A finite text built of calls names them.
        let built = compile(
            "def export [input] (replace-text \"a\" \"b\" (concat \"x\" \"a\"))",
            "built.alc",
        )
        .unwrap();
        let text = explain(&built);
        assert!(
            text.starts_with("export: concat → replace-text\n"),
            "{text}"
        );
        assert!(text.contains("Protocol:              Text\n"), "{text}");
    }

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
        assert_eq!(
            crate::lower::tests::run(&lf, crate::lower::tests::RECORDS, true, None).unwrap(),
            "\"123\";\"Alice\";\"50.25\"\n\"456\";\"Bob\";\"72\"\n"
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
        }
    }

    #[test]
    fn duplicate_members_follow_the_policy_where_scopes_are_captured() {
        let program = compile(PROGRAM, "export.alc").unwrap();
        assert_eq!(
            summarize(&program).duplicates,
            "rejected in captured scopes (DUPLICATE_MEMBER)"
        );
        let last = program.with_duplicates(Duplicates::LastWins).unwrap();
        assert_eq!(
            explain_json(&last)["duplicates"],
            "the last value wins in captured scopes"
        );
        let first = program.with_duplicates(Duplicates::FirstWins).unwrap();
        assert_eq!(
            summarize(&first).duplicates,
            "the first value wins in captured scopes"
        );
        let identity = compile("def export [input] input", "id.alc").unwrap();
        assert_eq!(
            summarize(&identity).duplicates,
            "preserved; the events are copied as they arrive, not mapped by key"
        );
    }
}
