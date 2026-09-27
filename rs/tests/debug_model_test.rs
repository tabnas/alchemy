// Composition test: the alchemy grammar plugin layered with the
// tabnas-debug introspection plugin, as every grammar in the fleet
// carries. tabnas-debug is a dev-dependency on the sibling checkout, so
// the test fails to build when the checkout is missing rather than
// reporting green having run nothing.

use tabnas::Tabnas;
use tabnas_debug::{apply, model, DebugOptions};

/// An alchemy instance with the debug plugin layered on top, quiet:
/// introspection only, no `USE:` dump and no tracing.
fn build() -> Tabnas {
    let mut parser = tabnas_alchemy::make();
    apply(&mut parser, DebugOptions::quiet()).expect("the debug plugin installs over alchemy");
    parser
}

#[test]
fn parses_normally_with_the_debug_plugin_installed() {
    let parser = build();
    let value = parser
        .parse("def x [a]\n  f a 1")
        .expect("a definition parses");
    let json = value.to_json();
    assert_eq!(json[0]["$"], "list");
    assert_eq!(json[0]["items"][0]["name"], "def");
    assert_eq!(json[0]["items"][3]["$"], "list");
}

#[test]
fn the_model_is_the_structured_alchemy_grammar() {
    let parser = build();
    let m = model(&parser);

    let mut names: Vec<&str> = m.rules.iter().map(|rule| rule.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["block", "bracket", "form", "line", "paren", "program"]
    );
    assert_eq!(m.config.start, "program");

    // The layout matcher is the one custom lexer entry, below the first
    // built-in band. The model names a matcher by its `options.lex.match`
    // key; the function reference is not recoverable in Rust.
    let layout = m
        .lexer
        .iter()
        .find(|matcher| matcher.matcher == "alchemy")
        .expect("the layout matcher is listed");
    assert!(
        layout.order < 1e6,
        "order {} is below the bands",
        layout.order
    );

    // The rule graph: a program is lines, a line takes a block of lines
    // and pushes forms, a form opens the two delimited sequences.
    let edge = |name: &str| {
        m.graph
            .iter()
            .find(|edges| edges.name == name)
            .unwrap_or_else(|| panic!("an edge entry for {name}"))
    };
    assert_eq!(edge("program").open_push, ["line"]);
    assert_eq!(edge("program").close_push, ["line"]);
    assert_eq!(edge("line").open_push, ["form"]);
    let mut line_close = edge("line").close_push.clone();
    line_close.sort_unstable();
    line_close.dedup();
    assert_eq!(line_close, ["block", "form"]);
    assert_eq!(edge("block").open_push, ["line"]);
    let mut form_open = edge("form").open_push.clone();
    form_open.sort_unstable();
    assert_eq!(form_open, ["bracket", "paren"]);
    assert_eq!(edge("paren").close_push, ["form"]);
    assert_eq!(edge("bracket").close_push, ["form"]);

    // The structural tokens the matcher emits are registered.
    let tokens: Vec<&str> = m.tokens.iter().map(|token| token.name.as_str()).collect();
    for name in ["#IN", "#DE", "#NL", "#KW", "#OP", "#CP"] {
        assert!(tokens.contains(&name), "token {name} in {tokens:?}");
    }
}

#[test]
fn the_grammar_portion_serialises_and_round_trips() {
    let parser = build();
    let m = model(&parser);
    let grammar = serde_json::json!({
        "tokens": m.tokens,
        "rules": m.rules,
        "graph": m.graph,
        "config": m.config,
        "abnf": m.abnf,
    });
    let text = serde_json::to_string(&grammar).expect("the grammar portion serialises");
    let back: serde_json::Value = serde_json::from_str(&text).expect("and parses back");
    assert_eq!(back, grammar);
    assert_eq!(back["config"]["start"], "program");
}
