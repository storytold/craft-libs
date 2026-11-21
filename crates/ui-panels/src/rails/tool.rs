//! `ToolRail`: the rail of tool slots, with hidden tools, the last-used tool per slot and as many columns as its width
//! holds.

use super::flyout::{FlyoutButton, SlotEdit};
use super::words::RailWords;
use crate::buttons::IconButton;
use crate::contract::{
    Access, Common, Component, Container, ContainerProps, Cx, FooterRegion, Gap, NoValue, Outcome, Pad, Props as _, Role, Scroll, Show as _,
    ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::contract::{BodyCx, BodyLayout, MemoProps, MemoView, Outbox, PlainData, Tracked};
use crate::docking::{DockGrid, DockSizes};
use crate::layout::RowAlign;
use crate::layout_components::{Row, Stack};
use crate::runtime::access::Name;
use crate::runtime::commands::resolver_in;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, GlyphRole, StyleCx, SurfaceKind};
use craft_behavior::overlay::Side;
use craft_behavior::{IconId, Metrics, NavShape, Proposal, Roving, Sense, Value};
use craft_theme::ControlSize;
use egui::{Id, Response, Ui, Vec2};

mod data;
mod drag;
#[cfg(feature = "suite")]
pub(crate) mod stories;
#[cfg(feature = "suite")]
pub(crate) mod trials;

use data::row_groups;
pub use data::{LastUsed, RailTool, SlotId, ToolSlot, remembered, shown_tool, visible_slots};
pub(crate) use drag::SlotDrop;
pub use drag::{DropAt, ToolDrag, ToolDropped, drop_in_slot};

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
    /// A tool dropped on a slot while the toolbar is edited, and where it goes.
    pub dropped: Option<ToolDropped>,
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
/// grid. While the toolbar is edited the buttons are dragged and take drops, and report the drop in the output.
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
    /// The toolbar is being edited: the buttons are dragged and take drops, and show no flyouts.
    #[prop(flag)]
    editing: bool,
    /// The tools the user's toolbar settings hide.
    #[prop(into)]
    hidden: Option<&'a Tracked<Vec<T>>>,
    footer: Option<FooterFn<'a>>,
    /// The columns the rail lays out in this pass.
    columns: usize,
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
            editing: false,
            hidden: None,
            footer: None,
            columns: 1,
        }
        .label(label)
    }

    /// The cells the rail lays out: its visible slots and the Edit Toolbar button.
    fn cells(&self) -> usize {
        visible_slots(self.slots, hidden_in(self.hidden)).len() + usize::from(self.edit_toolbar.is_some())
    }

    /// Sets up the pass for a rail `width` wide: its columns, its roving grid and its footer. Returns what the body
    /// draws from.
    fn prepare(&mut self, ui: &Ui, width: f32, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> Draw<'a, T> {
        // The grid in the scope's style, as the dock column around the rail sizes it.
        let grid = DockGrid::rail(self.cells()).resolved(&DockSizes::of_scope(ui));
        self.columns = usize::from(grid.steps_within(width));
        self.props = std::mem::take(&mut self.props).roving(Roving::new(NavShape::wrapped_grid(self.columns)));
        self.footer = Some(Box::new(footer));
        Draw {
            slots: self.slots,
            current: self.current,
            last_used: self.last_used,
            hidden: self.hidden,
            editing: self.editing,
            edit_label: self.edit_toolbar.clone(),
            opens: self.opens,
            columns: self.columns,
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
    opens: Side,
    columns: usize,
    sent: Outbox<'a, RailEvent<T>>,
}

/// What the body of one pass draws from.
struct Draw<'a, T: PlainData> {
    slots: &'a Tracked<Vec<ToolSlot<T>>>,
    current: &'a T,
    last_used: &'a Tracked<LastUsed<T>>,
    hidden: Option<&'a Tracked<Vec<T>>>,
    editing: bool,
    edit_label: Option<Label>,
    opens: Side,
    columns: usize,
}

impl<T: Value + MemoProps + PlainData> Draw<'_, T> {
    /// Shows the grid as a memoized view; what the buttons report goes into `events` (nothing on a replay). Returns the
    /// Edit Toolbar button's id while the rail shows it.
    fn run(&self, ui: &mut Ui, events: &mut Vec<RailEvent<T>>) -> Option<Id> {
        let props = GridProps {
            slots: self.slots,
            current: self.current,
            last_used: self.last_used,
            hidden: self.hidden,
            editing: self.editing,
            edit_label: self.edit_label.clone(),
            opens: self.opens,
            columns: self.columns,
            sent: Outbox::new(events),
        };
        MemoView::new("slots", props, grid::<T>).show(ui).flatten()
    }
}

/// The grid of buttons in rows of `columns`. The result is the Edit Toolbar button's id, which the rail's output
/// turns into its response.
fn grid<T: Value + MemoProps + PlainData>(props: &mut GridProps<'_, T>, ui: &mut Ui) -> Option<Id> {
    let GridProps { slots, current, last_used, hidden, editing, columns, opens, .. } = *props;
    let visible = visible_slots(slots, hidden_in(hidden));
    let cells = visible.len() + usize::from(props.edit_label.is_some());
    let resolve = resolver_in(ui);
    let enabled = |tool: &RailTool<T>| resolve(tool.command.clone()).is_some_and(|resolved| resolved.enabled);
    let at: Vec<usize> = visible.iter().map(|(index, _)| *index).collect();
    let mut edit_toolbar = None;
    let sent = &mut props.sent;
    // The grid of buttons sits at the centre of the rail's width.
    Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
        Stack::new().gap(Gap::Inline).show(ui, |ui, _| {
            for group in row_groups(slots, &at, cells, columns) {
                Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
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
                                        let out = button.show(ui);
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
                                        let more = IconButton::new(IconId::new("more")).id_salt("edit-toolbar").label(label).show(ui);
                                        edit_toolbar = Some(more.response.id);
                                    }
                                }
                            }
                        });
                    }
                });
            }
        });
    });
    edit_toolbar
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
        let one = DockGrid::rail(1).resolved(&DockSizes::of_scope(ui)).width(1);
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
        let (mut events, mut edit_toolbar) = (Vec::new(), None);
        let shown = self.show(ui, |ui, _| edit_toolbar = draw.run(ui, &mut events));
        let (mut select, mut double_clicked, mut flyout, mut dropped) = (None, None, None, None);
        for event in events {
            match event {
                RailEvent::Select(tool) => select = Some(tool),
                RailEvent::DoubleClicked(tool) => double_clicked = Some(tool),
                RailEvent::Flyout(slot) => flyout = Some(slot),
                RailEvent::Dropped(drop) => dropped = Some(drop),
            }
        }
        ToolRailOutput {
            select,
            double_clicked,
            edit_toolbar: edit_toolbar.and_then(|id| ui.ctx().read_response(id)),
            flyout,
            dropped,
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

    fn finish(self, cx: Cx<'_, Self>) -> ToolRailOutput<T> {
        ToolRailOutput {
            response: cx.response.clone(),
            select: None,
            double_clicked: None,
            edit_toolbar: None,
            flyout: None,
            dropped: None,
            last_used: None,
        }
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
