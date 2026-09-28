// The reader repeats by replacement, never by a push chain: every
// repetition in the grammar is a replace loop (`r`), whose items all run in
// one frame, so rule depth follows a program's nesting and never its
// length. The engine's rule depth `d` is the observable, read here with a
// rule subscriber on `make()`: the maximum over ten thousand items of each
// repetition is what one item needs, while real nesting still grows it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The maximum rule depth `d` any rule reaches while `src` parses.
fn max_depth(src: &str) -> usize {
    let mut parser = tabnas_alchemy::make();
    let max = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&max);
    parser.subscribe_rules(move |rule, _context| {
        seen.fetch_max(rule.d, Ordering::Relaxed);
    });
    if let Err(error) = parser.parse(src) {
        panic!("{:?} does not parse: {error}", &src[..src.len().min(40)]);
    }
    max.load(Ordering::Relaxed)
}

const ITEMS: usize = 10_000;

/// Each repetition, as one item and as ten thousand, with the depth one
/// item needs: top-level lines, child lines under one line, a line's
/// inline forms (a head and ten thousand more), and the items of `( )`
/// and `[ ]`.
fn repetitions() -> Vec<(&'static str, String, String, usize)> {
    vec![
        ("top-level lines", "x".into(), "x\n".repeat(ITEMS), 2),
        (
            "child lines",
            "p\n  x".into(),
            format!("p\n{}", "  x\n".repeat(ITEMS)),
            4,
        ),
        (
            "inline forms",
            "f".into(),
            format!("f{}", " x".repeat(ITEMS)),
            2,
        ),
        (
            "items of ( )",
            "(x)".into(),
            format!("({})", "x ".repeat(ITEMS)),
            4,
        ),
        (
            "items of [ ]",
            "[x]".into(),
            format!("[{}]", "x ".repeat(ITEMS)),
            4,
        ),
    ]
}

#[test]
fn every_repetition_stays_at_the_depth_of_one_item() {
    for (name, one, many, depth) in repetitions() {
        assert_eq!(max_depth(&one), depth, "{name}: one item");
        assert_eq!(
            max_depth(&many),
            depth,
            "{name}: {ITEMS} items reach deeper than one"
        );
    }
}

/// Real recursion still nests: each `(` is a form and the list it opens,
/// two frames deeper than the one around it.
#[test]
fn nesting_still_grows_the_depth() {
    let nested = |levels: usize| format!("{}{}", "(".repeat(levels), ")".repeat(levels));
    assert_eq!(max_depth(&nested(1)), 3);
    assert_eq!(max_depth(&nested(10)), 21);
    assert_eq!(max_depth(&nested(100)), 201);
}

/// The installed grammar says the same: the only close-phase push is a
/// line's block (structure), and the two loops are the line and the form
/// replacing themselves. Every other push is a container taking its first
/// item, or a form opening a delimited list.
#[test]
fn the_repetitions_are_replace_loops() {
    let parser = tabnas_alchemy::make();
    let mut pushes = Vec::new();
    let mut replaces = Vec::new();
    for spec in parser.rule_specs() {
        for (phase, alts) in [("open", &spec.open), ("close", &spec.close)] {
            for alt in alts {
                if let Some(target) = &alt.p {
                    pushes.push(format!("{} {phase} p:{target}", spec.name));
                }
                if let Some(target) = &alt.r {
                    replaces.push(format!("{} {phase} r:{target}", spec.name));
                }
            }
        }
    }
    pushes.sort();
    pushes.dedup();
    replaces.sort();
    replaces.dedup();
    assert_eq!(
        pushes,
        [
            "block open p:line",
            "bracket open p:form",
            "form open p:bracket",
            "form open p:paren",
            "line close p:block",
            "line open p:form",
            "paren open p:form",
            "program open p:line",
        ]
    );
    assert_eq!(replaces, ["form close r:form", "line close r:line"]);
}

/// The fastest of a few parses of `src`, to keep a loaded machine's
/// stalls and a coarse clock out of the comparison.
fn fastest_parse(src: &str) -> Duration {
    let parser = tabnas_alchemy::make();
    (0..3)
        .map(|_| {
            let start = Instant::now();
            parser.parse(src).expect("the lines parse");
            start.elapsed()
        })
        .min()
        .expect("three runs")
}

/// Ten times the lines take about ten times as long. Quadratic work would
/// take a hundred; the bound is generous so a busy machine cannot fail it.
#[test]
fn parse_time_grows_linearly_with_the_lines() {
    let small = fastest_parse(&"f a 1\n".repeat(1_000));
    let large = fastest_parse(&"f a 1\n".repeat(10_000));
    let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-6);
    assert!(
        ratio < 30.0,
        "1,000 lines took {small:?} and 10,000 took {large:?}: {ratio:.1}x"
    );
}
