//! `TransformBox`: the box of Free Transform and of a Move tool's transform controls, a canvas control (20.6). Its
//! corners, edges, reference point, inside and outside are parts in the canvas's arena; the value is the box in
//! document units.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::geometry::SnapTargets;
use craft_behavior::geometry::quad::{self, BoxDrag, BoxPart, BoxRules, BoxState, TransformMode};
use craft_behavior::geometry::snap::SNAP_PX;
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{
    Axis, CanvasView, Cursor, CursorIcon, CursorPriority, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PaintedCursor, PartKey,
    PartRef, PointerButton, PressPhase, Roving, Sense, resize_cursor,
};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasSpace, canvas_space, readout};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::runtime::focus::{FocusShape, KeyUse};
use craft_ui::style::{ComponentStyle, HandleLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

/// The part of the box a handle, the inside or the reference point is: `BOX.at(part.index())`.
pub const BOX: PartKey = PartKey::new("box");

/// The parts that stand for the outside of the box: `OUTSIDE.at(0)` in [`RotateZone::Outside`], and `OUTSIDE.at(i)`
/// for the band at corner `i` in [`RotateZone::Band`].
const OUTSIDE: PartKey = PartKey::new("outside");

/// The keyboard path of a rotation by the outside parts: the options bar's angle field, which the host shows beside the box.
const ROTATE_BY_KEY: &str = "the angle field in the options bar";

/// The distance an arrow key moves a part, in document units; Shift moves it by `NUDGE_FAR`.
const NUDGE: f64 = 1.0;
const NUDGE_FAR: f64 = 10.0;

/// Where a press outside the box rotates it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RotateZone {
    /// Anywhere outside (PhotoCraft's Free Transform); the rotate pointer shows within the rotate reach of a corner.
    #[default]
    Outside,
    /// Only the band outside a corner (PhotoCraft's Move tool controls); the rest of the outside is the tool's.
    Band,
}

craft_ui::words! {
    /// The names of the box and its parts, and the labels of its readout.
    pub struct TransformBoxWords {
        group: "transform.group" = "Free Transform";
        whole: "transform.whole" = "Whole box";
        top_left_corner: "transform.topLeftCorner" = "Top left corner";
        top_right_corner: "transform.topRightCorner" = "Top right corner";
        bottom_right_corner: "transform.bottomRightCorner" = "Bottom right corner";
        bottom_left_corner: "transform.bottomLeftCorner" = "Bottom left corner";
        top_edge: "transform.topEdge" = "Top edge";
        right_edge: "transform.rightEdge" = "Right edge";
        bottom_edge: "transform.bottomEdge" = "Bottom edge";
        left_edge: "transform.leftEdge" = "Left edge";
        pivot: "transform.pivot" = "Reference point";
        readout_angle: "transform.readoutAngle" = "Angle:";
        readout_width: "transform.readoutWidth" = "W:";
        readout_height: "transform.readoutHeight" = "H:";
        readout_dx: "transform.readoutDx" = "ΔX:";
        readout_dy: "transform.readoutDy" = "ΔY:";
    }
}

impl TransformBoxWords {
    /// The name of `part` in the accessibility tree.
    fn name_of(&self, part: BoxPart) -> &str {
        match part {
            BoxPart::Corner(0) => self.top_left_corner.as_str(),
            BoxPart::Corner(1) => self.top_right_corner.as_str(),
            BoxPart::Corner(2) => self.bottom_right_corner.as_str(),
            BoxPart::Corner(_) => self.bottom_left_corner.as_str(),
            BoxPart::Edge(0) => self.top_edge.as_str(),
            BoxPart::Edge(1) => self.right_edge.as_str(),
            BoxPart::Edge(2) => self.bottom_edge.as_str(),
            BoxPart::Edge(_) => self.left_edge.as_str(),
            BoxPart::Pivot => self.pivot.as_str(),
            _ => self.whole.as_str(),
        }
    }
}

/// The style of the transform box.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TransformBoxStyle {
    #[style(metrics)]
    metrics: Metrics,
    handle: HandleLook,
    line: StyleColor,
    halo: StyleColor,
    /// The hit radius of a handle.
    hit: f32,
    /// How far from a corner the rotate pointer shows outside the box.
    rotate_reach: f32,
    band_inner: f32,
    band_outer: f32,
    pivot_radius: f32,
    pivot_arm: f32,
}

/// What the box reports beyond its edit.
#[derive(Clone, Copy, Debug, Default)]
struct Report {
    apply: bool,
    discard: bool,
    hovered: Option<BoxPart>,
    picked: Option<BoxPart>,
}

/// The press the box is dragging a part with, kept from the press to its release.
#[derive(Clone, Copy, Debug)]
struct Grip {
    part: BoxPart,
    /// The box the drag works from: the box at the press, its reference point moved by an Alt press.
    start: BoxState,
    /// The press in document space.
    from: Point,
    began: bool,
    /// An Alt press away from the handles: a click there moves the reference point.
    moves_pivot: bool,
    /// The box the drag last produced.
    last: BoxState,
}

impl Default for Grip {
    fn default() -> Self {
        let none = BoxState::from_rect(DocRect::ZERO);
        Grip { part: BoxPart::Inside, start: none, from: Point::ZERO, began: false, moves_pivot: false, last: none }
    }
}

/// The box of Free Transform: eight handles and a reference point over the host's content, dragged through
/// [`quad::drag`]'s rules, moved and picked from the keyboard.
///
/// - The value is the box in document units. A drag is one gesture (`Begin` on the first move, `Commit` on the
///   release); Escape cancels it. The host applies the box on [`TransformBoxOutput::apply`] (Enter, a click outside,
///   a double click inside) and closes it on [`discard`](TransformBoxOutput::discard).
/// - The pointer takes the nearest handle or the reference point, else the inside or the outside. Parts the
///   [rules](Self::rules) refuse take no press, except the outside in [`RotateZone::Outside`], where a click applies.
///   An Alt press away from the handles moves the reference point there and drags it.
/// - The box is one Tab stop with one part group, whose arrival is the picked part. Page Down and Page Up pick the
///   next and previous part (the whole box, the corners and edges clockwise from the top left, the reference
///   point), Home and End the first and last. The arrows move the picked part as a plain drag of it would, one
///   document unit, Shift ten.
/// - Nothing mirrors in a right-to-left interface: the box is the document's.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct TransformBox<'a> {
    #[common]
    common: Common,
    state: Bind<'a, BoxState>,
    start: DocRect,
    skew_v: f64,
    /// The mode, scaling rule, aspect link, what the content allows and the rotate step.
    #[prop]
    rules: BoxRules,
    /// Whether the reference point shows. Hidden, it is not drawn, hit or picked, and an Alt press away from the
    /// handles does nothing; its stored position still pivots Alt scaling and rotating.
    #[prop]
    reference_point: bool,
    /// Where a press outside rotates.
    #[prop]
    rotate_zone: RotateZone,
    /// Whether a press inside moves the box. Off leaves it to the tool.
    #[prop]
    inside_moves: bool,
    /// Rounds an upright box and its corners to whole document units before it is proposed (Snap Transforms to Pixel
    /// Grid). Keys are not rounded.
    #[prop]
    pixel_grid: bool,
    /// The modifiers every test of this box reads in place of the pointer's: a touch host's sticky modifiers.
    #[prop]
    mods: Option<Mods>,
    /// The host's snapping of a dragged point.
    #[prop]
    snap: Option<&'a SnapTargets>,
    report: Report,
}

impl<'a> TransformBox<'a> {
    /// A box named `label` over `state`, a box in document units. `start` is the rect before the transform and
    /// `skew_v` the vertical skew the host keeps in degrees: the readout reads the width and height against them.
    pub fn new(label: impl Into<Label>, state: Bind<'a, BoxState>, start: DocRect, skew_v: f64) -> Self {
        TransformBox {
            common: Common::new(),
            state,
            start,
            skew_v,
            rules: BoxRules::default(),
            reference_point: true,
            rotate_zone: RotateZone::Outside,
            inside_moves: true,
            pixel_grid: false,
            mods: None,
            snap: None,
            report: Report::default(),
        }
        .label(label)
    }

    /// The parts the keyboard walks, in order.
    fn keyboard_parts(&self) -> Vec<BoxPart> {
        let corners = (0..4u8).map(BoxPart::Corner);
        let edges = (0..4u8).map(BoxPart::Edge);
        let pivot = self.reference_point.then_some(BoxPart::Pivot);
        [BoxPart::Inside].into_iter().chain(corners).chain(edges).chain(pivot).filter(|&part| self.rules.allows(part)).collect()
    }

    /// The box after an arrow key moves `part` by `by`: as a plain drag of the part would.
    fn nudged(&self, state: &BoxState, part: BoxPart, by: DocVec) -> BoxState {
        let at = match part {
            BoxPart::Corner(i) => state.quad[usize::from(i % 4)],
            BoxPart::Edge(i) => state.quad[usize::from(i % 4)].midpoint(state.quad[usize::from(i + 1) % 4]),
            BoxPart::Pivot => state.pivot,
            _ => return state.moved(by),
        };
        quad::drag(state, state, part, at, at + by, Mods::NONE, &self.rules)
    }

    /// The box the pointer at screen position `pos` gives a drag, snapped and rounded.
    fn proposal(&self, space: &CanvasSpace, grip: &Grip, pos: Pos2, mods: Mods) -> BoxState {
        let to = space.view.to_doc(pos);
        let to = self.snap.map_or(to, |targets| targets.snap_point(to, space.view.doc_len(SNAP_PX as f32)).0);
        let dragged = quad::drag(&grip.start, &grip.last, grip.part, grip.from, to, mods, &self.rules);
        if self.pixel_grid { on_pixel_grid(dragged) } else { dragged }
    }

    /// The pointer for `part` at `pointer`, `dragging` during its drag.
    fn cursor(&self, look: &CursorLook<'_>, part: BoxPart, pointer: Pos2, mods: Mods, dragging: bool) -> Cursor {
        let Some(kind) = self.rules.drag_kind(part, mods) else { return CursorIcon::Default.into() };
        let (screen, view, style) = (look.screen, &look.space.view, look.style);
        let (corner, away) = nearest_corner(screen, pointer);
        let turn = || Cursor::from(PaintedCursor::turn(quad::bisector(screen, corner)));
        let along = |from: usize| vec2_of(screen.quad[(from + 1) % 4] - screen.quad[from % 4]);
        match (kind, part) {
            (BoxDrag::Rotate, BoxPart::Outside) => {
                if dragging || self.rotate_zone == RotateZone::Band || away <= style.rotate_reach {
                    turn()
                } else {
                    CursorIcon::Alias.into()
                }
            }
            (BoxDrag::Rotate, _) if dragging || away <= style.rotate_reach => turn(),
            (BoxDrag::Move, _) => CursorIcon::Move.into(),
            (BoxDrag::Pivot, _) => CursorIcon::Crosshair.into(),
            (BoxDrag::Distort | BoxDrag::Perspective, BoxPart::Corner(i)) => PaintedCursor::distort(-quad::bisector(screen, usize::from(i))).into(),
            (BoxDrag::Skew | BoxDrag::SlideCorner, BoxPart::Edge(i) | BoxPart::Corner(i)) => PaintedCursor::skew(along(usize::from(i))).into(),
            (_, BoxPart::Corner(i)) => resize_cursor(view, view.unmap_vec(-quad::bisector(screen, usize::from(i)))).into(),
            (_, BoxPart::Edge(i)) => {
                let i = usize::from(i);
                let mid = |k: usize| screen.quad[k % 4].midpoint(screen.quad[(k + 1) % 4]);
                resize_cursor(view, view.unmap_vec(vec2_of(mid(i) - mid(i + 2)))).into()
            }
            _ => CursorIcon::Default.into(),
        }
    }

    /// The rows of the readout of a drag of `part`: the box `now` against the box `at_press`.
    fn readout_rows(&self, cx: &Cx<'_, Self>, space: &CanvasSpace, part: BoxPart, now: &BoxState, at_press: &BoxState) -> Vec<(Label, String)> {
        let words = cx.words::<TransformBoxWords>();
        let units = cx.env.units();
        let row = |label: &Label, value: String| (label.clone(), value);
        let rotating = part == BoxPart::Outside
            || (matches!(part, BoxPart::Corner(_) | BoxPart::Edge(_)) && self.rules.effective_mode() == TransformMode::Rotate);
        if rotating {
            return vec![row(&words.readout_angle, units.text(UnitKind::Degrees, fixed(now.angle(), 1), 1..=1))];
        }
        match part {
            BoxPart::Inside => {
                let moved = now.quad[0] - at_press.quad[0];
                let rulers = space.rulers();
                let (across_x, _) = rulers.across(Axis::Y);
                let (across_y, _) = rulers.across(Axis::X);
                vec![
                    row(&words.readout_dx, across_x.format_value(&units, fixed(moved.x, 4), 0.0)),
                    row(&words.readout_dy, across_y.format_value(&units, fixed(moved.y, 4), 0.0)),
                ]
            }
            BoxPart::Corner(_) | BoxPart::Edge(_) if now.is_parallelogram() => {
                let parts = now.parts(self.start, self.skew_v);
                let percent = |v: f64| units.text(UnitKind::Percent, fixed(v * 100.0, 1), 1..=1);
                vec![row(&words.readout_width, percent(parts.w)), row(&words.readout_height, percent(parts.h))]
            }
            _ => Vec::new(),
        }
    }

    /// The parts that take a press, with the box part each stands for.
    fn pointer_parts(&self) -> Vec<(PartRef, BoxPart)> {
        let mut parts: Vec<_> = self.keyboard_parts().into_iter().map(|part| (BOX.at(part.index()), part)).collect();
        parts.extend((0..4).map(|corner| (OUTSIDE.at(corner), BoxPart::Outside)));
        parts
    }

    /// The pointer on the parts: presses, drags, releases and the cursor.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &BoxState) {
        let screen = on_screen(&space.view, shown);
        let style = cx.style;
        let mut held: Option<Grip> = *cx.press_slot::<Grip>();
        for (reference, part) in self.pointer_parts() {
            let Some(result) = cx.part_if_declared(reference) else { continue };
            let (press, hover) = (result.press, result.hover_pos.filter(|_| result.hovered));
            let mods = self.mods.unwrap_or(press.mods);
            if let Some(pos) = hover
                && held.is_none()
            {
                self.report.hovered = Some(part);
                let cursor = self.cursor(&CursorLook { space, screen: &screen, style }, part, pos, mods, false);
                cx.cursor(cursor, CursorPriority::Part);
            }
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let from = space.view.to_doc(start.origin);
                    let moves_pivot = mods.alt && self.reference_point && matches!(part, BoxPart::Inside | BoxPart::Outside);
                    let (part, begin) = if moves_pivot { (BoxPart::Pivot, BoxState::new(shown.quad, from)) } else { (part, *shown) };
                    held = Some(Grip { part, start: begin, from, began: false, moves_pivot, last: *shown });
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    let Some(mut grip) = held else { continue };
                    if !self.rules.allows(grip.part) {
                        continue;
                    }
                    if matches!(press.phase, PressPhase::DragStart { .. }) {
                        cx.begin();
                        grip.began = true;
                    }
                    if !grip.began {
                        continue;
                    }
                    let next = self.proposal(space, &grip, pos, mods);
                    grip.last = next;
                    held = Some(grip);
                    cx.propose(next);
                    let rows = self.readout_rows(cx, space, grip.part, &next, &grip.start);
                    readout(&*cx, pos, &rows);
                    let cursor = self.cursor(&CursorLook { space, screen: &on_screen(&space.view, &next), style }, grip.part, pos, mods, true);
                    cx.cursor(cursor, CursorPriority::Drag);
                }
                PressPhase::Release { start, click, .. } => {
                    // A double click inside applies, whether or not the first press of the release was seen.
                    if click && start.clicks >= 2 && start.button == PointerButton::Primary && part == BoxPart::Inside {
                        self.report.apply = true;
                    }
                    let Some(grip) = held.take() else { continue };
                    if grip.began {
                        cx.commit();
                    } else if grip.part == BoxPart::Outside && click && start.clicks <= 1 {
                        self.report.apply = true;
                    } else if grip.moves_pivot && click {
                        cx.set(grip.start);
                    }
                }
                PressPhase::Cancel { .. } => held = None,
                _ => {}
            }
        }
        *cx.press_slot::<Grip>() = held;
    }

    /// The keys of the focused box: the arrows nudge the picked part, Enter applies and Escape closes the box.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &BoxState, pressing: bool) {
        let picked = cx.active_part().filter(|part| part.key == BOX).and_then(|part| part.index).and_then(BoxPart::from_index);
        self.report.picked = picked;
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let doc = space.view.unmap_vec(direction);
            let length = doc.hypot();
            if length > 0.0 {
                let part =
                    picked.filter(|&part| self.rules.allows(part) && (self.reference_point || part != BoxPart::Pivot)).unwrap_or(BoxPart::Inside);
                let next = self.nudged(shown, part, doc * (step / length));
                if next != *shown {
                    cx.set(next);
                }
            }
        }
        self.report.apply |= cx.intents().take(Intent::Submit);
        if !pressing && cx.intents().take(Intent::Cancel) {
            self.report.discard = true;
        }
    }
}

/// What the cursor of a part reads: the box on the screen, the canvas space and the style.
struct CursorLook<'a> {
    space: &'a CanvasSpace,
    screen: &'a BoxState,
    style: &'a TransformBoxStyle,
}

impl Component for TransformBox<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves();
    type Style = TransformBoxStyle;
    type Output = TransformBoxOutput;

    fn style(cx: &StyleCx<'_>, _: TransformBoxStyleKey) -> TransformBoxStyle {
        let overlay = &cx.tokens.color.overlay;
        let (hit, handle) = (&cx.tokens.size.hit, &cx.tokens.size.handle);
        TransformBoxStyle {
            metrics: Metrics::new(),
            handle: cx.handle(),
            line: cx.color(overlay.line),
            halo: cx.color(overlay.halo),
            hit: handle.hit,
            rotate_reach: handle.rotate_reach,
            band_inner: handle.band_inner,
            band_outer: handle.band_outer,
            pivot_radius: cx.tokens.size.handle.sm / 2.0,
            pivot_arm: hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, BoxState>> {
        Some(self.state.reborrow())
    }

    /// The whole box, the corners, the edges and the reference point the rules allow, in keyboard order, then the
    /// outside. A handle and the reference point rank alike, so the nearest wins where they overlap on a small box; they
    /// rank above the inside, which ranks above the outside.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(state)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let style = cx.style;
        let screen = on_screen(&space.view, &state);
        let quad: [Pos2; 4] = screen.quad.map(pos);
        let mut any = false;
        for part in self.keyboard_parts() {
            let (shape, sense, priority, focus) = match part {
                BoxPart::Inside => {
                    let sense = if self.inside_moves { Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS } else { Sense::TAKE_FOCUS };
                    (HitShape::Polygon(quad.to_vec()), sense, HitPriority::new(HitLevel::Body, 0), FocusShape::Rect)
                }
                BoxPart::Pivot => (
                    HitShape::circle(pos(screen.pivot), style.hit),
                    Sense::DRAG | Sense::TAKE_FOCUS,
                    HitPriority::new(HitLevel::Handle, 0),
                    FocusShape::Circle,
                ),
                handle => {
                    let at = handle_point(&screen, handle).map_or(Pos2::ZERO, pos);
                    (HitShape::circle(at, style.hit), Sense::DRAG | Sense::TAKE_FOCUS, HitPriority::new(HitLevel::Handle, 0), FocusShape::Circle)
                }
            };
            cx.part(BOX.at(part.index()), shape).sense(sense).priority(priority).focus_shape(focus).focusable();
            any = true;
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
        match self.rotate_zone {
            RotateZone::Outside => {
                cx.part(OUTSIDE.at(0), HitShape::outside_polygon(rect, quad.to_vec()))
                    .sense(Sense::DRAG | Sense::CLICK)
                    .priority(HitPriority::new(HitLevel::Background, 0))
                    .pointer_only(ROTATE_BY_KEY);
            }
            RotateZone::Band if self.rules.allows(BoxPart::Outside) => {
                for (corner, ring) in band_rings(&screen, style).into_iter().enumerate() {
                    cx.part(OUTSIDE.at(corner as u64), HitShape::Polygon(ring))
                        .sense(Sense::DRAG | Sense::CLICK)
                        .priority(HitPriority::new(HitLevel::Body, 0))
                        .pointer_only(ROTATE_BY_KEY);
                }
            }
            RotateZone::Band => {}
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(shown)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let held = cx.press_slot::<Grip>().is_some();
        self.run_pointer(cx, &space, &shown);
        let pressing = held || cx.press_slot::<Grip>().is_some();
        self.run_keys(cx, &space, &shown, pressing);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(state)) = (canvas_space(&cx.env), cx.value()) else { return };
        let screen = on_screen(&space.view, state);
        let style = cx.style;
        let draw = ui.draw();
        let outline = PaintShape::Polygon(screen.quad.map(pos).to_vec());
        draw.stroke(&outline, StrokeWidth::Thick, style.halo);
        draw.stroke(&outline, StrokeWidth::Medium, style.line);
        let look = &style.handle;
        for i in 0..4u8 {
            for part in [BoxPart::Corner(i), BoxPart::Edge(i)] {
                let Some(at) = handle_point(&screen, part) else { continue };
                let square = PaintShape::Rect(Rect::from_center_size(pos(at), Vec2::splat(look.side)), CornerRadius::ZERO);
                let fill = if self.rules.allows(part) { cx.resolve_part(BOX.at(part.index()), &look.fill) } else { cx.resolve(&look.fill) };
                draw.fill(&square, fill);
                draw.stroke(&square, StrokeWidth::Thin, look.stroke);
            }
        }
        if self.reference_point {
            let centre = pos(screen.pivot);
            let (radius, arm) = (style.pivot_radius, style.pivot_arm);
            for (width, color) in [(StrokeWidth::Thick, style.halo), (StrokeWidth::Medium, style.line)] {
                draw.stroke(&PaintShape::Circle(centre, radius), width, color);
                draw.stroke(&PaintShape::Path(vec![centre - Vec2::new(arm, 0.0), centre + Vec2::new(arm, 0.0)]), width, color);
                draw.stroke(&PaintShape::Path(vec![centre - Vec2::new(0.0, arm), centre + Vec2::new(0.0, arm)]), width, color);
            }
        }
    }

    /// One node per part: the whole box, the corners, the edges and the reference point, each described by its point
    /// in the canvas's units.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(state)) = (canvas_space(&cx.env), cx.value()) else { return };
        let words = cx.words::<TransformBoxWords>();
        let units = cx.env.units();
        let rulers = space.rulers();
        let ((across_x, zero_x), (across_y, zero_y)) = (rulers.across(Axis::Y), rulers.across(Axis::X));
        for part in self.keyboard_parts() {
            let node = access.node(BOX.at(part.index()), Role::BUTTON);
            node.name = Name::Text(words.name_of(part).to_owned());
            if let Some(at) = handle_point(state, part) {
                node.description = Some(format!("{}, {}", across_x.format_value(&units, at.x, zero_x), across_y.format_value(&units, at.y, zero_y)));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TransformBoxOutput {
        let Report { apply, discard, hovered, picked } = self.report;
        TransformBoxOutput { edit: cx.edit_response(), apply, discard, picked, hovered }
    }
}

/// What the box reports: the edit of the box and what it asks the host to do.
#[derive(Debug)]
#[non_exhaustive]
pub struct TransformBoxOutput {
    /// A drag (`Begin` on the first move, `Update`, `Commit` on the release; Escape cancels it) or an arrow key's
    /// `Set`.
    pub edit: EditResponse<BoxState>,
    /// Apply the box: Enter, a click outside it, a double click inside it.
    pub apply: bool,
    /// Close the box without applying it: Escape while no drag is on and the box has the keyboard.
    pub discard: bool,
    /// The part the keys act on, while the box has the keyboard.
    pub picked: Option<BoxPart>,
    /// The part a press at the pointer would take; `None` where it is the tool's.
    pub hovered: Option<BoxPart>,
}

impl Outcome for TransformBoxOutput {
    type Value = BoxState;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

/// `state` as the view draws it, in screen points.
fn on_screen(view: &CanvasView, state: &BoxState) -> BoxState {
    let affine = view.affine();
    BoxState::new(state.quad.map(|p| affine * p), affine * state.pivot)
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn vec2_of(v: DocVec) -> Vec2 {
    Vec2::new(v.x as f32, v.y as f32)
}

/// The position of a corner, an edge's midpoint or the reference point; `None` for the inside and the outside.
fn handle_point(state: &BoxState, part: BoxPart) -> Option<Point> {
    let q = &state.quad;
    match part {
        BoxPart::Corner(i) => Some(q[usize::from(i % 4)]),
        BoxPart::Edge(i) => Some(q[usize::from(i % 4)].midpoint(q[usize::from(i + 1) % 4])),
        BoxPart::Pivot => Some(state.pivot),
        _ => None,
    }
}

/// The corner of `screen` nearest `p` and its distance.
fn nearest_corner(screen: &BoxState, p: Pos2) -> (usize, f32) {
    let p = Point::new(f64::from(p.x), f64::from(p.y));
    (0..4).map(|i| (i, screen.quad[i].distance(p) as f32)).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, f32::INFINITY))
}

/// The rotate band at each corner of `screen`: the L-shaped ring outside the box between the band's inner and outer
/// reach, as long as the outer reach along both edges.
fn band_rings(screen: &BoxState, style: &TransformBoxStyle) -> [Vec<Pos2>; 4] {
    let corners: [Pos2; 4] = screen.quad.map(pos);
    let inner = quad::outset(&corners, style.band_inner);
    let outer = quad::outset(&corners, style.band_outer);
    std::array::from_fn(|i| {
        let toward = |k: usize| (corners[k % 4] - corners[i]).normalized() * style.band_outer;
        let (previous, next) = (toward(i + 3), toward(i + 1));
        vec![inner[i] + previous, outer[i] + previous, outer[i], outer[i] + next, inner[i] + next, inner[i]]
    })
}

/// `state` with its corners on whole units, when it is an upright rectangle of at least one unit each way;
/// otherwise as it is. The reference point stays where it is.
fn on_pixel_grid(state: BoxState) -> BoxState {
    let q = state.quad;
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let upright = near(q[0].y, q[1].y) && near(q[2].y, q[3].y) && near(q[0].x, q[3].x) && near(q[1].x, q[2].x);
    let rounded = q.map(|p| Point::new(p.x.round(), p.y.round()));
    let ok = rounded.iter().all(|p| p.x.is_finite() && p.y.is_finite())
        && (rounded[1].x - rounded[0].x).abs() >= 1.0
        && (rounded[3].y - rounded[0].y).abs() >= 1.0;
    if upright && ok { BoxState::new(rounded, state.pivot) } else { state }
}

/// `v` rounded to `places` decimals, with no negative zero (a hair below zero never reads "-0").
pub(crate) fn fixed(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    let v = (v * k).round() / k;
    if v == 0.0 { 0.0 } else { v }
}
