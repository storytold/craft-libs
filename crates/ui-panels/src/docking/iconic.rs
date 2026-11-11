//! `IconicColumn`: a column collapsed to one icon per panel, and the flyouts its icons open.

use super::divider::{self, Divider, Drag};
use super::dock_column::{BarRecord, column_grip, register_edges, show_bar};
use super::flyout;
use super::{DockSizes, DockSpot, DockTargets, DockWords, DragUnit, Flyout, GroupAt, PanelKey, PanelSpec, Splitter, start_group_drag};
use crate::buttons::IconButton;
use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, HeaderRegion, Name, Outcome, OverlayUi,
    Overlays as _, Pad, PaintUi, PartsCx, Pointer as _, Props as _, Role, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::menu::GroupItem;
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, Corners, StyleCx, SurfaceKind};
use craft_behavior::dock::{Column, DockEdge, Group};
use craft_behavior::{Axis, CursorIcon, DockSide, IconId, Metrics, PartKey, PressPhase, Sense, Value};
use egui::{Id, Pos2, Rect, Response, Ui, Vec2};
use std::hash::Hash;

#[cfg(feature = "testing")]
mod stories;

const CELL: PartKey = PartKey::new("cell");
const GRIP: PartKey = PartKey::new("grip");
const EDGE: PartKey = PartKey::new("edge");

/// The sizes of an iconic column on top of the column sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct IconicSizes {
    /// The sizes every column shares.
    pub column: DockSizes,
    /// The height of an icon cell (the rail button).
    pub cell: f32,
    /// How far an icon group's drag band reaches into its first icon.
    pub grip: f32,
    /// The room between two icon groups.
    pub separator: f32,
}

impl IconicSizes {
    /// The sizes of the style context `cx`.
    pub fn style(cx: &StyleCx<'_>) -> Self {
        let size = &cx.tokens.size;
        IconicSizes { column: DockSizes::style(cx), cell: size.rail.button, grip: size.handle.sm, separator: cx.tokens.space.gap.tight }
    }
}

/// What an iconic column reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct IconicColumnOutput<P> {
    /// The open flyout's edits: the panel the user opened or switched to (the host makes it its group's active
    /// panel), `None` when it closed. The value is the bound open panel.
    pub edit: EditResponse<Option<P>>,
    /// The rect the column took, for `DockAreaOutput::columns`.
    pub placed: Rect,
    /// The user asked to expand the column (`false`); the host sets `Column::iconic`.
    pub iconic: Option<bool>,
    /// The dock row the user picked in the column menu.
    pub picked: Option<GroupItem>,
    /// The width the user gave the iconic column on its inner edge this pass.
    pub width: Option<f32>,
}

impl<P: Value + Hash + 'static> Outcome for IconicColumnOutput<P> {
    type Value = Option<P>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The style of an iconic column.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct IconicColumnStyle {
    #[style(metrics)]
    metrics: Metrics,
    sizes: IconicSizes,
}

/// A column collapsed to icons: the column bar over icon groups, one cell per panel. A click or Enter on an icon
/// opens the flyout of its panel beside the column, switches it to that panel or closes it; one flyout is open at a
/// time. A drag of an icon starts a move of its panel, a drag of the band on top of a group a move of the group, a
/// drag of the bar beside its button a move of the column; each starts through the drag session, and a press outside
/// the flyout closes it. Escape and a press
/// outside close the flyout; with `auto_collapse` the focus leaving it does too.
///
/// The open flyout is the bound `Option<P>` of [`new`](IconicColumn::new), owned by the host like the layout: a click
/// on an icon, Escape and auto-collapse write it, and the host opens a panel (the Window menu) by writing it. The column registers the drop bands of its edges and of the halves of
/// each icon group with the dock `dock`. The flyout shows its group through the `view` of
/// [`show_flyouts`](IconicColumn::show_flyouts), with the opened panel active, as tall as the view measures the
/// group within the room and at least the height of its panels' default heights.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, overlay, probe(IconicColumn<'static, u8>))]
pub struct IconicColumn<'a, P: Value + Hash + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    dock: Id,
    column: &'a Column<P>,
    spec: &'a dyn Fn(&P) -> PanelSpec,
    /// The window edge the column sits on, which the flyout opens away from.
    #[prop]
    edge: DockEdge,
    /// Whether Auto-Collapse Iconic Panels is on.
    #[prop]
    auto_collapse: bool,
    /// The rect the dock may grow within, which bounds how wide its inner edge can make the column.
    #[prop]
    within: Rect,
    open: Bind<'a, Option<P>>,
    view: Option<FlyoutView<'a, P>>,
    bar: Rect,
    record: BarRecord,
    shown: Option<usize>,
    width: Option<f32>,
    /// The plan `parts` laid out this pass, which update and access read.
    laid: Option<Plan>,
}

/// Where the parts of an iconic column sit.
struct Plan {
    bar: Rect,
    /// `(group, panel, rect)` of each icon cell, top to bottom.
    cells: Vec<(usize, usize, Rect)>,
    /// `(group, rect)` of each group's drag band.
    grips: Vec<(usize, Rect)>,
    /// `(group, rect)` of the cells of each group.
    groups: Vec<(usize, Rect)>,
    body: Rect,
}

impl<'a, P: Value + Hash + 'static> IconicColumn<'a, P> {
    /// A column named `label` of the dock `dock`, over `column`, whose open flyout is the panel in `open`; `spec`
    /// reads each panel's label, icon and limits.
    pub fn new(label: impl Into<Label>, dock: Id, column: &'a Column<P>, open: Bind<'a, Option<P>>, spec: &'a dyn Fn(&P) -> PanelSpec) -> Self {
        let props = ContainerProps::new().sizing(Sizing::Fill).pad(Pad::None).header(HeaderRegion::new());
        IconicColumn {
            common: Common::new(),
            props,
            dock,
            column,
            spec,
            edge: DockEdge::Start,
            auto_collapse: false,
            within: Rect::NOTHING,
            open,
            view: None,
            bar: Rect::NOTHING,
            record: BarRecord::default(),
            shown: None,
            width: None,
            laid: None,
        }
        .label(label)
    }

    /// Shows the column; `view` measures the group of an open flyout at the width it gets and returns its height
    /// (`None` takes the flyout's least height), then shows it in the rect it gets.
    pub fn show_flyouts(
        mut self,
        ui: &mut Ui,
        view: impl FnMut(&Group<P>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a,
    ) -> IconicColumnOutput<P> {
        self.view = Some(Box::new(view));
        self.show(ui, |_, _| ()).output
    }

    /// Shows the column at `rect` with no allocation, so its paint is clipped by the dock and not by a region of its
    /// own: the accent line of its edge reaches into the gutter beside it.
    #[doc(hidden)]
    pub fn show_flyouts_at(
        mut self,
        ui: &mut Ui,
        rect: Rect,
        view: impl FnMut(&Group<P>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a,
    ) -> IconicColumnOutput<P> {
        self.view = Some(Box::new(view));
        self.show_at(ui, rect, |_, _| ()).output
    }

    /// The panels of the column in order, each with its group.
    /// The divider on the column's inner edge: the width runs from the rail to the width the column expands to.
    fn edge_divider(&self, rect: Rect, sizes: &DockSizes) -> Divider {
        let least = sizes.iconic_least;
        let most = sizes.expanded_width(self.column, self.spec).max(least).min((self.within.width() - sizes.least_pane).max(rect.width()));
        Divider::column_edge(rect, self.edge, (least, most), sizes.reach)
    }

    /// Takes the width the coordinate `coord` of the inner edge asks for, kept within the divider's travel.
    fn move_edge(&mut self, rect: Rect, divider: &Divider, coord: f32) {
        let width = Divider::column_width(rect, self.edge, coord.clamp(divider.lo, divider.hi));
        if (width - rect.width()).abs() > f32::EPSILON {
            self.width = Some(width);
        }
    }

    fn flat(&self) -> Vec<(usize, usize)> {
        self.column.groups.iter().enumerate().flat_map(|(group, g)| (0..g.panels.len()).map(move |panel| (group, panel))).collect()
    }

    fn panel(&self, group: usize, panel: usize) -> Option<&P> {
        self.column.groups.get(group).and_then(|group| group.panels.get(panel))
    }

    fn plan(&self, rect: Rect, sizes: &IconicSizes) -> Plan {
        let bar = Rect::from_min_max(rect.min, Pos2::new(rect.right(), (rect.top() + sizes.column.bar).min(rect.bottom())));
        let body = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom()), rect.max);
        let (mut y, mut cells, mut grips, mut groups) = (body.top(), Vec::new(), Vec::new(), Vec::new());
        for (at, group) in self.column.groups.iter().enumerate() {
            if at > 0 {
                y += sizes.separator;
            }
            let top = y;
            for panel in 0..group.panels.len() {
                cells.push((at, panel, Rect::from_min_size(Pos2::new(body.left(), y), Vec2::new(body.width(), sizes.cell))));
                y += sizes.cell;
            }
            grips.push((at, Rect::from_min_max(Pos2::new(body.left(), top), Pos2::new(body.right(), (top + sizes.grip).min(y)))));
            groups.push((at, Rect::from_min_max(Pos2::new(body.left(), top), Pos2::new(body.right(), y))));
        }
        Plan { bar, cells, grips, groups, body }
    }

    /// The height of the flyout of `group`: the group's own height, else what its panels ask for, else the least.
    fn flyout_height(&self, group: &Group<P>, sizes: &DockSizes) -> f32 {
        let content = group
            .panels
            .iter()
            .filter_map(|panel| (self.spec)(panel).heights.filter(|heights| heights.valid()))
            .map(|heights| heights.default + sizes.strip)
            .fold(sizes.strip + sizes.least, f32::max);
        group.height.filter(|height| height.is_finite() && *height > 0.0).unwrap_or(content).max(sizes.strip + sizes.least)
    }

    /// The flyout of the cell `at`: the group with that cell's panel active.
    fn flyout_group(&self, at: usize) -> Option<Group<P>> {
        let (group, panel) = *self.flat().get(at)?;
        let mut shown = self.column.groups.get(group)?.clone();
        shown.active = panel;
        Some(shown)
    }
}

/// What measures and shows the group of an open flyout.
type FlyoutView<'a, P> = Box<dyn FnMut(&Group<P>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a>;

impl<P: Value + Hash + 'static> Component for IconicColumn<'_, P> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = IconicColumnStyle;
    type Output = IconicColumnOutput<P>;

    /// The column's edge line lies centred on the edge, so half of it is outside the column.
    fn paint_outset(tokens: &crate::style::UiTokens) -> f32 {
        tokens.stroke.thick / 2.0
    }

    fn style(cx: &StyleCx<'_>, _: IconicColumnStyleKey<P>) -> IconicColumnStyle {
        IconicColumnStyle { metrics: Metrics::new(), sizes: IconicSizes::style(cx) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<P>>> {
        Some(self.open.reborrow())
    }

    /// The icon cells, each toggling the flyout of its panel, and the drag band of every group. The cells win the
    /// top of the first icon over the band, which is declared first.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = cx.style.sizes;
        let plan = self.plan(rect, &sizes);
        self.bar = plan.bar;
        for (at, (_, _, cell)) in plan.cells.iter().enumerate() {
            let Some(panel) = self.flat().get(at).and_then(|(group, panel)| self.panel(*group, *panel)) else { continue };
            let label = (self.spec)(panel).label;
            cx.part(CELL.at(at as u64), *cell)
                .sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS)
                .focusable()
                .tooltip(label)
                .toggles_at("flyout", at);
        }
        for (group, grip) in &plan.grips {
            cx.part(GRIP.at(*group as u64), *grip)
                .sense(Sense::DRAG)
                .cursor(CursorIcon::Grab)
                .drag_cursor(CursorIcon::Grabbing)
                .pointer_only("Move Panel To in the panel menu moves the group's panels");
        }
        divider::declare(cx, rect, EDGE, 0, &self.edge_divider(rect, &sizes.column));
        self.laid = Some(plan);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let sizes = cx.style.sizes;
        let (rect, rtl, ctx) = (cx.rect, cx.flow.is_rtl(), cx.core.ctx.clone());
        let Some(plan) = self.laid.take() else { return };
        let flat = self.flat();
        let popup = cx.popup("flyout");
        let dragging =
            |phase: PressPhase| matches!(phase, PressPhase::DragStart { start, .. } if start.button == craft_behavior::PointerButton::Primary);
        let mut began = false;
        for (group, extent) in &plan.groups {
            if dragging(cx.part(GRIP.at(*group as u64)).press.phase)
                && let Some(lead) = self.column.groups.get(*group)
                && let Some(anchor) = lead.panels.first()
            {
                let shown = lead.active_panel().unwrap_or(anchor);
                let label = Label::from((self.spec)(shown).label.as_str().to_owned());
                start_group_drag(cx, DragUnit::Group(anchor.clone()), PanelKey::of(shown), Some((extent.size(), label)));
                began = true;
            }
        }
        for (at, (group, panel, cell)) in plan.cells.iter().enumerate() {
            if dragging(cx.part(CELL.at(at as u64)).press.phase)
                && let Some(panel) = self.panel(*group, *panel)
            {
                let label = Label::from((self.spec)(panel).label.as_str().to_owned());
                start_group_drag(cx, DragUnit::Panel(panel.clone()), PanelKey::of(panel), Some((cell.size(), label)));
                began = true;
            }
        }
        let edge = self.edge_divider(rect, &sizes.column);
        if let Some(Drag::To(coord)) = divider::pointer(cx, EDGE, 0, &edge) {
            self.move_edge(rect, &edge, coord);
        }
        if let Some((_, coord)) = divider::keys(cx, EDGE, &[(0, edge)], sizes.column.step, sizes.column.jump) {
            self.move_edge(rect, &edge, coord);
        }
        let handles: Vec<_> = (0..flat.len()).map(|at| popup.at(at)).collect();
        // A toggle of an icon (click, Enter, Alt+Down) changed a popup this pass: the user chose. Otherwise the
        // bound value is the truth and the popups follow it.
        let toggled = handles.iter().filter(|handle| handle.changed(cx)).count() > 0;
        let open_at =
            |panel: &Option<P>| panel.as_ref().and_then(|open| flat.iter().position(|(group, panel)| self.panel(*group, *panel) == Some(open)));
        let mut opens: Vec<bool> = handles.iter().map(|handle| handle.is_open(cx)).collect();
        let chosen = if began {
            Some(None)
        } else if toggled {
            Some(opens.iter().position(|open| *open).and_then(|at| flat.get(at)).and_then(|(group, panel)| self.panel(*group, *panel)).cloned())
        } else {
            None
        };
        if let Some(chosen) = chosen
            && cx.value() != Some(&chosen)
        {
            cx.set(chosen);
        }
        let at = open_at(&cx.value().cloned().flatten());
        for (index, handle) in handles.iter().enumerate() {
            let want = at == Some(index);
            if opens[index] != want {
                if want {
                    handle.open(cx);
                } else {
                    handle.close(cx);
                }
                opens[index] = want;
            }
        }
        self.shown = at;
        self.register(&ctx, rect, rtl, &plan, &sizes);
        self.laid = Some(plan);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        // Painted here, not as the container's surface, which lies over what `paint` draws.
        ui.draw().surface(cx.rect, SurfaceKind::Panel, Corners::All);
        ui.draw().surface(self.bar, SurfaceKind::Background, Corners::None);
        let flat = self.flat();
        for (at, (group, panel)) in flat.iter().enumerate() {
            let Some(panel) = self.panel(*group, *panel) else { continue };
            let spec = (self.spec)(panel);
            let icon: IconSpec<'static> = spec.icon.unwrap_or_else(|| IconId::new("more").into());
            ui.face(CELL.at(at as u64), IconButton::new(icon).label(spec.label).selected(self.shown == Some(at)));
        }
        // The edge's line lies on the column's inner edge; its handle stays inside the column.
        let (rect, edge) = (cx.rect, self.edge_divider(cx.rect, &cx.style.sizes.column));
        let x = if cx.flow.is_rtl() { rect.right() - (edge.at - rect.left()) } else { edge.at };
        ui.face(EDGE.at(0), Splitter::new(Axis::X).boundary(x));
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<DockWords>();
        let sizes = cx.style.sizes;
        let Some(plan) = self.laid.as_ref() else { return };
        for (at, (group, panel)) in self.flat().iter().enumerate() {
            let Some(panel) = self.panel(*group, *panel) else { continue };
            access.node(CELL.at(at as u64), Role::BUTTON).name = Name::Text((self.spec)(panel).label.as_str().to_owned());
        }
        for (group, _) in &plan.grips {
            let Some(shown) = self.column.groups.get(*group).and_then(|group| group.active_panel()) else { continue };
            let name = words.move_group.fill(&[("name", (self.spec)(shown).label.as_str())]);
            access.node(GRIP.at(*group as u64), Role::BUTTON).name = Name::Text(name);
        }
        divider::describe(access, EDGE, 0, &self.edge_divider(cx.rect, &sizes.column), words.resize.as_str(), sizes.column.step);
    }

    /// The flyout of every cell whose popup is open, beside the column.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("flyout");
        let sizes = cx.style.sizes.column;
        let width = sizes.expanded_width(self.column, self.spec);
        let (edge, auto_collapse) = (self.edge, self.auto_collapse);
        let groups: Vec<Option<(Group<P>, f32)>> = (0..self.flat().len())
            .map(|at| {
                self.flyout_group(at).map(|group| {
                    let height = self.flyout_height(&group, &sizes);
                    (group, height)
                })
            })
            .collect();
        let Some(view) = self.view.as_mut() else { return };
        let mut dismissed = false;
        for (at, group) in groups.into_iter().enumerate() {
            let Some((group, height)) = group else { continue };
            let anchor = ui.anchor_part(CELL.at(at as u64));
            Flyout::new(edge, Vec2::new(width, height), auto_collapse)
                .show(ui, popup.at(at), anchor, |ui, body, place| view(&group, place, ui, body));
            dismissed |= flyout::dismissed(ui, popup.at(at));
        }
        if dismissed {
            cx.set(None);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> IconicColumnOutput<P> {
        IconicColumnOutput { edit: cx.edit_response(), placed: cx.rect, iconic: self.record.iconic, picked: self.record.picked, width: self.width }
    }
}

impl<P: Value + Hash + 'static> IconicColumn<'_, P> {
    /// Registers the drop bands of the column's edges, and the upper and lower half of every icon group: a drop on
    /// the upper half stacks above the group, one on the lower half below it.
    fn register(&self, ctx: &egui::Context, rect: Rect, rtl: bool, plan: &Plan, sizes: &IconicSizes) {
        let Some(first) = self.column.groups.first().and_then(|group| group.panels.first()) else { return };
        register_edges(ctx, self.dock, first, rect, rtl, &sizes.column);
        let half = sizes.separator * 0.5;
        let last = plan.groups.len().saturating_sub(1);
        for (at, group) in &plan.groups {
            let Some(anchor) = self.column.groups.get(*at).and_then(|group| group.panels.first()) else { continue };
            let middle = group.center().y;
            let upper_top = if *at == 0 { plan.body.top() } else { group.top() - half };
            let lower_bottom = if *at == last { plan.body.bottom() } else { group.bottom() + half };
            let upper = Rect::from_x_y_ranges(group.x_range(), upper_top..=middle);
            let lower = Rect::from_x_y_ranges(group.x_range(), middle..=lower_bottom);
            DockTargets::register(ctx, self.dock, DockSpot::group(anchor.clone(), DockSide::Top), upper);
            DockTargets::register(ctx, self.dock, DockSpot::group(anchor.clone(), DockSide::Bottom), lower);
        }
    }
}

impl<P: Value + Hash + 'static> Container for IconicColumn<'_, P> {
    /// The column bar, over the icons. What the user did on it is read in `finish`.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if cx.shell().is_some_and(|shell| !shell.dock_visible()) {
            return;
        }
        if let Some(grip) = column_grip(self.column, self.spec, cx) {
            self.record = show_bar(ui, cx, self.bar, false, self.auto_collapse, self.edge, grip);
        }
    }
}
