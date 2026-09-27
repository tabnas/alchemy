// The shared fixtures in ../../test/spec, run through tabnas_support's
// runner as every grammar repository runs its own: the loader, the escape
// codec, the `ERROR:<code>` contract and the row loop are the fleet's.
//
// What is specific to this crate is what a row's input becomes: the
// canonical form of the parsed program (reader.tsv), or of the desugared
// program (pipe.tsv), as a JSON string in the expected column.

mod common;

use std::collections::BTreeSet;

use tabnas_alchemy::{canonical, desugar, format, parse, same_program};
use tabnas_support::{is_error_expect, load_spec_dir, Runner, SpecOptions};

use common::{spec_dir, text_value, to_failure};

/// Every fixture the directory holds has a runner below; a new file added
/// without one fails here rather than passing silently.
#[test]
fn every_fixture_has_a_runner() {
    let files: BTreeSet<String> = load_spec_dir(spec_dir(), &SpecOptions::default())
        .expect("the fixtures load")
        .into_iter()
        .map(|spec| spec.file)
        .collect();
    let expected: BTreeSet<String> = ["pipe.tsv", "reader.tsv"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(files, expected, "each fixture has a test in this file");
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
