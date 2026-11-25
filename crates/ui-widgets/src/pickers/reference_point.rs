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
use craft_theme::Rgba;
use egui::{CornerRadius, Pos2, Rect, Vec2, vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

const CELL: PartKey = PartKey::new("cell");

/// Points on each side of the grid.
const GRID: usize = 3;

/// The eight outer points in the order the `Squares` line joins them round the box; the centre point is left out.
const RING: [usize; 8] = [0, 1, 2, 5, 8, 7, 6, 3];

/// The side of the dot every point shows; the large picker draws the picked one twice as wide.
const DOT: f32 = 3.5;

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

/// The length of an alignment cell's short bar, as a share of the long bar.
const BAR_SHORT: f32 = 0.6;
/// The distance between an alignment cell's bars, as a share of the long bar.
const BAR_GAP: f32 = 0.25;

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
    /// The side of one point's cell; in the `Squares` look, the step from one square's centre to the next.
    cell: f32,
    /// The side of an unpicked point's dot.
    dot: f32,
    /// The side of the picked point's dot.
    picked_dot: f32,
    /// The dots' corner radius.
    dot_radius: egui::CornerRadius,
    /// The picked point's dot or square: `accent.text`, `text.normal` in the `Squares` look.
    ink: Stateful<StyleColor>,
    /// The side of the square one growth arrow fits in.
    arrow: f32,
    /// The unpicked dots and squares, the line joining the squares and the growth arrows.
    arrow_ink: Stateful<StyleColor>,
    /// The side of a square of the `Squares` look, which is two cells and a square wide.
    square: f32,
    /// The width of the `Squares` line and of the unpicked squares' outline.
    line: StrokeWidth,
    /// The length of a long text bar of an alignment cell.
    bar: f32,
    /// The length of an alignment cell's short text bar.
    bar_short: f32,
    /// The distance between an alignment cell's text bars.
    bar_gap: f32,
    /// The fill of the picked alignment cell.
    picked_fill: Stateful<StyleColor>,
    /// The corner radius of the picked alignment cell's fill.
    fill_radius: egui::CornerRadius,
    /// The ink of an alignment cell's text bars.
    bar_ink: Stateful<StyleColor>,
}

/// What a [`ReferencePoint`]'s pick means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReferencePointKind {
    /// A point of a box, drawn as the picker's [`ReferencePointLook`] sets.
    #[default]
    Point,
    /// The alignment of text in a cell: three text bars in each cell, set to its side and edge.
    Alignment,
}

/// How a [`ReferencePoint`] draws its nine points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum ReferencePointLook {
    /// A dot for each point, the pick's twice as wide.
    #[default]
    Dots,
    /// Eight outer squares joined by one line round the box and a lone centre square in the muted ink, the pick
    /// filled in the normal text ink, on the surface around the picker with no well of its own. The squares are
    /// `shape.locator.square` on a side, `shape.locator.cell` apart, the same in the compact picker. Growth arrows are
    /// in `Dots` only.
    Squares,
}

/// The 3 by 3 picker of a box's nine points: the point a transform, a resize or a watermark works about. The nine
/// points are one Tab stop; the arrows move the focus and stop at the grid's edges, and Space, Enter or a click picks
/// the focused point. The arrows are directions in the box, so they do not mirror in a right-to-left interface and
/// neither does the grid. Every point shows a muted dot, the pick's twice as wide in the accent colour, and the cell
/// under the pointer takes the hover fill. A mixed value picks no point. With [`growth`](ReferencePoint::growth) the
/// cells next to the pick show arrows toward where the box grows. The [`compact`](ReferencePoint::compact) picker is
/// the size of an options bar's icon, its pick's dot as wide as the others, with no arrows.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(exempt(case = "rtl", reason = "The nine points are positions in a box, not reading order: the left point stays on the left."))]
pub struct ReferencePoint<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Option<Anchor9>>,
    /// What the pick means: a point of a box, or the alignment of text in a cell.
    #[style_key]
    #[prop]
    kind: ReferencePointKind,
    /// The change in the box's width and height: the arrows point away from the pick where the box grows and toward
    /// it where the box shrinks. Without it the cells show no arrows.
    #[prop]
    growth: Option<(f64, f64)>,
    /// The options-bar size: the whole grid is `size.icon.md` square. It shows no growth arrows.
    #[style_key]
    #[prop(flag)]
    compact: bool,
    /// How the nine points are drawn: dots, or squares joined by a line round the box.
    #[style_key]
    #[prop]
    look: ReferencePointLook,
}

impl<'a> ReferencePoint<'a> {
    /// A picker named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<Anchor9>>) -> Self {
        ReferencePoint {
            common: Common::new(),
            value,
            kind: ReferencePointKind::default(),
            growth: None,
            compact: false,
            look: ReferencePointLook::Dots,
        }
        .label(label)
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

    fn position_of(value: Option<&Option<Anchor9>>) -> Option<usize> {
        let value = value.and_then(Option::as_ref);
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
    type Output = EditResponse<Option<Anchor9>>;

    fn style(cx: &StyleCx<'_>, key: ReferencePointStyleKey) -> ReferencePointStyle {
        let sunken = cx.surface(SurfaceKind::Sunken);
        let colors = &cx.tokens.color;
        let edge = Stateful::new(cx.color(colors.line.control_edge));
        let disabled = cx.color(colors.text.disabled);
        let look = |rest: StyleColor| Stateful::new(rest).off(States::new(rest, rest, rest, disabled));
        let squares = key.look == ReferencePointLook::Squares;
        // The squares stand on the surface around them, joined by their own line; the dots sit in a sunken well.
        let surface = if squares {
            SurfaceLook::new(Stateful::new(cx.color(Rgba::TRANSPARENT)))
        } else {
            SurfaceLook::new(Stateful::new(StyleColor(sunken.fill)))
                .kind(SurfaceKind::Sunken)
                .radius(sunken.radius)
                .border(edge, cx.tokens.stroke.thin)
        };
        ReferencePointStyle {
            metrics: Metrics::new(),
            surface,
            focus: cx.focus_look(),
            cell: match (squares, key.compact) {
                (true, _) => cx.tokens.shape.locator.cell,
                (false, true) => cx.tokens.size.icon.md / GRID as f32,
                (false, false) => cx.tokens.size.control.md,
            },
            dot: DOT,
            picked_dot: if key.compact { DOT } else { DOT * 2.0 },
            dot_radius: UiTokens::corner(cx.tokens.shape.radius.sm),
            ink: look(cx.color(if squares { colors.text.normal } else { colors.accent.text })),
            arrow: cx.tokens.size.icon.sm,
            arrow_ink: look(cx.color(colors.text.muted)),
            square: cx.tokens.shape.locator.square,
            line: StrokeWidth::Thin,
            bar: cx.tokens.size.icon.sm,
            bar_short: cx.tokens.size.icon.sm * BAR_SHORT,
            bar_gap: cx.tokens.size.icon.sm * BAR_GAP,
            picked_fill: look(cx.color(colors.state.selected)),
            fill_radius: UiTokens::corner(cx.tokens.shape.radius.control),
            bar_ink: look(cx.color(colors.text.muted)),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(if self.look == ReferencePointLook::Squares {
            cx.style.cell * (GRID - 1) as f32 + cx.style.square
        } else {
            cx.style.cell * GRID as f32
        })
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let current = Self::position_of(cx.value());
        let words = cx.words::<PickerWords>();
        let names: Vec<Label> = Anchor9::ALL.iter().map(|point| Self::name_of(*point, &words).clone()).collect();
        for (at, name) in names.into_iter().enumerate() {
            let part = cx.part(Self::part(at), Self::cell_rect(rect, at)).sense(Sense::CLICK | Sense::FOCUS).focusable().state_layer().tooltip(name);
            if current == Some(at) {
                part.flags(Flags::CHECKED).selected();
            }
        }
        cx.roving(Roving::new(NavShape::bounded_grid(GRID)));
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<Anchor9>>> {
        Some(self.value.reborrow())
    }

    /// The roving group moves the focus and stops at the grid's edges; a click, Space or Enter on a point picks it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let current = Self::position_of(cx.value());
        let activated = (0..Anchor9::ALL.len()).find(|at| cx.part_activated(Self::part(*at)).is_some());
        if let Some(at) = activated {
            if current != Some(at) {
                cx.set(Some(Anchor9::ALL[at]));
            } else if self.kind == ReferencePointKind::Alignment {
                cx.set(None);
            }
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
        let squares = self.look == ReferencePointLook::Squares;
        debug_assert!(!(self.growth.is_some() && (self.compact || squares)), "the compact picker and the squares look show no growth arrows");
        debug_assert!(
            self.kind == ReferencePointKind::Point || (!self.compact && self.growth.is_none() && !squares),
            "only the point picker has growth arrows, a compact size and the squares look"
        );
        let draw = ui.draw();
        if self.kind == ReferencePointKind::Alignment {
            let color = cx.resolve(&cx.style.bar_ink);
            let gap = cx.style.bar_gap;
            for (at, point) in Anchor9::ALL.iter().enumerate() {
                let cell = Self::cell_rect(cx.rect, at);
                if current == Some(at) {
                    draw.fill(&PaintShape::Rect(cell, cx.style.fill_radius), cx.resolve(&cx.style.picked_fill));
                }
                let inset = ((cell.width() - cx.style.bar) / 2.0).max(0.0);
                let inner = cell.shrink(inset);
                let (column, row) = point.cell();
                // The middle bar's height; the other two sit one gap above and below it.
                let middle = match row {
                    0 => inner.top() + gap,
                    1 => inner.center().y,
                    _ => inner.bottom() - gap,
                };
                for (line, length) in [cx.style.bar, cx.style.bar_short, cx.style.bar].into_iter().enumerate() {
                    let start = match column {
                        0 => inner.left(),
                        1 => inner.center().x - length / 2.0,
                        _ => inner.right() - length,
                    };
                    let y = middle + (line as f32 - 1.0) * gap;
                    draw.stroke(&PaintShape::Path(vec![Pos2::new(start, y), Pos2::new(start + length, y)]), StrokeWidth::Medium, color);
                }
            }
            return;
        }
        if squares {
            // The outer squares' edges meet the picker's edges, a cell apart.
            let first = cx.rect.min + Vec2::splat(cx.style.square / 2.0);
            let centres: Vec<Pos2> = Anchor9::ALL
                .iter()
                .map(|point| {
                    let (column, row) = point.cell();
                    first + vec2(column as f32, row as f32) * cx.style.cell
                })
                .collect();
            let (half, outline) = (cx.style.square / 2.0, cx.resolve(&cx.style.arrow_ink));
            // Each side of the ring runs between the squares it joins, so the line never crosses a square.
            for (from, to) in RING.iter().zip(RING.iter().cycle().skip(1)) {
                let (from, to) = (centres[*from], centres[*to]);
                let along = (to - from).normalized() * half;
                draw.stroke(&PaintShape::Path(vec![from + along, to - along]), cx.style.line, outline);
            }
            for (at, centre) in centres.iter().enumerate() {
                let square = PaintShape::Rect(Rect::from_center_size(*centre, Vec2::splat(cx.style.square)), CornerRadius::ZERO);
                if current == Some(at) {
                    draw.fill(&square, cx.resolve(&cx.style.ink));
                } else {
                    draw.stroke(&square, cx.style.line, outline);
                }
            }
            return;
        }
        for (at, point) in Anchor9::ALL.iter().enumerate() {
            let cell = Self::cell_rect(cx.rect, at);
            let (side, ink) = if current == Some(at) { (cx.style.picked_dot, &cx.style.ink) } else { (cx.style.dot, &cx.style.arrow_ink) };
            draw.fill(&PaintShape::Rect(Rect::from_center_size(cell.center(), Vec2::splat(side)), cx.style.dot_radius), cx.resolve(ink));
            if let Some(growth) = self.growth
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

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Option<Anchor9>> {
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
