// The standard library's own definitions, embedded from `stdlib/*.alc`,
// load. The differential test, which runs the library's text against the
// native compositions over transduce's fixtures and generated documents,
// runs on transduce's routers and render's renderers and is in
// tabnas-alchemy-cli's stdlib_test.rs.

#[test]
fn the_library_loads() {
    let lib = tabnas_alchemy::stdlib::stdlib();
    assert!(lib.get("table-from-json").is_some());
    assert!(lib.get("csv").is_some());
}
