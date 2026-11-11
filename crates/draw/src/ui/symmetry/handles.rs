//! `SymmetryHandles`: the centre and the rotation handle of Transform Symmetry, a canvas control.

use super::HANDLE_SHARE;
use super::menu::SymmetryWords;
use super::model::{Symmetry, TURN_STEP_DEGREES};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{
    Axis, Cursor, CursorIcon, CursorPriority, HitLevel, HitPriority, HitShape, Metrics, NavShape, PartKey, PointerButton, PressPhase, Roving, Sense,
};
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Numeric, Outcome, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{FocusShape, KeyUse};
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Pos2, Rect, Response, Vec2};

/// The parts: `SYMMETRY.at(CENTRE)` and `SYMMETRY.at(HANDLE)`.
pub(super) const SYMMETRY: PartKey = PartKey::new("symmetry");
pub(super) const CENTRE: u64 = 0;
const HANDLE: u64 = 1;

/// The style of the handles: the reach of a press.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SymmetryHandlesStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A press within this radius of the centre or the rotation handle takes it.
    hit: f32,
}

/// The drag a part is held with, kept from the press to the release.
#[derive(Clone, Copy, Debug, Default)]
enum Grip {
    #[default]
    Idle,
    /// Moves the centre: the press in document space and the centre then.
    Move { from: Point, centre: Point },
    /// Turns the axes: the pointer's angle from the centre at the press (radians) and the rotation then.
    Turn { angle: f64, rotation: f64 },
}

/// The centre of a painting symmetry and its rotation handle, two parts of one group.
///
/// - The value is the symmetry in document units. A drag of either part is one gesture (`Begin` on the first move,
///   `Commit` on the release); Escape cancels it.
/// - A drag of the centre moves it. A drag of the rotation handle turns the axes about the centre; Shift rounds the
///   rotation to [`TURN_STEP_DEGREES`] steps, absolute from 0.
/// - The cursor is Move over the centre and Alias over the handle.
/// - One Tab stop. Page Down and Page Up pick the other part. The arrows move the centre 1 document unit, Shift 10;
///   with the handle picked Left and Right turn the axes 1 degree, Shift [`TURN_STEP_DEGREES`]. Right is clockwise in
///   every language: the canvas does not mirror.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct SymmetryHandles<'a> {
    #[common]
    common: Common,
    symmetry: Bind<'a, Symmetry>,
    page: Size,
}

impl<'a> SymmetryHandles<'a> {
    /// The handles named `label` over `symmetry`, on a page of `page` document units.
    pub fn new(label: impl Into<Label>, symmetry: Bind<'a, Symmetry>, page: Size) -> Self {
        SymmetryHandles { common: Common::new(), symmetry, page }.label(label)
    }

    /// The symmetry `Symmetry::new` makes of `centre` and `rotation`, or `shown` when it refuses them.
    fn with(shown: &Symmetry, centre: Point, rotation: f64) -> Symmetry {
        Symmetry::new(shown.mode, centre, rotation).unwrap_or(*shown)
    }

    /// The symmetry the pointer at `pos` gives a drag held with `grip`.
    fn proposal(&self, space: &CanvasSpace, shown: &Symmetry, grip: Grip, pos: Pos2, shift: bool) -> Symmetry {
        match grip {
            Grip::Idle => *shown,
            Grip::Move { from, centre } => Self::with(shown, centre + (space.view.to_doc(pos) - from), shown.rotation),
            Grip::Turn { angle, rotation } => {
                let from = space.view.to_doc(pos) - shown.center;
                let mut turned = rotation + (from.y.atan2(from.x) - angle).to_degrees();
                if shift {
                    turned = (turned / TURN_STEP_DEGREES).round() * TURN_STEP_DEGREES;
                }
                Self::with(shown, shown.center, turned)
            }
        }
    }

    /// The pointer on the two parts: presses, drags, releases and the cursor.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &Symmetry) {
        let mut held = (*cx.press_slot::<GripSlot>()).map_or(Grip::Idle, |slot| slot.0);
        for (index, icon) in [(CENTRE, CursorIcon::Move), (HANDLE, CursorIcon::Alias)] {
            let Some(result) = cx.part_if_declared(SYMMETRY.at(index)) else { continue };
            let (press, hovered) = (result.press, result.hovered);
            if hovered && matches!(held, Grip::Idle) {
                cx.cursor(Cursor::System(icon), CursorPriority::Part);
            }
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    held = if index == CENTRE {
                        Grip::Move { from: space.view.to_doc(start.origin), centre: shown.center }
                    } else {
                        let from = space.view.to_doc(start.origin) - shown.center;
                        Grip::Turn { angle: from.y.atan2(from.x), rotation: shown.rotation }
                    };
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    if matches!(held, Grip::Idle) {
                        continue;
                    }
                    if matches!(press.phase, PressPhase::DragStart { .. }) {
                        cx.begin();
                    }
                    cx.propose(self.proposal(space, shown, held, pos, press.mods.shift));
                    cx.cursor(Cursor::System(icon), CursorPriority::Drag);
                }
                PressPhase::Release { pos, click, .. } => {
                    if !click && !matches!(held, Grip::Idle) {
                        cx.propose(self.proposal(space, shown, held, pos, press.mods.shift));
                        cx.commit();
                    }
                    held = Grip::Idle;
                }
                PressPhase::Cancel { .. } => held = Grip::Idle,
                _ => {}
            }
        }
        *cx.press_slot::<GripSlot>() = Some(GripSlot(held));
    }

    /// The keys of the focused part: the arrows move the centre, or turn the axes while the handle is picked.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &Symmetry) {
        let picked = cx.active_part().filter(|part| part.key == SYMMETRY).and_then(|part| part.index);
        let Some((direction, mods)) = cx.intents().take_nudge() else { return };
        let next = if picked == Some(HANDLE) {
            // Right is clockwise in every language: the canvas does not mirror.
            if direction.x == 0.0 {
                return;
            }
            let step = if mods.shift { TURN_STEP_DEGREES } else { 1.0 };
            Self::with(shown, shown.center, shown.rotation + step * f64::from(direction.x.signum()))
        } else {
            let doc = space.view.unmap_vec(direction);
            let length = doc.hypot();
            if length <= 0.0 {
                return;
            }
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            Self::with(shown, shown.center + doc * (step / length), shown.rotation)
        };
        if next != *shown {
            cx.set(next);
        }
    }
}

/// The drag held between passes.
#[derive(Clone, Copy, Debug, Default)]
struct GripSlot(Grip);

impl Component for SymmetryHandles<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves();
    type Style = SymmetryHandlesStyle;
    type Output = SymmetryHandlesOutput;

    fn style(cx: &StyleCx<'_>, _: SymmetryHandlesStyleKey) -> SymmetryHandlesStyle {
        SymmetryHandlesStyle { metrics: Metrics::new(), hit: cx.tokens.size.hit.target }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, Symmetry>> {
        Some(self.symmetry.reborrow())
    }

    /// The centre, then the rotation handle: circles of the hit reach, one roving group.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(shown)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let hit = cx.style.hit;
        let centre = space.view.to_screen(shown.center);
        let handle = space.view.to_screen(shown.rotation_handle(self.page, HANDLE_SHARE));
        for (index, at) in [(CENTRE, centre), (HANDLE, handle)] {
            cx.part(SYMMETRY.at(index), HitShape::circle(at, hit))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focus_shape(FocusShape::Circle)
                .focusable();
        }
        cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(shown)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        self.run_pointer(cx, &space, &shown);
        self.run_keys(cx, &space, &shown);
    }

    /// The centre named by its position in the canvas's units, the rotation handle by its angle.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(shown)) = (canvas_space(&cx.env), cx.value()) else { return };
        let words = cx.words::<SymmetryWords>();
        let units = cx.env.units();
        let rulers = space.rulers();
        let ((across_x, zero_x), (across_y, zero_y)) = (rulers.across(Axis::Y), rulers.across(Axis::X));
        let centre = access.node(SYMMETRY.at(CENTRE), Role::BUTTON);
        centre.name = Name::Text(words.center.as_str().to_owned());
        centre.description =
            Some(format!("{}, {}", across_x.format_value(&units, shown.center.x, zero_x), across_y.format_value(&units, shown.center.y, zero_y)));
        let handle = access.node(SYMMETRY.at(HANDLE), Role::SLIDER);
        handle.name = Name::Text(words.rotation_handle.as_str().to_owned());
        handle.numeric = Some(Numeric::new(shown.rotation, 0.0..=360.0).step(1.0).wraps(true).text(units.degrees(shown.rotation)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> SymmetryHandlesOutput {
        SymmetryHandlesOutput { edit: cx.edit_response() }
    }
}

/// What the handles report.
#[derive(Debug)]
#[non_exhaustive]
pub struct SymmetryHandlesOutput {
    /// A drag (`Begin` on the first move, `Update`, `Commit` on the release; Escape cancels it) or a key's `Set`.
    /// Apply it with [`SymmetryTransform::apply`](super::SymmetryTransform::apply).
    pub edit: EditResponse<Symmetry>,
}

impl Outcome for SymmetryHandlesOutput {
    type Value = Symmetry;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}
