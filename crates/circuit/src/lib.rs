//! Electrical schematic model shared by the Craft apps.
//!
//! A [`Project`] is what an editor hands over: parts, and per sheet the pins of those parts
//! at their drawing positions, wires, junctions and labels. This crate does not know how a
//! symbol is drawn; the app places each pin in sheet coordinates before calling it.
//!
//! - [`Connectivity::build`] joins pins, wires, junctions and labels into nets.
//! - [`Netlist::new`] lists the parts and the pins on each net, as JSON or CSV.
//! - [`bom()`] groups parts into parts-list rows.
//! - [`erc::check`] reports connection problems as [`erc::Finding`]s with a stable rule id.
//!
//! The connection rules are our own, written from what a schematic means, not from another
//! program's source:
//!
//! - Points closer than [`Project::tolerance`] meet.
//! - Every point of one wire is connected.
//! - A wire end, pin, junction or label that lies on a wire joins that wire.
//! - Two wires that cross, with neither ending on the other, stay apart unless a junction
//!   sits on both.
//! - Sheet labels with the same name join on their own sheet; global labels and supply
//!   labels with the same name join across the project.
//!
//! Input read with [`Project::from_json`] is bounded by [`Limits`], and connection building
//! stops with [`Error::TooComplex`] instead of running for an unbounded time.

mod bom;
mod connect;
pub mod erc;
mod limits;
mod model;
mod natural;
mod netlist;

pub use bom::{BomOptions, BomRow, bom, bom_csv};
pub use connect::{Connectivity, Net, WireEnd};
pub use limits::{Error, Limits};
pub use model::{DEFAULT_TOLERANCE, ItemKind, ItemRef, Label, LabelScope, Part, Pin, PinKind, Point, Project, Sheet, Wire};
pub use natural::natural_cmp;
pub use netlist::{NETLIST_FORMAT, NETLIST_VERSION, Netlist, NetlistNet, NetlistNode, NetlistPart};
