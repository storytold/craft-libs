#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// The code came from an app whose lints allow this.
#![allow(clippy::type_complexity)]
//! Equations: one tree and the syntaxes it is read from and written to.
//!
//! - [`math`]: the tree ([`MNode`], [`MRun`], an argument [`Arg`] is a `Vec<MNode>`), modelled on
//!   the Office Math object model (ECMA-376 Part 1 §22.1), and [`Math`], an equation with its kept
//!   source OMML.
//! - [`math_linear`]: the linear format (UnicodeMath, the plain text Word shows for an equation),
//!   read and written ([`math::parse_linear`], [`math::to_linear`]).
//! - [`math_latex`]: LaTeX math, read and written ([`math_latex::parse_latex`],
//!   [`math_latex::to_latex`]), with the named symbols of [`math_symbols`].
//! - [`omml`]: Office Math Markup, read ([`omml::read_omath`], [`omml::read_str`]) and written
//!   ([`omml::write_equation`], [`omml::write_str`]).
//! - [`mathml`]: presentation MathML, written ([`mathml::to_mathml`]) and read
//!   ([`mathml::parse`], [`mathml::read`], [`mathml::to_tree`]).
//! - [`latex_text`]: LaTeX math and the linear format converted as text, keeping the source's
//!   spacing, for LaTeX documents ([`latex_text::latex_to_linear`], [`latex_text::linear_to_latex`]).
//! - [`math_tree`]: the arguments of a node and a walk over every run.
//! - [`symbol_fonts`]: text in symbol fonts (Symbol, Wingdings…) as Unicode, as OMML runs and
//!   Word text need it.
//! - [`Rgb`]: the colour of a run.
//!
//! Every reader is bounded (nesting [`math::MAX_DEPTH`], node budgets) and never panics on any
//! input.
//!
//! ```
//! use craft_math::{math, math_latex, mathml, omml};
//!
//! let tree = math::parse_linear("x=(-b±√(b^2-4ac))/2a");
//! assert_eq!(math::to_linear(&tree), "x=(-b±√(b^2-4ac))/2a");
//! assert!(math_latex::to_latex(&tree).contains("\\frac"));
//!
//! let eq = math::Math { nodes: tree.clone(), ..Default::default() };
//! let xml = omml::write_str("", false, &eq, None);
//! assert_eq!(omml::read_str(&xml).map(|m| m.nodes), Some(tree.clone()));
//!
//! let ml = mathml::to_mathml(&tree, false, "");
//! assert_eq!(mathml::parse(&ml).as_ref().map(mathml::to_tree), Some(tree));
//! ```

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

pub mod latex_text;
pub mod math;
pub mod math_latex;
pub mod math_linear;
pub mod math_symbols;
pub mod math_tree;
pub mod mathml;
pub mod omml;
mod rgb;
pub mod symbol_fonts;

pub use math::{Arg, MNode, MRun, Math, MathProps};
pub use rgb::Rgb;
