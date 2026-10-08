// `compile_sources`: several sources linked into one program, as a host
// links a format's parts (libraries of definitions prefixed by the
// format's name, with no `export`) with the program that calls them. A
// failure in any source carries that source's file, row and column, in
// the field and in the failure's display, at every stage compiling
// positions one: the reader, the desugarer, the resolver and the checker;
// and the linking refuses what it cannot link. Linked programs run, and a
// run's failure in a part, are in tabnas-alchemy-cli's sources_test.rs,
// since running needs transduce's routers and render's renderers.

mod common;

use tabnas_alchemy::shared::{Code, Fail};
use tabnas_alchemy::{Program, Source};

use common::{compile, compile_sources};

/// A render part: definitions prefixed by the format's name, no `export`.
const PART: &str = "; A part of the lines format: each item quoted, one to a line.
def lines-line [item]
  concat (quoted item) \"\\n\"

def lines-render [items]
  concat-map lines-line items
";

const PART_FILE: &str = "lines/render.alc";

/// The program that calls the part.
const MAIN: &str = "def export [input]
  lines-render (select (path each-index) input)
";

const MAIN_FILE: &str = "main.alc";

fn sources<'a>(list: &[(&'a str, &'a str)]) -> Vec<Source<'a>> {
    list.iter()
        .map(|&(file, text)| Source::new(file, text))
        .collect()
}

/// The part second, as the design's diagnostics test has it.
fn linked(part: &str) -> Result<Program, Fail> {
    compile_sources(&sources(&[(MAIN_FILE, MAIN), (PART_FILE, part)]))
}

/// The 1-based row and column of `needle` on the 1-based `row` of `text`,
/// as a failure carries them.
fn at(text: &str, row: usize, needle: &str) -> (Option<u64>, Option<u64>) {
    let line = text.lines().nth(row - 1).expect("the row is in the text");
    let byte = line.find(needle).expect("the needle is on the row");
    (
        Some(row as u64),
        Some(line[..byte].chars().count() as u64 + 1),
    )
}

/// The failure names `file` and the position `(row, column)`, in the
/// fields and in its display.
fn assert_at(fail: &Fail, file: &str, (row, column): (Option<u64>, Option<u64>)) {
    assert_eq!(fail.file.as_deref(), Some(file), "{fail}");
    assert_eq!((fail.row, fail.column), (row, column), "{fail}");
    let shown = format!("({file}:{}:{})", row.unwrap(), column.unwrap());
    assert!(
        fail.to_string().ends_with(&shown),
        "{fail} does not end {shown}"
    );
    assert_eq!(fail.to_json()["file"], file, "{fail}");
}

/// Every stage that positions a failure positions one in the second
/// source by that source's own rows and columns, and names its file.
#[test]
fn a_failure_in_the_second_source_names_the_second_file() {
    // The resolver: a name nothing defines.
    let typo = PART.replace("(quoted item)", "(quoted itme)");
    let fail = linked(&typo).unwrap_err();
    assert_eq!(fail.code, Code::DslTypeError, "{fail}");
    assert!(fail.message.starts_with("unknown_name: "), "{fail}");
    assert_at(&fail, PART_FILE, at(&typo, 3, "itme"));

    // The checker: an argument of the wrong type.
    let number = PART.replace("(quoted item)", "(quoted 1)");
    let fail = linked(&number).unwrap_err();
    assert_eq!(fail.code, Code::DslTypeError, "{fail}");
    assert!(fail.message.starts_with("type_mismatch: "), "{fail}");
    assert_at(&fail, PART_FILE, at(&number, 3, "1)"));

    // The checker, following a stream into the part: a render handed the
    // wrong shape fails at the render's file and line, not at the call.
    let csv = "def lines-render [items]\n  csv csv-options items\n";
    let events = "def export [input]\n  lines-render (events input)\n";
    let fail = compile_sources(&sources(&[(MAIN_FILE, events), (PART_FILE, csv)])).unwrap_err();
    assert_eq!(fail.code, Code::DslTypeError, "{fail}");
    assert!(fail.message.starts_with("protocol_mismatch: "), "{fail}");
    assert_eq!(fail.file.as_deref(), Some(PART_FILE), "{fail}");
    assert_eq!(fail.row, Some(2), "{fail}");

    // The reader: a list left open.
    let open = PART.replace("(quoted item)", "(quoted item");
    let fail = linked(&open).unwrap_err();
    assert_eq!(fail.code, Code::DslParseError, "{fail}");
    assert_eq!(fail.file.as_deref(), Some(PART_FILE), "{fail}");
    assert!(fail.row.is_some(), "{fail}");
    assert!(
        fail.to_string().contains(&format!("({PART_FILE}:")),
        "{fail}"
    );

    // The desugarer: a `pipe` step that is no call.
    let pipe = PART.replace("concat-map lines-line items", "pipe items ()");
    let fail = linked(&pipe).unwrap_err();
    assert_eq!(fail.code, Code::DslParseError, "{fail}");
    assert!(fail.message.starts_with("empty_step: "), "{fail}");
    assert_eq!(fail.file.as_deref(), Some(PART_FILE), "{fail}");
    assert_eq!(fail.row, Some(6), "{fail}");
}

/// The linking's own refusals: a name defined in two sources, the one
/// file name given twice, and no `export` in any source.
#[test]
fn the_linking_refuses_a_collision_and_a_program_with_no_export() {
    // A definition in both: the second is refused, and the message says
    // where the first is.
    let twice = format!("{PART}\ndef export [input] (json input)\n");
    let fail = linked(&twice).unwrap_err();
    assert_eq!(fail.code, Code::DslTypeError, "{fail}");
    assert!(
        fail.message
            .starts_with("duplicate_def: export is defined twice; first at main.alc:1:1"),
        "{fail}"
    );
    assert_at(&fail, PART_FILE, at(&twice, 8, "def export"));

    // Each source has its own name.
    let fail = compile_sources(&sources(&[(MAIN_FILE, MAIN), (MAIN_FILE, PART)])).unwrap_err();
    assert_eq!(fail.code, Code::DslTypeError, "{fail}");
    assert!(
        fail.message.starts_with("duplicate_file: main.alc "),
        "{fail}"
    );
    assert_eq!((fail.file, fail.row), (None, None));

    // A part alone is no program.
    let fail = compile_sources(&sources(&[(PART_FILE, PART)])).unwrap_err();
    assert!(fail.message.starts_with("no_export: "), "{fail}");
    let fail = compile_sources(&[]).unwrap_err();
    assert!(fail.message.starts_with("no_export: "), "{fail}");
}

/// One source is `compile`: its failures carry a row and a column and no
/// file, and display as they always have.
#[test]
fn one_source_names_no_file() {
    let text = "def export [input]\n  csv csv-options (events input)\n";
    let fail = compile_sources(&sources(&[("one.alc", text)])).unwrap_err();
    let same = compile(text, "one.alc").unwrap_err();
    assert_eq!(fail, same);
    assert_eq!(fail.file, None, "{fail}");
    assert!(fail.row.is_some(), "{fail}");
    assert!(fail.to_json().get("file").is_none(), "{fail}");
    let shown = format!("({}:{})", fail.row.unwrap(), fail.column.unwrap());
    assert!(fail.to_string().ends_with(&shown), "{fail}");
}
