//! `TileZoom`, `ZoomStep` and `ThumbnailSlider`: the size of tiles, by steps or continuously.

use super::words::TileWords;
use crate::buttons::IconButton;
use crate::contract::{
    Access, Bind, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Name, PaintUi, PartsCx, Props as _, Role, WordSetCx as _,
};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{IconId, Metrics, Mixed, NavShape, PartKey, Proposal, Roving, Sense};
use egui::{Rect, Vec2};
use std::ops::RangeInclusive;

/// A step of the tile size, from Cmd/Ctrl + / - / 0 or the slider's end buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ZoomStep {
    /// Bigger.
    In,
    /// Smaller.
    Out,
    /// Back to the default.
    Reset,
}

/// How a tile size changes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum TileZoom {
    /// Factors on [`TileZoom::LADDER`], default 1.0 (card grids).
    Ladder,
    /// A continuous factor on the card thumbnail side (1 is the card thumbnail), `step` per key press, `default`
    /// for Reset.
    #[non_exhaustive]
    Range {
        /// Smallest factor.
        min: f64,
        /// Largest factor.
        max: f64,
        /// Factor change per key press.
        step: f64,
        /// Factor that Reset returns to.
        default: f64,
    },
}

impl TileZoom {
    /// The zoom ladder: 0.6, 0.8, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0.
    pub const LADDER: [f64; 11] = [0.6, 0.8, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0];

    /// A continuous zoom from the factor `min` to `max`, `step` per key press, `default` for Reset.
    pub const fn bounded(min: f64, max: f64, step: f64, default: f64) -> Self {
        TileZoom::Range { min, max, step, default }
    }

    /// The value after `step` from `value`: the next ladder value above or below (a tolerance of 0.001, so a value
    /// between rungs goes to the next one), clamped at the ends; for `Range`, `value` plus or minus `step`, clamped;
    /// `Reset` gives the default.
    pub fn step(&self, value: f64, step: ZoomStep) -> f64 {
        const TOLERANCE: f64 = 0.001;
        match *self {
            TileZoom::Ladder => {
                let ladder = Self::LADDER;
                let (first, last) = (ladder[0], ladder[ladder.len() - 1]);
                match step {
                    ZoomStep::Reset => self.default_value(),
                    ZoomStep::In => ladder.iter().copied().find(|s| *s > value + TOLERANCE).unwrap_or(last),
                    ZoomStep::Out => ladder.iter().rev().copied().find(|s| *s < value - TOLERANCE).unwrap_or(first),
                }
            }
            TileZoom::Range { min, max, step: by, .. } => match step {
                ZoomStep::Reset => self.default_value(),
                ZoomStep::In => (value + by).clamp(min, max),
                ZoomStep::Out => (value - by).clamp(min, max),
            },
        }
    }

    /// The value Reset returns to: 1.0 on the ladder, else the range's default inside its ends.
    pub fn default_value(&self) -> f64 {
        match *self {
            TileZoom::Ladder => 1.0,
            TileZoom::Range { min, max, default, .. } => default.clamp(min, max),
        }
    }

    /// The slider's range: the ladder's ends, or `min..=max`.
    pub fn range(&self) -> RangeInclusive<f64> {
        match *self {
            TileZoom::Ladder => Self::LADDER[0]..=Self::LADDER[Self::LADDER.len() - 1],
            TileZoom::Range { min, max, .. } => min..=max,
        }
    }
}

const OUT: PartKey = PartKey::new("out");
const IN: PartKey = PartKey::new("in");

/// The glyph of the smaller-tiles button and of the larger-tiles button.
const SMALLER_ICON: &str = "grid_small";
const LARGER_ICON: &str = "grid";

/// The look of the thumbnail slider: the gap between its buttons and the slider.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ThumbnailSliderStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// Where the thumbnail slider's three controls lie.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    smaller: Rect,
    track: Rect,
    larger: Rect,
}

/// The tile size slider: a small picture glyph button, a `Slider` over [`TileZoom::range`] and a large picture glyph
/// button. The value is the factor `TileGrid::zoom` takes; a double click on the slider restores the binding's reset
/// value, the zoom's default without one. The buttons step `Out` and `In` by the zoom; a `Range` zoom's step is also the slider's arrow and wheel
/// step, and a `Ladder` zoom's rungs are the slider's marks. The label names the slider.
#[derive(crate::contract::Component)]
pub struct ThumbnailSlider<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f64>,
    zoom: TileZoom,
    placed: Option<Placed>,
}

impl<'a> ThumbnailSlider<'a> {
    /// A slider of the tile size `value` stepping by `zoom`, named `label`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, f64>, zoom: TileZoom) -> Self {
        ThumbnailSlider { common: Common::new(), value, zoom, placed: None }.label(label)
    }

    fn track(&self, shown: Mixed<f64>, name: Label) -> crate::value_drag::Slider<'static> {
        let reset = self.value.reset_value().copied().unwrap_or_else(|| self.zoom.default_value());
        let slider = crate::value_drag::Slider::new(Bind::given(shown).reset(reset), self.zoom.range()).label(name);
        match self.zoom {
            TileZoom::Ladder => slider.marks(&TileZoom::LADDER),
            TileZoom::Range { step, .. } => slider.step(step).wheel_step(step),
        }
    }
}

impl Component for ThumbnailSlider<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    type Style = ThumbnailSliderStyle;
    type Output = EditResponse<f64>;

    fn style(cx: &StyleCx<'_>, _: ThumbnailSliderStyleKey) -> ThumbnailSliderStyle {
        ThumbnailSliderStyle { metrics: Metrics::new().gap(cx.control().gap) }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let button = cx.face(IconButton::new(IconId::new(SMALLER_ICON)));
        let shown = cx.value().copied().map_or(Mixed::Mixed, Mixed::Same);
        let track = cx.child("slider", &mut self.track(shown, cx.label()), Proposal::Ideal);
        let width = 2.0 * (button.x + cx.gap()) + track.x;
        Vec2::new(cx.room.map_or(width, |room| width.min(room)), button.y.max(track.y))
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let button = cx.face(IconButton::new(IconId::new(SMALLER_ICON)));
        let gap = cx.style.metrics.gap;
        let value = cx.value().copied();
        let range = self.zoom.range();
        let flow = craft_behavior::Flow::new(rect, cx.flow.is_rtl());
        let (smaller, rest) = flow.take_start(button.x);
        let (larger, rest) = rest.take_start(gap).1.take_end(button.x);
        let track = rest.take_end(gap).1.rect();
        let centred = |cell: Rect| Rect::from_center_size(cell.center(), button);
        let placed = Placed { smaller: centred(smaller), track, larger: centred(larger) };
        let steps = [(OUT, placed.smaller, value.is_none_or(|v| v > *range.start())), (IN, placed.larger, value.is_none_or(|v| v < *range.end()))];
        for (key, at, usable) in steps {
            let part = cx.part(key, at);
            if usable {
                part.sense(Sense::CLICK | Sense::FOCUS).focusable();
            }
        }
        cx.roving(Roving::new(NavShape::Flat));
        self.placed = Some(placed);
    }

    fn bind(&mut self) -> Option<crate::contract::BindMut<'_, f64>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let current = cx.value().copied().unwrap_or_else(|| self.zoom.default_value());
        for (key, step) in [(OUT, ZoomStep::Out), (IN, ZoomStep::In)] {
            let next = self.zoom.step(current, step);
            if cx.part_activated(key).is_some() && next != current {
                cx.set(next);
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<TileWords>();
        access.node(OUT, Role::BUTTON).name = Name::Text(words.smaller.as_str().to_owned());
        access.node(IN, Role::BUTTON).name = Name::Text(words.larger.as_str().to_owned());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(placed) = self.placed else { return };
        let words = cx.words::<TileWords>();
        let value = cx.value().copied();
        let range = self.zoom.range();
        let smaller = IconButton::new(IconId::new(SMALLER_ICON)).label(words.smaller.clone()).enabled(value.is_none_or(|v| v > *range.start()));
        let larger = IconButton::new(IconId::new(LARGER_ICON)).label(words.larger.clone()).enabled(value.is_none_or(|v| v < *range.end()));
        ui.face(OUT, smaller);
        let shown = value.map_or(Mixed::Mixed, Mixed::Same);
        let track = ui.child("slider", placed.track, self.track(shown, cx.label()));
        ui.face(IN, larger);
        cx.adopt(&track, &self.zoom.default_value(), |picked, mine| *mine = *picked);
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f64> {
        cx.edit_response()
    }
}
