//! The alchemy language: declarative streaming transducers and renderers
//! for the tabnas engine, parsed by a tabnas grammar plugin.
//!
//! This crate is the reader half of the language today:
//!
//! - [`lex`] and [`grammar`]: the grammar plugin, installed on an engine
//!   with [`alchemy`], or built with [`make`]; [`parse_value`] answers the
//!   reader's tagged tree and [`parse`] the [`Expr`] forms.
//! - [`ast`]: [`Expr`] with [`SourceSpan`]s, the [`canonical`] and layout
//!   [`format`] printers, and [`MAX_NESTING`], the bound every form this
//!   crate builds respects.
//! - [`desugar`]: the core-form rewrites (`def` with parameters, `pipe`,
//!   the shapes of `let`, `if` and `match`).
//!
//! The checker, planner and interpreter that follow read [`Expr`] from
//! [`ast`] after [`desugar`] has run; they never see the tagged tree.
//!
//! ```
//! use tabnas_alchemy::{canonical, desugar, parse};
//! let src = "def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options";
//! let core = desugar::program(parse(src)?, src)?;
//! assert_eq!(
//!     canonical(&core),
//!     "(def export (fn [input] (csv csv-options (table-from-json api-binding input))))"
//! );
//! # Ok::<(), tabnas_transduce::Fail>(())
//! ```

#![forbid(unsafe_code)]

/// The README's Rust examples run as doctests, so a stale one fails the
/// gate rather than misleading the reader.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
mod readme_examples {}

pub mod ast;
pub mod check;
pub mod desugar;
pub mod grammar;
pub mod interp;
pub mod lex;
pub mod lower;
pub mod program;
pub mod resolve;
pub mod stdlib;
pub mod types;
pub mod value;

pub use ast::{canonical, canonical_form, format, same_program, Expr, SourceSpan, MAX_NESTING};
pub use grammar::{alchemy, make, parse, parse_file, parse_value, UNNAMED};
pub use program::{compile, Output, Program, Renderer};

/// This crate's version, as `Cargo.toml` declares it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
