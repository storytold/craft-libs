//! `ToolRail`: the rail of tool slots, with hidden tools, the last-used tool per slot and as many columns as its width
//! holds.

use super::flyout::{FlyoutButton, SlotEdit};
use super::words::RailWords;
use crate::buttons::IconButton;
use crate::contract::{
    Access, Common, Component, Container, ContainerProps, Cx, FooterRegion, Gap, NoValue, Outcome, Pad, Props as _, Role, Scroll, Show as _,
    ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::contract::{BodyCx, BodyLayout};
use crate::docking::DockGrid;
use crate::layout::RowAlign;
use crate::layout_components::{Row, Stack};
use crate::runtime::access::Name;
use crate::runtime::commands::Commands as _;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, GlyphRole, StyleCx, SurfaceKind};
use craft_behavior::overlay::Side;
use craft_behavior::{IconId, Metrics, NavShape, Proposal, Roving, Sense, Value};
use craft_theme::ControlSize;
use egui::{Response, Ui, Vec2};

mod data;
mod drag;
#[cfg(feature = "testing")]
pub(super) mod stories;
#[cfg(feature = "testing")]
mod trials;

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

/// What the buttons of one pass reported.
struct Collected<T> {
    select: Option<T>,
    double_clicked: Option<T>,
    edit_toolbar: Option<Response>,
    flyout: Option<SlotId>,
    dropped: Option<ToolDropped>,
}

impl<T> Default for Collected<T> {
    fn default() -> Self {
        Collected { select: None, double_clicked: None, edit_toolbar: None, flyout: None, dropped: None }
    }
}

type FooterFn<'a> = Box<dyn FnMut(&mut Ui, RailFooter) + 'a>;

/// The rail of tool slots, laid out in a grid of columns: one `FlyoutButton` per visible slot showing the slot's shown
/// tool, the slot with the current tool toggled on. A click selects the shown tool, a row of the flyout selects that
/// tool. The rail takes as many columns as the width it gets holds, the most whole columns that fit
/// ([`dock_grid`](ToolRail::dock_grid)): it is `first` wide at one column and `step` wider for each further column,
/// and the slots fill the rows in reading order. The control padding surrounds the buttons; a slot that ends a group
/// leaves `space.gap.inline` after its row; slots within a group sit `space.gap.tight` apart. The Edit Toolbar button
/// follows the last slot; the host's footer stays in view under the slots, which scroll when the room is short under
/// an overlay bar in the end padding that shows only under the pointer (`Scroll::overlay`), and its
/// buttons show the rail's glyphs. The rail is exactly as tall as its rows and footer at the width it gets
/// ([`measure_rail`](ToolRail::measure_rail)). The buttons are one roving group: arrow keys move between them by the
/// grid. While the toolbar is edited the buttons are dragged and take drops, and report the drop in the output.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(ToolRail<'static, u8>))]
pub struct ToolRail<'a, T: Value + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    slots: &'a [ToolSlot<T>],
    current: &'a T,
    last_used: &'a LastUsed<T>,
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
    #[prop]
    hidden: &'a [T],
    footer: Option<FooterFn<'a>>,
    /// The columns the rail lays out in this pass.
    columns: usize,
}

impl<'a, T: Value + 'static> ToolRail<'a, T> {
    /// A one-column rail named `label` over `slots`, with `current` the host's tool and `last_used` what the slots
    /// showed last.
    pub fn new(label: impl Into<Label>, slots: &'a [ToolSlot<T>], current: &'a T, last_used: &'a LastUsed<T>) -> Self {
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
            hidden: &[],
            footer: None,
            columns: 1,
        }
        .label(label)
    }

    /// The grid of a rail of `cells` cells (its visible slots and the Edit Toolbar button) as its dock column sizes
    /// it ([`PanelSpec::grid`](crate::docking::PanelSpec::grid)): `first` is a button and the control padding on both
    /// sides, `step` a button and the tight gap before it, and `most` the cells.
    pub fn dock_grid(ctx: &egui::Context, cells: usize) -> DockGrid {
        StyleCx::with_root(ctx, |cx| {
            let button = cx.tokens.size.rail.button;
            let most = u16::try_from(cells).unwrap_or(u16::MAX).max(1);
            DockGrid::new(button + 2.0 * cx.tokens.space.pad.control, button + cx.tokens.space.gap.tight, most)
        })
    }

    /// The cells the rail lays out: its visible slots and the Edit Toolbar button.
    fn cells(&self) -> usize {
        visible_slots(self.slots, self.hidden).len() + usize::from(self.edit_toolbar.is_some())
    }

    /// Sets up the pass for a rail `width` wide: its columns, its roving grid and its footer. Returns what the body
    /// draws from.
    fn prepare(&mut self, ctx: &egui::Context, width: f32, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> Draw<'a, T> {
        self.columns = usize::from(Self::dock_grid(ctx, self.cells()).steps_within(width));
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

/// What the body of one pass draws from.
struct Draw<'a, T> {
    slots: &'a [ToolSlot<T>],
    current: &'a T,
    last_used: &'a LastUsed<T>,
    hidden: &'a [T],
    editing: bool,
    edit_label: Option<Label>,
    opens: Side,
    columns: usize,
}

impl<T: Value + 'static> Draw<'_, T> {
    /// The grid of buttons in rows of `columns`, with the pass's results in `collected`.
    fn run(&self, ui: &mut Ui, cx: &mut BodyCx<'_>, collected: &mut Collected<T>) {
        let Draw { slots, current, last_used, hidden, editing, columns, opens, .. } = *self;
        *collected = Collected::default();
        let visible = visible_slots(slots, hidden);
        let cells = visible.len() + usize::from(self.edit_label.is_some());
        let enabled = |tool: &RailTool<T>| cx.resolve_command(tool.command.clone()).is_some_and(|resolved| resolved.enabled);
        let at: Vec<usize> = visible.iter().map(|(index, _)| *index).collect();
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
                                            if out.select.is_some() {
                                                collected.select = out.select;
                                            }
                                            if out.double_clicked {
                                                collected.double_clicked = Some(shown.id.clone());
                                            }
                                            if out.opened {
                                                collected.flyout = Some(slot.id);
                                            }
                                            if out.dropped.is_some() {
                                                collected.dropped = out.dropped;
                                            }
                                        }
                                        None => {
                                            let label = self.edit_label.clone().unwrap_or(Label::Empty);
                                            let more = IconButton::new(IconId::new("more")).id_salt("edit-toolbar").label(label).show(ui);
                                            collected.edit_toolbar = Some(more.response);
                                        }
                                    }
                                }
                            });
                        }
                    });
                }
            });
        });
    }
}

impl<'a, T: Value + 'static> ToolRail<'a, T> {
    /// Shows the rail in the width `ui` has left. `footer` draws under the last slot (colour chips, Quick Mask,
    /// Screen Mode); it stays in view when the room is short.
    pub fn show_rail(self, ui: &mut Ui, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> ToolRailOutput<T> {
        let room = ui.available_width();
        self.show_within(ui, room, footer)
    }

    /// Shows the rail one column wide, for a host that lays it at the start of a row (the full-window workspaces'
    /// tool strips): the width `ui` has left belongs to what follows it in the row.
    pub fn show_column(self, ui: &mut Ui, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> ToolRailOutput<T> {
        let one = Self::dock_grid(ui.ctx(), 1).first;
        self.show_within(ui, one, footer)
    }

    /// Shows the rail laid out `width` wide.
    fn show_within(mut self, ui: &mut Ui, width: f32, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> ToolRailOutput<T> {
        let draw = self.prepare(ui.ctx(), width, footer);
        let last = remembered(draw.slots, draw.current, draw.last_used);
        let mut collected = Collected::default();
        let shown = self.show(ui, |ui, cx| draw.run(ui, cx, &mut collected));
        ToolRailOutput {
            select: collected.select,
            double_clicked: collected.double_clicked,
            edit_toolbar: collected.edit_toolbar,
            flyout: collected.flyout,
            dropped: collected.dropped,
            last_used: last,
            ..shown.output
        }
    }

    /// The size of the rail laid out `width` wide, with `footer` under its rows: as many columns as `width` holds,
    /// as tall as its rows and footer (at most the room along the axis it scrolls on).
    pub fn measure_rail(mut self, ui: &Ui, width: f32, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> Vec2 {
        let draw = self.prepare(ui.ctx(), width, footer);
        self.measure_in(ui, Proposal::Width(width), |ui, cx| draw.run(ui, cx, &mut Collected::default()))
    }
}

impl<T: Value + 'static> Component for ToolRail<'_, T> {
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

impl<T: Value + 'static> Container for ToolRail<'_, T> {
    /// The host's footer.
    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let columns = self.columns;
        if let Some(footer) = &mut self.footer {
            footer(ui, RailFooter { columns });
        }
    }
}
