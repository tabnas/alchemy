// The structural translation interface's text-shape composition: a
// program and a render part linked into one namespace. The fleet
// conformance over every grammar's own translation parts, which renders
// each sample on transduce's routers and render's renderers, is in
// tabnas-alchemy-cli's translation_parts_test.rs.

mod common;

use tabnas_alchemy::Source;

use common::compile_sources;

#[derive(Clone, Copy)]
struct StructuralPart {
    entry: &'static str,
    source: Option<&'static str>,
}

struct StructuralParts {
    // Read by tabnas-alchemy-cli's copy, which checks each grammar's
    // descriptor; the composition here only names one.
    #[allow(dead_code)]
    manifest: &'static str,
    lift: Option<StructuralPart>,
    render: Option<StructuralPart>,
}

fn conformance_main(parts: &StructuralParts, reads: &str, writes: &str, lifted: bool) -> String {
    let render = parts.render.expect("the descriptor declares a render");
    // The producer is structural fact: only the preferred shape may come
    // from a lift; every other shape is the grammar's raw tree. The adapter
    // is selected from the descriptor, so a false reads value type-fails.
    let source = if lifted {
        format!(
            "{} (events input)",
            parts.lift.expect("a lifted read shape has a lift").entry
        )
    } else {
        "events input".to_string()
    };
    let mut definitions = String::new();
    let adapted = match (reads, writes) {
        (reads, writes) if reads == writes => source,
        ("tree", "records") => {
            definitions.push_str(concat!(
                "def conformance-binding\n  record\n",
                "    entry :columns :infer\n",
                "    entry :rows (path each-index)\n\n",
            ));
            format!("table-from-json conformance-binding ({source})")
        }
        ("records", "tree") => format!("records ({source})"),
        (_, "text") => {
            definitions.push_str(concat!(
                "def conformance-text [input]\n",
                "  join \"\"\n",
                "    map\n",
                "      fn [event]\n",
                "        match event\n",
                "          case (scalar value) (scalar-text csv-options value)\n",
                "          case _ \"\"\n",
                "      input\n\n",
            ));
            format!("conformance-text ({source})")
        }
        _ => panic!("no {reads}-to-{writes} conformance adapter"),
    };
    let call = if render.entry == "csv" {
        format!("csv csv-options ({adapted})")
    } else {
        format!("{} ({adapted})", render.entry)
    };
    format!("{definitions}def export [input]\n  {call}\n")
}

#[test]
fn text_shape_composition_compiles() {
    let parts = StructuralParts {
        manifest: "{}",
        lift: None,
        render: Some(StructuralPart {
            entry: "textual-render",
            source: Some("def textual-render [input]\n  input\n"),
        }),
    };
    let main = conformance_main(&parts, "tree", "text", false);
    let sources = [
        Source::new("conformance.alc", &main),
        Source::new(
            "alchemy/render.alc",
            parts.render.expect("render").source.expect("source"),
        ),
    ];
    let program = compile_sources(&sources).expect("text composition compiles");
    assert!(program.resolved().get("textual-render").is_some());
}
