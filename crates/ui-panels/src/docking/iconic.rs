//! `IconicColumn`: a column collapsed to one icon per panel, and the flyouts its icons open.

use super::divider::{self, Divider, Drag};
use super::dock_column::{
    BarRecord, CHEVRON, ColumnArrow, ColumnBar, bar_band, bar_slots, card_rect, card_style, column_grip, describe_chevron, paint_card, paint_chevron,
    register_edges, show_bar,
};
use super::group::GripMarks;
use super::{CardInset, DockSizes, DockSpot, DockTargets, DockWords, DragUnit, Flyout, GroupAt, PanelKey, PanelSpec, Splitter, start_icon_drag};
use crate::buttons::{ActiveLook, IconButton};
use crate::contract::overlay::Anchor;
use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, HeaderEdge, HeaderParts, HeaderRegion,
    MeasureCx, Name, Outcome, OverlayUi, Overlays as _, Pad, PaintUi, PartsCx, Pointer as _, Props as _, Role, ShowContainer as _, Sizing,
    WordSetCx as _,
};
use crate::menu::GroupItem;
use crate::primitives::{IconSpec, TextRun};
use crate::runtime::env::{Env, EnvValue, Label, with_env};
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, Corners, GlyphRole, RuleKind, StyleColor, StyleCx, SurfaceKind, SurfaceStyle};
use crate::tabs::PanelTabLook;
use craft_behavior::dock::{Column, DockEdge, Group};
use craft_behavior::overlay::Side;
use craft_behavior::{Axis, CursorIcon, DockSide, Metrics, PartKey, PressPhase, Sense, Value};
use craft_theme::ControlSize;
use egui::{Id, Pos2, Rect, Response, Ui, Vec2};
use std::fmt::Debug;
use std::hash::Hash;

#[cfg(feature = "suite")]
pub(crate) mod stories;
#[cfg(feature = "suite")]
pub(crate) mod trials;

const CELL: PartKey = PartKey::new("cell");
const NAME: PartKey = PartKey::new("name");
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
    /// The height of the grip strip at the top of each icon group, above its first icon: none with
    /// [`IconGroupSeparator::Rule`].
    pub grip: f32,
    /// How far a group's grip band reaches into its first icon: none with [`IconGroupSeparator::Grip`], whose strip is
    /// the band, `space.gap.inline` with [`IconGroupSeparator::Rule`].
    pub band: f32,
    /// The room between two icon groups: the tight gap, or with [`IconGroupSeparator::Rule`] the stack gap and the
    /// rule.
    pub separator: f32,
    /// The column width from which each cell shows its panel's name beside its icon.
    pub labels: f32,
}

/// The column width, in rail widths, from which the cells show their panels' names: 96 points at the Compact size.
const LABELS_IN_RAIL_WIDTHS: f32 = 2.4;

impl IconicSizes {
    /// The sizes of the style context `cx`.
    pub fn style(cx: &StyleCx<'_>) -> Self {
        let size = &cx.tokens.size;
        let gap = &cx.tokens.space.gap;
        let (grip, band, separator) = match cx.env::<IconGroupSeparator>() {
            IconGroupSeparator::Grip => (super::group::grip_strip(cx.tokens), 0.0, gap.tight),
            IconGroupSeparator::Rule => (0.0, gap.inline, gap.stack + cx.rule(RuleKind::Edge).width.points(cx.frame_ctx().pixels_per_point())),
        };
        IconicSizes { column: DockSizes::style(cx), cell: size.rail.button, grip, band, separator, labels: size.rail.width * LABELS_IN_RAIL_WIDTHS }
    }

    /// The sizes in the root scope's style, for code outside a style pass:
    /// `StyleCx::with_root(ctx, IconicSizes::style)`.
    pub fn of(ctx: &egui::Context) -> Self {
        StyleCx::with_root(ctx, IconicSizes::style)
    }

    /// The height of the icons of `column` under its bar: a grip strip per group, a cell per panel and the room between
    /// every two groups.
    pub fn icons_height<P>(&self, column: &Column<P>) -> f32 {
        let cells: usize = column.groups.iter().map(|group| group.panels.len()).sum();
        let groups = column.groups.len();
        groups as f32 * self.grip + cells as f32 * self.cell + groups.saturating_sub(1) as f32 * self.separator
    }
}

/// What the icons of an iconic column open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IconicMode {
    /// A click on an icon opens its panel in a flyout beside the collapsed column.
    #[default]
    Flyouts,
    /// The icons are a rail that stays beside the column when it expands: a click on an icon expands the column with
    /// that panel in front of its group, and a click on the icon of a panel in front of the expanded column collapses
    /// the column again. The column opens no flyouts.
    Rail,
}

/// What marks the icon groups of a collapsed column apart. An app sets it once at its root (`RootEnv::values`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum IconGroupSeparator {
    /// Each group has a grip strip above its first icon, which shows the dock's grip marks (`shape.grip`, in
    /// `line.grip`) and drags the group.
    #[default]
    Grip,
    /// No grip strips: a group's first icon stands right under the column's bar, and an edge rule (`line.border`) lies
    /// level in the middle of the `space.gap.stack` between two groups, `space.gap.inline` short of the column's sides.
    /// A bare band drags the group: the first group's top `space.gap.inline` of its first icon, a later group's room
    /// above it and that much of its first icon. The band takes a press there, so a click in it opens no flyout;
    /// below it the icon drags its own panel. VectorCraft's icon groups, as production drew them.
    Rule,
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
    /// The user asked to expand the column (`false`), or in [`IconicMode::Rail`] to collapse the expanded column
    /// (`true`); the host sets `Column::iconic`.
    pub iconic: Option<bool>,
    /// In [`IconicMode::Rail`], the panel whose icon the user chose to bring to the front of its group; the host
    /// makes it its group's active panel.
    pub front: Option<P>,
    /// The dock row the user picked in the column menu.
    pub picked: Option<GroupItem>,
    /// The width the user gave a docked iconic column on its inner edge this pass. A floating window's sides set the
    /// width of the column it holds (`FloatingGroupOutput::collapsed_width`).
    pub width: Option<f32>,
    /// The size the user gave the open flyout, with the top edge's offset from the icons when it moved.
    pub flyout: Option<FlyoutResize<P>>,
}

/// The size the user gave an open flyout.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FlyoutResize<P> {
    /// The panel whose flyout was resized.
    pub panel: P,
    /// The new width, when it differs from the width the column placed the flyout with: a drag of the top or
    /// bottom edge leaves it `None`, so the group's width keeps following the column's expanded width.
    pub width: Option<f32>,
    /// The new height, when it differs from the height the flyout showed at.
    pub height: Option<f32>,
    /// The top edge's offset from the icons' top, when the top edge moved.
    pub offset: Option<f32>,
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
    /// The ink of a panel's name in a wide column.
    name: StyleColor,
    /// The ink of the name of the panel whose flyout is open.
    name_open: StyleColor,
    /// The room between an icon and its name.
    name_gap: f32,
    /// The marks of a group's grip.
    grip: GripMarks,
    /// What marks the icon groups apart.
    separator: IconGroupSeparator,
    /// How far short of the column's sides a [`IconGroupSeparator::Rule`] stops.
    rule_inset: f32,
    /// The card of a column shown as one ([`card`](IconicColumn::card)).
    card: SurfaceStyle,
}

/// A column collapsed to icons: the column bar over icon groups, one cell per panel. A panel without an icon shows the
/// first letter of its name in a box. A column widened past `IconicSizes::labels` shows each panel's name beside its
/// icon, cut with an ellipsis to the cell; a click or drag on the name acts as one on the icon. A click or Enter on an icon
/// opens the flyout of its panel beside the column, switches it to that panel or closes it; one flyout is open at a
/// time. A drag of an icon starts a move of its panel, a drag of the grip on top of a group a move of the group, a
/// drag of the bar beside its button a move of the column; each starts through the drag session. Escape closes the
/// flyout; with `auto_collapse` a press outside it and the focus leaving it do too.
///
/// The open flyout is the bound `Option<P>` of [`new`](IconicColumn::new), owned by the host like the layout: a click
/// on an icon, Escape and auto-collapse write it, and the host opens a panel (the Window menu) by writing it. The
/// column registers its drop spots with the dock `dock` as an expanded column does: its edges, the seams between its
/// icon groups, the top and bottom of the stack and each icon group's cells, which join its tabs. The flyout shows
/// its group through the `view` of [`show_flyouts`](IconicColumn::show_flyouts), with the opened panel active, as tall as the view measures the
/// group within the room and at least the height of its panels' default heights; a group the user sized keeps that
/// size. The flyout's top is level with its group's icons, shifted by the group's `flyout_offset`, inside `within`;
/// a flyout taller than the room below that top moves up by what it overflows and no further.
/// The user resizes it by its top and bottom edges and the edge away from the column; each step comes back in
/// [`IconicColumnOutput::flyout`], which the host keeps in the group (`flyout_width`, `height`, `flyout_offset`).
///
/// In a floating window ([`ColumnBar::Window`]) the window's title row is the column's bar and the window's sides set
/// its width: the column shows its icons under it, with no divider on its inner edge, and registers no edge bands.
///
/// In [`IconicMode::Rail`] the icons open no flyouts and the bound open panel is not used: the column is a rail the
/// host also shows beside the expanded column. While `Column::iconic` is off the panels in front of their groups show
/// selected and the bar's chevron collapses the column; a click on an icon reports
/// [`IconicColumnOutput::front`] and [`IconicColumnOutput::iconic`].
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
    /// The edge the column keeps to: a dock's window edge, or the edge of the float area a floating window keeps to.
    /// The bar's double arrow sits at that end and the inner edge, which sets the column's width, faces away from it.
    #[prop]
    edge: DockEdge,
    /// The side of the column, in reading order, its flyouts open on (`Side::Start` or `Side::End`): away from a
    /// dock's window edge, towards the room of the float area for a floating column.
    #[prop]
    opens: Side,
    /// Where the column's bar is: its own (a dock column) or its floating window's title row.
    #[prop]
    bar: ColumnBar,
    /// Whether Auto-Collapse Iconic Panels is on.
    #[prop]
    auto_collapse: bool,
    /// What the icons open: flyouts, or the column itself as a rail.
    #[prop]
    mode: IconicMode,
    /// The rect the dock may grow within, which bounds how wide its inner edge can make the column, and the rect its
    /// flyouts open in. Unset, the flyouts take the room layers opened there have (`overlay::layer_room`): around a
    /// palette window, the work area.
    #[prop]
    within: Rect,
    /// Which inset the outer side of the docked column's card takes, when it shows as one
    /// ([`DockConfig::column_cards`](super::DockConfig::column_cards), [`DockSizes::card_place`]): its bar and icons
    /// sit inside the card, inset by that on its outer side and by [`DockSizes::card`] on its top and bottom.
    #[prop]
    card: Option<CardInset>,
    /// The look of the dock's panel tabs ([`DockConfig::tab_look`](super::DockConfig::tab_look)): the column's bar
    /// lies on the surface the groups' tab strips do.
    #[prop]
    tab_look: PanelTabLook,
    /// The docked column shows the `line.border` frame an expanded docked column shows, and its flyouts stand as docked
    /// columns do ([`Flyout::docked`], [`DockConfig::flush_docks`](super::DockConfig::flush_docks)).
    #[prop]
    framed: bool,
    open: Bind<'a, Option<P>>,
    view: Option<FlyoutView<'a, P>>,
    /// The bar's rect this pass.
    bar_rect: Rect,
    record: BarRecord,
    shown: Option<usize>,
    width: Option<f32>,
    /// The plan `parts` laid out this pass, which update and access read.
    laid: Option<Plan>,
    /// The bar's chevron was clicked this pass: the column expands.
    expand: bool,
    /// The size the user gave the open flyout this pass.
    flyout: Option<FlyoutResize<P>>,
    /// In rail mode, what a click on an icon asked for this pass: the column collapsed (`true`) or expanded.
    rail_iconic: Option<bool>,
    /// In rail mode, the panel a click on its icon brought to the front this pass.
    front: Option<P>,
}

/// Where the flyout of one cell shows: the group with the cell's panel active, its top left, its size, whether the
/// height is the group's own (the body's measure does not change it) and the top of the group's icons.
struct FlyoutPlan<P> {
    group: Group<P>,
    top_left: Pos2,
    size: Vec2,
    fixed: bool,
    icons_top: f32,
}

/// Where the parts of an iconic column sit.
struct Plan {
    bar: Rect,
    /// `(group, panel, rect)` of each icon cell, top to bottom: the whole row, or in a wide column the icon's square at
    /// the reading start.
    cells: Vec<(usize, usize, Rect)>,
    /// The rest of each row in a wide column, where the panel's name shows; empty in a narrow column.
    names: Vec<Rect>,
    /// `(group, rect)` of each group's grip: its strip above its first icon, or its band (`IconicSizes::band`).
    grips: Vec<(usize, Rect)>,
    /// `(group, rect)` of the whole of each group: its grip strip and its cells.
    groups: Vec<(usize, Rect)>,
    body: Rect,
}

impl<'a, P: Value + Hash + Debug + 'static> IconicColumn<'a, P> {
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
            opens: Side::End,
            bar: ColumnBar::Own,
            auto_collapse: false,
            mode: IconicMode::Flyouts,
            within: Rect::NOTHING,
            card: None,
            tab_look: PanelTabLook::Plain,
            framed: false,
            open,
            view: None,
            bar_rect: Rect::NOTHING,
            record: BarRecord::default(),
            shown: None,
            width: None,
            laid: None,
            expand: false,
            flyout: None,
            rail_iconic: None,
            front: None,
        }
        .label(label)
    }

    /// Shows the column; `view` measures the group of an open flyout at the width it gets and returns its height
    /// (`None` takes the flyout's least height), then shows it in the rect it gets.
    pub fn show_flyouts(
        self,
        ui: &mut Ui,
        view: impl FnMut(&Group<P>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a,
    ) -> IconicColumnOutput<P> {
        self.in_look(ui, view, |column, ui| column.show(ui, |_, _| ()).output)
    }

    /// Shows the column at `rect` with no allocation, so its paint is clipped by the dock and not by a region of its
    /// own: the accent line of its edge reaches into the gutter beside it.
    #[doc(hidden)]
    pub fn show_flyouts_at(
        self,
        ui: &mut Ui,
        rect: Rect,
        view: impl FnMut(&Group<P>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a,
    ) -> IconicColumnOutput<P> {
        self.in_look(ui, view, |column, ui| column.show_at(ui, rect, |_, _| ()).output)
    }

    /// Shows the column through `show` with its flyouts' `view`. Beside [`PanelTabLook::Block`] tabs the open panel's
    /// icon shows on `surface.control`, as the shown tab does ([`ActiveLook::Control`]); the flyouts' groups keep the
    /// active look around the column.
    fn in_look(
        mut self,
        ui: &mut Ui,
        mut view: impl FnMut(&Group<P>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a,
        show: impl FnOnce(Self, &mut Ui) -> IconicColumnOutput<P>,
    ) -> IconicColumnOutput<P> {
        if self.tab_look != PanelTabLook::Block {
            self.view = Some(Box::new(view));
            return show(self, ui);
        }
        let outer = Env::of(ui).get::<ActiveLook>();
        self.view = Some(Box::new(move |group, at, ui, body| {
            with_env(
                ui,
                |overrides| {
                    overrides.values.set(outer);
                },
                |ui| view(group, at, ui, body),
            )
        }));
        with_env(
            ui,
            |overrides| {
                overrides.values.set(ActiveLook::Control);
            },
            |ui| show(self, ui),
        )
    }

    /// Whether the column is a rail beside its expanded column: its chevron collapses the column.
    fn expanded(&self) -> bool {
        self.mode == IconicMode::Rail && !self.column.iconic
    }

    /// Whether the cell `at`, of the panel `(group, panel)`, shows selected: the open flyout's, or in rail mode a
    /// panel in front of the expanded column.
    fn shows_selected(&self, at: usize, (group, panel): (usize, usize)) -> bool {
        match self.mode {
            IconicMode::Flyouts => self.shown == Some(at),
            IconicMode::Rail => self.expanded() && self.column.groups.get(group).is_some_and(|group| group.active == panel),
        }
    }

    /// In rail mode, what a click on the cell `at` asks for: on a panel in front of the expanded column, the column
    /// collapses; on any other, the column expands with that panel in front.
    fn rail_click(&mut self, at: usize) {
        let Some((group, panel)) = self.flat().get(at).copied() else { return };
        if self.shows_selected(at, (group, panel)) {
            self.rail_iconic = Some(true);
            return;
        }
        if self.column.iconic {
            self.rail_iconic = Some(false);
        }
        if self.column.groups.get(group).is_some_and(|shown| shown.active != panel) {
            self.front = self.panel(group, panel).cloned();
        }
    }

    /// The divider on the column's inner edge: the width runs from the rail to the width the column expands to, and
    /// a card's inset on top.
    fn edge_divider(&self, rect: Rect, sizes: &DockSizes) -> Divider {
        let (least, most) = sizes.iconic_bounds(self.column, self.spec);
        let inset = self.card.map_or(0.0, |place| sizes.inset(place));
        let (least, most) = (least + inset, most + inset);
        let most = most.min((self.within.width() - sizes.least_pane).max(rect.width()));
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

    /// Where the parts sit in the column's `rect`: inside its card, if it shows as one.
    fn plan(&self, rect: Rect, sizes: &IconicSizes, rtl: bool) -> Plan {
        let rect = card_rect(rect, self.card, &sizes.column, self.edge, rtl);
        let bar = Rect::from_min_max(rect.min, Pos2::new(rect.right(), (rect.top() + self.bar.height(&sizes.column)).min(rect.bottom())));
        let body = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom()), rect.max);
        let wide = body.width() >= sizes.labels;
        let (mut y, mut cells, mut names, mut grips, mut groups) = (body.top(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (at, group) in self.column.groups.iter().enumerate() {
            let above = y;
            if at > 0 {
                y += sizes.separator;
            }
            let top = y;
            y = top + sizes.grip;
            // The grip strip, or a band that reaches into the first icon and covers the room above the group.
            let band = if sizes.band > 0.0 { above..=y + sizes.band } else { top..=y };
            for panel in 0..group.panels.len() {
                let row = Rect::from_min_size(Pos2::new(body.left(), y), Vec2::new(body.width(), sizes.cell));
                if wide {
                    let (icon, name) = split_row(row, sizes.cell, rtl);
                    cells.push((at, panel, icon));
                    names.push(name);
                } else {
                    cells.push((at, panel, row));
                }
                y += sizes.cell;
            }
            grips.push((at, Rect::from_x_y_ranges(body.x_range(), *band.start()..=band.end().min(y))));
            groups.push((at, Rect::from_min_max(Pos2::new(body.left(), top), Pos2::new(body.right(), y))));
        }
        Plan { bar, cells, names, grips, groups, body }
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

    /// The flyout of the cell `at` beside `column`, inside `bounds`, on the screen side `right` of the column: its top
    /// is the top of its group's icons shifted by the group's saved offset and kept inside the bounds, its width the
    /// group's flyout width else `expanded`, its height the group's own else what its panels ask for, and neither is
    /// less than `least`.
    fn flyout_plan(
        &self,
        at: usize,
        column: Rect,
        bounds: Rect,
        right: bool,
        (expanded, least): (f32, Vec2),
        sizes: &DockSizes,
    ) -> Option<FlyoutPlan<P>> {
        let (group_at, _) = *self.flat().get(at)?;
        let icons = self.laid.as_ref()?.cells.iter().find(|(group, _, _)| *group == group_at)?.2.top();
        let group = self.flyout_group(at)?;
        let width = group.flyout_width.filter(|width| width.is_finite() && *width > 0.0).unwrap_or(expanded).max(least.x).min(bounds.width());
        let fixed = group.height.is_some_and(|height| height.is_finite() && height > 0.0);
        let height = self.flyout_height(&group, sizes).max(least.y).min(bounds.height());
        let offset = group.flyout_offset.filter(|offset| offset.is_finite()).unwrap_or(0.0);
        let top = (icons + offset).min(bounds.bottom() - height).max(bounds.top());
        let left = if right { column.right() } else { column.left() - width };
        Some(FlyoutPlan { group, top_left: Pos2::new(left, top), size: Vec2::new(width, height), fixed, icons_top: icons })
    }
}

/// What measures and shows the group of an open flyout.
type FlyoutView<'a, P> = Box<dyn FnMut(&Group<P>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a>;

impl<P: Value + Hash + Debug + 'static> Component for IconicColumn<'_, P> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = IconicColumnStyle;
    type Output = IconicColumnOutput<P>;

    /// The column's edge line lies centred on the edge, so half of it is outside the column.
    fn paint_outset(tokens: &crate::style::UiTokens) -> f32 {
        tokens.stroke.thick / 2.0
    }

    fn style(cx: &StyleCx<'_>, _: IconicColumnStyleKey<P>) -> IconicColumnStyle {
        let colors = &cx.tokens.color;
        IconicColumnStyle {
            metrics: Metrics::new(),
            sizes: IconicSizes::style(cx),
            name: cx.color(colors.text.normal),
            name_open: cx.color(colors.accent.text),
            name_gap: cx.tokens.space.gap.item,
            // The references' grip: a row of upright ticks, in the theme's grip ink.
            grip: GripMarks::style(cx),
            separator: cx.env::<IconGroupSeparator>(),
            rule_inset: cx.tokens.space.gap.inline,
            card: card_style(cx),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<P>>> {
        Some(self.open.reborrow())
    }

    /// The icon cells, each toggling the flyout of its panel, and the grip strip of every group, which takes exactly its
    /// rect so that it never reaches into the group's first icon.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = cx.style.sizes;
        let plan = self.plan(rect, &sizes, cx.flow.is_rtl());
        self.bar_rect = plan.bar;
        if self.bar == ColumnBar::Own {
            let (chevron, _) = bar_slots(plan.bar, self.edge, cx.flow.is_rtl());
            let name = ColumnArrow::Collapse.name(&cx.words::<DockWords>(), self.expanded());
            let _ = cx.header_part(chevron_edge(self.edge), CHEVRON, chevron).sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(name);
        }
        for (at, (_, _, cell)) in plan.cells.iter().enumerate() {
            let Some(panel) = self.flat().get(at).and_then(|(group, panel)| self.panel(*group, *panel)) else { continue };
            let label = (self.spec)(panel).label;
            // A rail's icons expand and collapse the column, which `update` reads from their activation.
            let rail = self.mode == IconicMode::Rail;
            let part = cx.part(CELL.at(at as u64), *cell).sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS).focusable().tooltip(label);
            if !rail {
                part.toggles_at("flyout", at).on_click();
            }
            if let Some(name) = plan.names.get(at) {
                let part = cx.part(NAME.at(at as u64), *name).sense(Sense::CLICK | Sense::DRAG).keyboard_via(CELL);
                if !rail {
                    part.toggles_at("flyout", at).on_click();
                }
            }
        }
        for (group, grip) in &plan.grips {
            cx.part(GRIP.at(*group as u64), *grip)
                .sense(Sense::DRAG)
                .exact_hit()
                // A band over the first icon's top takes the press there, as production's, drawn after the icons, did.
                .stacking(u32::from(sizes.band > 0.0))
                .cursor(CursorIcon::Grab)
                .drag_cursor(CursorIcon::Grabbing)
                .pointer_only("Move Panel To in the panel menu moves the group's panels");
        }
        if self.bar == ColumnBar::Own {
            divider::declare(cx, rect, EDGE, 0, &self.edge_divider(rect, &sizes.column));
        }
        self.laid = Some(plan);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let sizes = cx.style.sizes;
        let (rect, rtl, ctx) = (cx.rect, cx.flow.is_rtl(), cx.core.ctx.clone());
        self.expand = self.bar == ColumnBar::Own && cx.part_activated(CHEVRON).is_some();
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
                start_icon_drag(cx, DragUnit::Group(anchor.clone()), PanelKey::of(shown), extent.size(), label);
                began = true;
            }
        }
        for (at, (group, panel, cell)) in plan.cells.iter().enumerate() {
            let named = cx.part_if_declared(NAME.at(at as u64)).is_some_and(|name| dragging(name.press.phase));
            if (dragging(cx.part(CELL.at(at as u64)).press.phase) || named)
                && let Some(panel) = self.panel(*group, *panel)
            {
                let label = Label::from((self.spec)(panel).label.as_str().to_owned());
                start_icon_drag(cx, DragUnit::Panel(panel.clone()), PanelKey::of(panel), cell.size(), label);
                began = true;
            }
        }
        if self.bar == ColumnBar::Own {
            let edge = self.edge_divider(rect, &sizes.column);
            if let Some(Drag::To(coord)) = divider::pointer(cx, EDGE, 0, &edge) {
                self.move_edge(rect, &edge, coord);
            }
            if let Some((_, coord)) = divider::keys(cx, EDGE, &[(0, edge)], sizes.column.step, sizes.column.jump) {
                self.move_edge(rect, &edge, coord);
            }
        }
        if self.mode == IconicMode::Rail {
            for at in 0..flat.len() {
                if !began && (cx.part_activated(CELL.at(at as u64)).is_some() || cx.part_activated(NAME.at(at as u64)).is_some()) {
                    self.rail_click(at);
                }
            }
            self.register(&ctx, rect, rtl, &plan, &sizes);
            self.laid = Some(plan);
            return;
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
        let card = card_rect(cx.rect, self.card, &cx.style.sizes.column, self.edge, cx.flow.is_rtl());
        let bar = (self.bar == ColumnBar::Own).then_some(self.bar_rect);
        paint_card(ui, cx.rect, card, &cx.style.card, bar, self.tab_look);
        if self.bar == ColumnBar::Own {
            paint_chevron(ui, self.edge, ColumnArrow::Collapse, self.expanded(), cx.flow.is_rtl(), &cx.words::<DockWords>());
        }
        let flat = self.flat();
        for (at, &cell) in flat.iter().enumerate() {
            let Some(panel) = self.panel(cell.0, cell.1) else { continue };
            let spec = (self.spec)(panel);
            let icon: IconSpec<'static> = spec.icon.unwrap_or_else(|| IconSpec::initial(spec.label.as_str()));
            let selected = self.shows_selected(at, cell);
            if let Some(name) = self.laid.as_ref().and_then(|plan| plan.names.get(at)) {
                let style = &cx.style;
                let room = if cx.flow.is_rtl() { name.with_max_x(name.max.x - style.name_gap) } else { name.with_min_x(name.min.x + style.name_gap) };
                let ink = if selected { style.name_open } else { style.name };
                let _ = ui.draw().text(room, &TextRun::new(spec.label.as_str().to_owned()).color(ink).elide());
            }
            // A cell bound to the panel's toggle command shows that command's marker and hint; the column's `CELL`
            // part keeps the press, as the face only resolves the command.
            let face = match spec.command {
                Some(command) => IconButton::command(icon, command),
                None => IconButton::new(icon),
            };
            // A cell is a rail button: the tool rail's size and glyphs, so a collapsed column's icons match the tools'.
            let cell = face.label(spec.label).selected(selected).size(ControlSize::Md).glyphs(GlyphRole::ToolRail);
            ui.face(CELL.at(at as u64), cell);
        }
        let style = &cx.style;
        if let Some(plan) = self.laid.as_ref() {
            match style.separator {
                IconGroupSeparator::Grip => plan.grips.iter().for_each(|(_, strip)| style.grip.paint_in(ui, *strip)),
                IconGroupSeparator::Rule => {
                    let xs = plan.body.x_range().shrink(style.rule_inset);
                    // In the middle of the room from a group's last icon to the next group's first.
                    for pair in plan.groups.windows(2) {
                        let y = (pair[0].1.bottom() + pair[1].1.top()) / 2.0;
                        ui.draw().rule(RuleKind::Edge, Pos2::new(xs.min, y), Pos2::new(xs.max, y));
                    }
                }
            }
        }
        if self.framed {
            ui.draw().surface(cx.rect, SurfaceKind::Frame, Corners::None);
        }
        // The edge's line lies on the column's inner edge; its handle stays inside the column. A floating window's
        // edges are the window's.
        if self.bar == ColumnBar::Own {
            let (rect, edge) = (cx.rect, self.edge_divider(cx.rect, &cx.style.sizes.column));
            let x = if cx.flow.is_rtl() { rect.right() - (edge.at - rect.left()) } else { edge.at };
            ui.face(EDGE.at(0), Splitter::new(Axis::X).boundary(x));
        }
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
        if self.bar == ColumnBar::Own {
            divider::describe(access, EDGE, 0, &self.edge_divider(cx.rect, &sizes.column), words.resize.as_str(), sizes.column.step);
            describe_chevron(access, &ColumnArrow::Collapse.name(&words, self.expanded()));
        }
    }

    /// The flyout of every cell whose popup is open, beside the column on the side it opens on. A resize the user makes
    /// comes back as the open group's new size and, when its top moved, its top's offset from the group's icons.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if self.mode == IconicMode::Rail {
            return;
        }
        let popup = cx.popup("flyout");
        let sizes = cx.style.sizes.column;
        let (opens, auto_collapse, column) = (self.opens, self.auto_collapse, cx.rect);
        // The screen side the flyouts open on, mirrored in a right-to-left layout.
        let right = (opens == Side::End) != cx.flow.is_rtl();
        let within = if self.within.is_positive() { self.within } else { crate::contract::overlay::layer_room(&cx.core.ctx) };
        // The column side of the bounds stops at the column, so a saved width wider than the room keeps that side.
        let bounds = if right { within.with_min_x(within.min.x.max(column.right())) } else { within.with_max_x(within.max.x.min(column.left())) };
        if !bounds.is_positive() {
            return;
        }
        let least = Vec2::new(sizes.least_pane, sizes.strip + sizes.least).min(bounds.size());
        let expanded = sizes.expanded_width(self.column, self.spec);
        let plans: Vec<Option<FlyoutPlan<P>>> =
            (0..self.flat().len()).map(|at| self.flyout_plan(at, column, bounds, right, (expanded, least), &sizes)).collect();
        let Some(view) = self.view.as_mut() else { return };
        let mut dismissed = false;
        let mut resized = None;
        for (at, plan) in plans.into_iter().enumerate() {
            let Some(FlyoutPlan { group, top_left, size, fixed, icons_top }) = plan else { continue };
            let shown = Flyout::new(opens, size, auto_collapse).within(bounds).least(least).docked(self.framed).show_popup(
                ui,
                popup.at(at),
                Anchor::Point(top_left),
                |ui, body, place| match place {
                    // A height the user gave the group is the flyout's: its body's measure leaves it.
                    GroupAt::Measure(_) if fixed => None,
                    _ => view(&group, place, ui, body),
                },
            );
            let Some(shown) = shown else { continue };
            dismissed |= shown.dismissed;
            if let Some(change) = shown.placed
                && let Some(panel) = group.panels.first()
            {
                // Only the dimensions the user changed are reported: the column placed the flyout at `size`'s width,
                // and at the group's height when it has one, else at the height its body took.
                let changed = |now: f32, placed: f32| ((now - placed).abs() > 0.5).then_some(now);
                let placed_height = if fixed { size.y } else { shown.rect.height() };
                let top = change.rect.top();
                let offset = (top != top_left.y).then_some(top - icons_top);
                let (width, height) = (changed(change.rect.width(), size.x), changed(change.rect.height(), placed_height));
                resized = Some(FlyoutResize { panel: panel.clone(), width, height, offset });
            }
        }
        if dismissed {
            cx.set(None);
        }
        self.flyout = resized;
    }

    fn finish(self, cx: Cx<'_, Self>) -> IconicColumnOutput<P> {
        // The chevron expands a collapsed column, and collapses the expanded one a rail stands beside.
        let iconic = self.record.iconic.or(self.expand.then_some(self.expanded())).or(self.rail_iconic);
        IconicColumnOutput {
            edit: cx.edit_response(),
            placed: cx.rect,
            iconic,
            front: self.front,
            picked: self.record.picked,
            width: self.width,
            flyout: self.flyout,
        }
    }
}

impl<P: Value + Debug + Hash + 'static> IconicColumn<'_, P> {
    /// Registers the drop spots of the column the way an expanded column registers them: the bands of a dock
    /// column's two edges (a new column beside it), the seam on the separator between two icon groups (a new group
    /// between them), the top of the first group and the bottom of the last down to the column's foot (a new group
    /// above or below), and the cells of each icon group (join its tabs). A floating column registers no edge bands:
    /// nothing goes beside it.
    fn register(&self, ctx: &egui::Context, rect: Rect, rtl: bool, plan: &Plan, sizes: &IconicSizes) {
        let anchor = |group: usize| self.column.groups.get(group).and_then(|group| group.panels.first());
        let Some(first) = anchor(0) else { return };
        if self.bar == ColumnBar::Own {
            register_edges(ctx, self.dock, first, rect, rtl, &sizes.column);
        }
        let half = sizes.column.seam_band * 0.5;
        // How far the top and bottom bands reach into a group's cells: a quarter of a short group leaves its middle
        // to the join.
        let reach = |group: &Rect| half.min(group.height() * 0.25);
        let mut above: Option<(&P, Rect)> = None;
        for (at, group) in &plan.groups {
            let Some(panel) = anchor(*at) else { continue };
            DockTargets::register(ctx, self.dock, DockSpot::group(panel.clone(), DockSide::Center), *group);
            if let Some((before, over)) = above {
                let middle = (over.bottom() + group.top()) * 0.5;
                let seam = Rect::from_x_y_ranges(group.x_range(), (middle - half)..=(middle + half));
                DockTargets::register(ctx, self.dock, DockSpot::seam(before.clone(), panel.clone()), seam);
            }
            above = Some((panel, *group));
        }
        if let Some((_, top)) = plan.groups.first() {
            let band = Rect::from_x_y_ranges(top.x_range(), plan.body.top()..=(top.top() + reach(top)));
            DockTargets::register_in(ctx, self.dock, DockSpot::group(first.clone(), DockSide::Top), band, *top);
        }
        if let Some((at, bottom)) = plan.groups.last()
            && let Some(last) = anchor(*at)
        {
            let band = Rect::from_x_y_ranges(bottom.x_range(), (bottom.bottom() - reach(bottom))..=plan.body.bottom().max(bottom.bottom()));
            DockTargets::register_in(ctx, self.dock, DockSpot::group(last.clone(), DockSide::Bottom), band, *bottom);
        }
    }
}

/// A row of a wide column split into the icon's square at the reading start, `side` wide, and the room for the name.
fn split_row(row: Rect, side: f32, rtl: bool) -> (Rect, Rect) {
    let side = side.min(row.width());
    if rtl {
        let icon = Rect::from_min_max(Pos2::new(row.right() - side, row.top()), row.max);
        (icon, Rect::from_min_max(row.min, Pos2::new(icon.left(), row.bottom())))
    } else {
        let icon = Rect::from_min_max(row.min, Pos2::new(row.left() + side, row.bottom()));
        (icon, Rect::from_min_max(Pos2::new(icon.right(), row.top()), row.max))
    }
}

/// The edge of the header line the chevron sits at: the bar's outer end, the window edge the dock is on.
fn chevron_edge(edge: DockEdge) -> HeaderEdge {
    if edge == DockEdge::End { HeaderEdge::End } else { HeaderEdge::Start }
}

impl<P: Value + Hash + Debug + 'static> Container for IconicColumn<'_, P> {
    /// The chevron's square at the bar's outer end; nothing in a floating window, whose title row is the bar.
    fn header_parts(&self, cx: &MeasureCx<'_, Self>) -> HeaderParts {
        if self.bar == ColumnBar::Window {
            return HeaderParts::NONE;
        }
        let side = cx.style.sizes.column.bar;
        if chevron_edge(self.edge) == HeaderEdge::End { HeaderParts::NONE.end(side) } else { HeaderParts::NONE.start(side) }
    }

    /// The column bar, over the icons. What the user did on it is read in `finish`.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if self.bar == ColumnBar::Window {
            return;
        }
        let (column, spec, auto_collapse) = (self.column, self.spec, self.auto_collapse);
        let band = bar_band(ui, self.bar_rect, self.edge);
        self.record = show_bar(ui, band, self.expanded(), auto_collapse, ColumnArrow::Collapse, true, column_grip(column, spec, cx));
    }
}
