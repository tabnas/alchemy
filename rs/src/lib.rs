//! The alchemy language: declarative streaming transducers and renderers
//! for the tabnas engine, parsed by a tabnas grammar plugin.
//!
//! This crate is the reader half of the language today:
//!
//! - [`lex`] and [`grammar`]: the grammar plugin, installed on an engine
//!   with [`alchemy`], or built with [`make`]; [`parse_value`] answers the
//!   reader's tagged tree and [`parse`] the [`Expr`] forms.
//! - [`ast`]: [`Expr`] with [`SourceSpan`]s, the [`canonical`] and layout
//!   [`format`] printers.
//!
//! The checker, planner and interpreter that follow read [`Expr`] from
//! [`ast`]; they never see the tagged tree.
//!
//! ```
//! use tabnas_alchemy::{canonical, parse};
//! let program = parse("def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options")?;
//! assert_eq!(
//!     canonical(&program),
//!     "(def export [input] (pipe input (table-from-json api-binding) (csv csv-options)))"
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
pub mod grammar;
pub mod lex;

pub use ast::{canonical, canonical_form, format, same_program, Expr, SourceSpan};
pub use grammar::{alchemy, make, parse, parse_file, parse_value, UNNAMED};

/// This crate's version, as `Cargo.toml` declares it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
