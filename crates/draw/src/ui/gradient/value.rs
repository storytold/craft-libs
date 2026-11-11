//! [`GradientStops`]: the value a gradient stop bar edits.

use std::borrow::Cow;

use craft_theme::Srgba;
use craft_ui::runtime::requests::ColorRequest;

/// How a gradient's opacity is stored, which decides the rows of its stop bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StopRows {
    /// Opacity stops in their own row above the ramp, colour stops below; a colour's alpha is
    /// ignored. An empty [`GradientStops::opacity`] means opaque and shows as two 100 % stops at
    /// 0 and 100 %.
    #[default]
    Split,
    /// One row of colour stops; each stop's opacity is its colour's alpha and
    /// [`GradientStops::opacity`] stays empty.
    Combined,
    /// One row of colour stops and no opacity at all: a colour's alpha is forced to 1 and the
    /// editor shows no Opacity field (PhotoCraft's Gradient Map). [`GradientStops::opacity`]
    /// stays empty.
    ColorOnly,
}

/// One of the two rows of a [`StopRows::Split`] bar. [`StopRows::Combined`] and
/// [`StopRows::ColorOnly`] have only `Color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StopRow {
    /// Colour stops.
    Color,
    /// Opacity stops.
    Opacity,
}

/// A colour stop.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorStop {
    /// Where the stop is, 0..=1.
    pub location: f32,
    /// Its colour, with the opacity in the alpha under [`StopRows::Combined`].
    pub color: Srgba,
    /// Where between this stop and the next the mix is half and half, 0..=1; 0.5 is centred. The
    /// last stop's is unused.
    pub midpoint: f32,
    /// The host's: it travels with the stop through moves and reorders, so the host can keep what
    /// the stop doesn't model (a swatch link, a stable id). An added stop has tag 0; a copy keeps
    /// its source's.
    pub tag: u64,
}

/// An opacity stop of a [`StopRows::Split`] gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct OpacityStop {
    /// Where the stop is, 0..=1.
    pub location: f32,
    /// Its opacity, 0..=1.
    pub opacity: f32,
    /// As [`ColorStop::midpoint`].
    pub midpoint: f32,
    /// As [`ColorStop::tag`].
    pub tag: u64,
}

impl ColorStop {
    /// A stop with a centred midpoint and tag 0.
    pub const fn new(location: f32, color: Srgba) -> Self {
        Self { location, color, midpoint: 0.5, tag: 0 }
    }

    /// The same stop with the host's `tag`.
    #[must_use]
    pub const fn with_tag(mut self, tag: u64) -> Self {
        self.tag = tag;
        self
    }

    /// The same stop with its midpoint at `midpoint`.
    #[must_use]
    pub const fn with_midpoint(mut self, midpoint: f32) -> Self {
        self.midpoint = midpoint;
        self
    }
}

impl OpacityStop {
    /// A stop with a centred midpoint and tag 0.
    pub const fn new(location: f32, opacity: f32) -> Self {
        Self { location, opacity, midpoint: 0.5, tag: 0 }
    }

    /// The same stop with the host's `tag`.
    #[must_use]
    pub const fn with_tag(mut self, tag: u64) -> Self {
        self.tag = tag;
        self
    }

    /// The same stop with its midpoint at `midpoint`.
    #[must_use]
    pub const fn with_midpoint(mut self, midpoint: f32) -> Self {
        self.midpoint = midpoint;
        self
    }
}

/// A gradient's stops, each list sorted by location.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GradientStops {
    /// The colour stops.
    pub color: Vec<ColorStop>,
    /// The opacity stops; empty under [`StopRows::Combined`], and for an opaque
    /// [`StopRows::Split`] gradient.
    pub opacity: Vec<OpacityStop>,
}

impl GradientStops {
    /// Colour stops and no opacity stops.
    pub fn new(color: Vec<ColorStop>) -> Self {
        Self { color, opacity: Vec::new() }
    }

    /// Colour stops and the opacity stops of a [`StopRows::Split`] gradient.
    pub fn with_opacity(color: Vec<ColorStop>, opacity: Vec<OpacityStop>) -> Self {
        Self { color, opacity }
    }

    /// The opacity stops as the bar shows them: the two 100 % stops an empty list stands for.
    pub fn opacity_stops(&self) -> Cow<'_, [OpacityStop]> {
        if self.opacity.is_empty() { Cow::Owned(vec![OpacityStop::new(0.0, 1.0), OpacityStop::new(1.0, 1.0)]) } else { Cow::Borrowed(&self.opacity) }
    }

    /// How many stops `row` shows.
    pub fn len(&self, row: StopRow) -> usize {
        match row {
            StopRow::Color => self.color.len(),
            StopRow::Opacity => self.opacity_stops().len(),
        }
    }

    /// Whether the colour list is empty.
    pub fn is_empty(&self) -> bool {
        self.color.is_empty()
    }
}

/// A marker of the stop bar: a colour stop, an opacity stop, or the midpoint after stop `i` of a
/// row. Indices are into the stops as the bar shows them ([`GradientStops::opacity_stops`] for
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
}

impl StopColor {
    /// A request to edit colour stop `index`, whose tag is `tag`.
    pub fn new(index: usize, tag: u64, request: ColorRequest) -> Self {
        Self { index, tag, request }
    }
}
