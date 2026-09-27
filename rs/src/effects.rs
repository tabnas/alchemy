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
use tabnas_transduce::Selector;

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
    /// The CSV renderer: always quoted, CRLF, header; `host` when the host
    /// chose it for a table result.
    Csv { host: bool },
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
    /// The calls on the data-last spine of the entry's body, innermost
    /// first: `api-table → csv`.
    pub chain: Vec<String>,
    pub passes: u8,
    pub protocol: Vec<String>,
    pub selection: String,
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
            Plan::Concat(items) => match items.iter().find(|v| v.is_live()) {
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
        Plan::TableFromJson { .. } => "TableRows/1",
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
            Plan::TableFromJson { .. } | Plan::Csv { .. } | Plan::Records { .. }
        )
    }) || output == Output::TableRows
}

/// The summary of a compiled program.
pub fn summarize(program: &Program) -> EffectSummary {
    let output = program.output();
    let plan: Option<&Plan> = match program.result() {
        Val::Stream(p) | Val::Text(p) => Some(p),
        _ => None,
    };
    let stages: Vec<&Plan> = plan.map(stages).unwrap_or_default();
    let chain = program
        .resolved()
        .get("export")
        .and_then(|def| match &*def.value {
            Expr::List { items, .. } if items.len() == 3 => Some(chain_of(&items[2])),
            _ => None,
        })
        .unwrap_or_default();

    let mut protocol: Vec<String> = vec!["JsonEvents/1".to_string()];
    for stage in stages.iter().skip(1) {
        let p = protocol_of(stage);
        if protocol.last().map(String::as_str) != Some(p) {
            protocol.push(p.to_string());
        }
    }
    let renderer = match (stages.last(), output) {
        (Some(Plan::Csv { .. }), _) => RendererProfile::Csv { host: false },
        (Some(Plan::Json { .. }), _) => RendererProfile::Json { host: false },
        (_, Output::TableRows) => {
            protocol.push("Text".to_string());
            RendererProfile::Csv { host: true }
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
                        limit: Some("max_capture_bytes"),
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
                    reason: "what the step returns; bounded by the program, not by a limit"
                        .to_string(),
                    selector: None,
                    limit: None,
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
        "Memory is independent of the number of rows under the configured\n  depth, capture, scalar/key and output limits, provided the state the\n  step returns stays bounded; the runtime does not cap it.".to_string()
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

    EffectSummary {
        entry: "export".to_string(),
        chain,
        passes: 1,
        protocol,
        selection,
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
        let mut lines: Vec<(String, String)> = Vec::new();
        lines.push(("Source reads:".into(), self.passes.to_string()));
        lines.push(("Protocol:".into(), self.protocol.join(" → ")));
        lines.push(("Selection:".into(), self.selection.clone()));
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
            RendererProfile::Csv { host } => lines.push((
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
        out.push_str(if self.chain.is_empty() { "input" } else { "" });
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
            RendererProfile::Csv { host } => {
                json!({"name": "csv", "quoting": "always", "newline": "\r\n", "header": true, "host": host})
            }
            RendererProfile::Json { host } => {
                json!({"name": "json", "indent": null, "trailing_newline": true, "host": host})
            }
            RendererProfile::Text => json!({"name": "text"}),
        };
        json!({
            "entry": self.entry,
            "chain": self.chain,
            "output": self.output.as_str(),
            "passes": self.passes,
            "protocol": self.protocol,
            "selection": self.selection,
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
            "qualification": self.qualification,
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
        assert_eq!(
            s.protocol,
            ["JsonEvents/1", "Stream<Selected>", "Stream<Value>", "Text"]
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
        assert!(
            text.contains(
                "Capture:               .response.metadata.fields, capped at max_capture_bytes\n"
            ),
            "{text}"
        );
        assert!(text.contains("Retained state:        what the step returns; bounded by the program, not by a limit\n"), "{text}");
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
}
