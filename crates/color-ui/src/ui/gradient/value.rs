//! The stop bar's marker vocabulary: [`StopRef`] and [`StopColor`].

use craft_ui::runtime::requests::ColorRequest;
use egui::Rect;

use crate::gradient::StopRow;

/// A marker of the stop bar: a colour stop, an opacity stop, or the midpoint after stop `i` of a
/// row. Indices are into the stops as the bar shows them ([`GradientStops::opacity_stops`](crate::gradient::GradientStops::opacity_stops) for
/// the opacity row).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StopRef {
    /// Colour stop `i`.
    Color(usize),
    /// Opacity stop `i`.
    Opacity(usize),
    /// The midpoint after stop `i` of the given row.
    Midpoint(StopRow, usize),
}

impl StopRef {
    /// The row the marker belongs to.
    pub fn row(self) -> StopRow {
        match self {
            StopRef::Color(_) => StopRow::Color,
            StopRef::Opacity(_) => StopRow::Opacity,
            StopRef::Midpoint(row, _) => row,
        }
    }

    /// Stop `i` of `row`.
    pub(crate) fn stop(row: StopRow, i: usize) -> Self {
        match row {
            StopRow::Color => StopRef::Color(i),
            StopRow::Opacity => StopRef::Opacity(i),
        }
    }

    /// The stop's index, or for a midpoint the index of the stop before it.
    pub(crate) fn index(self) -> usize {
        match self {
            StopRef::Color(i) | StopRef::Opacity(i) | StopRef::Midpoint(_, i) => i,
        }
    }

    /// The same kind of marker at index `i`.
    pub(crate) fn with_index(self, i: usize) -> Self {
        match self {
            StopRef::Color(_) => StopRef::Color(i),
            StopRef::Opacity(_) => StopRef::Opacity(i),
            StopRef::Midpoint(row, _) => StopRef::Midpoint(row, i),
        }
    }

    /// Whether the marker is a stop and not a midpoint.
    pub(crate) fn is_stop(self) -> bool {
        !matches!(self, StopRef::Midpoint(..))
    }
}

/// The colour stop whose colour the host is asked to edit, with the request to hand to its colour
/// picker dialog.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct StopColor {
    /// The colour stop's index.
    pub index: usize,
    /// The stop's tag.
    pub tag: u64,
    /// The request, opened at the stop's colour.
    pub request: ColorRequest,
    /// What asked for the colour (the stop's marker, or the Color well), where the host opens its picker; `None`
    /// when the request carries no place.
    pub rect: Option<Rect>,
}

impl StopColor {
    /// A request to edit colour stop `index`, whose tag is `tag`.
    pub fn new(index: usize, tag: u64, request: ColorRequest) -> Self {
        Self { index, tag, request, rect: None }
    }

    /// The request raised at `rect`.
    #[must_use]
    pub fn at(mut self, rect: Rect) -> Self {
        self.rect = Some(rect);
        self
    }
}
