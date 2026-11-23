//! The neutral ink model shared by WordCraft, DeckCraft and GridCraft.
//!
//! A drawing app keeps its own document model, anchors and package wiring; this crate owns only
//! what the three read and write the same way:
//!
//! - [`stroke`]: a stroke's points, pressure, brush kind, colour, thickness, draw order and
//!   `a16:creationId`-style key. Coordinates are centimetres, the unit of an Office ink part, at
//!   the 1/1000 cm resolution the format stores; [`stroke::px_to_cm`] and [`stroke::cm_to_px`]
//!   convert a host's 96 dpi sheet points.
//! - [`inkml`]: a bounded reader and writer for the InkML part an Office drawing's
//!   `xdr:contentPart` points at, using the W3C InkML difference encoding. The container
//!   relationship, the content type, the `a16:creationId` and the PNG fallback picture stay in
//!   the application, which is where the document mapping lives.
//! - [`select`]: lasso containment for freehand ink (even–odd rule, 60 % of a stroke's samples).
//! - [`replay`]: stroke ordering and an animation schedule that reveals a stroke along its path.
//! - [`shape`]: a small geometric ink-to-shape recogniser (line, arrow, rectangle, ellipse,
//!   triangle, polygon).
//! - [`recognize`]: the handwriting recogniser seam — a [`recognize::Recognizer`] trait, a
//!   fixed-answer double for tests and the model descriptor — without a model runtime.
//!
//! # Format
//!
//! The codec writes the subset of InkML that Office writes behind a drawing's content part:
//! `<inkml:ink>` → `<inkml:definitions>` (`context` with an `inkSource`/`traceFormat`/
//! `channelProperties`, and `brush` with `brushProperty`) and one `<inkml:trace>` per stroke.
//! Traces are encoded with the W3C difference-order grammar (`!` explicit, `'` first difference,
//! `"` second difference, `*` unchanged).
//!
//! Public specifications:
//!
//! - W3C, *Ink Markup Language (InkML) 1.0 Recommendation*, 20 September 2011,
//!   <https://www.w3.org/TR/InkML/> — trace format, channels, brushes and the difference encoding.
//! - Microsoft, *[MS-ODRAWXML]: Office Drawing Extensions to Office Open XML Structure*,
//!   <https://learn.microsoft.com/openspecs/office_standards/ms-odrawxml/> — the
//!   `xdr:contentPart` of a drawing whose `customXml` relationship names an InkML part, and the
//!   `a16:creationId` extension. The application owns that container.
//!
//! # Bounds
//!
//! Everything a file can say is bounded before it is trusted: [`inkml::Limits`] caps bytes,
//! strokes and points; a stroke holds at most [`stroke::MAX_POINTS`] points at most
//! [`stroke::MAX_COORD_CM`] centimetres from the origin and pressure at most
//! [`stroke::MAX_PRESSURE`]. Non-finite numbers are refused, arithmetic that could overflow is
//! checked or saturating, and no function panics on hostile input.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
#![forbid(unsafe_code)]

pub mod inkml;
pub mod recognize;
pub mod replay;
pub mod select;
pub mod shape;
pub mod stroke;

pub use inkml::{Error as InkmlError, Ink, Limits};
pub use stroke::{Brush, Color, Effect, Stroke, StrokePoint, Tool};
