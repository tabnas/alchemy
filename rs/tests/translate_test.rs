//! Every route the translation module composes compiles with the standard
//! library's adapters, and its output is a text: the root adapters, the
//! inferred table over a wrapped root, an embed, a lift, a program's
//! events and its table. The parts here are stand-ins (a render that is
//! `json`, an embed that hands its input on); running real parts is
//! alchemy-cli's, which composes the routers and renderers.

mod common;

use std::sync::Arc;

use common::NoStages;
use tabnas_alchemy::translate::{
    compose, compose_program, Descriptor, Front, Options, Part, PartText,
};
use tabnas_alchemy::{Output, Source};

fn part(id: &str, translate: &str, lift: Option<PartText>, embed: Option<PartText>) -> Part {
    let render = if translate.contains("\"render\": \"csv\"") {
        PartText::new("csv", None)
    } else if translate.contains("\"render\": \"json\"") {
        PartText::new("json", None)
    } else {
        PartText::new(
            &format!("{id}-render"),
            Some(&format!("def {id}-render [input] (json input)")),
        )
    };
    Part::from_descriptor(&Descriptor {
        package: format!("tabnas-{id}"),
        manifest: format!(r#"{{"languageId": "{id}", "translate": {translate}}}"#),
        lift,
        embed,
        render: Some(render),
    })
    .unwrap_or_else(|| panic!("{id}: a part"))
}

fn tree(id: &str, root: &str) -> Part {
    part(
        id,
        &format!(
            r#"{{"reads": "tree", "writes": "tree", "root": "{root}", "render": "alchemy/render.alc"}}"#
        ),
        None,
        None,
    )
}

#[test]
fn every_route_compiles_to_a_text() {
    let o = Options::default();
    let csv = part(
        "csv",
        r#"{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}"#,
        None,
        None,
    );
    let md = part(
        "markdown",
        r#"{"reads": ["records", "tree"], "writes": "records", "root": "array", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}"#,
        Some(PartText::new(
            "markdown-lift",
            Some("def markdown-lift [input] (table-from-json (record (entry :columns :infer) (entry :rows (path each-index))) input)"),
        )),
        None,
    );
    let xml = part(
        "xml",
        r#"{"reads": "tree", "writes": "tree", "root": "any", "schema": "xml-element", "embed": "alchemy/embed.alc", "render": "alchemy/render.alc"}"#,
        None,
        Some(PartText::new(
            "xml-embed",
            Some("def xml-embed [input] (wrap-array input)"),
        )),
    );
    let targets = [
        tree("toml", "object"),
        tree("jsonl", "array"),
        tree("yaml", "any"),
        csv.clone(),
        xml.clone(),
    ];
    let sources: [Option<&Part>; 4] = [None, Some(&csv), Some(&md), Some(&xml)];
    for target in &targets {
        for source in sources {
            let c = compose(source, target, &o, "main").unwrap();
            let program = c
                .compile(None, Arc::new(NoStages), Arc::new(NoStages))
                .unwrap_or_else(|f| {
                    panic!(
                        "{} into {}: {f}\n{}",
                        source.map_or("tree", |p| &p.id),
                        target.id,
                        c.main
                    )
                });
            assert_eq!(program.output(), Output::Text, "{}", c.main);
        }
        for output in [Output::JsonEvents, Output::TableRows] {
            let c = compose_program(output, target, &o, "main").unwrap();
            let user = Source::new("p.alc", match output {
                Output::TableRows => "def export [input] (table-from-json (record (entry :columns :infer) (entry :rows (path each-index))) input)",
                _ => "def export [input] input",
            });
            let program = c
                .compile(Some(user), Arc::new(NoStages), Arc::new(NoStages))
                .unwrap_or_else(|f| panic!("{output:?} into {}: {f}\n{}", target.id, c.main));
            assert_eq!(program.output(), Output::Text, "{}", c.main);
            assert_eq!(c.front, Front::None);
        }
    }
}

/// A part whose render does not compile is a failure naming its file.
#[test]
fn a_render_that_does_not_compile_names_its_file() {
    let broken = Part::from_descriptor(&Descriptor {
        package: "tabnas-x".into(),
        manifest: r#"{"languageId": "x", "translate": {"reads": "tree", "writes": "tree", "render": "alchemy/render.alc"}}"#.into(),
        lift: None,
        embed: None,
        render: Some(PartText::new("x-render", Some("def x-render [input]\n  (nope input)"))),
    })
    .unwrap();
    let c = compose(None, &broken, &Options::default(), "main").unwrap();
    let fail = c
        .compile(None, Arc::new(NoStages), Arc::new(NoStages))
        .unwrap_err();
    assert!(fail.message.starts_with("unknown_name"), "{fail}");
    assert_eq!(fail.file.as_deref(), Some("tabnas-x/alchemy/render.alc"));
    assert_eq!((fail.row, fail.column), (Some(2), Some(4)));
}
