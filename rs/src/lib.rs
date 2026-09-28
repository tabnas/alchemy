//! The alchemy language: declarative streaming transducers and renderers
//! for the tabnas engine, parsed by a tabnas grammar plugin.
//!
//! The reader:
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
//! The language, reading [`Expr`] after [`desugar`] has run:
//!
//! - [`resolve`]: scopes and linking; [`types`] and [`check`]: inference,
//!   affine streams, protocols, strict mode; [`effects`]: the effect
//!   summary and the `explain` report.
//! - [`value`]: runtime values, every one `Send`, with streams and texts
//!   as plans; [`interp`]: the evaluator; [`lower`]: plans to transduce
//!   and render sinks; [`stdlib`]: the natives and the embedded library.
//! - [`program`]: [`compile`] and [`Program`], the API a host embeds.
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
pub mod effects;
pub mod grammar;
pub mod interp;
pub mod lex;
pub mod lower;
pub mod program;
pub mod resolve;
pub mod stdlib;
pub mod types;
pub mod value;

pub use ast::{
    canonical, canonical_form, format, same_program, Expr, SourceSpan, Sources, MAX_NESTING,
};
pub use check::MAX_APPLIED;
pub use grammar::{alchemy, make, parse, parse_file, parse_value, UNNAMED};
pub use interp::{MAX_EVAL_DEPTH, MAX_PLAN_STEPS};
pub use program::{compile, compile_sources, Output, Program, Renderer, Source};

/// The stack the checker and the evaluator are given: [`compile`] runs on
/// a thread of this size, and so does the `alchemy` command, so a program
/// at [`MAX_NESTING`] and an evaluation at [`MAX_EVAL_DEPTH`] fit in a
/// debug build as in a release one. A host that pushes events into a
/// [`Program::sink`] runs the program's per-item functions on its own
/// thread and gives it at least this much (aless's parse thread has it).
pub const STACK_BYTES: usize = 64 << 20;

/// This crate's version, as `Cargo.toml` declares it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
