//! The standard library: the natives ([`registry`]) and the definitions
//! written in alchemy itself. The crate embeds copies under
//! `rs/stdlib/*.alc`, because a crates.io package cannot contain the
//! canonical `stdlib/*.alc` files above its package root; `make embed`
//! (`scripts/embed.js`) writes them. A unit test holds each packaged copy
//! byte-for-byte to its canonical file, and `tests/shared_sources_test.rs`
//! holds the two directories to the same files and [`SOURCES`] to all of
//! them.
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

use crate::ast::{SourceSpan, Sources};
use crate::desugar;
use crate::grammar::parse_file;
use crate::resolve::{resolve, Def, NameKind, Resolved};
use crate::shared::Fail;

use registry::Kind;

/// The embedded sources, by the file name their spans carry.
pub const SOURCES: &[(&str, &str)] = &[
    ("stdlib/table.alc", include_str!("../../stdlib/table.alc")),
    ("stdlib/csv.alc", include_str!("../../stdlib/csv.alc")),
    ("stdlib/root.alc", include_str!("../../stdlib/root.alc")),
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
        let resolved = resolve(forms, &Sources::one(file, src), &outer)?;
        crate::check::stdlib_file(&resolved, src)?;
        for (name, def) in &resolved.defs {
            if defs.insert(name.clone(), def.clone()).is_some() {
                return Err(Fail::new(
                    crate::shared::Code::DslTypeError,
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

/// The embedded file a span belongs to, as its name and its text, by
/// identity: every span of one file shares the allocation of the file's
/// name the reader made, and a program's spans never share it, so a
/// program that happens to be named `stdlib/table.alc` is not mistaken
/// for the library file.
pub fn file_of(span: &SourceSpan) -> Option<(&'static str, &'static str)> {
    stdlib()
        .files
        .iter()
        .zip(SOURCES)
        .find(|(resolved, _)| {
            resolved
                .defs
                .values()
                .next()
                .is_some_and(|def| Arc::ptr_eq(&def.span.file, &span.file))
        })
        .map(|(_, (name, src))| (*name, *src))
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
    use std::fs;
    use std::path::Path;

    use super::*;

    /// A crates.io package holds nothing above `rs/`, so the crate embeds
    /// package-local copies. In a repository checkout, hold each one to its
    /// canonical root file. In an extracted crate there is no repository
    /// root, and the package verification build has already proved the
    /// embedded copies are present.
    #[test]
    fn packaged_sources_match_the_repository_sources() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("rs/ has a parent");
        if !root.join(".tabnas-kind").is_file() {
            return;
        }
        for (path, embedded) in SOURCES {
            let canonical = fs::read_to_string(root.join(path))
                .unwrap_or_else(|error| panic!("cannot read canonical {path}: {error}"));
            assert_eq!(
                canonical, *embedded,
                "rs/{path} is not the canonical {path}: copy it into rs/stdlib"
            );
        }
    }

    #[test]
    fn the_library_loads_with_the_spec_definitions() {
        let lib = stdlib();
        let names: Vec<&str> = lib.names().collect();
        assert_eq!(
            names,
            [
                "public-column",
                "table-inferred-column",
                "table-row",
                "table-positional-column",
                "table-value-column",
                "table-inferred-columns",
                "table-first-row",
                "table-step",
                "table-finish",
                "table-finish-for",
                "table-captures",
                "table-from-json",
                "csv-options",
                "csv-field",
                "csv-row",
                "csv",
                "wrap-object-close",
                "wrap-object-step",
                "wrap-finish",
                "wrap-object",
                "wrap-array-close",
                "wrap-array-step",
                "wrap-array",
            ]
        );
        assert_eq!(lib.files.len(), 3);
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
