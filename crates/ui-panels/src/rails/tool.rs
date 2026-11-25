//! `ToolRail`: the rail of tool slots, with hidden tools, the last-used tool per slot and as many columns as its width
//! holds.

use super::flyout::{FlyoutButton, SlotEdit};
use super::words::RailWords;
use crate::buttons::{ActiveLook, IconButton};
use crate::contract::{
    Access, Common, Component, Container, ContainerProps, Cx, FooterRegion, Gap, NoValue, Outcome, Pad, Pointer as _, Props as _, Role, Scroll,
    Show as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::contract::{BodyCx, BodyLayout, MemoProps, MemoView, Outbox, PlainData, ThemeScope, Tracked};
use crate::display::{Elide, Shade, TargetMark, Text};
use crate::docking::{DockGrid, DockSizes};
use crate::layout::RowAlign;
use crate::layout_components::{Row, Stack};
use crate::runtime::access::Name;
use crate::runtime::commands::resolver_in;
use crate::runtime::env::{AppValues, EnvValue, Label};
use crate::style::{ComponentStyle, GlyphRole, StyleCx, SurfaceKind, TypeRole, UiTokens};
use crate::tabs::TornOff;
use craft_behavior::overlay::Side;
use craft_behavior::{IconId, Metrics, NavShape, Proposal, Roving, Sense, Value};
use craft_theme::ControlSize;
use egui::{Id, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod data;
mod drag;
#[cfg(feature = "suite")]
pub(crate) mod stories;
#[cfg(feature = "suite")]
pub(crate) mod trials;

use data::row_groups;
pub use data::{LastUsed, RailTool, SlotId, ToolSlot, ToolTips, remembered, shown_tool, visible_slots};
use drag::RailDrop;
pub(crate) use drag::SlotDrop;
pub use drag::{DropAt, ToolDrag, ToolDropped, drop_in_slot};

/// How tightly a tool rail packs its buttons. A rail publishes its density to its content ([`ToolRail::density`]),
/// and the dock sizes the rail's column from the same density ([`DockGrid::density`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum RailDensity {
    /// Buttons `size.rail.button` square.
    #[default]
    Regular,
    /// Buttons a control high (`size.control.md`) square: a denser step than the scope's density gives, for an app
    /// whose compact toolbar packs its tools tighter than its other controls.
    Dense,
}

impl RailDensity {
    /// The side of a rail button at this density.
    pub fn button(self, tokens: &UiTokens) -> f32 {
        match self {
            RailDensity::Regular => tokens.size.rail.button,
            RailDensity::Dense => tokens.size.control.md,
        }
    }
}

/// What the footer closure of [`ToolRail::show_rail`] is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RailFooter {
    /// The rail's columns: with more than one, the host lays its buttons side by side.
    pub columns: usize,
}

/// The style of a tool rail: it paints its chrome surface and takes its spacing from the body gap its props name.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ToolRailStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a tool rail reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ToolRailOutput<T> {
    /// The rail's response.
    pub response: Response,
    /// The tool the host should make current: the shown tool of a clicked slot, or the tool chosen in a flyout. Its
    /// command has run.
    pub select: Option<T>,
    /// The shown tool of a slot that was double clicked; the host decides what that means.
    pub double_clicked: Option<T>,
    /// The Edit Toolbar button's response while the rail shows it: a click on it opens the drawer, and the drawer
    /// takes it as its opener ([`ToolbarDrawer::opened_by`](crate::rails::ToolbarDrawer::opened_by)).
    pub edit_toolbar: Option<Response>,
    /// The slot whose flyout is open.
    pub flyout: Option<SlotId>,
    /// A tool dropped on the rail while the toolbar is edited, and where it goes.
    pub dropped: Option<ToolDropped>,
    /// A group torn off the rail: `keys` and `grabbed` the slot, `pointer` where it tore off.
    pub torn_off: Option<TornOff<SlotId>>,
    /// The last-used tools after this pass, when the current tool moved a slot's remembered tool.
    pub last_used: Option<LastUsed<T>>,
}

impl<T: Value> Outcome for ToolRailOutput<T> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the slot grid's view sends out of a run.
enum RailEvent<T> {
    Select(T),
    DoubleClicked(T),
    Flyout(SlotId),
    Dropped(ToolDropped),
    TornOff(TornOff<SlotId>),
    /// The Edit Toolbar button's response in the run.
    EditToolbar(Response),
}

type FooterFn<'a> = Box<dyn FnMut(&mut Ui, RailFooter) + 'a>;

/// The rail of tool slots, laid out in a grid of columns: one `FlyoutButton` per visible slot showing the slot's shown
/// tool, the slot with the current tool toggled on. A click selects the shown tool, a row of the flyout selects that
/// tool. The rail takes as many columns as the width it gets holds, the most whole columns that fit
/// ([`DockGrid::rail`] in the scope's style): it is a button and the control padding wide at one column and a button and the tight gap wider for each further column,
/// and the slots fill the rows in reading order. The control padding surrounds the buttons; a slot that ends a group
/// leaves `space.gap.inline` after its row; slots within a group sit `space.gap.tight` apart. The Edit Toolbar button
/// follows the last slot; the host's footer stays in view under the slots, which scroll when the room is short under
/// an overlay bar in the end padding that shows only under the pointer (`Scroll::overlay`), and its
/// buttons show the rail's glyphs. The rail is exactly as tall as its rows and footer at the width it gets
/// ([`measure_rail`](ToolRail::measure_rail)). The buttons are one roving group: arrow keys move between them by the
/// grid. While the toolbar is edited the buttons are dragged and the rail takes drops, and reports the drop in the
/// output: on a button by where over it the drop falls ([`drop_in_slot`]), elsewhere on the rail as a slot of its own
/// beside the nearest button, and on a rail with no buttons as the last slot.
///
/// With `captions` the caption of the first slot of a row group that has one ([`ToolSlot::captioned`]) stands above
/// the group in the caption type, cut at the rail's width; without it a caption takes no room. The grid's width
/// ([`DockGrid::rail`]) does not change for captions, which only add height. With `tear_off` a group
/// torn off its flyout comes out in `torn_off`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(ToolRail<'static, u8>))]
pub struct ToolRail<'a, T: Value + MemoProps + PlainData + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    slots: &'a Tracked<Vec<ToolSlot<T>>>,
    current: &'a T,
    last_used: &'a Tracked<LastUsed<T>>,
    /// The side of the rail its flyouts and drawer open on: the side of the canvas.
    #[prop]
    opens: Side,
    /// Shows the Edit Toolbar button after the last slot, named by the label.
    #[prop(into)]
    edit_toolbar: Option<Label>,
    /// The icon of the Edit Toolbar button; an ellipsis unless set (a settings gear is production's).
    #[prop(into)]
    edit_icon: Option<IconId>,
    /// The toolbar is being edited: the buttons are dragged and take drops, and show no flyouts.
    #[prop(flag)]
    editing: bool,
    /// The tools the user's toolbar settings hide.
    #[prop(into)]
    hidden: Option<&'a Tracked<Vec<T>>>,
    /// Whether each flyout shows a tear-off bar along its end edge: tearing a group off by it reports `torn_off`.
    #[prop(flag)]
    tear_off: bool,
    /// Whether the slots' captions show above their row groups.
    #[prop(flag)]
    captions: bool,
    /// The tool whose button, or whose row while that slot's flyout is open, wears the target mark.
    #[prop]
    mark: Option<&'a T>,
    /// A request of the pass: scrolls the tool's slot into view and opens its flyout when the slot shows another tool.
    /// It selects nothing.
    #[prop]
    reveal: Option<&'a T>,
    /// The tips the tools' buttons and flyout rows show in place of the command tooltip, where a tool has one.
    #[prop]
    tips: Option<&'a dyn ToolTips<T>>,
    /// How tightly the rail packs its buttons; its columns follow the rail grid at that density.
    #[prop]
    density: RailDensity,
    /// How the button of the current tool shows it is on.
    #[prop]
    active_look: ActiveLook,
    footer: Option<FooterFn<'a>>,
    /// The columns the rail lays out in this pass.
    columns: usize,
    /// The host's index and rect of each button the grid showed in this pass.
    shown: Rc<RefCell<Vec<(usize, Rect)>>>,
}

impl<'a, T: Value + MemoProps + PlainData + 'static> ToolRail<'a, T> {
    /// A one-column rail named `label` over `slots`, with `current` the host's tool and `last_used` what the slots
    /// showed last.
    pub fn new(label: impl Into<Label>, slots: &'a Tracked<Vec<ToolSlot<T>>>, current: &'a T, last_used: &'a Tracked<LastUsed<T>>) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Chrome)
            .pad(Pad::Control)
            .sizing(Sizing::Content)
            .layout(BodyLayout::stack())
            .child_size(ControlSize::Md)
            .child_glyphs(GlyphRole::ToolRail)
            .scroll(Scroll::vertical().overlay())
            .footer(FooterRegion::new());
        ToolRail {
            common: Common::new(),
            props,
            slots,
            current,
            last_used,
            opens: Side::End,
            edit_toolbar: None,
            edit_icon: None,
            editing: false,
            hidden: None,
            tear_off: false,
            captions: false,
            mark: None,
            reveal: None,
            tips: None,
            density: RailDensity::Regular,
            active_look: ActiveLook::Accent,
            footer: None,
            columns: 1,
            shown: Rc::default(),
        }
        .label(label)
    }

    /// The rail, what it shows and the flyouts it opens resolve their tokens through `scope` (a compact rail's
    /// density), and its columns follow the rail grid in that scope ([`DockGrid::rail`]). A host whose dock is not in
    /// `scope` gives the rail's column that grid resolved there:
    /// `DockGrid::rail(cells).resolved(&StyleCx::with_root_scoped(ctx, &scope, DockSizes::style))`.
    // PLAN: the sheet has `prepare` read the rail's own `ContainerProps::theme`, which no builder of the rail set: a
    // bare `#[container]` does not forward `theme`.
    #[must_use]
    pub fn theme(mut self, scope: ThemeScope) -> Self {
        self.props = std::mem::take(&mut self.props).theme(scope);
        self
    }

    /// The cells the rail lays out: its visible slots and the Edit Toolbar button.
    fn cells(&self) -> usize {
        visible_slots(self.slots, hidden_in(self.hidden)).len() + usize::from(self.edit_toolbar.is_some())
    }

    /// The rail grid of `cells` cells in the rail's own scope (`ContainerProps::theme`), else in the scope `ui` is in, as
    /// the dock column around the rail sizes it.
    fn grid(&self, ui: &Ui, cells: usize) -> DockGrid {
        let sizes = match &self.props.theme {
            Some(scope) => StyleCx::with_root_scoped(ui.ctx(), scope, DockSizes::style),
            None => DockSizes::of_scope(ui),
        };
        DockGrid::rail(cells).density(self.density).resolved(&sizes)
    }

    /// Sets up the pass for a rail `width` wide: its columns, its roving grid and its footer. Returns what the body
    /// draws from.
    fn prepare(&mut self, ui: &Ui, width: f32, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> Draw<'a, T> {
        self.columns = usize::from(self.grid(ui, self.cells()).steps_within(width));
        self.props = std::mem::take(&mut self.props).roving(Roving::new(NavShape::wrapped_grid(self.columns)));
        self.footer = Some(Box::new(footer));
        Draw {
            slots: self.slots,
            current: self.current,
            last_used: self.last_used,
            hidden: self.hidden,
            editing: self.editing,
            edit_label: self.edit_toolbar.clone(),
            edit_icon: self.edit_icon.clone(),
            opens: self.opens,
            columns: self.columns,
            captions: self.captions,
            tear_off: self.tear_off,
            tips: self.tips,
            mark: self.mark,
            reveal: self.reveal,
        }
    }
}

/// The tools a rail hides: the host's, or none.
fn hidden_in<T: PlainData>(hidden: Option<&Tracked<Vec<T>>>) -> &[T] {
    hidden.map_or(&[], |hidden| hidden.as_slice())
}

/// What the grid's memoized view reads: the slots and the tool state they show, and an outbox for what the buttons
/// report. The host's footer is not in it: it runs every pass, outside the view.
#[derive(MemoProps)]
struct GridProps<'a, T: PlainData> {
    slots: &'a Tracked<Vec<ToolSlot<T>>>,
    current: &'a T,
    last_used: &'a Tracked<LastUsed<T>>,
    hidden: Option<&'a Tracked<Vec<T>>>,
    editing: bool,
    edit_label: Option<Label>,
    edit_icon: Option<IconId>,
    opens: Side,
    columns: usize,
    captions: bool,
    tear_off: bool,
    tips: Option<&'a HostTips<'a, T>>,
    mark: Option<&'a T>,
    reveal: Option<&'a T>,
    sent: Outbox<'a, RailEvent<T>>,
}

/// The host's tool tips, which the grid reads through a trait object: while the host sets them the grid runs every pass.
#[derive(MemoProps)]
#[memo_props(every_pass = "the host's tips are a trait object, whose answers no key can follow")]
struct HostTips<'a, T>(&'a dyn ToolTips<T>);

/// What the body of one pass draws from.
struct Draw<'a, T: PlainData> {
    slots: &'a Tracked<Vec<ToolSlot<T>>>,
    current: &'a T,
    last_used: &'a Tracked<LastUsed<T>>,
    hidden: Option<&'a Tracked<Vec<T>>>,
    editing: bool,
    edit_label: Option<Label>,
    edit_icon: Option<IconId>,
    opens: Side,
    columns: usize,
    captions: bool,
    tear_off: bool,
    tips: Option<&'a dyn ToolTips<T>>,
    mark: Option<&'a T>,
    reveal: Option<&'a T>,
}

impl<T: Value + MemoProps + PlainData> Draw<'_, T> {
    /// Shows the grid as a memoized view; what the buttons report goes into `events` (nothing on a replay). Returns the
    /// Edit Toolbar button's id while the rail shows it, and the host's index and rect of each slot button.
    fn run(&self, ui: &mut Ui, events: &mut Vec<RailEvent<T>>) -> (Option<Id>, Vec<(usize, Rect)>) {
        let tips = self.tips.map(HostTips);
        let props = GridProps {
            slots: self.slots,
            current: self.current,
            last_used: self.last_used,
            hidden: self.hidden,
            editing: self.editing,
            edit_label: self.edit_label.clone(),
            edit_icon: self.edit_icon.clone(),
            opens: self.opens,
            columns: self.columns,
            captions: self.captions,
            tear_off: self.tear_off,
            tips: tips.as_ref(),
            mark: self.mark,
            reveal: self.reveal,
            sent: Outbox::new(events),
        };
        MemoView::new("slots", props, grid::<T>).show(ui).unwrap_or_default()
    }
}

/// The grid of buttons in rows of `columns`, each row group under its caption while the rail shows captions. The result
/// is the Edit Toolbar button's id, which the rail's output turns into its response, and the host's index and rect of
/// each slot button, around which the rail takes drops.
fn grid<T: Value + MemoProps + PlainData>(props: &mut GridProps<'_, T>, ui: &mut Ui) -> (Option<Id>, Vec<(usize, Rect)>) {
    let GridProps { slots, current, last_used, hidden, editing, columns, opens, captions, tear_off, tips, mark, reveal, .. } = *props;
    let tips = tips.map(|tips| tips.0);
    let visible = visible_slots(slots, hidden_in(hidden));
    let cells = visible.len() + usize::from(props.edit_label.is_some());
    let resolve = resolver_in(ui);
    let enabled = |tool: &RailTool<T>| resolve(tool.command.clone()).is_some_and(|resolved| resolved.enabled);
    let at: Vec<usize> = visible.iter().map(|(index, _)| *index).collect();
    // The first caption among a group's slots heads the group.
    let caption_of = |group: &[usize]| {
        let first = *group.first()?;
        let end = group.last().map_or(first, |last| (last + columns).min(cells));
        visible.get(first..end.min(visible.len()))?.iter().find_map(|(index, _)| slots[*index].caption.clone())
    };
    let (mut edit_toolbar, mut shown_at) = (None, Vec::with_capacity(visible.len()));
    let sent = &mut props.sent;
    // The grid of buttons sits at the centre of the rail's width.
    Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
        Stack::new().gap(Gap::Inline).show(ui, |ui, _| {
            for group in row_groups(slots, &at, cells, columns) {
                let caption = captions.then(|| caption_of(&group)).flatten();
                Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                    if let Some(caption) = caption.clone() {
                        Text::new(caption).id_salt(("caption", group[0])).role(TypeRole::Caption).shade(Shade::Muted).elide(Elide::End).show(ui);
                    }
                    for &first in &group {
                        Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                            for k in first..(first + columns).min(cells) {
                                match visible.get(k) {
                                    Some((index, tools)) => {
                                        let slot = &slots[*index];
                                        let owned: Vec<RailTool<T>> = tools.iter().map(|tool| (*tool).clone()).collect();
                                        let Some(shown) = shown_tool(current, slot.id, tools, last_used, enabled) else { continue };
                                        let selected = tools.iter().any(|tool| tool.id == *current);
                                        let mut button = FlyoutButton::new(&owned, &shown.id)
                                            .id_salt(("slot", slot.id))
                                            .selected(selected)
                                            .enabled(tools.iter().any(|tool| enabled(tool)))
                                            .opens(opens);
                                        if editing {
                                            button = button.edit(SlotEdit::new(slot.id, *index, slot.tools.len(), columns == 1));
                                        }
                                        if tear_off {
                                            button = button.tear_off();
                                        }
                                        if let Some(tips) = tips {
                                            button = button.tips(tips);
                                        }
                                        let row_of = |tool: Option<&T>| tool.and_then(|tool| owned.iter().position(|own| own.id == *tool));
                                        let marked = row_of(mark);
                                        if let Some(at) = marked {
                                            button = button.marked_row(at);
                                        }
                                        if let Some(at) = row_of(reveal) {
                                            button = button.reveal_row(at);
                                        }
                                        let out = match tips.filter(|tips| tips.has(&shown.id)) {
                                            Some(tips) => button.show_with_tip(ui, |ui| tips.show(&shown.id, ui)),
                                            None => button.show(ui),
                                        };
                                        shown_at.push((*index, out.response.rect));
                                        // The slot wears the mark; while its flyout is open the tool's row does.
                                        if marked.is_some() && !out.opened {
                                            TargetMark::new(out.response.rect).id_salt(("mark", slot.id)).show_mark(ui);
                                        }
                                        if let Some(pointer) = out.torn_off {
                                            sent.send(RailEvent::TornOff(TornOff::new(vec![slot.id], slot.id, pointer)));
                                        }
                                        if let Some(tool) = out.select {
                                            sent.send(RailEvent::Select(tool));
                                        }
                                        if out.double_clicked {
                                            sent.send(RailEvent::DoubleClicked(shown.id.clone()));
                                        }
                                        if out.opened {
                                            sent.send(RailEvent::Flyout(slot.id));
                                        }
                                        if let Some(dropped) = out.dropped {
                                            sent.send(RailEvent::Dropped(dropped));
                                        }
                                    }
                                    None => {
                                        let label = props.edit_label.clone().unwrap_or(Label::Empty);
                                        let icon = props.edit_icon.clone().unwrap_or_else(|| IconId::new("more"));
                                        let more = IconButton::new(icon).id_salt("edit-toolbar").label(label).show(ui);
                                        edit_toolbar = Some(more.response.id);
                                        sent.send(RailEvent::EditToolbar(more.response));
                                    }
                                }
                            }
                        });
                    }
                });
            }
        });
    });
    (edit_toolbar, shown_at)
}

impl<'a, T: Value + MemoProps + PlainData + 'static> ToolRail<'a, T> {
    /// Shows the rail in the width `ui` has left. `footer` draws under the last slot (colour chips, Quick Mask,
    /// Screen Mode); it stays in view when the room is short.
    pub fn show_rail(self, ui: &mut Ui, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> ToolRailOutput<T> {
        let room = ui.available_width();
        self.show_within(ui, room, footer)
    }

    /// Shows the rail one column wide, for a host that lays it at the start of a row (the full-window workspaces'
    /// tool strips): the width `ui` has left belongs to what follows it in the row.
    pub fn show_column(self, ui: &mut Ui, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> ToolRailOutput<T> {
        let one = self.grid(ui, 1).width(1);
        self.show_within(ui, one, footer)
    }

    /// Shows the rail laid out `width` wide.
    fn show_within(mut self, ui: &mut Ui, width: f32, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> ToolRailOutput<T> {
        let draw = self.prepare(ui, width, footer);
        // The last-used tools after this pass: derived while the slots, the remembered and hidden tools and the current tool are as they were.
        let key = (draw.slots.revision(), draw.last_used.revision(), draw.hidden.map(Tracked::revision), draw.current);
        let last = crate::runtime::derived::derived(ui.ctx(), ui.scope_id().with("remembered"), &key, || {
            remembered(draw.slots, draw.current, draw.last_used)
        });
        let last = last.as_ref().clone();
        let (mut events, mut edit_toolbar, laid) = (Vec::new(), None, Rc::clone(&self.shown));
        let shown = self.show(ui, |ui, _| {
            let (edit, slots) = draw.run(ui, &mut events);
            edit_toolbar = edit;
            *laid.borrow_mut() = slots;
        });
        let (mut select, mut double_clicked, mut flyout, mut dropped, mut torn_off, mut ran) = (None, None, None, None, None, None);
        for event in events {
            match event {
                RailEvent::Select(tool) => select = Some(tool),
                RailEvent::DoubleClicked(tool) => double_clicked = Some(tool),
                RailEvent::Flyout(slot) => flyout = Some(slot),
                RailEvent::Dropped(drop) => dropped = Some(drop),
                RailEvent::TornOff(torn) => torn_off = Some(torn),
                RailEvent::EditToolbar(response) => ran = Some(response),
            }
        }
        ToolRailOutput {
            select,
            double_clicked,
            // A replayed grid sends nothing: its button is shown again, and its response is read back.
            edit_toolbar: ran.or_else(|| edit_toolbar.and_then(|id| crate::contract::response_of(ui, id))),
            flyout,
            dropped: dropped.or(shown.output.dropped),
            torn_off,
            last_used: last,
            ..shown.output
        }
    }

    /// The size of the rail laid out `width` wide, with `footer` under its rows: as many columns as `width` holds,
    /// as tall as its rows and footer (at most the room along the axis it scrolls on).
    pub fn measure_rail(mut self, ui: &Ui, width: f32, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> Vec2 {
        let draw = self.prepare(ui, width, footer);
        self.measure_in(ui, Proposal::Width(width), |ui, _| {
            draw.run(ui, &mut Vec::new());
        })
    }
}

impl<T: Value + MemoProps + PlainData + 'static> Component for ToolRail<'_, T> {
    const ROLE: Role = Role::TOOLBAR;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ToolRailStyle;
    type Output = ToolRailOutput<T>;

    fn style(_: &StyleCx<'_>, _: ToolRailStyleKey<T>) -> ToolRailStyle {
        ToolRailStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// A rail the caller did not label is named by the words of the family.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.name(Name::Text(cx.words::<RailWords>().tools.as_str().to_owned()));
        }
    }

    /// The rail's density and active look reach its buttons.
    fn content_env(&self, _: &Cx<'_, Self>, values: &mut AppValues) {
        values.set(self.density);
        values.set(self.active_look);
    }

    /// While the toolbar is edited the rail takes the drops its buttons do not: a blank toolbar's, and those in the room
    /// around the buttons. It registers after them, and declines over them.
    fn finish(self, mut cx: Cx<'_, Self>) -> ToolRailOutput<T> {
        let dropped = self.editing.then(|| self.rail_drop(cx.rect, cx.flow.is_rtl())).and_then(|target| cx.drop_target::<ToolDrag, _>(&target));
        ToolRailOutput {
            response: cx.response.clone(),
            select: None,
            double_clicked: None,
            edit_toolbar: None,
            flyout: None,
            dropped,
            torn_off: None,
            last_used: None,
        }
    }
}

impl<T: Value + MemoProps + PlainData + 'static> ToolRail<'_, T> {
    /// The rail `rect` as the target of the drops around the buttons the grid showed in this pass.
    fn rail_drop(&self, rect: Rect, rtl: bool) -> RailDrop {
        let vertical = self.columns == 1;
        let slots = self
            .shown
            .take()
            .into_iter()
            .map(|(index, rect)| {
                let slot = &self.slots[index];
                SlotDrop { rect, slot: slot.id, index, tools: slot.tools.len(), vertical, rtl }
            })
            .collect();
        RailDrop { rect, slots, count: self.slots.len(), beside: Cell::new(None) }
    }
}

impl<T: Value + MemoProps + PlainData + 'static> Container for ToolRail<'_, T> {
    /// The host's footer.
    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let columns = self.columns;
        if let Some(footer) = &mut self.footer {
            footer(ui, RailFooter { columns });
        }
    }
}
