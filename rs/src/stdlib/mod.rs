//! The standard library: the natives ([`registry`]) and the definitions
//! written in alchemy itself, embedded from `stdlib/*.alc` at the
//! repository root.
//!
//! `table.alc` is the metadata-first table transducer (design document
//! section 12.2: `public-column`, `table-step`, `table-finish`,
//! `table-from-json`) and `csv.alc` the always-quoted CSV renderer
//! (sections 13.1 and 13.2: `csv-options`, `csv-field`, `csv-row`, `csv`).
//! Both are parsed, desugared and resolved once, on first use, and their
//! names are usable from any program; a program's own `def` of the same
//! name shadows the library's for that program, never for the library
//! itself, whose definitions resolve in their own scope.
//!
//! The library text is the reference: the runtime runs `table-from-json`
//! and `csv` natively when their arguments have the standard shapes, and
//! the differential test in `rs/tests/stdlib_test.rs` proves the native
//! path and this text produce the same bytes.

pub mod registry;

use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;
use tabnas_transduce::Fail;

use crate::desugar;
use crate::grammar::parse_file;
use crate::resolve::{resolve, Def, NameKind, Resolved};

use registry::Kind;

/// The embedded sources, by the file name their spans carry.
pub const SOURCES: &[(&str, &str)] = &[
    (
        "stdlib/table.alc",
        include_str!("../../../stdlib/table.alc"),
    ),
    ("stdlib/csv.alc", include_str!("../../../stdlib/csv.alc")),
];

/// The library, loaded: each file's resolved definitions, and all of them
/// by name.
pub struct Stdlib {
    /// One entry per source file, in [`SOURCES`] order.
    pub files: Vec<Resolved>,
    /// Every definition, by name, in file order.
    pub defs: IndexMap<Arc<str>, Def>,
}

impl Stdlib {
    pub fn get(&self, name: &str) -> Option<&Def> {
        self.defs.get(name)
    }

    /// The names the library defines, in file order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.defs.keys().map(|k| &**k)
    }
}

/// The kind of a native, as the resolver classifies names.
pub fn native_kind(name: &str) -> Option<NameKind> {
    registry::native(name).map(|n| match n.kind {
        Kind::Function => NameKind::Function,
        Kind::Constant => NameKind::Constant,
        Kind::Constructor => NameKind::Constructor,
    })
}

/// Parse, desugar, resolve and check the embedded sources. Each file
/// resolves against the natives and the other files' definitions, so a
/// definition may use one from another file but no file may redefine
/// another's; each definition is checked against the signature
/// [`crate::check::stdlib_signature`] declares for it.
pub fn load() -> Result<Stdlib, Fail> {
    // Every definition's name first, so a reference across files resolves
    // whatever the file order.
    let mut parsed = Vec::with_capacity(SOURCES.len());
    let mut names: Vec<Arc<str>> = Vec::new();
    for (file, src) in SOURCES {
        let forms = desugar::program(parse_file(src, file)?, src)?;
        for form in &forms {
            if let crate::ast::Expr::List { items, .. } = form {
                if let Some(crate::ast::Expr::Symbol { name, .. }) = items.get(1) {
                    names.push(Arc::from(name.as_str()));
                }
            }
        }
        parsed.push((*file, *src, forms));
    }
    let mut files = Vec::with_capacity(parsed.len());
    let mut defs: IndexMap<Arc<str>, Def> = IndexMap::new();
    for (file, src, forms) in parsed {
        let outer = |name: &str| -> Option<NameKind> {
            if names.iter().any(|n| &**n == name) {
                return Some(NameKind::Value);
            }
            native_kind(name)
        };
        let resolved = resolve(forms, src, file, &outer)?;
        crate::check::stdlib_file(&resolved, src)?;
        for (name, def) in &resolved.defs {
            if defs.insert(name.clone(), def.clone()).is_some() {
                return Err(Fail::new(
                    tabnas_transduce::Code::DslTypeError,
                    format!("duplicate_def: {name} is defined in two standard library files"),
                ));
            }
        }
        files.push(resolved);
    }
    Ok(Stdlib { files, defs })
}

/// The library, loaded once. The embedded text is part of this crate, so
/// a text that does not load is a defect of the build, not of any
/// program; `rs/tests/stdlib_test.rs` asserts it loads and checks clean.
pub fn stdlib() -> &'static Stdlib {
    static LIB: OnceLock<Stdlib> = OnceLock::new();
    LIB.get_or_init(|| match load() {
        Ok(lib) => lib,
        Err(fail) => panic!("the embedded standard library does not load: {fail}"),
    })
}

/// The source text of an embedded file, by the name its spans carry.
pub fn source(file: &str) -> Option<&'static str> {
    SOURCES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, src)| *src)
}

/// What a program sees outside itself: the library's definitions and the
/// natives.
pub fn outer(name: &str) -> Option<NameKind> {
    if stdlib().get(name).is_some() {
        return Some(NameKind::Value);
    }
    native_kind(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_loads_with_the_spec_definitions() {
        let lib = stdlib();
        let names: Vec<&str> = lib.names().collect();
        assert_eq!(
            names,
            [
                "public-column",
                "table-step",
                "table-finish",
                "table-from-json",
                "csv-options",
                "csv-field",
                "csv-row",
                "csv",
            ]
        );
        assert_eq!(lib.files.len(), 2);
        assert_eq!(
            lib.get("table-from-json").unwrap().params(),
            Some(vec!["binding", "input"])
        );
        assert_eq!(lib.get("csv-options").unwrap().params(), None);
    }

    #[test]
    fn library_names_and_natives_are_outer_to_a_program() {
        assert_eq!(outer("csv"), Some(NameKind::Value));
        assert_eq!(outer("map"), Some(NameKind::Function));
        assert_eq!(outer("table-end"), Some(NameKind::Constant));
        assert_eq!(outer("selected"), Some(NameKind::Constructor));
        assert_eq!(outer("nope"), None);
        assert_eq!(source("stdlib/csv.alc"), Some(SOURCES[1].1));
        assert_eq!(source("nope"), None);
    }
}
