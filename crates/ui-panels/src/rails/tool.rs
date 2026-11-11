//! `ToolRail`: the rail of tool slots, with hidden tools, the last-used tool per slot and one or two columns.

use super::flyout::{FlyoutButton, SlotEdit};
use super::words::RailWords;
use crate::buttons::IconButton;
use crate::contract::{
    Access, Common, Component, Container, ContainerProps, Cx, FooterRegion, Gap, HeaderRegion, NoValue, Outcome, Pad, PaneEdge, Props as _, Role,
    Scroll, Show as _, ShowContainer as _, Sizing, WordSetCx as _, clicked,
};
use crate::contract::{BodyCx, BodyLayout};
use crate::layout_components::Row;
use crate::runtime::access::Name;
use crate::runtime::commands::Commands as _;
use crate::runtime::env::{Env, Label};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::overlay::Side;
use craft_behavior::{IconId, Metrics, NavShape, Roving, Sense, Value};
use craft_theme::ControlSize;
use egui::{Response, Ui};
use std::num::NonZeroU8;

mod data;
mod drag;
#[cfg(feature = "testing")]
pub(super) mod stories;

pub use data::{LastUsed, RailTool, SlotId, ToolSlot, remembered, shown_tool, visible_slots};
pub(crate) use drag::SlotDrop;
pub use drag::{DropAt, ToolDrag, ToolDropped, drop_in_slot};

/// What the footer closure of [`ToolRail::show_rail`] is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RailFooter {
    /// The rail's columns: with two, the host lays its two mode buttons side by side.
    pub columns: usize,
}

/// The column toggle's icon. Widening (from one column) points away from the rail's edge, towards the canvas;
/// narrowing points back to the edge. Directional icons mirror in a right-to-left interface, so the icon that draws
/// pointing right there is the left one.
pub fn column_toggle_icon(columns: usize, at_left: bool, rtl: bool) -> &'static str {
    let points_right = (columns == 1) == at_left;
    if points_right != rtl { "chevrons_right" } else { "chevrons_left" }
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
    /// The column toggle was clicked; the host flips its column count.
    pub toggle_columns: bool,
    /// The Edit Toolbar button was clicked.
    pub edit_toolbar: bool,
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
    edit_toolbar: bool,
    flyout: Option<SlotId>,
    dropped: Option<ToolDropped>,
}

impl<T> Default for Collected<T> {
    fn default() -> Self {
        Collected { select: None, double_clicked: None, edit_toolbar: false, flyout: None, dropped: None }
    }
}

type FooterFn<'a> = Box<dyn FnMut(&mut Ui, RailFooter) + 'a>;

/// The rail of tool slots, as a side panel at one edge of the window: one `FlyoutButton` per visible slot showing the
/// slot's shown tool, the slot with the current tool toggled on. A click selects the shown tool, a row of the flyout
/// selects that tool. With two columns the slots fill rows in reading order. The column toggle is the header and the
/// Edit Toolbar button follows the last slot; the host's footer stays in view under the slots, which scroll when the
/// window is short. The buttons are one roving group: arrow keys move between them by the grid. While the toolbar is
/// edited the buttons are dragged and take drops, and report the drop in the output.
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
    /// One or two columns of slots (more count as two); the host owns the count.
    #[prop]
    columns: NonZeroU8,
    /// The edge the rail sits at: `PaneEdge::End` for the end edge, any other value for the start edge. The
    /// flyouts open towards the canvas and the column toggle points that way.
    #[prop]
    edge: PaneEdge,
    /// Shows the column toggle at the top, named by the label.
    #[prop(into)]
    column_toggle: Option<Label>,
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
    toggled: bool,
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
            .body_gap(Gap::Tight)
            .child_size(ControlSize::Md)
            .scroll(Scroll::vertical())
            .footer(FooterRegion::new());
        ToolRail {
            common: Common::new(),
            props,
            slots,
            current,
            last_used,
            columns: NonZeroU8::MIN,
            edge: PaneEdge::Start,
            column_toggle: None,
            edit_toolbar: None,
            editing: false,
            hidden: &[],
            footer: None,
            toggled: false,
        }
        .label(label)
    }

    fn column_count(&self) -> usize {
        usize::from(self.columns.get().min(2))
    }

    fn at_end(&self) -> bool {
        self.edge == PaneEdge::End
    }

    /// Shows the rail. `footer` draws under the last slot (colour chips, Quick Mask, Screen Mode); it stays in view
    /// when the window is short.
    pub fn show_rail(mut self, ui: &mut Ui, footer: impl FnMut(&mut Ui, RailFooter) + 'a) -> ToolRailOutput<T> {
        let columns = self.column_count();
        self.props = self.props.roving(Roving::new(NavShape::wrapped_grid(columns)));
        if self.column_toggle.is_some() {
            self.props = self.props.header(HeaderRegion::new());
        }
        self.footer = Some(Box::new(footer));
        let (slots, current, last_used, hidden) = (self.slots, self.current, self.last_used, self.hidden);
        let (editing, edit_label) = (self.editing, self.edit_toolbar.clone());
        let opens = if self.at_end() { Side::Start } else { Side::End };
        let last = remembered(slots, current, last_used);
        let mut collected = Collected::default();
        let shown = self.show(ui, |ui, cx| {
            collected = Collected::default();
            let visible = visible_slots(slots, hidden);
            let cells = visible.len() + usize::from(edit_label.is_some());
            let enabled = |tool: &RailTool<T>| cx.resolve_command(tool.command.clone()).is_some_and(|resolved| resolved.enabled);
            let rows: Vec<usize> = (0..cells).step_by(columns).collect();
            for first in rows {
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
                                let label = edit_label.clone().unwrap_or(Label::Empty);
                                let more = IconButton::new(IconId::new("more")).id_salt("edit-toolbar").label(label).show(ui);
                                collected.edit_toolbar |= clicked(&more);
                            }
                        }
                    }
                });
            }
        });
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
            toggle_columns: self.toggled,
            edit_toolbar: false,
            flyout: None,
            dropped: None,
            last_used: None,
        }
    }
}

impl<T: Value + 'static> Container for ToolRail<'_, T> {
    /// The column toggle.
    fn header(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let Some(label) = self.column_toggle.clone() else { return };
        let (columns, rtl) = (self.column_count(), Env::of(ui).direction.is_rtl());
        let icon = column_toggle_icon(columns, !self.at_end() != rtl, rtl);
        let toggle = IconButton::new(IconId::new(icon)).id_salt("column-toggle").label(label).show(ui);
        self.toggled |= clicked(&toggle);
    }

    /// The host's footer.
    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let columns = self.column_count();
        if let Some(footer) = &mut self.footer {
            footer(ui, RailFooter { columns });
        }
    }
}
