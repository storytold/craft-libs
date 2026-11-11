//! The context types (1.5): `CxCore`, `Cx`, `BodyCx`, `ItemCx`, `MeasureCx`, `PartsCx` and `PartBuilder`, and the
//! chapter traits implemented on them.

mod access;
mod body;
mod commands;
mod context;
mod drives;
mod edit;
mod focus;
mod item;
mod measure;
mod parts;
mod pointer;
mod scroll;
mod styled;
mod words;

pub use access::Accessible;
pub use body::BodyCx;
pub(crate) use body::BodyOwner;
pub use context::{Activation, Cx, CxCore};
pub(crate) use context::{RecordedText, child_env};
pub use edit::Edit;
pub(crate) use edit::gesture_key;
pub use focus::Focus;
pub use item::{ItemCx, ItemOpener};
pub use measure::MeasureCx;
pub(crate) use parts::{Decls, PartExtras, Toggle};
pub use parts::{PartBuilder, PartsCx};
pub use pointer::Pointer;
pub(crate) use pointer::write_cursor;
#[doc(hidden)]
pub use scroll::THUMB;
pub(crate) use scroll::{ScrollDecl, scroll_moves, thumb_rect};
pub use styled::Styled;
pub use words::WordSetCx;
