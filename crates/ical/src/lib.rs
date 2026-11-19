#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! iCalendar (RFC 5545): the layers a calendar program reads and writes `.ics` text with.
//!
//! - [`lines`]: content lines. [`Lines`] unfolds a text, [`ContentLine::parse`] splits a line
//!   into its name, parameters (RFC 6868 caret escapes undone) and value, [`fold`] and
//!   [`param_value`] write them.
//! - [`values`]: DATE and DATE-TIME values ([`When`]), durations, TEXT escaping, lists.
//! - [`rrule`]: the common subset of recurrence rules ([`Recurrence`]), read and written, with the
//!   start's weekday and month spelled out where a rule leans on them.
//! - [`zones`]: `VTIMEZONE` definitions a text brings ([`TimeZones`]), which answer the offset at
//!   an instant and the instant of a wall-clock time, and [`write_vtimezone`], which writes a
//!   definition from the caller's zone rules ([`ZoneRules`]).
//!
//! Times are `chrono` values; instants are seconds since 1970-01-01T00:00:00Z. There is no
//! time-zone database here: what a `TZID` means beyond the text is the caller's.
//!
//! What events and to-dos mean (which properties become which fields, which zone a time is shown
//! in) is the caller's too: it reads them from [`ContentLine`]s with these values.
//!
//! Input is hostile. Nothing here panics on any text: lines, parameters, rules, zones, parts and
//! dates are capped, and work on zone definitions is budgeted ([`TimeZones::spend`]).

pub mod lines;
pub mod rrule;
pub mod values;
pub mod zones;

pub use lines::{ContentLine, Lines, Param, fold, param_value};
pub use rrule::{Freq, Recurrence, format_rrule, parse_rrule};
pub use values::When;
pub use zones::{TimeZones, Transition, VTimezone, ZoneRules, write_vtimezone};
