//! `FlyoutButton`: a rail button that stands for a group of tools and opens them as a flyout.

use super::tool::{RailDensity, RailTool, SlotDrop, SlotId, ToolDrag, ToolDropped, ToolTips};
use super::words::RailWords;
use crate::buttons::{IconButton, PopupMark};
use crate::contract::overlay::{Anchor, Place};
use crate::contract::{
    Access, Common, Component, Cx, DragPreview, Focus as _, MeasureCx, Name, NoValue, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
    Pointer as _, PopupKind, Props as _, Remember as _, Role, Show as _, ShowContainer as _, Trigger, WordSetCx as _,
};
use crate::display::TargetMark;
use crate::layout_components::Row;
use crate::menu::{self, Menu, MenuItem};
use crate::primitives::{IconSpec, PaintShape, StrokeWidth};
use crate::runtime::commands::{CommandRef, FireSource, ResolvedCommand};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, GlyphRole, StyleColor, StyleCx};
use craft_behavior::overlay::{Align, Side};
use craft_behavior::{CursorIcon, HitLevel, HitPriority, IconId, Metrics, PartKey, PointerButton, PressPhase, Sense, Value};
use egui::{Pos2, Rect, Response, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The face over the whole button.
const FACE: PartKey = PartKey::new("face");
/// The corner mark: a press on it opens the flyout at once, and releasing over a row picks it.
const MARK: PartKey = PartKey::new("mark");
/// The tear-off bar's one part, over the whole bar.
const BAR: PartKey = PartKey::new("bar");

/// The style of a flyout button: it paints through its face, so it holds no look of its own besides its side, the rail
/// button's, and the size of the corner mark's press area, which is the size and the inset of the mark the face paints.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FlyoutButtonStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side of the corner mark, which is also its inset from the face's corner.
    mark: f32,
    /// The button's glyph role, which its face shows the icon in.
    glyphs: GlyphRole,
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
    /// The pointer where the flyout was torn off by its tear-off bar; `None` when it was not.
    pub torn_off: Option<Pos2>,
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
    resolved: Vec<Option<std::sync::Arc<ResolvedCommand>>>,
    /// The click command as resolved, when the button has one.
    click: Option<std::sync::Arc<ResolvedCommand>>,
    shown: usize,
}

/// One rail button that may stand for a group of tools. The face shows the tool `shown` with a mark at the corner
/// when there is a flyout. A click, Enter or Space selects the shown tool and runs its command; holding the press,
/// dragging it away from the button, a right click or the Menu key opens the flyout; a press on the corner mark opens it
/// at once; after a press that opened it, releasing over a row picks it. Rows are the tools' commands, the shown tool with the radio mark. A hold selects the shown tool
/// unless `hold_selects` is off, as for a command button. An Alt-click selects the next enabled tool of the slot,
/// wrapping, and opens no flyout. With a `click` command, a click fires that command and selects nothing, and the
/// release that ends a hold fires nothing. A modified press is never a double click.
///
/// With `tear_off` a bar runs down the flyout's end edge, named by `RailWords::tear_off`: a press on it dragged past
/// the press slop, a click on it, or the release over it of the press that opened the flyout closes the flyout and
/// reports the pointer in `torn_off`, once per press. The bar is the pointer's: the rows are the keyboard's way to the
/// tools. With `tips` a row whose tool has a tip shows it, and the button leaves its command tooltip to the host, which
/// shows the shown tool's tip with `Show::show_with_tip`.
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
    /// A click, Enter or Space fires this command in place of selecting the shown tool; the flyout opens by a hold, a
    /// drag away, a right click or the Menu key.
    #[prop(into)]
    click: Option<CommandRef>,
    /// A tear-off bar along the flyout's end edge: a drag past slop, a click, or a hold-release over it closes the flyout
    /// and reports the pointer.
    #[prop(flag)]
    tear_off: bool,
    /// The tips of the tools: where one `has` a tip, its row shows it, and while it is the shown tool the button leaves
    /// its command tooltip to the tip the host shows the button with.
    #[prop]
    tips: Option<&'a dyn ToolTips<T>>,
    /// A request of the pass, the index of a tool among `tools`: the button comes into view in its scroll owner and,
    /// when the tool is not the shown one, opens its flyout.
    #[prop(vis = "pub(crate)")]
    reveal_row: Option<usize>,
    /// The index of the tool among `tools` whose row wears the target mark while the flyout is open.
    #[prop(vis = "pub(crate)")]
    marked_row: Option<usize>,
    laid: Option<Laid>,
    picked: Option<T>,
    open: bool,
    double: bool,
    /// The press of this pass opened the flyout (`Held`): its release over the bar tears the group off.
    held: bool,
    dropped: Option<ToolDropped>,
    torn_off: Option<Pos2>,
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
            click: None,
            tear_off: false,
            tips: None,
            reveal_row: None,
            marked_row: None,
            laid: None,
            picked: None,
            open: false,
            double: false,
            held: false,
            dropped: None,
            torn_off: None,
        }
    }

    /// Whether the button has a flyout.
    fn has_flyout(&self) -> bool {
        self.edit.is_none() && self.tools.len() > 1
    }

    fn shown_at(&self) -> usize {
        self.tools.iter().position(|tool| tool.id == *self.shown).unwrap_or(0)
    }

    /// The face the button shows: the shown tool's icon in the button's glyph role, in the look of its command (label,
    /// enabled state, marked dot).
    /// The button itself runs the command, so the face only wears it, and the button's selected state with it.
    fn face(&self, laid: &Laid, selected: bool, glyphs: GlyphRole) -> IconButton<'static> {
        // The shown tool's own icon, when it has one and the face stands for it, else its command's.
        let own = if self.click.is_none() { self.tools.get(laid.shown).and_then(|tool| tool.icon.clone()) } else { None };
        let icon = own.unwrap_or_else(|| {
            IconSpec::svg(self.lead(laid).and_then(|resolved| resolved.icon.clone()).map_or_else(|| IconId::new("more"), IconId::from))
        });
        let command = self.click.clone().or_else(|| self.tools.get(laid.shown).map(|tool| tool.command.clone()));
        let face = match command {
            Some(command) => IconButton::command(icon, command),
            None => IconButton::new(icon),
        };
        let mut face = face.selected(selected).glyphs(glyphs);
        if self.has_flyout() {
            face = face.popup(PopupKind::Menu).popup_mark(PopupMark::ToolGroup);
        }
        face
    }

    fn resolve(&self, cx: &MeasureCx<'_, Self>) -> Laid {
        Laid {
            resolved: self.tools.iter().map(|tool| cx.resolve_command(tool.command.clone())).collect(),
            click: self.click.clone().and_then(|command| cx.resolve_command(command)),
            shown: self.shown_at(),
        }
    }

    /// The command the face stands for: the click command when there is one, else the shown tool's.
    fn lead<'l>(&self, laid: &'l Laid) -> Option<&'l ResolvedCommand> {
        if self.click.is_some() { laid.click.as_deref() } else { laid.resolved.get(laid.shown).and_then(Option::as_deref) }
    }
}

impl<T: Value + 'static> Component for FlyoutButton<'_, T> {
    const ROLE: Role = Role::BUTTON;
    const SHOWS_LABEL: bool = false;
    type Style = FlyoutButtonStyle;
    type Output = FlyoutOutput<T>;

    fn style(cx: &StyleCx<'_>, _: FlyoutButtonStyleKey<T>) -> FlyoutButtonStyle {
        let side = cx.env::<RailDensity>().button(cx.tokens);
        FlyoutButtonStyle { metrics: Metrics::new().min_size(Vec2::splat(side)), mark: cx.tokens.space.gap.tight, glyphs: cx.glyph_role() }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let laid = self.resolve(cx);
        cx.face(self.face(&laid, cx.resolved.selected == Some(true), cx.style.glyphs))
    }

    /// A button that is edited is dragged; one with a flyout senses a drag away from it and a right click, which open
    /// the flyout. Without `CONTEXT_CLICK` the press machine never takes a secondary press, so `Trigger::Context` has
    /// nothing to fire on.
    fn sense(&self) -> Sense {
        let base = Sense::CLICK | Sense::FOCUS;
        match (self.edit.is_some(), self.has_flyout()) {
            (_, true) => base | Sense::DRAG | Sense::CONTEXT_CLICK,
            (true, false) => base | Sense::DRAG,
            (false, false) => base,
        }
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let laid = self.resolve(cx);
        let face = cx.part(FACE, rect).priority(HitPriority::new(HitLevel::Body, 0));
        let tipped = self.tips.is_some_and(|tips| tips.has(self.shown));
        if let Some(resolved) = self.lead(&laid).filter(|_| !tipped) {
            face.tooltip(match &resolved.chord_text {
                Some(chord) => Label::from(format!("{} ({chord})", resolved.label)),
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
                .pointer_only("the Menu key and a right click open the flyout")
                .toggles("menu");
        }
        self.laid = Some(laid);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let Some(laid) = &self.laid else { return };
        let shown_at = laid.shown;
        let enabled = self.lead(laid).is_some_and(|resolved| resolved.enabled);
        let alt = self.has_flyout() && cx.activation().is_some_and(|activation| !activation.by_key && activation.mods.alt);
        let stepped = if alt {
            let len = self.tools.len();
            (1..len)
                .map(|step| (shown_at + step) % len)
                .find(|at| laid.resolved.get(*at).and_then(Option::as_ref).is_some_and(|resolved| resolved.enabled))
        } else {
            None
        };
        let mut open = false;
        if self.has_flyout() {
            let menu = cx.popup("menu");
            menu.trigger(Trigger::Hold, cx);
            menu.trigger(Trigger::Context, cx);
            let away = match cx.press().phase {
                PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } => {
                    start.button == PointerButton::Primary && !cx.rect.contains(pos)
                }
                _ => false,
            };
            // A reveal of another tool than the shown one opens the flyout on its row; it selects nothing.
            let reveal = self.reveal_row.is_some_and(|at| at != shown_at);
            if (away || reveal) && !menu.is_open(cx) {
                menu.open(cx);
            }
            open = menu.is_open(cx);
        }
        if self.reveal_row.is_some() {
            cx.reveal(cx.rect);
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
        self.held = held;
        self.double = cx.press().double_pressed() && cx.press().mods.is_none();
        if alt {
            // The release that ends a hold or a drag away fires nothing, with Alt held or not.
            if let Some(at) = stepped.filter(|_| !held) {
                let tool = &self.tools[at];
                cx.fire(tool.command.clone(), FireSource::Pointer);
                self.picked = Some(tool.id.clone());
            }
        } else if activated && enabled && !(held && (!self.hold_selects || self.click.is_some())) {
            if let Some(click) = &self.click {
                cx.fire(click.clone(), FireSource::Pointer);
            } else {
                let tool = &self.tools[shown_at];
                cx.fire(tool.command.clone(), FireSource::Pointer);
                self.picked = Some(tool.id.clone());
            }
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

    /// The flyout: one row per tool, each the tool's command, the shown tool with the radio mark, and with `tear_off`
    /// the bar beside the rows. Running a row reports its tool and closes the flyout; tearing it off by the bar
    /// reports the pointer and closes it.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if !self.has_flyout() {
            return;
        }
        let popup = cx.popup("menu");
        let (tools, shown, tips, marked, tear_off) = (self.tools, self.shown, self.tips, self.marked_row, self.tear_off);
        let name = self.laid.as_ref().and_then(|laid| laid.resolved.get(laid.shown)).and_then(Option::as_ref).map(|resolved| resolved.label.clone());
        let name = name.map_or(Label::Empty, Label::Shared);
        let bar_name = cx.words::<RailWords>().tear_off.clone();
        let layer = menu::layer(false).place(Place::anchored(Anchor::Response, self.opens, Align::Start));
        // PLAN: dp.T2 asks the flyout to bring `reveal_row` into view, but a menu row has no reveal request of its own
        // (`MenuItem` calls no `Focus::reveal`); the rows show from the top, which holds a tool group in every window
        // the rail fits. Owner of the menu rows: ui-widgets.
        let shown_rows = ui.show(popup, layer, |ui, _| {
            Row::new()
                .show(ui, |ui, _| {
                    let rows = Menu::inline(name.clone()).show(ui, |ui, _| {
                        let mut ran = None;
                        for (at, tool) in tools.iter().enumerate() {
                            let mut item = MenuItem::command(tool.command.clone()).radio_mark(tool.id == *shown);
                            if let Some(icon) = &tool.icon {
                                item = item.icon(icon.clone());
                            }
                            let row = match tips.filter(|tips| tips.has(&tool.id)) {
                                Some(tips) => item.show_with_tip(ui, |ui| tips.show(&tool.id, ui)),
                                None => item.show(ui),
                            };
                            if marked == Some(at) {
                                TargetMark::new(row.response.rect).id_salt("marked_row").show_mark(ui);
                            }
                            if row.ran {
                                ran = Some(at);
                            }
                        }
                        ran
                    });
                    let height = rows.output.response.rect.height();
                    let bar = tear_off.then(|| TearOffBar::new(height).label(bar_name.clone()).show(ui));
                    (rows.inner.flatten(), bar)
                })
                .inner
        });
        let Some((ran, bar)) = shown_rows.flatten() else { return };
        // The release of the press that opened the flyout tears it off over the bar, as a click on the bar does.
        let released = match cx.press().phase {
            PressPhase::Release { pos, .. } if self.held => Some(pos),
            _ => None,
        };
        let torn = bar.and_then(|bar| bar.torn.or(released.filter(|pos| bar.response.rect.contains(*pos))));
        if let Some(at) = ran {
            self.picked = Some(tools[at].id.clone());
            popup.close(cx);
        } else if let Some(pointer) = torn {
            self.torn_off = Some(pointer);
            popup.close(cx);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let resolved = self.laid.as_ref().and_then(|laid| self.lead(laid));
        // A command that does not resolve leaves the button's own label as its name.
        let own = (!cx.resolved.label.is_empty()).then(|| cx.resolved.label.to_owned());
        let name = resolved.map(|resolved| resolved.label.as_ref().to_owned()).or(own);
        if let Some(name) = &name {
            access.name(Name::Text(name.clone()));
        }
        if let Some(hint) = resolved.and_then(|resolved| resolved.hint.as_ref()) {
            access.description(hint.as_ref().to_owned());
        }
        // The face covers the whole button, so the button's own node is the face's node.
        access.node(FACE, Role::None);
        if self.has_flyout() {
            access.has_popup(PopupKind::Menu);
            access.node(MARK, Role::BUTTON).name = Name::Text(name.unwrap_or_default());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(laid) = &self.laid else { return };
        let face = self.face(laid, cx.resolved.selected == Some(true), cx.style.glyphs);
        ui.face(FACE, face);
    }

    fn finish(self, cx: Cx<'_, Self>) -> FlyoutOutput<T> {
        FlyoutOutput {
            response: cx.response.clone(),
            select: self.picked,
            double_clicked: self.double,
            opened: self.open,
            dropped: self.dropped,
            torn_off: self.torn_off,
        }
    }
}

/// The style of a tear-off bar: a large handle's width, its grip in the faint ink.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct TearOffBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    width: f32,
    /// The grip's lines: their length, the gap between them and their ink.
    grip: f32,
    gap: f32,
    ink: StyleColor,
}

/// What a tear-off bar reports.
struct TearOffOutput {
    response: Response,
    /// Where the pointer tore the group off this pass: the drag past slop began, or a click ended.
    torn: Option<Pos2>,
}

impl Outcome for TearOffOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The bar down a flyout's end edge that tears the group off, as tall as the rows it runs beside: a press on it
/// dragged past the press slop reports where the drag began, a click where it ended, so a press reports once. It is
/// the pointer's alone and takes no focus.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(sample = TearOffBar::sample)]
struct TearOffBar {
    #[common]
    common: Common,
    height: f32,
    torn: Option<Pos2>,
}

impl TearOffBar {
    /// A bar `height` tall.
    fn new(height: f32) -> Self {
        TearOffBar { common: Common::new(), height, torn: None }
    }

    fn sample() -> Self {
        TearOffBar::new(96.0).label("Tear Off Group")
    }
}

impl Component for TearOffBar {
    const ROLE: Role = Role::BUTTON;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = TearOffBarStyle;
    type Output = TearOffOutput;

    fn style(cx: &StyleCx<'_>, _: TearOffBarStyleKey) -> TearOffBarStyle {
        let tokens = cx.tokens;
        TearOffBarStyle {
            metrics: Metrics::new(),
            width: tokens.size.handle.md,
            grip: tokens.size.icon.sm,
            gap: tokens.stroke.thick,
            ink: cx.color(tokens.color.text.faint),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.width, self.height)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let _ = cx
            .part(BAR, rect)
            .sense(Sense::DRAG | Sense::CLICK)
            .cursor(CursorIcon::Grab)
            .drag_cursor(CursorIcon::Grabbing)
            .state_layer()
            .pointer_only("the flyout's rows reach its tools from the keys");
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.torn = match cx.part(BAR).press.phase {
            PressPhase::DragStart { pos, .. } | PressPhase::Release { pos, click: true, .. } => Some(pos),
            _ => None,
        };
    }

    /// Three short lines across the middle of the bar.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let centre = cx.rect.center();
        for step in [-1.0, 0.0, 1.0] {
            let x = centre.x + step * style.gap;
            let line = vec![Pos2::new(x, centre.y - style.grip / 2.0), Pos2::new(x, centre.y + style.grip / 2.0)];
            ui.draw().stroke(&PaintShape::Path(line), StrokeWidth::Thin, style.ink);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TearOffOutput {
        TearOffOutput { response: cx.response.clone(), torn: self.torn }
    }
}
