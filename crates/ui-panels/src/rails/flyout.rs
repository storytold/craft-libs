//! `FlyoutButton`: a rail button that stands for a group of tools and opens them as a flyout.

use super::tool::{RailTool, SlotDrop, SlotId, ToolDrag, ToolDropped};
use crate::buttons::IconButton;
use crate::contract::overlay::{Anchor, Place};
use crate::contract::{
    Access, Common, Component, Cx, DragPreview, MeasureCx, Name, NoValue, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _,
    PopupKind, Props as _, Remember as _, Role, Show as _, ShowContainer as _, Trigger,
};
use crate::menu::{self, Menu, MenuItem};
use crate::runtime::commands::{FireSource, ResolvedCommand};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::overlay::{Align, Side};
use craft_behavior::{HitLevel, HitPriority, IconId, Metrics, PartKey, PressPhase, Sense, Value};
use egui::{Rect, Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

/// The face over the whole button.
const FACE: PartKey = PartKey::new("face");
/// The corner mark: a press on it opens the flyout at once, and releasing over a row picks it.
const MARK: PartKey = PartKey::new("mark");

/// The style of a flyout button: it paints through its face, so it holds no look of its own besides the size of the
/// corner mark's press area, which is the size and the inset of the mark the face paints.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FlyoutButtonStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side of the corner mark, which is also its inset from the face's corner.
    mark: f32,
}

/// A rail button that is dragged and takes drops while the toolbar is edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SlotEdit {
    /// What dragging the button carries.
    pub drag: ToolDrag,
    /// The slot the button stands for.
    pub slot: SlotId,
    /// The slot's index among the host's slots.
    pub index: usize,
    /// How many tools the slot holds.
    pub tools: usize,
    /// The rail has one column, so buttons are placed one under another.
    pub vertical: bool,
}

impl SlotEdit {
    /// The edit state of the button of `slot`, the `index`-th of the host's slots, which holds `tools` tools.
    pub fn new(slot: SlotId, index: usize, tools: usize, vertical: bool) -> Self {
        SlotEdit { drag: ToolDrag::rail(slot, 0), slot, index, tools, vertical }
    }
}

/// What a flyout button reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FlyoutOutput<T> {
    /// The button's response.
    pub response: Response,
    /// The tool made current this pass: the shown tool of a click, Enter, Space or a hold that selects, or the tool
    /// picked in the flyout. Its command has run.
    pub select: Option<T>,
    /// The button was double clicked; the host decides what that means.
    pub double_clicked: bool,
    /// The flyout is open after this pass.
    pub opened: bool,
    /// A drag dropped on the button while the toolbar is edited.
    pub dropped: Option<ToolDropped>,
}

impl<T: Value> Outcome for FlyoutOutput<T> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Whether the press that is ongoing opened the flyout by a hold.
#[derive(Clone, Copy, Default)]
struct Held(bool);

/// What the parts step resolved for the tools this pass.
struct Laid {
    resolved: Vec<Option<ResolvedCommand>>,
    shown: usize,
}

/// One rail button that may stand for a group of tools. The face shows the tool `shown` with a mark at the corner
/// when there is a flyout. A click, Enter or Space selects the shown tool and runs its command; holding the press,
/// a right click or Alt+Down opens the flyout; a press on the corner mark opens it at once, and releasing over a
/// row picks it. Rows are the tools' commands, the shown tool with the radio mark. A hold selects the shown tool
/// unless `hold_selects` is off, as for a command button.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(FlyoutButton<'static, u8>))]
pub struct FlyoutButton<'a, T: Value + 'static> {
    #[common]
    common: Common,
    tools: &'a [RailTool<T>],
    shown: &'a T,
    /// Whether holding the press also selects the shown tool.
    #[prop]
    hold_selects: bool,
    /// The side of the button the flyout opens on, the side of the canvas.
    #[prop]
    opens: Side,
    /// The button is dragged and takes drops: the toolbar is being edited, and it has no flyout.
    #[prop(into)]
    edit: Option<SlotEdit>,
    laid: Option<Laid>,
    picked: Option<T>,
    open: bool,
    double: bool,
    dropped: Option<ToolDropped>,
}

impl<'a, T: Value + 'static> FlyoutButton<'a, T> {
    /// A button for `tools` showing the tool `shown`. With fewer than two tools it is a plain button.
    pub fn new(tools: &'a [RailTool<T>], shown: &'a T) -> Self {
        FlyoutButton {
            common: Common::new(),
            tools,
            shown,
            hold_selects: true,
            opens: Side::End,
            edit: None,
            laid: None,
            picked: None,
            open: false,
            double: false,
            dropped: None,
        }
    }

    /// Whether the button has a flyout.
    fn has_flyout(&self) -> bool {
        self.edit.is_none() && self.tools.len() > 1
    }

    fn shown_at(&self) -> usize {
        self.tools.iter().position(|tool| tool.id == *self.shown).unwrap_or(0)
    }

    /// The face the button shows: the shown tool's icon, in the look of its command (label, enabled state, marked dot).
    /// The button itself runs the command, so the face only wears it.
    fn face(&self, laid: &Laid) -> IconButton<'static> {
        let resolved = laid.resolved.get(laid.shown).and_then(Option::as_ref);
        let icon = resolved.and_then(|resolved| resolved.icon.clone()).map_or_else(|| IconId::new("more"), IconId::from);
        let face = match self.tools.get(laid.shown) {
            Some(tool) => IconButton::command(icon, tool.command.clone()),
            None => IconButton::new(icon),
        };
        let mut face = face.selected(self.common.selected.unwrap_or(false));
        if self.has_flyout() {
            face = face.popup(PopupKind::Menu);
        }
        face
    }

    fn resolve(&self, cx: &MeasureCx<'_, Self>) -> Laid {
        Laid { resolved: self.tools.iter().map(|tool| cx.resolve_command(tool.command.clone())).collect(), shown: self.shown_at() }
    }
}

impl<T: Value + 'static> Component for FlyoutButton<'_, T> {
    const ROLE: Role = Role::BUTTON;
    const SHOWS_LABEL: bool = false;
    type Style = FlyoutButtonStyle;
    type Output = FlyoutOutput<T>;

    fn style(cx: &StyleCx<'_>, _: FlyoutButtonStyleKey<T>) -> FlyoutButtonStyle {
        FlyoutButtonStyle { mark: cx.tokens.space.gap.tight, ..FlyoutButtonStyle::default() }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let laid = self.resolve(cx);
        cx.face(self.face(&laid))
    }

    fn sense(&self) -> Sense {
        if self.edit.is_some() { Sense::CLICK | Sense::FOCUS | Sense::DRAG } else { Sense::CLICK | Sense::FOCUS }
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let laid = self.resolve(cx);
        let os = cx.env.shell().os;
        let face = cx.part(FACE, rect).priority(HitPriority::new(HitLevel::Body, 0));
        if let Some(resolved) = laid.resolved.get(laid.shown).and_then(Option::as_ref) {
            face.tooltip(match &resolved.chord {
                Some(chord) => Label::from(format!("{} ({})", resolved.label, chord.display(os))),
                None => Label::Shared(resolved.label.clone()),
            });
        }
        if self.has_flyout() {
            let side = cx.style.mark;
            let inner = rect.shrink(side);
            let mark = if cx.flow.is_rtl() {
                Rect::from_min_size(egui::pos2(inner.left(), inner.bottom() - side), Vec2::splat(side))
            } else {
                Rect::from_min_size(inner.max - Vec2::splat(side), Vec2::splat(side))
            };
            cx.part(MARK, mark)
                .sense(Sense::CLICK)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .pointer_only("Alt+Down, the Menu key and a right click open the flyout")
                .toggles("menu")
                .on_press();
        }
        self.laid = Some(laid);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let Some(laid) = &self.laid else { return };
        let (shown_at, enabled) = (laid.shown, laid.resolved.get(laid.shown).and_then(Option::as_ref).is_some_and(|resolved| resolved.enabled));
        let mut open = false;
        if self.has_flyout() {
            let menu = cx.popup("menu");
            menu.trigger(Trigger::Hold, cx);
            menu.trigger(Trigger::Context, cx);
            open = menu.is_open(cx);
        }
        let pressing = cx.press().phase.is_press();
        let activated = cx.activation().is_some();
        let held = {
            let slot = cx.memory::<Held>();
            if pressing {
                slot.0 |= open;
            }
            let held = slot.0;
            if !pressing {
                slot.0 = false;
            }
            held
        };
        self.open = open;
        self.double = cx.press().double_pressed();
        if activated && enabled && !(held && !self.hold_selects) {
            let tool = &self.tools[shown_at];
            cx.fire(tool.command.clone(), FireSource::Pointer);
            self.picked = Some(tool.id.clone());
        }
        if let Some(edit) = self.edit {
            if matches!(cx.press().phase, PressPhase::DragStart { .. }) {
                cx.start_drag(ToolDrag::rail(edit.slot, shown_at), DragPreview::None);
            }
            let target =
                SlotDrop { rect: cx.rect, slot: edit.slot, index: edit.index, tools: edit.tools, vertical: edit.vertical, rtl: cx.flow.is_rtl() };
            self.dropped = cx.drop_target::<ToolDrag, _>(&target);
        }
    }

    /// The flyout: one row per tool, each the tool's command, the shown tool with the radio mark. Running a row
    /// reports its tool and closes the flyout.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if !self.has_flyout() {
            return;
        }
        let popup = cx.popup("menu");
        let (tools, shown) = (self.tools, self.shown);
        let name = self.laid.as_ref().and_then(|laid| laid.resolved.get(laid.shown)).and_then(Option::as_ref).map(|resolved| resolved.label.clone());
        let name = name.map_or(Label::Empty, Label::Shared);
        let layer = menu::layer(false).place(Place::anchored(Anchor::Response, self.opens, Align::Start));
        let ran = ui.show(popup, layer, |ui, _| {
            Menu::inline(name.clone())
                .show(ui, |ui, _| {
                    let mut ran = None;
                    for (at, tool) in tools.iter().enumerate() {
                        if MenuItem::command(tool.command.clone()).radio_mark(tool.id == *shown).show(ui).ran {
                            ran = Some(at);
                        }
                    }
                    ran
                })
                .inner
                .flatten()
        });
        if let Some(at) = ran.flatten() {
            self.picked = Some(tools[at].id.clone());
            popup.close(cx);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        let resolved = self.laid.as_ref().and_then(|laid| laid.resolved.get(laid.shown)).and_then(Option::as_ref);
        let name = resolved.map(|resolved| resolved.label.as_ref().to_owned());
        if let Some(resolved) = resolved {
            access.name(Name::Text(resolved.label.as_ref().to_owned()));
            if let Some(hint) = &resolved.hint {
                access.description(hint.as_ref().to_owned());
            }
        }
        // The face covers the whole button, so the button's own node is the face's node.
        access.node(FACE, Role::None);
        if self.has_flyout() {
            access.has_popup(PopupKind::Menu);
            access.node(MARK, Role::BUTTON).name = Name::Text(name.unwrap_or_default());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        let Some(laid) = &self.laid else { return };
        let face = self.face(laid);
        ui.face(FACE, face);
    }

    fn finish(self, cx: Cx<'_, Self>) -> FlyoutOutput<T> {
        FlyoutOutput { response: cx.response.clone(), select: self.picked, double_clicked: self.double, opened: self.open, dropped: self.dropped }
    }
}
