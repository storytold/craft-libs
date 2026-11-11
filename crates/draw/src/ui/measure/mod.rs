//! The Ruler tool ([`RulerTool`]) with its handles ([`RulerOverlay`]), its options bar ([`RulerBar`]), and the
//! distances between two boxes that the canvas's [`SmartGuides`](craft_ui::canvas_controls::SmartGuides) draws
//! ([`box_distances`]).
//!
//! The Ruler line is document state outside the undo history: the tool reports `EditEvent<Option<RulerLine>>` and the
//! app stores the line.

mod bar;
mod distances;
mod model;
mod overlay;
#[cfg(feature = "screen-testing")]
mod stories;
mod tool;
mod words;

pub use bar::{RulerBar, RulerBarOutput};
pub use distances::box_distances;
pub use model::{MIN_LINE, RulerLine, RulerPart, RulerReading};
pub use overlay::{RulerOverlay, RulerOverlayStyle};
pub use tool::{RulerPass, RulerTool};
pub use words::RulerWords;

use craft_ui::patterns::form_dialogs::{Param, ParamDialog, ParamKind};

/// The key of the Ruler Line form.
const FORM_KEY: &str = "draw.ruler.line";

/// The Ruler Line… command form, the keyboard path of the Ruler tool: Start and End as points in document units,
/// opened with the current line's.
pub fn ruler_line_form(line: Option<&RulerLine>) -> ParamDialog<'static> {
    let (start, end) = line.map_or(([0.0, 0.0], [0.0, 0.0]), |line| ([line.start.x, line.start.y], [line.end.x, line.end.y]));
    ParamDialog::command(
        FORM_KEY,
        "Ruler Line",
        vec![Param::new("start", "Start", ParamKind::point(start)), Param::new("end", "End", ParamKind::point(end))],
    )
}
