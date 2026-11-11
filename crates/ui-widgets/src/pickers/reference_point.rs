//! `ReferencePoint`: the 3 by 3 picker of a box's nine points. A leaf with a part per point, one roving group over a
//! grid: one Tab stop, the arrows move the focus over the grid in both directions and stop at its edges, Home and End
//! go to the first and last point, and Space, Enter or a click picks the focused one.

use super::words::PickerWords;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, Name, PaintUi, PartsCx, Props as _, Role, SetPosition,
    Styled as _, WordSetCx as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, KeyUse};
use crate::style::{ComponentStyle, Flags, FocusLook, Stateful, States, StyleColor, StyleCx, SurfaceKind, SurfaceLook, UiTokens};
use craft_behavior::{Anchor9, Metrics, NavShape, PartKey, PartRef, Roving, Sense};
use egui::{Pos2, Rect, Vec2, vec2};

#[cfg(feature = "testing")]
mod stories;

const CELL: PartKey = PartKey::new("cell");

/// Points on each side of the grid.
const GRID: usize = 3;

/// The side of the dot every point shows in the compact picker.
const COMPACT_DOT: f32 = 3.5;

/// The arrow's tip, from the cell's centre, as a share of the arrow's extent.
const ARROW_TIP: f32 = 0.5;
/// The arrow's tail, behind the cell's centre, as a share of the arrow's extent.
const ARROW_TAIL: f32 = 5.0 / 12.0;
/// How far the head's point reaches past the shaft's tip, as a share of the arrow's extent.
const ARROW_POINT: f32 = 1.5 / 12.0;
/// The head's length along the arrow, as a share of the arrow's extent.
const ARROW_HEAD: f32 = 3.5 / 12.0;
/// The head's half width, as a share of the arrow's extent.
const ARROW_HEAD_HALF_WIDTH: f32 = 3.5 / 12.0;

/// The style of a reference point picker.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ReferencePointStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(surface)]
    surface: SurfaceLook,
    #[style(focus)]
    focus: FocusLook,
    /// The side of one point's cell.
    cell: f32,
    /// The side of the mark in the picked cell.
    mark: f32,
    /// The side of the dot every point shows in the compact picker.
    compact_dot: f32,
    /// The mark's corner radius.
    mark_radius: egui::CornerRadius,
    /// The picked cell's fill.
    picked: Stateful<StyleColor>,
    /// The mark in the picked cell.
    ink: Stateful<StyleColor>,
    /// The side of the square one growth arrow fits in.
    arrow: f32,
    /// The growth arrows.
    arrow_ink: Stateful<StyleColor>,
}

/// The 3 by 3 picker of a box's nine points: the point a transform, a resize or a watermark works about. The nine
/// points are one Tab stop; the arrows move the focus and stop at the grid's edges, and Space, Enter or a click picks
/// the focused point. The arrows are directions in the box, so they do not mirror in a right-to-left interface and
/// neither does the grid. With [`growth`](ReferencePoint::growth) the cells next to the pick show arrows toward where
/// the box grows. The [`compact`](ReferencePoint::compact) picker is the size of an options bar's icon: a dot in
/// every cell, the pick's in the accent colour, and no arrows.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(exempt(case = "rtl", reason = "The nine points are positions in a box, not reading order: the left point stays on the left."))]
pub struct ReferencePoint<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Anchor9>,
    /// The change in the box's width and height: the arrows point away from the pick where the box grows and toward
    /// it where the box shrinks. Without it the cells show no arrows.
    #[prop]
    growth: Option<(f64, f64)>,
    /// The options-bar size: the whole grid is `size.icon.md` square. It shows no growth arrows.
    #[style_key]
    #[prop(flag)]
    compact: bool,
}

impl<'a> ReferencePoint<'a> {
    /// A picker named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Anchor9>) -> Self {
        ReferencePoint { common: Common::new(), value, growth: None, compact: false }.label(label)
    }

    /// The direction of the arrow `cell` shows when the pick is `anchor`, in cell steps (each -1, 0 or 1): away from
    /// the pick where the box grows along an axis and toward it where it shrinks. Only the cells next to the pick
    /// have one.
    pub fn arrow(anchor: Anchor9, cell: Anchor9, growth: (f64, f64)) -> Option<(i32, i32)> {
        let ((pick_column, pick_row), (column, row)) = (anchor.cell(), cell.cell());
        let (dx, dy) = (column as i32 - pick_column as i32, row as i32 - pick_row as i32);
        if (dx, dy) == (0, 0) || dx.abs() > 1 || dy.abs() > 1 {
            return None;
        }
        let direction = |change: f64| if change < 0.0 { -1 } else { 1 };
        Some((dx * direction(growth.0), dy * direction(growth.1)))
    }

    fn part(at: usize) -> PartRef {
        CELL.at(at as u64)
    }

    fn position_of(value: Option<&Anchor9>) -> Option<usize> {
        value.and_then(|value| Anchor9::ALL.iter().position(|point| point == value))
    }

    /// The cell of point `at`: the grid's rect in thirds, laid out as the points sit in the box, left to right
    /// whatever the reading direction.
    fn cell_rect(rect: Rect, at: usize) -> Rect {
        let (column, row) = Anchor9::ALL[at].cell();
        let cell = rect.size() / GRID as f32;
        Rect::from_min_size(rect.min + vec2(column as f32 * cell.x, row as f32 * cell.y), cell)
    }

    fn name_of(point: Anchor9, words: &PickerWords) -> &Label {
        match point {
            Anchor9::TopLeft => &words.point_top_left,
            Anchor9::Top => &words.point_top,
            Anchor9::TopRight => &words.point_top_right,
            Anchor9::Left => &words.point_left,
            Anchor9::Centre => &words.point_centre,
            Anchor9::Right => &words.point_right,
            Anchor9::BottomLeft => &words.point_bottom_left,
            Anchor9::Bottom => &words.point_bottom,
            Anchor9::BottomRight => &words.point_bottom_right,
            _ => &words.point_centre,
        }
    }
}

impl Component for ReferencePoint<'_> {
    const ROLE: Role = Role::RADIO_GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Both);
    type Style = ReferencePointStyle;
    type Output = EditResponse<Anchor9>;

    fn style(cx: &StyleCx<'_>, key: ReferencePointStyleKey) -> ReferencePointStyle {
        let sunken = cx.surface(SurfaceKind::Sunken);
        let colors = &cx.tokens.color;
        let edge = Stateful::new(cx.color(colors.line.control_edge));
        let disabled = cx.color(colors.text.disabled);
        let look = |rest: StyleColor| Stateful::new(rest).off(States::new(rest, rest, rest, disabled));
        ReferencePointStyle {
            metrics: Metrics::new(),
            surface: SurfaceLook::new(Stateful::new(StyleColor(sunken.fill)))
                .kind(SurfaceKind::Sunken)
                .radius(sunken.radius)
                .border(edge, cx.tokens.stroke.thin),
            focus: cx.focus_look(),
            cell: if key.compact { cx.tokens.size.icon.md / GRID as f32 } else { cx.tokens.size.control.sm },
            compact_dot: COMPACT_DOT,
            mark: cx.tokens.size.icon.sm,
            mark_radius: UiTokens::corner(cx.tokens.shape.radius.sm),
            picked: look(cx.color(colors.accent.soft)),
            ink: look(cx.color(colors.accent.text)),
            arrow: cx.tokens.size.icon.sm,
            arrow_ink: look(cx.color(colors.text.muted)),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.cell * GRID as f32)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let current = Self::position_of(cx.value());
        for at in 0..Anchor9::ALL.len() {
            let part = cx.part(Self::part(at), Self::cell_rect(rect, at)).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if current == Some(at) {
                part.flags(Flags::CHECKED).selected();
            }
        }
        cx.roving(Roving::new(NavShape::bounded_grid(GRID)));
    }

    fn bind(&mut self) -> Option<BindMut<'_, Anchor9>> {
        Some(self.value.reborrow())
    }

    /// The roving group moves the focus and stops at the grid's edges; a click, Space or Enter on a point picks it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let current = Self::position_of(cx.value());
        let activated = (0..Anchor9::ALL.len()).find(|at| cx.part_activated(Self::part(*at)).is_some());
        if let Some(at) = activated.filter(|at| current != Some(*at)) {
            cx.set(Anchor9::ALL[at]);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<PickerWords>();
        for (at, point) in Anchor9::ALL.iter().enumerate() {
            let node = access.node(Self::part(at), Role::RADIO_BUTTON);
            node.name = Name::Text(cx.env.resolve(Self::name_of(*point, &words)).as_str().to_owned());
            node.position = Some(SetPosition::new(at).of(Anchor9::ALL.len()));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let current = Self::position_of(cx.value());
        debug_assert!(!(self.compact && self.growth.is_some()), "the compact picker shows no growth arrows");
        let draw = ui.draw();
        if self.compact {
            for (at, cell) in (0..Anchor9::ALL.len()).map(|at| (at, Self::cell_rect(cx.rect, at))) {
                let dot = Rect::from_center_size(cell.center(), Vec2::splat(cx.style.compact_dot));
                let ink = if current == Some(at) { &cx.style.ink } else { &cx.style.arrow_ink };
                draw.fill(&PaintShape::Rect(dot, cx.style.mark_radius), cx.resolve(ink));
            }
            return;
        }
        for (at, point) in Anchor9::ALL.iter().enumerate() {
            let cell = Self::cell_rect(cx.rect, at);
            if current == Some(at) {
                draw.fill(&PaintShape::Rect(cell, cx.style.mark_radius), cx.resolve(&cx.style.picked));
                let mark = Rect::from_center_size(cell.center(), Vec2::splat(cx.style.mark));
                draw.fill(&PaintShape::Rect(mark, cx.style.mark_radius), cx.resolve(&cx.style.ink));
            } else if let Some(growth) = self.growth
                && let Some(anchor) = current.map(|pick| Anchor9::ALL[pick])
                && let Some((dx, dy)) = Self::arrow(anchor, *point, growth)
            {
                let direction = vec2(dx as f32, dy as f32).normalized();
                let color = cx.resolve(&cx.style.arrow_ink);
                let [tail, tip, base_left, base_right, point_tip] = arrow(cell.center(), direction, cx.style.arrow);
                draw.stroke(&PaintShape::Path(vec![tail, tip]), StrokeWidth::Medium, color);
                draw.fill(&PaintShape::Polygon(vec![point_tip, base_left, base_right]), color);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Anchor9> {
        cx.edit_response()
    }
}

/// The corners of an arrow centred on `center` along `direction` (unit length) in a square of side `extent`: tail,
/// tip of the shaft, the two corners of the head's base and the head's point.
fn arrow(center: Pos2, direction: Vec2, extent: f32) -> [Pos2; 5] {
    let tip = center + direction * ARROW_TIP * extent;
    let tail = center - direction * ARROW_TAIL * extent;
    let side = vec2(-direction.y, direction.x) * ARROW_HEAD_HALF_WIDTH * extent;
    let base = tip - direction * ARROW_HEAD * extent;
    [tail, tip, base + side, base - side, tip + direction * ARROW_POINT * extent]
}
