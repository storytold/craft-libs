//! [`CornerWidgets`]: the round widgets inside the corners of the host's shapes, a canvas control.

use super::geometry::{self, Widget};
use super::style::CornerStyle;
use super::words::CornerWords;
use super::{CornerEdit, CornerRef, CornerTarget};
use crate::geometry::fillet::fillet;
use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_behavior::{HitLevel, HitPriority, HitShape, NavShape, PartKey, PointerButton, PressPhase, Roving, Sense};
use craft_ui::canvas::{NUDGE, NUDGE_FAR, canvas_space, readout};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::StyleCx;
use egui::{Rect, Response, Vec2};

/// The part of one widget: `WIDGET.at(i)`, `i` counted over the shown widgets in target order, then corner order.
pub(super) const WIDGET: PartKey = PartKey::new("widget");

/// What the widgets report beyond the edit.
#[derive(Clone, Debug, Default)]
struct Report {
    change: Option<CornerEdit>,
    cycle_kind: Option<CornerRef>,
    activate: Option<CornerRef>,
}

/// The press being dragged: the widget it began on, the radius and the corner's step there, and the targets as they
/// were, so a host that applies the values live still gets every changed target's radii from the press.
#[derive(Clone, Debug, Default)]
struct Grab {
    corner: Option<CornerRef>,
    from: f64,
    step: Option<DocVec>,
    press: Point,
    targets: Vec<CornerTarget>,
}

/// One round widget inside each corner of the host's shapes, dragged to round them.
///
/// - Look: per shown corner a circle in `overlay.handle_fill`, outlined in `overlay.line`, with a dot in
///   `overlay.line`, at the centre of the corner's arc and at least the inset inside the corner. Hidden when a side
///   is shorter than the least side on screen or the corner's angle is wider than [`hide_above`](Self::hide_above).
/// - Pointer: a press within reach of a widget takes it (the nearest; of equal distances the first target's, so list
///   the topmost shape first). A drag sets the radius at the press plus the move projected on the corner's centre
///   step; every target takes that radius, each capped at its own maximum; ⌥ changes only the dragged corner of
///   `individual` targets. A click without movement changes nothing. While dragging the readout "R: 25.0 px" (in the
///   canvas's ruler units) follows the pointer; Escape puts the radii back. ⌥-click reports
///   [`cycle_kind`](CornerWidgetsOutput::cycle_kind) with [`kinds`](Self::kinds); a double press reports
///   [`activate`](CornerWidgetsOutput::activate).
/// - Keyboard: one Tab stop. Page Down and Page Up pick the next and previous widget; the arrows change the radius of
///   the picked widget's corner on every target by 1 (⇧ 10; up and right grow it), ⌥ that corner only; Enter reports
///   `activate`.
/// - AccessKit: one object per widget named "Corner i of n", valued "radius 12 px".
/// - Geometry is in document space and does not mirror right to left.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct CornerWidgets<'a> {
    #[common]
    common: Common,
    targets: Bind<'a, Vec<CornerTarget>>,
    /// Whether ⌥-click cycles the corner kind.
    #[prop]
    kinds: bool,
    /// The widest inner angle whose corner shows a widget.
    #[prop(quantity = "degrees")]
    hide_above: f64,
    report: Report,
}

impl<'a> CornerWidgets<'a> {
    /// The widgets of the host's `targets`, topmost first, named `label`.
    pub fn new(label: impl Into<Label>, targets: Bind<'a, Vec<CornerTarget>>) -> Self {
        CornerWidgets { common: Common::new(), targets, kinds: false, hide_above: Self::HIDE_ABOVE, report: Report::default() }.label(label)
    }

    /// The widest inner angle, in degrees, a widget shows at unless [`hide_above`](Self::hide_above) says otherwise.
    pub const HIDE_ABOVE: f64 = 180.0;
}

/// What the widgets report: the edit of the targets and what the widgets ask the host to do.
#[derive(Debug)]
#[non_exhaustive]
pub struct CornerWidgetsOutput {
    /// The edit of the targets: a drag is one `Begin` to `Commit`; an arrow key is a `Set`. The host makes one
    /// history step for every target the value changes.
    pub edit: EditResponse<Vec<CornerTarget>>,
    /// The radii of every target the pass changed, for a host that applies the operation to its own types.
    pub change: Option<CornerEdit>,
    /// ⌥-click without drag on a widget, with [`kinds`](CornerWidgets::kinds) on: the corner whose
    /// [`CornerKind::next`](super::CornerKind::next) the host applies.
    pub cycle_kind: Option<CornerRef>,
    /// A double press on a widget, or Enter on the picked one: the host opens its Corners dialog for that corner.
    pub activate: Option<CornerRef>,
}

impl Outcome for CornerWidgetsOutput {
    type Value = Vec<CornerTarget>;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

impl CornerWidgets<'_> {
    /// The widgets of the targets now, on screen.
    fn shown(&self, style: &CornerStyle, targets: Option<&Vec<CornerTarget>>, view: &craft_behavior::CanvasView) -> Vec<Widget> {
        targets.map_or_else(Vec::new, |targets| geometry::widgets(targets, view, style.inset, style.min_side, self.hide_above))
    }

    /// The pointer on the widgets: grab, drag and the click reports.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, widgets: &[Widget], view: &craft_behavior::CanvasView) {
        for (at, widget) in widgets.iter().enumerate() {
            let Some(result) = cx.part_if_declared(WIDGET.at(at as u64)) else { continue };
            let press = result.press;
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary && !press.mods.cmd => {
                    let targets = cx.value().cloned().unwrap_or_default();
                    let step = targets
                        .iter()
                        .find(|t| t.key == widget.corner.target)
                        .and_then(|t| t.corners.get(widget.corner.corner))
                        .and_then(|c| fillet(c.prev, c.vertex, c.next, 1.0))
                        .map(|f| f.step());
                    *cx.memory::<Grab>() = Grab { corner: Some(widget.corner), from: widget.radius, step, press: view.to_doc(start.origin), targets };
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    let grab = cx.memory::<Grab>().clone();
                    let (Some(corner), Some(step)) = (grab.corner, grab.step) else { continue };
                    if corner != widget.corner {
                        continue;
                    }
                    if matches!(press.phase, PressPhase::DragStart { .. }) {
                        cx.begin();
                    }
                    let radius = geometry::dragged_radius(grab.from, step, grab.press, view.to_doc(pos));
                    let edit = geometry::drag_edit(&grab.targets, corner, radius, press.mods.alt);
                    cx.propose(geometry::apply(&grab.targets, &edit));
                    self.show_readout(cx, pos, radius);
                    self.report.change = Some(edit);
                }
                PressPhase::Release { start, click, .. } if start.button == PointerButton::Primary => {
                    let grab = cx.memory::<Grab>().clone();
                    if grab.corner != Some(widget.corner) {
                        continue;
                    }
                    if click {
                        if press.mods.alt && self.kinds {
                            self.report.cycle_kind = Some(widget.corner);
                        } else if start.clicks >= 2 {
                            self.report.activate = Some(widget.corner);
                        }
                    } else {
                        cx.commit();
                    }
                    *cx.memory::<Grab>() = Grab::default();
                }
                _ => {}
            }
        }
    }

    /// The readout beside the pointer while dragging: the radius in the canvas's ruler units.
    fn show_readout(&self, cx: &Cx<'_, Self>, at: egui::Pos2, radius: f64) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let words = cx.words::<CornerWords>();
        let text = space.rulers().x.format_value(&cx.env.units(), radius, 0.0);
        readout(cx, at, &[(Label::from(words.readout.as_str().to_owned()), text)]);
    }

    /// The keys of the focused widget: the arrows step the radius, Enter activates.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, widgets: &[Widget]) {
        let Some(at) = cx.active_part().filter(|part| part.key == WIDGET).and_then(|part| part.index) else { return };
        let Some(widget) = widgets.get(at as usize) else { return };
        let targets = cx.value().cloned().unwrap_or_default();
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            // Up and right grow the radius; the screen's y axis points down.
            let radius = (widget.radius + f64::from(direction.x - direction.y) * step).max(0.0);
            let edit = geometry::drag_edit(&targets, widget.corner, radius, mods.alt);
            if !edit.targets.is_empty() {
                cx.set(geometry::apply(&targets, &edit));
                self.report.change = Some(edit);
            }
        }
        if matches!(cx.part_activated(WIDGET.at(at)), Some(FireSource::Key)) {
            self.report.activate = Some(widget.corner);
        }
    }
}

impl Component for CornerWidgets<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_activate(ActivateKeys::Enter);
    type Style = CornerStyle;
    type Output = CornerWidgetsOutput;

    fn style(cx: &StyleCx<'_>, _: CornerWidgetsStyleKey) -> CornerStyle {
        CornerStyle::from_cx(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, Vec<CornerTarget>>> {
        Some(self.targets.reborrow())
    }

    /// One drag part per shown widget at handle priority, in one roving group; Page Up and Page Down move the group's
    /// arrival and the arrows are the widget's nudge.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let widgets = self.shown(cx.style, cx.value(), &space.view);
        let reach = cx.style.reach;
        for (at, widget) in widgets.iter().enumerate() {
            cx.part(WIDGET.at(at as u64), HitShape::circle(widget.at, reach))
                .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focusable();
        }
        if !widgets.is_empty() {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let widgets = self.shown(cx.style, cx.value(), &space.view);
        self.run_pointer(cx, &widgets, &space.view);
        self.run_keys(cx, &widgets);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let widgets = self.shown(cx.style, cx.value(), &space.view);
        let (draw, style) = (ui.draw(), &cx.style);
        for widget in &widgets {
            let ring = PaintShape::Circle(widget.at, style.widget / 2.0);
            draw.fill(&ring, style.fill);
            draw.stroke(&ring, StrokeWidth::Medium, style.color);
            draw.fill(&PaintShape::Circle(widget.at, style.dot / 2.0), style.color);
        }
    }

    /// One object per widget, named "Corner i of n" and valued with its radius.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let widgets = self.shown(cx.style, cx.value(), &space.view);
        let (words, units) = (cx.words::<CornerWords>(), cx.env.units());
        let n = widgets.len().to_string();
        for (at, widget) in widgets.iter().enumerate() {
            let radius = space.rulers().x.format_value(&units, widget.radius, 0.0);
            let node = access.node(WIDGET.at(at as u64), Role::BUTTON);
            node.name = Name::Text(words.corner.fill(&[("i", &(at + 1).to_string()), ("n", &n)]));
            node.value(words.value.fill(&[("radius", &radius)]));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CornerWidgetsOutput {
        let Report { change, cycle_kind, activate } = self.report;
        CornerWidgetsOutput { edit: cx.edit_response(), change, cycle_kind, activate }
    }
}
