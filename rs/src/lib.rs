//! The alchemy language: declarative streaming transducers and renderers
//! for the tabnas engine, parsed by a tabnas grammar plugin; and the types
//! every tabnas transducer and renderer shares.
//!
//! [`shared`] is the types: the event and table protocols, sinks,
//! failures and their codes, limits, selectors, datums, the renderers'
//! options and text boundary, and the two interfaces a program is lowered
//! through, [`shared::Routers`] and [`shared::Renderers`]. It depends on
//! nothing else in the fleet; with `default-features = false` it is the
//! whole crate, and tabnas-transduce and tabnas-render build on it.
//!
//! Everything else is the language, behind the default `language`
//! feature. The reader:
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
//!   as plans; [`interp`]: the evaluator; [`lower`]: plans to the stages
//!   the host's routers and renderers make; [`stdlib`]: the natives and
//!   the embedded library.
//! - [`program`]: [`compile`] and [`Program`], the API a host embeds. The
//!   host passes the routers (tabnas-transduce's) and the renderers
//!   (tabnas-render's) in; the `alchemy` command, which does, is
//!   tabnas-alchemy-cli.
//!
//! ```
//! use tabnas_alchemy::{canonical, desugar, parse};
//! let src = "def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options";
//! let core = desugar::program(parse(src)?, src)?;
//! assert_eq!(
//!     canonical(&core),
//!     "(def export (fn [input] (csv csv-options (table-from-json api-binding input))))"
//! );
//! # Ok::<(), tabnas_alchemy::shared::Fail>(())
//! ```

#![forbid(unsafe_code)]
// The crate's documentation above names the language, which a build
// without the `language` feature leaves out.
#![cfg_attr(not(feature = "language"), allow(rustdoc::broken_intra_doc_links))]

/// The README's Rust examples run as doctests, so a stale one fails the
/// gate rather than misleading the reader.
#[cfg(all(doctest, feature = "language"))]
#[doc = include_str!("../README.md")]
mod readme_examples {}

pub mod shared;

#[cfg(feature = "language")]
pub mod ast;
#[cfg(feature = "language")]
pub mod check;
#[cfg(feature = "language")]
pub mod desugar;
#[cfg(feature = "language")]
pub mod effects;
#[cfg(feature = "language")]
pub mod grammar;
#[cfg(feature = "language")]
pub mod interp;
#[cfg(feature = "language")]
pub mod lex;
#[cfg(feature = "language")]
pub mod lower;
#[cfg(feature = "language")]
pub mod program;
#[cfg(feature = "language")]
pub mod resolve;
#[cfg(feature = "language")]
pub mod stdlib;
#[cfg(feature = "language")]
pub mod translate;
#[cfg(feature = "language")]
pub mod types;
#[cfg(feature = "language")]
pub mod value;

#[cfg(feature = "language")]
pub use ast::{
    canonical, canonical_form, format, same_program, Expr, SourceSpan, Sources, MAX_NESTING,
};
#[cfg(feature = "language")]
pub use check::MAX_APPLIED;
#[cfg(feature = "language")]
pub use grammar::{alchemy, make, parse, parse_file, parse_value, UNNAMED};
#[cfg(feature = "language")]
pub use interp::{MAX_EVAL_DEPTH, MAX_PLAN_STEPS};
#[cfg(feature = "language")]
pub use program::{compile, compile_sources, Output, Program, Renderer, Source};

/// The stack the checker and the evaluator are given: [`compile`] runs on
/// a thread of this size, and so does the `alchemy` command, so a program
/// at [`MAX_NESTING`] and an evaluation at [`MAX_EVAL_DEPTH`] fit in a
/// debug build as in a release one. A host that pushes events into a
/// [`Program::sink`] runs the program's per-item functions on its own
/// thread and gives it at least this much (aless's parse thread has it).
#[cfg(feature = "language")]
pub const STACK_BYTES: usize = 64 << 20;

/// This crate's version, as `Cargo.toml` declares it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
