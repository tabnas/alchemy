//! Translation between formats, composed from per-format parts (admin
//! ADR-27; the design is tabnas/transduce's `docs/translation.md`).
//!
//! A format's package hands over its translation parts through a
//! structural descriptor (`translate()` in each runtime): its manifest,
//! `tabnas.plugin.json`, whose `translate` object names the shapes the
//! format reads as and writes from, the root its render needs, the schema
//! its events carry when they are not a plain tree, the loss its render
//! declares, and, where its documents are read whole, why; and the alchemy
//! text of its lift, its embed and its render. This module reads a
//! descriptor into a [`Part`] and composes the program that translates a
//! document of one format into another:
//!
//! 1. the source's events, through its lift when the target writes from
//!    records and the source reads as records first;
//! 2. for a target that writes from a tree: an embed into the target's
//!    schema when the target has one and the source's events are not of
//!    it (a schema-only target, one with no embed, refuses before any
//!    output), else the root adapter when the target's render needs an
//!    object or an array (`wrap-object`, `wrap-array`: the standard
//!    library's, which pass a root of the right kind through);
//! 3. for a target that writes from records, a tree's rows through the
//!    inferred table, its root an array (`wrap-array`), or a program's
//!    table as it is;
//! 4. the target's render: a part's own, or alchemy's `json` or `csv`.
//!
//! A program's output takes the source's place ([`compose_program`]), its
//! JSON events a tree and its table records. A program writing to a
//! schema-only target makes that schema's tree, so the refusal does not
//! run for it; into a target with an embed its output is a plain tree,
//! embedded like any source's.
//!
//! The composed program is a one-line `export` linked with the parts'
//! sources ([`Composition::compile`]). A host runs it as it runs any
//! program, and keeps a tree's contract in front of a render that writes
//! from one ([`Front`]). Nothing here knows a format by name: every
//! decision is the descriptors'.
//!
//! A format whose package hands over a transducer part (its manifest's
//! `translate.transduce`, read by [`Part::transducer`]) can be read from
//! the parser's rule passes instead of its parsed value's events: a host
//! that declares that input composes with [`compose_parsed`] (or
//! [`compose_program_parsed`]), whose main reads `ParserEvents/1`, makes
//! the format's events with the transducer, and holds them to a tree's
//! contract inside the program (`tree-contract`), where the host would
//! hold a source's events in front; [`Front`] is then `None`.

use std::sync::Arc;

use serde_json::Value;

use crate::program::{compile_sources_with, Input, Output, Program, Source};
use crate::shared::{Code, Duplicates, Fail, Renderers, Routers};
use crate::value::Val;

/// A shape a format reads as or writes from: the manifest's `reads` and
/// `writes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A document's events.
    Tree,
    /// A table's rows.
    Records,
}

impl Shape {
    fn parse(name: &str) -> Option<Shape> {
        match name {
            "tree" => Some(Shape::Tree),
            "records" => Some(Shape::Records),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shape::Tree => "tree",
            Shape::Records => "records",
        }
    }
}

/// The root a format's render needs: the manifest's `root`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    /// A table or a section map at the root (TOML, INI).
    Object,
    /// A sequence at the root (JSON Lines; and every render that writes
    /// from records, whose rows are the elements of the root array).
    Array,
    /// Any value.
    Any,
}

impl Root {
    fn parse(name: &str) -> Option<Root> {
        match name {
            "object" => Some(Root::Object),
            "array" => Some(Root::Array),
            "any" => Some(Root::Any),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Root::Object => "object",
            Root::Array => "array",
            Root::Any => "any",
        }
    }
}

/// One part as a format's package hands it over: the entry point a host
/// calls, and its alchemy source (none for a render alchemy carries).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartText {
    pub entry: String,
    pub source: Option<String>,
}

impl PartText {
    pub fn new(entry: &str, source: Option<&str>) -> PartText {
        PartText {
            entry: entry.to_string(),
            source: source.map(str::to_string),
        }
    }
}

/// A format package's structural descriptor: what its `translate()`
/// answers, and the name its parts are named by in diagnostics (the
/// package's, `tabnas-yaml`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub package: String,
    pub manifest: String,
    pub lift: Option<PartText>,
    pub embed: Option<PartText>,
    pub render: Option<PartText>,
}

/// An alchemy part: the file it is named by in diagnostics, its entry
/// point, and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alc {
    pub file: String,
    pub entry: String,
    pub text: String,
}

/// How a format is written: an alchemy render of its own, or one alchemy
/// carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Render {
    Alc(Alc),
    /// alchemy's `json`: the render crate's JSON renderer.
    Json,
    /// alchemy's `csv`, under the export's policies ([`CSV_OPTIONS`]).
    Csv,
}

/// A format's translation parts, as its manifest names them and its
/// package hands them over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// The manifest's `languageId`.
    pub id: String,
    /// What the format reads as, in order of preference; never empty.
    pub reads: Vec<Shape>,
    /// What the render writes from.
    pub writes: Shape,
    /// The root the render needs.
    pub root: Root,
    /// The tree the format's events carry, when not a plain one.
    pub schema: Option<String>,
    /// From the events to the first read shape, where they do not carry it.
    pub lift: Option<Alc>,
    /// From a plain tree into the schema; its file also holds the reverse.
    pub embed: Option<Alc>,
    pub render: Render,
    /// What a written document does not keep, a sentence each.
    pub loss: Vec<String>,
    /// Why the format's documents are read whole, never streamed, where the
    /// format says so: a sentence a host shows beside the loss.
    pub whole: Option<String>,
}

impl Part {
    /// The part a descriptor describes, or `None` when its manifest names
    /// no `translate` object (a format read and not written) or one this
    /// module cannot take: a shape or a root it does not know, a part the
    /// package does not hand over, a render that is neither an alchemy file
    /// nor a render alchemy carries.
    pub fn from_descriptor(d: &Descriptor) -> Option<Part> {
        let manifest: Value = serde_json::from_str(&d.manifest).ok()?;
        let id = manifest.get("languageId")?.as_str()?.to_string();
        let t = manifest.get("translate")?;
        let reads: Vec<Shape> = match t.get("reads")? {
            Value::String(one) => vec![Shape::parse(one)?],
            Value::Array(list) => list
                .iter()
                .map(|v| v.as_str().and_then(Shape::parse))
                .collect::<Option<Vec<Shape>>>()?,
            _ => return None,
        };
        if reads.is_empty() {
            return None;
        }
        let writes = Shape::parse(t.get("writes")?.as_str()?)?;
        let root = match t.get("root") {
            None => Root::Any,
            Some(v) => Root::parse(v.as_str()?)?,
        };
        let schema = match t.get("schema") {
            None => None,
            Some(v) => Some(v.as_str().filter(|s| !s.is_empty())?.to_string()),
        };
        let whole = match t.get("whole") {
            None => None,
            Some(v) => Some(v.as_str().filter(|s| !s.is_empty())?.to_string()),
        };
        let file = |path: &str| format!("{}/{path}", d.package);
        let alc = |key: &str, part: &Option<PartText>| -> Option<Option<Alc>> {
            match (t.get(key), part) {
                (None, None) => Some(None),
                (Some(Value::String(path)), Some(p))
                    if path.ends_with(".alc") && !p.entry.is_empty() =>
                {
                    Some(Some(Alc {
                        file: file(path),
                        entry: p.entry.clone(),
                        text: p.source.clone()?,
                    }))
                }
                _ => None,
            }
        };
        let lift = alc("lift", &d.lift)?;
        let embed = alc("embed", &d.embed)?;
        if embed.is_some() && schema.is_none() {
            return None;
        }
        let part = d.render.as_ref()?;
        if part.entry.is_empty() {
            return None;
        }
        let render = match t.get("render")?.as_str()? {
            "json" if part.entry == "json" && part.source.is_none() => Render::Json,
            "csv" if part.entry == "csv" && part.source.is_none() => Render::Csv,
            path if path.ends_with(".alc") => Render::Alc(Alc {
                file: file(path),
                entry: part.entry.clone(),
                text: part.source.clone()?,
            }),
            _ => return None,
        };
        let loss = t
            .get("loss")
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Some(Part {
            id,
            reads,
            writes,
            root,
            schema,
            lift,
            embed,
            render,
            loss,
            whole,
        })
    }

    /// The transducer part a descriptor's manifest names, its
    /// `translate.transduce`: the `.alc` file whose entry makes the
    /// format's JSON events of the parser's rule passes, with the part the
    /// package hands over for it (`transduce`, its entry and text).
    /// `Some(None)` when the manifest names none and none is handed over;
    /// `None` when the two disagree or the manifest's entry is one this
    /// module cannot take, as for a lift. [`Part::from_descriptor`] reads
    /// no transducer, so a host that does not read parser events composes
    /// as it did.
    pub fn transducer(d: &Descriptor, transduce: Option<&PartText>) -> Option<Option<Alc>> {
        let manifest: Value = serde_json::from_str(&d.manifest).ok()?;
        let named = manifest.get("translate")?.get("transduce");
        match (named, transduce) {
            (None, None) => Some(None),
            (Some(Value::String(path)), Some(p))
                if path.ends_with(".alc") && !p.entry.is_empty() =>
            {
                Some(Some(Alc {
                    file: format!("{}/{path}", d.package),
                    entry: p.entry.clone(),
                    text: p.source.clone()?,
                }))
            }
            _ => None,
        }
    }

    /// The part a program's output stands for, in a source's place: JSON
    /// events are a plain tree, a table is records, and a text is no
    /// source (`render_of_text`).
    pub fn of_output(output: Output) -> Result<Part, Fail> {
        let reads = match output {
            Output::JsonEvents => Shape::Tree,
            Output::TableRows => Shape::Records,
            Output::Text => {
                return Err(Fail::new(
                    Code::DslTypeError,
                    "render_of_text: the program renders its own text, which no render takes",
                ))
            }
        };
        Ok(Part {
            id: "program".to_string(),
            reads: vec![reads],
            writes: reads,
            root: Root::Any,
            schema: None,
            lift: None,
            embed: None,
            render: Render::Json,
            loss: Vec::new(),
            whole: None,
        })
    }
}

/// What a composition runs between the source's events and the render.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adapter {
    /// A root that is not an object, as the one member of one.
    WrapObject(String),
    /// A root that is not an array, as the one element of one.
    WrapArray,
    /// A plain tree into the target's schema, by its embed.
    Embed,
    /// A tree's rows as records: the inferred table.
    InferredTable,
    /// Records as a tree: one object per row, keyed by the labels.
    Records,
}

impl Adapter {
    pub fn name(&self) -> &'static str {
        match self {
            Adapter::WrapObject(_) => "wrap-object",
            Adapter::WrapArray => "wrap-array",
            Adapter::Embed => "embed",
            Adapter::InferredTable => "the inferred table",
            Adapter::Records => "records",
        }
    }

    /// What the adapter changes, the host's sentences, printed when it
    /// runs; an embed's are its format's own loss declaration.
    pub fn loss(&self) -> Vec<String> {
        match self {
            Adapter::WrapObject(key) => vec![format!(
                "A document whose root is not an object is written as the one member {} of an \
                 object, since the format's document is one.",
                quoted(key)
            )],
            Adapter::WrapArray => vec![
                "A document whose root is not an array is written as the one element of an \
                 array, since the format's document is a sequence."
                    .to_string(),
            ],
            Adapter::Embed => Vec::new(),
            Adapter::InferredTable => vec![
                "The rows are the elements of the root array: an object row's members are its \
                 cells, an array row's cells are its positions, and a scalar row is one cell \
                 named value."
                    .to_string(),
                "The columns are the first row's: a member or a cell a later row adds is not \
                 written, one it lacks is written empty, a member repeated in a row keeps its \
                 last value, and a row of another kind than the first has a cell only where the \
                 first row's columns find one."
                    .to_string(),
            ],
            Adapter::Records => vec![
                "Each row is written as an object keyed by the column labels: a cell the row \
                 lacks is an absent member, and of two columns with one label the last gives \
                 the member."
                    .to_string(),
            ],
        }
    }
}

/// What a host holds the events to in front of the composed program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    /// The source's events, which a render that writes from a tree takes:
    /// each key once per object, one root value (transduce's
    /// `TreeContract`).
    Tree,
    /// Nothing: the program's own events, or a table's rows.
    None,
}

/// How the composition may be run by a host's own renderer instead,
/// when the route is the identity into alchemy's `json`. The composed
/// `json` writes a number that is not finite as `null` (JSON has no
/// spelling for one, and the target declares the loss), so a host that
/// runs its own JSON renderer instead writes such a number as `null` too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Native {
    /// The source's events as they are, into the JSON renderer.
    Json,
}

/// The composition's options: what the host chooses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The member a root that is not an object is written under, for a
    /// target that needs an object.
    pub key: String,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            key: "items".to_string(),
        }
    }
}

/// The CSV options a composed `csv` runs under: the library's, with the
/// export's policy for an absent member, an empty field; a number that is
/// not finite written as its word, `Infinity`, `-Infinity` or `NaN`,
/// since every CSV cell is text; and a table of no columns (an empty
/// document, or rows of no members) written as the empty document.
pub const CSV_OPTIONS: &str = "(record (entry :delimiter \",\") (entry :newline \"\\r\\n\") \
                               (entry :header true) (entry :null-text \"\") (entry :missing \"\") \
                               (entry :non-finite :literal) (entry :no-columns :empty))";

/// What a composed `json` runs under: a number that is not finite, which
/// JSON has no spelling for, is written as `null`.
pub const JSON_OPTIONS: &str = "(record (entry :non-finite :null))";

/// The binding of the inferred table: the rows are the root array's
/// elements, the columns the first row's.
const INFERRED: &str = "(record (entry :columns :infer) (entry :rows (path each-index)))";

/// The name a program is linked under when its output feeds a render.
pub const PROGRAM_EXPORT: &str = "program-export";

/// A translation ready to compile: the parts' sources, the one-line main,
/// what runs between, and what the host keeps in front.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    /// Each part's (file, text), in link order: the render, the embed, the
    /// lift.
    pub sources: Vec<(String, String)>,
    /// The main's file name in diagnostics, and its text.
    pub main_file: String,
    pub main: String,
    pub adapters: Vec<Adapter>,
    pub front: Front,
    pub native: Option<Native>,
    /// The render's loss declaration, then each adapter's.
    pub loss: Vec<String>,
    /// What the main's `input` reads: JSON events, or the parser's rule
    /// passes for a composition over a transducer ([`compose_parsed`]).
    pub input: Input,
}

impl Composition {
    /// Link the parts with the main, and `program`'s source when the
    /// composition is over a program's output, into one program under the
    /// policies the adapters need: over a document's events the inferred
    /// table keeps a repeated member's last value, as an export does; over
    /// a transducer part's ([`compose_parsed`]), which streams every
    /// occurrence of a repeated member, it refuses one (`DUPLICATE_MEMBER`),
    /// as a tree's contract does, since what the grammar's value keeps,
    /// the last or a merge of the two, only the document read whole shows,
    /// and a host reads it so where the part's route refuses.
    pub fn compile(
        &self,
        program: Option<Source<'_>>,
        routers: Arc<dyn Routers<Val>>,
        renderers: Arc<dyn Renderers>,
    ) -> Result<Program, Fail> {
        let mut sources: Vec<Source<'_>> = self
            .sources
            .iter()
            .map(|(file, text)| Source::new(file, text))
            .collect();
        if let Some(p) = program {
            sources.push(p.export_as(PROGRAM_EXPORT));
        }
        sources.push(Source::new(&self.main_file, &self.main));
        let program = compile_sources_with(&sources, self.input, routers, renderers)?;
        if self.input == Input::JsonEvents && self.adapters.contains(&Adapter::InferredTable) {
            program.with_duplicates(Duplicates::LastWins)
        } else {
            Ok(program)
        }
    }
}

/// Compose the translation of a document read as `source` describes (a
/// plain tree when `None`: a format with no parts, or a value a host
/// selected below the root) into `target`. `main_file` names the main in
/// diagnostics.
pub fn compose(
    source: Option<&Part>,
    target: &Part,
    options: &Options,
    main_file: &str,
) -> Result<Composition, Fail> {
    compose_over(source, "input", target, options, main_file, Front::Tree)
}

/// Compose a program's output into `target`, in the source's place: JSON
/// events are a tree and a table is records ([`Part::of_output`]). The
/// program is linked under [`PROGRAM_EXPORT`] by
/// [`Composition::compile`].
///
/// A program writing to a schema-only target (a schema and no embed) makes
/// that schema's tree: its part takes the target's schema, so the refusal
/// does not run for it, since the program route is the only route into
/// such a target. Into a target with an embed its output is a plain tree,
/// embedded like any source's, and so is its table, which `records` makes
/// a plain tree. The shape adapters (`records`, the inferred table, and
/// the root adapters, which pass a root of the right kind through) apply as
/// they do for any source.
pub fn compose_program(
    output: Output,
    target: &Part,
    options: &Options,
    main_file: &str,
) -> Result<Composition, Fail> {
    let mut source = Part::of_output(output)?;
    if target.embed.is_none() {
        source.schema = target.schema.clone();
    }
    let input = format!("({PROGRAM_EXPORT} input)");
    compose_over(
        Some(&source),
        &input,
        target,
        options,
        main_file,
        Front::None,
    )
}

/// [`compose`] over the parser's rule passes: the host declares a
/// parser-events input, and the source's transducer (its part's
/// [`Part::transducer`]) makes the document's events of them. Where a host
/// would hold a source's events to a tree's contract in front (a render
/// that writes from a tree, the events reaching it whole), the main holds
/// the transducer's events to it instead, before the lift and the
/// adapters (`tree-contract`), since what the host pushes is parser
/// events; the [`Front`] is `None`. The main holds them to it for a
/// records target too, where a source's events go as they come: a part's
/// events are the document's only as far as the walk of the grammar's
/// value agrees, and a shape no tree has (a second root, which an
/// implicit list whose first element is a container streams) must reach
/// the host as the contract's refusal (STREAMABILITY_UNKNOWN), which it
/// reads the document whole on, not as the root adapter's
/// (PROTOCOL_ORDER_ERROR), which it does not. The composition is never
/// run by a host's own renderer ([`Native`]), which has no events to
/// render.
pub fn compose_parsed(
    source: &Part,
    transducer: &Alc,
    target: &Part,
    options: &Options,
    main_file: &str,
) -> Result<Composition, Fail> {
    let events = format!("(tree-contract ({} input))", transducer.entry);
    let mut composition = compose_over(
        Some(source),
        &events,
        target,
        options,
        main_file,
        Front::None,
    )?;
    composition.front = Front::None;
    composition.native = None;
    composition
        .sources
        .push((transducer.file.clone(), transducer.text.clone()));
    composition.input = Input::ParserEvents;
    Ok(composition)
}

/// [`compose_program`] over the parser's rule passes: the program reads
/// the events the source's transducer makes of them, as it reads a
/// source's events, its output then composed into `target` as for
/// [`compose_program`].
pub fn compose_program_parsed(
    output: Output,
    transducer: &Alc,
    target: &Part,
    options: &Options,
    main_file: &str,
) -> Result<Composition, Fail> {
    let mut source = Part::of_output(output)?;
    if target.embed.is_none() {
        source.schema = target.schema.clone();
    }
    let input = format!("({PROGRAM_EXPORT} ({} input))", transducer.entry);
    let mut composition = compose_over(
        Some(&source),
        &input,
        target,
        options,
        main_file,
        Front::None,
    )?;
    composition.native = None;
    composition
        .sources
        .push((transducer.file.clone(), transducer.text.clone()));
    composition.input = Input::ParserEvents;
    Ok(composition)
}

fn compose_over(
    source: Option<&Part>,
    input: &str,
    target: &Part,
    options: &Options,
    main_file: &str,
    front: Front,
) -> Result<Composition, Fail> {
    let tree = [Shape::Tree];
    let reads = source.map_or(&tree[..], |p| &p.reads[..]);
    let lift = source.and_then(|p| p.lift.as_ref());
    let source_schema = source.and_then(|p| p.schema.as_deref());
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut adapters: Vec<Adapter> = Vec::new();
    let mut expr = input.to_string();
    // Whether the source's events reach the render whole, no adapter
    // having taken them apart; only a tree's render reads it (the front,
    // below), so a records render leaves it as it is.
    let mut tree_events = true;
    match target.writes {
        Shape::Records => {
            if reads[0] == Shape::Records {
                // A format read as records first: through its lift, or as
                // its events are (a program's table).
                if let Some(l) = lift {
                    expr = format!("({} {expr})", l.entry);
                    sources.push((l.file.clone(), l.text.clone()));
                }
            } else {
                // A tree's rows: the elements of the root array.
                if target.root == Root::Array {
                    expr = format!("(wrap-array {expr})");
                    adapters.push(Adapter::WrapArray);
                }
                expr = format!("(table-from-json {INFERRED} {expr})");
                adapters.push(Adapter::InferredTable);
            }
        }
        Shape::Tree => {
            if !reads.contains(&Shape::Tree) {
                // Records only (a program's table): one object per row.
                expr = format!("(records {expr})");
                adapters.push(Adapter::Records);
                tree_events = false;
            }
            match target.schema.as_deref() {
                Some(schema) if source_schema != Some(schema) => match &target.embed {
                    Some(embed) => {
                        expr = format!("({} {expr})", embed.entry);
                        sources.push((embed.file.clone(), embed.text.clone()));
                        adapters.push(Adapter::Embed);
                    }
                    None => {
                        return Err(Fail::new(
                            Code::TargetValueUnrepresentable,
                            format!(
                            "schema_only: {} writes a {schema} tree, the tree its own documents \
                                 read as, and this document is not one; a program that makes one \
                                 can be composed with the render",
                            target.id
                        ),
                        ))
                    }
                },
                _ => match target.root {
                    Root::Object => {
                        expr = format!("(wrap-object {} {expr})", quoted(&options.key));
                        adapters.push(Adapter::WrapObject(options.key.clone()));
                    }
                    Root::Array => {
                        expr = format!("(wrap-array {expr})");
                        adapters.push(Adapter::WrapArray);
                    }
                    Root::Any => {}
                },
            }
        }
    }
    let native = match (&target.render, expr == input) {
        (Render::Json, true) => Some(Native::Json),
        _ => None,
    };
    let render = match &target.render {
        Render::Json => format!("json {JSON_OPTIONS}"),
        Render::Csv => format!("csv {CSV_OPTIONS}"),
        Render::Alc(alc) => {
            sources.insert(0, (alc.file.clone(), alc.text.clone()));
            alc.entry.clone()
        }
    };
    let main = format!("def export [input] ({render} {expr})");
    let mut loss = target.loss.clone();
    for adapter in &adapters {
        loss.extend(adapter.loss());
    }
    // The source's events reach a tree's render as a tree only when no
    // adapter took them apart first; a records render, and a program's
    // events, have nothing in front.
    let front = match (front, tree_events, target.writes) {
        (Front::Tree, true, Shape::Tree) => Front::Tree,
        _ => Front::None,
    };
    Ok(Composition {
        sources,
        main_file: main_file.to_string(),
        main,
        adapters,
        front,
        native,
        loss,
        input: Input::JsonEvents,
    })
}

/// A string as an alchemy string literal (JSON's escapes).
fn quoted(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(id: &str, translate: &str, render: Option<PartText>) -> Option<Part> {
        Part::from_descriptor(&Descriptor {
            package: format!("tabnas-{id}"),
            manifest: format!(r#"{{"languageId": "{id}", "translate": {translate}}}"#),
            lift: None,
            embed: None,
            render,
        })
    }

    fn own(id: &str) -> Option<PartText> {
        Some(PartText::new(
            &format!("{id}-render"),
            Some(&format!("def {id}-render [input] (json input)")),
        ))
    }

    fn tree(id: &str, root: &str) -> Part {
        part(
            id,
            &format!(
                r#"{{"reads": "tree", "writes": "tree", "root": "{root}", "render": "alchemy/render.alc"}}"#
            ),
            own(id),
        )
        .unwrap()
    }

    #[test]
    fn a_descriptor_reads_into_a_part() {
        let toml = tree("toml", "object");
        assert_eq!(toml.root, Root::Object);
        assert_eq!(toml.reads, vec![Shape::Tree]);
        let Render::Alc(alc) = &toml.render else {
            panic!("an alchemy render")
        };
        assert_eq!(alc.file, "tabnas-toml/alchemy/render.alc");
        assert_eq!(alc.entry, "toml-render");
        let json = part(
            "json",
            r#"{"reads": "tree", "writes": "tree", "render": "json"}"#,
            Some(PartText::new("json", None)),
        )
        .unwrap();
        assert_eq!((json.render.clone(), json.root), (Render::Json, Root::Any));
        // A root, a shape or a render this module cannot take is no part;
        // an embed with no schema is none either.
        assert!(part("x", r#"{"reads": "tree", "writes": "tree", "root": "table", "render": "alchemy/render.alc"}"#, own("x")).is_none());
        assert!(part(
            "x",
            r#"{"reads": "blob", "writes": "tree", "render": "alchemy/render.alc"}"#,
            own("x")
        )
        .is_none());
        assert!(part(
            "x",
            r#"{"reads": "tree", "writes": "tree", "render": "x.txt"}"#,
            own("x")
        )
        .is_none());
        assert!(part("x", r#"{"reads": "tree"}"#, own("x")).is_none());
        let embedded = Part::from_descriptor(&Descriptor {
            package: "tabnas-xml".into(),
            manifest: r#"{"languageId": "xml", "translate": {"reads": "tree", "writes": "tree", "root": "any", "embed": "alchemy/embed.alc", "render": "alchemy/render.alc"}}"#.into(),
            lift: None,
            embed: Some(PartText::new("xml-embed", Some("def xml-embed [input] input"))),
            render: own("xml"),
        });
        assert!(embedded.is_none(), "an embed names a schema");
    }

    /// A format whose documents are read whole says why in `whole`, a
    /// sentence; one that says nothing has none, and a `whole` that is not
    /// a sentence is a manifest this module cannot take.
    #[test]
    fn whole_is_the_sentence_a_manifest_gives() {
        let whole = |value: &str| {
            part(
                "toml",
                &format!(
                    r#"{{"reads": "tree", "writes": "tree", "render": "alchemy/render.alc", "whole": {value}}}"#
                ),
                own("toml"),
            )
        };
        let toml = whole(r#""A table may be defined after the tables that follow it.""#).unwrap();
        assert_eq!(
            toml.whole.as_deref(),
            Some("A table may be defined after the tables that follow it.")
        );
        assert_eq!(tree("json", "any").whole, None);
        assert!(whole(r#""""#).is_none());
        assert!(whole("true").is_none());
    }

    #[test]
    fn the_route_wraps_a_root_and_embeds_into_a_schema() {
        let o = Options::default();
        let toml = tree("toml", "object");
        let c = compose(None, &toml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (toml-render (wrap-object \"items\" input))"
        );
        assert_eq!(c.adapters, vec![Adapter::WrapObject("items".into())]);
        assert_eq!(c.front, Front::Tree);
        assert_eq!(c.sources[0].0, "tabnas-toml/alchemy/render.alc");
        let jsonl = tree("jsonl", "array");
        let c = compose(None, &jsonl, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (jsonl-render (wrap-array input))"
        );
        let yaml = tree("yaml", "any");
        let c = compose(None, &yaml, &o, "main").unwrap();
        assert_eq!(c.main, "def export [input] (yaml-render input)");
        assert!(c.adapters.is_empty());
        // A schema the source's events are not of: through the embed.
        let xml = Part {
            schema: Some("xml-element".into()),
            embed: Some(Alc {
                file: "tabnas-xml/alchemy/embed.alc".into(),
                entry: "xml-embed".into(),
                text: "def xml-embed [input] input".into(),
            }),
            ..tree("xml", "any")
        };
        let c = compose(None, &xml, &o, "main").unwrap();
        assert_eq!(c.main, "def export [input] (xml-render (xml-embed input))");
        assert_eq!(c.sources.len(), 2);
        // A source of that schema: as it is.
        let c = compose(Some(&xml), &xml, &o, "main").unwrap();
        assert_eq!(c.main, "def export [input] (xml-render input)");
        // A schema-only target refuses another tree before any output.
        let css = Part {
            schema: Some("css".into()),
            ..tree("css", "any")
        };
        let f = compose(None, &css, &o, "main").unwrap_err();
        assert_eq!(f.code, Code::TargetValueUnrepresentable);
        assert!(
            f.message.starts_with("schema_only: css writes a css tree"),
            "{f}"
        );
    }

    #[test]
    fn records_targets_take_a_trees_rows_and_a_lifted_formats_records() {
        let o = Options::default();
        let csv = part(
            "csv",
            r#"{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}"#,
            Some(PartText::new("csv", None)),
        )
        .unwrap();
        let c = compose(None, &csv, &o, "main").unwrap();
        assert_eq!(
            c.main,
            format!("def export [input] (csv {CSV_OPTIONS} (table-from-json {INFERRED} (wrap-array input)))")
        );
        assert_eq!(c.adapters, vec![Adapter::WrapArray, Adapter::InferredTable]);
        assert_eq!(c.front, Front::None);
        let md = Part::from_descriptor(&Descriptor {
            package: "tabnas-markdown".into(),
            manifest: r#"{"languageId": "markdown", "translate": {"reads": ["records", "tree"], "writes": "records", "root": "array", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}}"#.into(),
            lift: Some(PartText::new("markdown-lift", Some("def markdown-lift [input] input"))),
            embed: None,
            render: own("markdown"),
        })
        .unwrap();
        let c = compose(Some(&md), &csv, &o, "main").unwrap();
        assert_eq!(
            c.main,
            format!("def export [input] (csv {CSV_OPTIONS} (markdown-lift input))")
        );
        assert!(c.adapters.is_empty());
        // A lifted format into a tree's render: its tree as it is.
        let yaml = tree("yaml", "any");
        let c = compose(Some(&md), &yaml, &o, "main").unwrap();
        assert_eq!(c.main, "def export [input] (yaml-render input)");
        // JSON over the source's events as they are: the host's renderer.
        let json = part(
            "json",
            r#"{"reads": "tree", "writes": "tree", "render": "json"}"#,
            Some(PartText::new("json", None)),
        )
        .unwrap();
        let c = compose(None, &json, &o, "main").unwrap();
        assert_eq!(c.native, Some(Native::Json));
        // A number that is not finite is written as null, which JSON has
        // a spelling for.
        assert_eq!(
            c.main,
            "def export [input] (json (record (entry :non-finite :null)) input)"
        );
    }

    #[test]
    fn a_programs_output_stands_in_the_sources_place() {
        let o = Options::default();
        let toml = tree("toml", "object");
        let c = compose_program(Output::TableRows, &toml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (toml-render (wrap-object \"items\" (records (program-export input))))"
        );
        assert_eq!(c.front, Front::None);
        let c = compose_program(Output::JsonEvents, &toml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (toml-render (wrap-object \"items\" (program-export input)))"
        );
        let f = compose_program(Output::Text, &toml, &o, "main").unwrap_err();
        assert!(f.message.starts_with("render_of_text"), "{f}");
        let key = Options {
            key: "a \"b\"".into(),
        };
        let c = compose(None, &toml, &key, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (toml-render (wrap-object \"a \\\"b\\\"\" input))"
        );
        assert!(c.loss.iter().any(|l| l.contains("\"a \\\"b\\\"\"")));
    }

    /// A program writing to a schema-only target makes that schema's tree:
    /// such a target, which refuses another format's tree, takes a
    /// program's events into its render. Into a target with an embed a
    /// program's output is a plain tree, embedded like any source's, its
    /// table through `records`; the shape adapters apply as before.
    #[test]
    fn a_program_makes_a_schema_only_targets_tree() {
        let o = Options::default();
        let css = Part {
            schema: Some("css".into()),
            ..tree("css", "any")
        };
        let c = compose_program(Output::JsonEvents, &css, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (css-render (program-export input))"
        );
        assert!(c.adapters.is_empty());
        let c = compose_program(Output::TableRows, &css, &o, "main").unwrap();
        assert_eq!(c.adapters, vec![Adapter::Records]);
        // A root of the wrong kind is wrapped for a schema-only target too.
        let object = Part {
            schema: Some("x-tree".into()),
            ..tree("x", "object")
        };
        let c = compose_program(Output::JsonEvents, &object, &o, "main").unwrap();
        assert_eq!(c.adapters, vec![Adapter::WrapObject("items".into())]);
        let xml = Part {
            schema: Some("xml-element".into()),
            embed: Some(Alc {
                file: "tabnas-xml/alchemy/embed.alc".into(),
                entry: "xml-embed".into(),
                text: "def xml-embed [input] input".into(),
            }),
            ..tree("xml", "any")
        };
        let c = compose_program(Output::JsonEvents, &xml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (xml-render (xml-embed (program-export input)))"
        );
        assert_eq!(c.adapters, vec![Adapter::Embed]);
        assert_eq!(c.sources.len(), 2);
        let c = compose_program(Output::TableRows, &xml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (xml-render (xml-embed (records (program-export input))))"
        );
        assert_eq!(c.adapters, vec![Adapter::Records, Adapter::Embed]);
    }

    /// The json grammar's descriptor, naming a transducer part.
    fn json_descriptor() -> Descriptor {
        Descriptor {
            package: "tabnas-json".into(),
            manifest: r#"{"languageId": "json", "translate": {"reads": "tree", "writes": "tree", "transduce": "alchemy/transduce.alc", "render": "json"}}"#.into(),
            lift: None,
            embed: None,
            render: Some(PartText::new("json", None)),
        }
    }

    const JSON_TRANSDUCE: &str = "def json-transduce [input]\n  transduce-tree\n    record\n      entry :map (record (entry :container :object))\n      entry :list (record (entry :container :array))\n      entry :pair (record (entry :key 0))\n      entry :val (record (entry :scalar 0))\n    input\n";

    /// A manifest's transducer part is read with the text the package
    /// hands over, as a lift is, and only by `Part::transducer`:
    /// `from_descriptor` composes as it did whatever the manifest names.
    #[test]
    fn a_transducer_part_is_read_beside_the_descriptor() {
        let d = json_descriptor();
        let json = Part::from_descriptor(&d).expect("the part reads as it did");
        assert_eq!(json.render, Render::Json);
        let text = PartText::new("json-transduce", Some(JSON_TRANSDUCE));
        let t = Part::transducer(&d, Some(&text)).unwrap().unwrap();
        assert_eq!(
            (t.file.as_str(), t.entry.as_str()),
            ("tabnas-json/alchemy/transduce.alc", "json-transduce")
        );
        // Named and not handed over, handed over and not named, or not an
        // alchemy file: no part.
        assert_eq!(Part::transducer(&d, None), None);
        let plain = Descriptor {
            manifest: r#"{"languageId": "json", "translate": {"reads": "tree", "writes": "tree", "render": "json"}}"#.into(),
            ..json_descriptor()
        };
        assert_eq!(Part::transducer(&plain, None), Some(None));
        assert_eq!(Part::transducer(&plain, Some(&text)), None);
        let not_alc = Descriptor {
            manifest: d.manifest.replace("transduce.alc", "transduce.txt"),
            ..json_descriptor()
        };
        assert_eq!(Part::transducer(&not_alc, Some(&text)), None);
    }

    /// Over the parser's rule passes the main reads them, the transducer
    /// makes the format's events, and the tree's contract holds them where
    /// a host would hold a source's in front: nothing is in front, and no
    /// host renders them itself.
    #[test]
    fn a_transducer_composes_over_the_parsers_rule_passes() {
        use crate::lower::tests::Unlowered;
        let o = Options::default();
        let d = json_descriptor();
        let json = Part::from_descriptor(&d).unwrap();
        let text = PartText::new("json-transduce", Some(JSON_TRANSDUCE));
        let t = Part::transducer(&d, Some(&text)).unwrap().unwrap();

        let yaml = tree("yaml", "any");
        let c = compose_parsed(&json, &t, &yaml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (yaml-render (tree-contract (json-transduce input)))"
        );
        assert_eq!(
            (c.front, c.native, c.input),
            (Front::None, None, Input::ParserEvents)
        );
        assert_eq!(
            c.sources.last().unwrap().0,
            "tabnas-json/alchemy/transduce.alc"
        );
        let program = c
            .compile(None, Arc::new(Unlowered), Arc::new(Unlowered))
            .unwrap();
        assert_eq!(program.input(), Input::ParserEvents);
        let report = program.explain();
        assert!(
            report.contains("Protocol:              ParserEvents/1 → Stream<ParserEvent> → Stream<Value> → JsonEvents/1 → Text"),
            "{report}"
        );
        assert!(
            report.contains("Tree contract:         the keys of each open object, each dropped as its object closes"),
            "{report}"
        );
        assert!(
            report.contains("Duplicate members:     rejected where a tree's contract holds the events (DUPLICATE_MEMBER)"),
            "{report}"
        );

        // Into JSON: the contract, and no native route.
        let c = compose_parsed(&json, &t, &json, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (json (record (entry :non-finite :null)) (tree-contract (json-transduce input)))"
        );
        assert_eq!(c.native, None);
        // A root adapter takes the contract's events.
        let toml = tree("toml", "object");
        let c = compose_parsed(&json, &t, &toml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (toml-render (wrap-object \"items\" (tree-contract (json-transduce input))))"
        );
        // A records target takes the contract's events too, where it takes
        // a source's as they come: a second root is the contract's refusal,
        // not the root adapter's.
        let csv = part(
            "csv",
            r#"{"reads": "records", "writes": "records", "root": "array", "render": "csv"}"#,
            Some(PartText::new("csv", None)),
        )
        .unwrap();
        let c = compose_parsed(&json, &t, &csv, &o, "main").unwrap();
        assert!(
            c.main
                .contains("(wrap-array (tree-contract (json-transduce input)))"),
            "{}",
            c.main
        );
        assert_eq!(c.input, Input::ParserEvents);
        // The inferred table over a part's events refuses a repeated
        // member, where over a document's it keeps the last.
        let program = c
            .compile(None, Arc::new(Unlowered), Arc::new(Unlowered))
            .unwrap();
        assert_eq!(program.duplicates(), Duplicates::Reject);
        let program = compose(None, &csv, &o, "main")
            .unwrap()
            .compile(None, Arc::new(Unlowered), Arc::new(Unlowered))
            .unwrap();
        assert_eq!(program.duplicates(), Duplicates::LastWins);

        // A program's output over the transducer's events.
        let c = compose_program_parsed(Output::JsonEvents, &t, &yaml, &o, "main").unwrap();
        assert_eq!(
            c.main,
            "def export [input] (yaml-render (program-export (json-transduce input)))"
        );
        assert_eq!((c.front, c.input), (Front::None, Input::ParserEvents));
        let program = c
            .compile(
                Some(Source::new("p.alc", "def export [input] (events input)")),
                Arc::new(Unlowered),
                Arc::new(Unlowered),
            )
            .unwrap();
        assert_eq!(program.input(), Input::ParserEvents);
    }
}
