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
    // and pushes forms, a form opens the two delimited sequences. Every
    // repetition is a replace loop: a container pushes its first item
    // from its open phase, and a line and a form replace themselves with
    // the next; the one push left in a close phase is a line's block.
    let edge = |name: &str| {
        m.graph
            .iter()
            .find(|edges| edges.name == name)
            .unwrap_or_else(|| panic!("an edge entry for {name}"))
    };
    assert_eq!(edge("program").open_push, ["line"]);
    assert!(edge("program").close_push.is_empty());
    assert_eq!(edge("line").open_push, ["form"]);
    assert_eq!(edge("line").close_push, ["block"]);
    assert_eq!(edge("line").close_replace, ["line"]);
    assert_eq!(edge("block").open_push, ["line"]);
    assert!(edge("block").close_push.is_empty());
    let mut form_open = edge("form").open_push.clone();
    form_open.sort_unstable();
    assert_eq!(form_open, ["bracket", "paren"]);
    assert_eq!(edge("form").close_replace, ["form"]);
    for sequence in ["paren", "bracket"] {
        assert_eq!(edge(sequence).open_push, ["form"], "{sequence}");
        assert!(edge(sequence).close_push.is_empty(), "{sequence}");
    }
    for edges in &m.graph {
        assert!(edges.open_replace.is_empty(), "{}", edges.name);
    }

    // The structural tokens the matcher emits are registered.
    let tokens: Vec<&str> = m.tokens.iter().map(|token| token.name.as_str()).collect();
    for name in ["#IN", "#DE", "#NL", "#KW", "#OP", "#CP"] {
        assert!(tokens.contains(&name), "token {name} in {tokens:?}");
    }
}

/// The grammar portion of the model, serialised as a tool would receive
/// it and read back, still describes this grammar: the rules, how a line
/// closes, the tokens the layout matcher registered and which of the
/// engine's own lexers are on. A grammar change shows up here, in the
/// text, as it would to the tool.
#[test]
fn the_grammar_portion_serialises_and_reads_back_with_its_content() {
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

    let mut names: Vec<&str> = back["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .filter_map(|rule| rule["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["block", "bracket", "form", "line", "paren", "program"]
    );

    // A line opens on a form; it closes on `#IN` (pushing its block), on a
    // `#NL` right before `#DE` or `#ZZ` (taking the `#NL` alone), on `#NL`
    // (replacing itself with the next line), on `#DE` or `#ZZ` left for the
    // rule above, or, on the condition that it took a block, on the next
    // line's first token (replacing itself again).
    let line = back["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["name"] == "line")
        .expect("the line rule");
    let alts = |phase: &str| -> Vec<serde_json::Value> {
        line[phase]
            .as_array()
            .expect("alternates")
            .iter()
            .map(|alt| {
                serde_json::json!([
                    alt["seq"],
                    alt["push"],
                    alt["replace"],
                    alt["back"],
                    alt["cond"]
                ])
            })
            .collect()
    };
    assert_eq!(
        alts("open"),
        [serde_json::json!([[], "form", null, null, false])]
    );
    assert_eq!(
        alts("close"),
        [
            serde_json::json!([["#IN"], "block", null, null, false]),
            serde_json::json!([["#NL", ["#DE", "#ZZ"]], null, null, 1, false]),
            serde_json::json!([["#NL"], null, "line", null, false]),
            serde_json::json!([["#DE"], null, null, 1, false]),
            serde_json::json!([["#ZZ"], null, null, 1, false]),
            serde_json::json!([[], null, "line", null, true]),
        ]
    );

    // The layout matcher's tokens are registered, the delimiters are the
    // only fixed tokens, and the engine lexes strings and comments but
    // leaves words to the matcher.
    let tokens: Vec<(&str, Option<&str>)> = back["tokens"]
        .as_array()
        .expect("tokens")
        .iter()
        .filter_map(|token| Some((token["name"].as_str()?, token["fixed"].as_str())))
        .collect();
    for name in ["#IN", "#DE", "#NL", "#KW"] {
        assert!(tokens.contains(&(name, None)), "token {name} in {tokens:?}");
    }
    let fixed: Vec<(&str, &str)> = tokens
        .iter()
        .filter_map(|(name, fixed)| Some((*name, (*fixed)?)))
        .collect();
    assert_eq!(
        fixed,
        [("#OS", "["), ("#CS", "]"), ("#OP", "("), ("#CP", ")")]
    );
    assert_eq!(back["config"]["start"], "program");
    assert_eq!(
        back["config"]["lex"],
        serde_json::json!({
            "fixed": true, "space": true, "line": true, "text": false,
            "number": false, "comment": true, "string": true, "value": false,
        })
    );
    assert!(
        back["abnf"]
            .as_str()
            .is_some_and(|abnf| abnf.contains("line = form IN block")),
        "{}",
        back["abnf"]
    );
}
