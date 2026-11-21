#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Compile-checked conversion between two models that have the same shape.
//!
//! Two crates sometimes hold the same model as separate Rust types: a library's document model
//! and the copy an app extends, or two versions of one schema. Converting through a serialized
//! form (JSON, say) works, but it is slow, it pulls a serializer into the binary, and a field
//! only one side has is dropped without a word. This crate converts field by field instead, and
//! the compiler checks the mapping:
//!
//! - [`Convert`] takes the source by value; [`ConvertRef`] reads it through a reference. Both
//!   are implemented here for the primitives, `String`, `Option`, `Vec`, `VecDeque`, `Box`,
//!   `Rc`, `Arc`, the standard maps and sets, tuples and arrays.
//! - [`convert_types!`] writes the impls for your structs and enums. A struct is taken apart
//!   with a pattern that names every field and built with a literal that names every field, and
//!   an enum is matched variant by variant, so **a field or variant added to either model is a
//!   compile error** until the mapping says what happens to it.
//!
//! ```
//! mod wire {
//!     pub struct Stop { pub at: f32, pub color: [u8; 3] }
//!     pub enum Fill { None, Solid([u8; 3]), Gradient { stops: Vec<Stop>, angle: f32 } }
//! }
//! mod app {
//!     pub struct Stop { pub at: f32, pub color: [u8; 3] }
//!     pub enum Fill { None, Solid([u8; 3]), Gradient { stops: Vec<Stop>, angle: f32 } }
//! }
//!
//! craft_convert::convert_types! {
//!     struct wire::Stop = app::Stop { at, color }
//!     enum wire::Fill = app::Fill { None, Solid(rgb), Gradient { stops, angle } }
//! }
//!
//! use craft_convert::{Convert, ConvertRef};
//! let fill = wire::Fill::Gradient { stops: vec![wire::Stop { at: 0.5, color: [1, 2, 3] }], angle: 90.0 };
//! let copy: app::Fill = fill.convert_ref(); // reads `fill`
//! let moved: app::Fill = fill.convert(); // consumes it, moving the strings and vectors
//! # let _ = (copy, moved);
//! ```
//!
//! Leave a field out and the crate does not build:
//!
//! ```compile_fail,E0027
//! mod wire { pub struct Stop { pub at: f32, pub color: [u8; 3] } }
//! mod app { pub struct Stop { pub at: f32, pub color: [u8; 3] } }
//! craft_convert::convert_types! {
//!     struct wire::Stop => app::Stop { at }
//! }
//! ```
//!
//! # Declarations
//!
//! Each entry is `struct` or `enum`, the two types, and the members:
//!
//! | Entry | Impls written |
//! |---|---|
//! | `struct A => B { … }` | `Convert<B> for A` |
//! | `struct &A => B { … }` | `ConvertRef<B> for A` |
//! | `struct A = B { … }` | all four: by value and by reference, both ways |
//!
//! and the same three for `enum`. Members of a struct, or of a variant with named fields:
//!
//! | Member | Meaning |
//! |---|---|
//! | `name` | the same field on both sides |
//! | `old => new` | renamed |
//! | `name = expr` | the target field is `expr`; the source field is in scope as `name` |
//! | `old => new = expr` | renamed, with its own expression (one-way entries) |
//! | `old => _` | only the source has it: dropped (one-way entries) |
//! | `_ => new = expr` | only the target has it: it is `expr` |
//! | `old => _ = expr` | in an `A = B` entry: only `A` has it; `expr` is its value coming back |
//!
//! In an `A = B` entry `_ => new = expr` likewise drops `new` on the way back. Variants:
//!
//! | Variant | Meaning |
//! |---|---|
//! | `Unit`, `Tuple(a, b)`, `Named { members }` | the same variant on both sides |
//! | `Old => New`, `Old(a) => New`, `Old { members } => New` | renamed |
//! | `Old = expr`, `Old(a) = expr`, `Old { a, b: _ } = expr` | the arm is `expr` (one-way entries) |
//! | `_ => New` | only the target has it: nothing converts to it (one-way entries) |
//!
//! A one-way enum entry also checks the target: a variant the entry neither produces nor lists
//! with `_ => New` does not compile.
//!
//! Types are named without generic parameters. A tuple struct is short enough to write by hand;
//! destructure it and the impl is just as exhaustive. A struct with a private field cannot be
//! taken apart from outside its crate, so its impl is written by hand too.
//!
//! # A mapping between two crates you do not own
//!
//! Rust lets a crate implement a foreign trait only for types it owns. The second parameter of
//! both traits is a *marker* for that: declare a unit struct, name it in the macro, and the
//! impls are yours to write even when both models are someone else's. Two crates can map the
//! same pair of models differently; each sees only its own marker.
//!
//! ```
//! # mod wire { pub struct Stop { pub at: f32 } }
//! # mod app { pub struct Stop { pub at: f32 } }
//! use craft_convert::{Convert, Floats};
//!
//! struct Bridge;
//! impl Floats for Bridge {}
//!
//! craft_convert::convert_types! {
//!     marker = Bridge;
//!     struct wire::Stop = app::Stop { at }
//! }
//!
//! let stop: app::Stop = Convert::<_, Bridge>::convert(wire::Stop { at: 0.25 });
//! # let _ = stop;
//! ```
//!
//! A marker implements [`Floats`], which sees every `f32` and `f64` on its way through. The
//! default passes them unchanged; a mapping that stands in for a text format can note or
//! replace NaN and the infinities there, since those formats cannot carry them.
//!
//! # Shared pointers
//!
//! By value, an `Rc` or `Arc` with one owner is unwrapped and its value moved; a shared one is
//! cloned first (so the pointee is `Clone`). Either way the result is a new pointer: two
//! pointers to one value become two values. Where sharing matters (deduplicated pictures,
//! large byte buffers that are the same type on both sides) give the field its own expression
//! and move or look up the pointer there.
//!
//! # Long declarations
//!
//! The macro walks members one at a time, so a struct with more than about fifty fields or an
//! enum with more than about fifty variants needs `#![recursion_limit = "256"]` in the crate
//! that declares it; `rustc` says so when it does.

mod impls;
mod macros;

/// Converts `self` into its twin `T`, consuming it.
///
/// `M` names the mapping. The default `()` is for crates that own one of the two models; see
/// the crate documentation for mappings between two foreign models.
pub trait Convert<T, M = ()>: Sized {
    /// The value as a `T`.
    fn convert(self) -> T;
}

/// Converts a borrowed value into its twin `T`, leaving the source in place.
///
/// `M` names the mapping, as in [`Convert`].
pub trait ConvertRef<T, M = ()> {
    /// The value as a `T`.
    fn convert_ref(&self) -> T;
}

/// What a mapping does with floating-point numbers. A marker type implements it.
///
/// Every `f32` and `f64` converted under the marker goes through these, whatever container it
/// is in. The defaults return the value unchanged.
pub trait Floats {
    /// An `f32` on its way through.
    fn f32(value: f32) -> f32 {
        value
    }

    /// An `f64` on its way through.
    fn f64(value: f64) -> f64 {
        value
    }
}

impl Floats for () {}

#[cfg(test)]
mod tests;
