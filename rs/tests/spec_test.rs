// The shared fixtures in ../../test/spec, run through tabnas_support's
// runner as every grammar repository runs its own: the loader, the escape
// codec, the `ERROR:<code>` contract and the row loop are the fleet's.
//
// What is specific to this crate is what a row's input becomes: the
// canonical form of the parsed program (reader.tsv), or of the desugared
// program (pipe.tsv), as a JSON string in the expected column; or the plan
// report (check.tsv). The fourth fixture, run.tsv, holds the bytes a run
// writes: running needs the routers and renderers a host passes in,
// transduce's and render's, which this crate does not depend on, so
// tabnas-alchemy-cli runs it against this checkout (see
// `every_fixture_has_a_runner`).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use tabnas_alchemy::shared::{Code, Fail};
use tabnas_alchemy::{canonical, desugar, format, parse, same_program};
use tabnas_support::{
    is_error_expect, load_spec, load_spec_dir, parse_expect, Row, Runner, SpecOptions, Value,
};

use common::{compile, repo_root, spec_dir, text_value, to_failure};

/// The fixtures this file runs, each with its runner below.
const RUN_HERE: [&str; 3] = ["check.tsv", "pipe.tsv", "reader.tsv"];

/// The fixtures another repository runs, and why. A run.tsv row compiles a
/// program and runs it over a JSON document, which needs real routers and
/// renderers: transduce's and render's, which depend on this crate's
/// shared types, so this crate cannot depend on them. tabnas-alchemy-cli,
/// the composition root that depends on all three, runs every row (its
/// rs/tests/spec_test.rs, against this checkout as a sibling).
const RUN_ELSEWHERE: [(&str, &str); 1] = [(
    "run.tsv",
    "run by tabnas-alchemy-cli: a run needs transduce's routers and render's renderers",
)];

/// Every fixture the directory holds has a runner: one below, or, for the
/// files [`RUN_ELSEWHERE`] names, tabnas-alchemy-cli's, for the reason it
/// gives. A new file added without one fails here rather than passing
/// silently.
#[test]
fn every_fixture_has_a_runner() {
    let files: BTreeSet<String> = load_spec_dir(spec_dir(), &SpecOptions::default())
        .expect("the fixtures load")
        .into_iter()
        .map(|spec| spec.file)
        .collect();
    let expected: BTreeSet<String> = RUN_HERE
        .into_iter()
        .chain(RUN_ELSEWHERE.into_iter().map(|(file, _)| file))
        .map(str::to_string)
        .collect();
    assert_eq!(
        files, expected,
        "each fixture has a test in this file, or is run elsewhere: {RUN_ELSEWHERE:?}"
    );
    // And test/AGENTS.md, the fixtures' guide, describes each one.
    let guide = std::fs::read_to_string(repo_root().join("test/AGENTS.md"))
        .expect("test/AGENTS.md is readable");
    for file in &files {
        assert!(
            guide.contains(&format!("[`{file}`](spec/{file})")),
            "test/AGENTS.md does not describe {file}"
        );
    }
}

#[test]
fn reader() {
    Runner::new(|input| {
        parse(input)
            .map(|program| text_value(canonical(&program)))
            .map_err(|fail| to_failure(&fail))
    })
    .file(spec_dir().join("reader.tsv"));
}

#[test]
fn pipe() {
    Runner::new(|input| {
        parse(input)
            .and_then(|program| desugar::program(program, input))
            .map(|program| text_value(canonical(&program)))
            .map_err(|fail| to_failure(&fail))
    })
    .file(spec_dir().join("pipe.tsv"));
}

/// A program that checks prints its plan report; one that does not fails
/// with the resolver's or the checker's code, at the position it names.
#[test]
fn check() {
    Runner::new(|input| {
        compile(input, "check")
            .map(|program| text_value(program.explain()))
            .map_err(|fail| to_failure(&fail))
    })
    .file(spec_dir().join("check.tsv"));
}

/// The columns a run row reads by name (tabnas-alchemy-cli's runner reads
/// `doc` and `render`), so a renamed header fails here, where run.tsv is
/// kept, rather than reading empty cells there.
fn row_doc_columns_are_named(row: &Row) -> bool {
    row.index_of("doc").is_some() && row.index_of("render").is_some()
}

#[test]
fn run_fixture_has_its_columns() {
    let spec =
        load_spec(spec_dir().join("run.tsv"), &SpecOptions::default()).expect("run.tsv loads");
    assert_eq!(*spec.rows[0].header, ["input", "expected", "doc", "render"]);
    assert!(spec.rows.iter().all(row_doc_columns_are_named));
}

/// The failures a row's program meets, as the runners above meet them:
/// the reader's (`reader.tsv`), the desugarer's (`pipe.tsv`) or compile's
/// (`check.tsv`).
fn row_failures(file: &str, row: &Row) -> Vec<Fail> {
    let input = row.unesc(0);
    match file {
        "reader.tsv" => parse(&input).err().into_iter().collect(),
        "pipe.tsv" => parse(&input)
            .and_then(|program| desugar::program(program, &input))
            .err()
            .into_iter()
            .collect(),
        "check.tsv" => compile(&input, "check").err().into_iter().collect(),
        other => panic!("{other} has no runner in this file"),
    }
}

/// The fixed parts of a template, between and around its `{name}`s.
fn fixed_parts(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut parts = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let name = bytes[i + 1..]
                .iter()
                .take_while(|b| b.is_ascii_lowercase() || **b == b'_')
                .count();
            if name > 0 && bytes.get(i + 1 + name) == Some(&b'}') {
                parts.push(&line[start..i]);
                i += name + 2;
                start = i;
                continue;
            }
        }
        i += 1;
    }
    parts.push(&line[start..]);
    parts
}

/// Whether `text` fills the template `line`: its fixed parts in order,
/// each `{name}` any text, none included.
fn fills(line: &str, text: &str) -> bool {
    let parts = fixed_parts(line);
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if parts.len() == 1 {
        return text == line;
    }
    if text.len() < first.len() + last.len() || !text.starts_with(first) || !text.ends_with(last) {
        return false;
    }
    let end = text.len() - last.len();
    let mut at = first.len();
    for part in &parts[1..parts.len() - 1] {
        match text[at..end].find(part) {
            Some(found) => at += found + part.len(),
            None => return false,
        }
    }
    true
}

/// Whether `text` is an instance of the template `line`. The engine trims
/// the messages it writes from a template, so a `{name}` that ends a line
/// may take the space before it with it.
fn instance_of(line: &str, text: &str) -> bool {
    if fills(line, text) {
        return true;
    }
    match (fixed_parts(line).last(), line.rfind('{')) {
        (Some(&""), Some(open)) if line.ends_with('}') => fills(line[..open].trim_end(), text),
        _ => false,
    }
}

/// Every failure the shared fixtures this file runs meet (reader.tsv,
/// pipe.tsv and check.tsv) is declared in the grammar document: the finer
/// code that leads the message is a key of the installed `options.error`,
/// the engine's and this grammar's, and the text after it is a line of
/// that entry, each `{name}` standing for what the raising site fills in
/// (a failure raised inside the standard library ends with its position
/// there, ` (at stdlib/...)`, which is not part of the text). Each code
/// this grammar adds to the engine's has a hint. A finer code travels with
/// one code wherever it is raised (`bad_let` is a `DSL_PARSE_ERROR` from
/// the desugarer and from the resolver alike). A raising site whose code or
/// text drifts from the document fails here.
///
/// That each line of each code this grammar adds is met by some row, so
/// the document holds no text nothing raises, needs run.tsv's rows as
/// well: tabnas-alchemy-cli's copy of this test runs every error row of
/// all four fixtures, these checks and that one.
#[test]
fn the_raised_messages_match_the_document() {
    let installed = tabnas_alchemy::make().config();
    let engine = tabnas::Tabnas::new().config().error;
    let mut codes: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut failures = 0;
    for file in RUN_HERE {
        let spec =
            load_spec(spec_dir().join(file), &SpecOptions::default()).expect("the fixture loads");
        for row in &spec.rows {
            if !is_error_expect(row.col(1)) {
                continue;
            }
            for fail in row_failures(&spec.file, row) {
                if !matches!(
                    fail.code,
                    Code::DslParseError
                        | Code::DslTypeError
                        | Code::StreamReused
                        | Code::StreamabilityUnknown
                ) {
                    continue;
                }
                failures += 1;
                let Some((code, text)) = fail.message.split_once(": ") else {
                    problems.push(format!(
                        "{}: {} has no finer code: {}",
                        row.location(),
                        fail.code.as_str(),
                        fail.message
                    ));
                    continue;
                };
                codes
                    .entry(code.to_string())
                    .or_default()
                    .insert(fail.code.as_str());
                let text = match text.rfind(" (at stdlib/") {
                    Some(library) if text.ends_with(')') => &text[..library],
                    _ => text,
                };
                let Some(entry) = installed.error.get(code) else {
                    problems.push(format!(
                        "{}: {code} is not declared in options.error",
                        row.location()
                    ));
                    continue;
                };
                if !entry.split('\n').any(|line| instance_of(line, text)) {
                    problems.push(format!(
                        "{}: no line of options.error.{code} is {text:?}",
                        row.location()
                    ));
                }
            }
        }
    }
    assert!(failures > 0, "the fixtures meet failures");
    for (code, uppers) in &codes {
        if uppers.len() > 1 {
            problems.push(format!(
                "{code} is raised as {}; a finer code has one code",
                uppers.iter().copied().collect::<Vec<_>>().join(" and ")
            ));
        }
    }
    let own: BTreeMap<&String, &String> = installed
        .error
        .iter()
        .filter(|(code, _)| !engine.contains_key(*code))
        .collect();
    assert!(!own.is_empty(), "the grammar declares codes of its own");
    for code in own.keys() {
        if !installed.hint.contains_key(*code) {
            problems.push(format!("options.hint.{code} is not declared"));
        }
    }
    assert!(
        problems.is_empty(),
        "{} problem(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// `format` prints a program the reader reads back to the same forms,
/// spans aside, for every program the fixtures parse. Failures are
/// collected across every row so one report names them all.
#[test]
fn format_round_trips_every_fixture_row() {
    let mut failures = Vec::new();
    let mut rows = 0;
    for spec in load_spec_dir(spec_dir(), &SpecOptions::default()).expect("the fixtures load") {
        for row in &spec.rows {
            if is_error_expect(row.col(1)) {
                continue;
            }
            rows += 1;
            let input = row.unesc(0);
            let Ok(program) = parse(&input) else {
                // The fixture runner reports a parse that should succeed.
                continue;
            };
            let layout = format(&program);
            match parse(&layout) {
                Ok(again) if same_program(&program, &again) => {}
                Ok(again) => failures.push(format!(
                    "{}: format changed the program\n  input:  {input:?}\n  layout: {layout:?}\n  read:   {}\n  was:    {}",
                    row.location(),
                    canonical(&again),
                    canonical(&program)
                )),
                Err(fail) => failures.push(format!(
                    "{}: the layout form does not parse: {fail}\n  layout: {layout:?}",
                    row.location()
                )),
            }
        }
    }
    assert!(rows > 0, "the fixtures hold value rows");
    assert!(
        failures.is_empty(),
        "{} row(s) do not round-trip:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The fenced examples of `docs/language.md`, each `alchemy` block with
/// the `canonical`, `core` or `check` block after it, are fixture rows:
/// the input is a row of reader.tsv (canonical), pipe.tsv (core) or
/// check.tsv (check: the plan report, or the error), and the result shown
/// is that row's expected value. A stale example fails here.
#[test]
fn every_language_reference_example_is_a_fixture_row() {
    let doc = std::fs::read_to_string(repo_root().join("docs/language.md"))
        .expect("docs/language.md is readable");
    let rows = |file: &str| -> BTreeMap<String, String> {
        load_spec(spec_dir().join(file), &SpecOptions::default())
            .expect("the fixture loads")
            .rows
            .iter()
            .map(|row| {
                (
                    row.unesc(0).trim_end_matches('\n').to_string(),
                    row.col(1).to_string(),
                )
            })
            .collect()
    };
    let reader = rows("reader.tsv");
    let pipe = rows("pipe.tsv");
    let check = rows("check.tsv");

    // The fences, in order: (language, text).
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut open: Option<(String, Vec<&str>)> = None;
    for line in doc.lines() {
        match (&mut open, line.strip_prefix("```")) {
            (None, Some(language)) if !language.is_empty() => {
                open = Some((language.to_string(), Vec::new()));
            }
            (Some((language, text)), Some("")) => {
                blocks.push((language.clone(), text.join("\n")));
                open = None;
            }
            (Some((_, text)), _) => text.push(line),
            (None, _) => {}
        }
    }

    let mut examples = 0;
    let mut failures = Vec::new();
    for (index, (language, input)) in blocks.iter().enumerate() {
        if language != "alchemy" {
            continue;
        }
        examples += 1;
        let (fixture, expected) = match blocks.get(index + 1) {
            Some((kind, text)) if kind == "canonical" => (&reader, text),
            Some((kind, text)) if kind == "core" => (&pipe, text),
            Some((kind, text)) if kind == "check" => (&check, text),
            _ => {
                failures.push(format!(
                    "{input:?}: an alchemy block is followed by a canonical, core or check block"
                ));
                continue;
            }
        };
        let Some(cell) = fixture.get(input.trim_end_matches('\n')) else {
            failures.push(format!("{input:?}: not a fixture row"));
            continue;
        };
        let shown = if is_error_expect(cell) {
            cell.clone()
        } else {
            match parse_expect(cell) {
                Ok(Value::String(text)) => text,
                other => {
                    failures.push(format!("{input:?}: the fixture expects {other:?}"));
                    continue;
                }
            }
        };
        // A report ends with a line feed; a fenced block has none.
        if shown.trim_end_matches('\n') != *expected {
            failures.push(format!(
                "{input:?}: the page shows {expected:?}, the fixture pins {shown:?}"
            ));
        }
    }
    assert!(examples > 0, "the reference holds examples");
    assert!(
        failures.is_empty(),
        "{} example(s) disagree with the fixtures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every definition of the standard library appears on the reference page
/// as it is in `stdlib/*.alc`, word for word: the page shows the library's
/// text, not a variant of it.
#[test]
fn the_library_definitions_on_the_page_are_the_library_text() {
    let doc = std::fs::read_to_string(repo_root().join("docs/language.md"))
        .expect("docs/language.md is readable");
    let mut shown = 0;
    for (file, src) in tabnas_alchemy::stdlib::SOURCES {
        // A definition runs from its `def` line to the line before the
        // next blank line (comments stay out).
        let mut defs: Vec<Vec<&str>> = Vec::new();
        for line in src.lines() {
            if line.starts_with("def ") {
                defs.push(vec![line]);
            } else if line.trim().is_empty() || line.starts_with(';') {
                if let Some(last) = defs.last() {
                    if !last.is_empty() && last.last() != Some(&"") {
                        defs.push(Vec::new());
                    }
                }
            } else if let Some(last) = defs.last_mut() {
                last.push(line);
            }
        }
        for def in defs.iter().filter(|d| !d.is_empty()) {
            let block = format!("```alchemy\n{}\n```", def.join("\n"));
            assert!(
                doc.contains(&block),
                "{file}: docs/language.md does not show {:?} as the library has it",
                def[0]
            );
            shown += 1;
        }
    }
    assert_eq!(shown, tabnas_alchemy::stdlib::stdlib().names().count());
}
