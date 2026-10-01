// The shared sources every runtime embeds, held to the files at the
// repository root.
//
// Two sources are authored once, at the root, and carried verbatim by each
// runtime: `alchemy-grammar.jsonic`, the grammar document, between the
// `BEGIN/END EMBEDDED` markers of `rs/src/grammar.rs`; and `stdlib/*.alc`,
// the standard library's own definitions, as the package-local copies in
// `rs/stdlib/` that `rs/src/stdlib/mod.rs` embeds (a crate holds nothing
// above `rs/`). `ts/embed-grammar.js` (`make embed`) writes both. This fails
// when a copy and its source differ, or when a file is in one place and
// not the other, so a forgotten embed is red rather than a quiet drift.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use common::repo_root;

#[test]
fn the_embedded_grammar_is_the_authored_one() {
    // The literal opens with the newline the embedder writes after `r##"`.
    let authored = fs::read_to_string(repo_root().join("alchemy-grammar.jsonic"))
        .expect("alchemy-grammar.jsonic is readable");
    assert!(
        tabnas_alchemy::grammar::grammar_text() == format!("\n{authored}"),
        "rs/src/grammar.rs embeds a different grammar from alchemy-grammar.jsonic: run make embed"
    );
}

/// The `.alc` files in `dir`, by name.
fn alc_files(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.expect("a directory entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".alc"))
        .collect()
}

#[test]
fn the_packaged_stdlib_is_the_repositorys() {
    let root = repo_root();
    let canonical = alc_files(&root.join("stdlib"));
    let packaged = alc_files(&root.join("rs/stdlib"));
    assert!(!canonical.is_empty(), "stdlib/ holds the library");
    assert_eq!(
        packaged, canonical,
        "rs/stdlib/ and stdlib/ hold different files: run make embed"
    );
    // The crate loads every file, by the name its spans carry.
    let embedded: BTreeSet<String> = tabnas_alchemy::stdlib::SOURCES
        .iter()
        .map(|(path, _)| {
            path.strip_prefix("stdlib/")
                .expect("named stdlib/")
                .to_string()
        })
        .collect();
    assert_eq!(
        embedded, canonical,
        "stdlib::SOURCES does not embed every stdlib/*.alc"
    );
    for (path, text) in tabnas_alchemy::stdlib::SOURCES {
        let source = fs::read_to_string(root.join(path)).expect("the source is readable");
        assert!(*text == source, "rs/{path} is not {path}: run make embed");
    }
}
