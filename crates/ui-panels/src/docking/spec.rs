//! `PanelSpec`: what the dock asks of a panel before it shows its body.

use super::DockSizes;
use crate::contract::SizeLimits;
use crate::primitives::IconSpec;
use crate::rails::RailDensity;
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use craft_host::target::WindowKind;
use craft_theme::Srgba;
use egui::Vec2;

/// Body heights a panel asks for in a dock column while the user has not resized its group. Like [`SizeLimits`]
/// they exclude the tab strip, and a group takes the largest of each among its panels. The dock picks the group that
/// fills a column: its last open group.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockHeights {
    /// The height the group gets when the column has room.
    pub default: f32,
    /// The height it gives way down to, so the group that fills the column keeps its `fill`.
    pub compact: f32,
    /// The height it asks for when it is the column's last open group, which fills the column: groups at their
    /// default shrink towards `compact` before it gets less.
    pub fill: f32,
}

impl DockHeights {
    /// A `default` height that does not give way (`compact` is `default`) and asks for no more than its minimum when
    /// it fills the column (`fill` is 0).
    pub const fn new(default: f32) -> Self {
        DockHeights { default, compact: default, fill: 0.0 }
    }

    /// The height the group gives way down to.
    #[must_use]
    pub const fn compact(mut self, compact: f32) -> Self {
        self.compact = compact;
        self
    }

    /// The height the group asks for when it fills the column.
    #[must_use]
    pub const fn fill(mut self, fill: f32) -> Self {
        self.fill = fill;
        self
    }

    /// Whether every height is finite, non-negative and at most [`SizeLimits::BOUND`].
    pub fn valid(self) -> bool {
        [self.default, self.compact, self.fill].iter().all(|height| height.is_finite() && (0.0..=SizeLimits::BOUND).contains(height))
    }
}

/// How a panel that lays its content in a grid of equal columns (a toolbar) sizes its dock column. The column is a
/// whole number of content columns wide, its steps ([`Column::steps`](craft_behavior::dock::Column::steps)), and as
/// tall as the content measures at that width: docked from the top of the dock, floating under the title row. A drag
/// of the column's edge sets whole steps; the content lays out as many columns as fit the width it gets.
///
/// A tool rail's grid is [`DockGrid::rail`]: the dock computes its widths from its own style
/// ([`resolved`](Self::resolved)), so the grid follows the theme and density without the panel's spec reading them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockGrid {
    widths: GridWidths,
    /// The most columns the content fills, its cell count: more steps show it no wider.
    pub most: u16,
}

/// The widths of a [`DockGrid`]'s columns.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GridWidths {
    /// `first`: the content's width at one column, a cell and the content's own padding on both sides; `step`: the
    /// width each further column adds, a cell and the gap before it.
    Content { first: f32, step: f32 },
    /// A tool rail's cells at a density, sized by the dock's style.
    Rail(RailDensity),
}

impl DockGrid {
    /// A grid `first` wide at one column, `step` wider for each further column, up to `most` columns.
    pub const fn new(first: f32, step: f32, most: u16) -> Self {
        DockGrid { widths: GridWidths::Content { first, step }, most }
    }

    /// The grid of a tool rail of `cells` cells: a rail button and the control padding on both sides at one column,
    /// a button and the tight gap for each further column, in the style the dock shows it in.
    pub fn rail(cells: usize) -> Self {
        DockGrid { widths: GridWidths::Rail(RailDensity::Regular), most: u16::try_from(cells).unwrap_or(u16::MAX).max(1) }
    }

    /// A [`rail`](Self::rail) grid whose cells are the buttons of a rail at `density`, as the rail shows them
    /// ([`ToolRail::density`](crate::rails::ToolRail::density)); a content grid is left as it is.
    #[must_use]
    pub fn density(mut self, density: RailDensity) -> Self {
        if let GridWidths::Rail(at) = &mut self.widths {
            *at = density;
        }
        self
    }

    /// The grid with its widths in points: a [`rail`](Self::rail) grid takes the rail widths of `sizes`.
    pub fn resolved(self, sizes: &DockSizes) -> Self {
        match self.widths {
            GridWidths::Content { .. } => self,
            GridWidths::Rail(RailDensity::Regular) => DockGrid::new(sizes.rail_first, sizes.rail_step, self.most),
            GridWidths::Rail(RailDensity::Dense) => DockGrid::new(sizes.dense_rail_first, sizes.dense_rail_step, self.most),
        }
    }

    /// The content's width at one column and the width each further column adds; zero for a rail grid not yet
    /// [`resolved`](Self::resolved).
    fn first_step(self) -> (f32, f32) {
        match self.widths {
            GridWidths::Content { first, step } => (first, step),
            GridWidths::Rail(_) => (0.0, 0.0),
        }
    }

    /// Whether the widths are finite and positive and the grid has a column. A rail grid not yet
    /// [`resolved`](Self::resolved) is not.
    pub fn valid(self) -> bool {
        let (first, step) = self.first_step();
        first.is_finite() && first > 0.0 && step.is_finite() && step > 0.0 && self.most > 0
    }

    /// `steps` kept between one column and [`most`](Self::most).
    pub fn clamp(self, steps: u16) -> u16 {
        steps.clamp(1, self.most.max(1))
    }

    /// The content's width at `steps` columns, kept within the grid's columns.
    pub fn width(self, steps: u16) -> f32 {
        let (first, step) = self.first_step();
        first + f32::from(self.clamp(steps) - 1) * step
    }

    /// The most columns whose width fits `width`, at least one.
    pub fn steps_within(self, width: f32) -> u16 {
        (1..=self.most.max(1)).rev().find(|steps| self.width(*steps) <= width + WIDTH_SLACK).unwrap_or(1)
    }

    /// The fewest columns at which `height`, the content's height at a width ([`width`](Self::width)), fits `room`;
    /// the most columns when none fits.
    pub fn steps_for_height(self, room: f32, mut height: impl FnMut(f32) -> f32) -> u16 {
        let most = self.most.max(1);
        (1..=most).find(|steps| height(self.width(*steps)) <= room + WIDTH_SLACK).unwrap_or(most)
    }
}

/// How far a width or height may pass the room and still fit it: the rounding of laid-out sizes.
const WIDTH_SLACK: f32 = 0.5;

/// A document a panel shows (a composition in a Timeline): a tab of the panel's group after the group's panel tabs,
/// whichever panel the group shows. Picking the tab shows the panel and reports
/// [`DockEvent::DocumentPicked`](super::DockEvent::DocumentPicked); its close button and a middle click report
/// [`DockEvent::DocumentClosed`](super::DockEvent::DocumentClosed). The host opens, shows and closes documents by editing
/// the panel's spec; the dock never changes them. A document tab is not dragged and takes no tab drop.
#[derive(Clone, Debug, PartialEq, crate::contract::MemoProps)]
#[non_exhaustive]
pub struct PanelDocument {
    /// The document's id, unique among the panel's documents.
    pub id: u64,
    /// The tab's text.
    pub label: Label,
    /// The data colour shown before the label ([`PanelSpec::swatch`]).
    pub swatch: Option<Option<Srgba>>,
    /// The tab shows a close button, where the group's tabs close.
    pub closable: bool,
    /// The panel shows this document: its tab is the selected tab while the panel is the one its group shows.
    pub shown: bool,
}

impl PanelDocument {
    /// The document `id` titled `label`: no swatch, not closable, not shown.
    pub fn new(id: u64, label: impl Into<Label>) -> Self {
        PanelDocument { id, label: label.into(), swatch: None, closable: false, shown: false }
    }

    /// The data colour before the label; `None` is the theme's empty swatch.
    #[must_use]
    pub fn swatch(mut self, color: Option<Srgba>) -> Self {
        self.swatch = Some(color);
        self
    }

    /// Whether the tab shows a close button.
    #[must_use]
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Whether the panel shows this document.
    #[must_use]
    pub fn shown(mut self, shown: bool) -> Self {
        self.shown = shown;
        self
    }
}

/// What the dock reads of a panel: its name and icon, its size policy and how its floating window behaves. The
/// permissions of a panel (movable, floatable, closable, takes tabs, takes splits) belong to the dock model, not
/// to this spec.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PanelSpec {
    /// The panel's name: its tab text, its menu rows' target and the accessible name of its handles.
    pub label: Label,
    /// The icon of the panel's iconic form and flyout cell.
    pub icon: Option<IconSpec<'static>>,
    /// Body size limits.
    pub limits: SizeLimits,
    /// How the panel's group sizes itself in a dock column until the user resizes it. `None` shares the column's
    /// free height with the column's last open group, which fills it, and with the other groups that have none.
    pub heights: Option<DockHeights>,
    /// The size the panel floats at: its width and the most height it takes. `None`: the size of the group it floats
    /// out of, else `FloatSpace`'s default size.
    pub float_size: Option<Vec2>,
    /// Whether the panel's floating window may be an OS window of its own. A panel that disallows it keeps its
    /// window in the main window.
    pub allow_native: bool,
    /// What an OS window holding the panel is for. A window holding any `WindowKind::Document` panel is a document
    /// window; the rest are tool windows.
    pub window_kind: WindowKind,
    /// The panel is a document canvas: its group shows it without the tab and side drops of a panel group, and the
    /// host draws it with `DockPanels::group_view`.
    pub canvas: bool,
    /// The grid the panel lays its content in, which makes a column with steps whole content columns wide and as
    /// tall as its content. `None` for a panel that fills the width it gets.
    pub grid: Option<DockGrid>,
    /// The panel's toggle command: an iconic cell's face is `IconButton::command` of it, so the marker and the hint
    /// follow the command.
    pub command: Option<CommandRef>,
    /// The panel has a footer pinned under its body, which `DockPanels::footer` fills: it stays in view while the
    /// body scrolls.
    pub footer: bool,
    /// A number after the panel's name on its tab, in faint type and in its accessible name ("Comments, 3"): what
    /// the panel lists.
    pub count: Option<usize>,
    /// A line of small faint text under the header row while the panel is the one its group shows ("2 replies").
    pub caption: Option<Label>,
    /// A data colour shown as a swatch before the name on the panel's tab: what the panel shows is labelled with it
    /// (a composition's label colour). `Some(None)` is the theme's empty swatch, for something without a colour; the
    /// swatch is not part of the tab's accessible name.
    pub swatch: Option<Option<Srgba>>,
    /// The documents the panel shows, as tabs of its group after the panel tabs ([`PanelDocument`]).
    pub documents: Vec<PanelDocument>,
    /// The panel's body is the app's own egui code: the dock hands `DockPanels::body`, `footer`, `group_header`,
    /// `rail_footer` and the group's `group_view` (while the panel is the one its group shows) a `Ui` with the style
    /// of the `Ui` the dock was shown in, so the code lays out as it does outside the dock, and runs them once per pass,
    /// in the pass's real run (a measuring run takes the size they last took). Other bodies lay out with
    /// the group's spacing, which framework components expect. An app moves onto the framework panel by panel by
    /// clearing it on each panel it ports.
    pub host_body: bool,
}

impl PanelSpec {
    /// A panel named `label` with no icon, no limits, no height policy and a tool window of its own when floated.
    pub fn new(label: impl Into<Label>) -> Self {
        PanelSpec {
            label: label.into(),
            icon: None,
            limits: SizeLimits::default(),
            heights: None,
            float_size: None,
            allow_native: true,
            window_kind: WindowKind::Tool,
            canvas: false,
            grid: None,
            command: None,
            footer: false,
            count: None,
            caption: None,
            swatch: None,
            documents: Vec::new(),
            host_body: false,
        }
    }

    /// Shows `n` after the panel's name on its tab.
    #[must_use]
    pub fn count(mut self, n: usize) -> Self {
        self.count = Some(n);
        self
    }

    /// Shows `text` under the header row while the panel is shown.
    #[must_use]
    pub fn caption(mut self, text: impl Into<Label>) -> Self {
        self.caption = Some(text.into());
        self
    }

    /// Shows a swatch of `color` before the panel's name on its tab; `None` is the theme's empty swatch.
    #[must_use]
    pub fn swatch(mut self, color: Option<Srgba>) -> Self {
        self.swatch = Some(color);
        self
    }

    /// The documents the panel shows, as tabs of its group after the panel tabs.
    #[must_use]
    pub fn documents(mut self, documents: Vec<PanelDocument>) -> Self {
        self.documents = documents;
        self
    }

    /// The panel has a footer pinned under its body, which `DockPanels::footer` fills.
    #[must_use]
    pub fn footer(mut self) -> Self {
        self.footer = true;
        self
    }

    /// The panel's toggle command.
    #[must_use]
    pub fn command(mut self, command: impl Into<CommandRef>) -> Self {
        self.command = Some(command.into());
        self
    }

    /// The panel's icon.
    #[must_use]
    pub fn icon(mut self, icon: IconSpec<'static>) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Body size limits.
    #[must_use]
    pub fn limits(mut self, limits: SizeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The heights the panel asks for in a dock column.
    #[must_use]
    pub fn heights(mut self, heights: DockHeights) -> Self {
        self.heights = Some(heights);
        self
    }

    /// The size the panel floats at.
    #[must_use]
    pub fn float_size(mut self, size: Vec2) -> Self {
        self.float_size = Some(size);
        self
    }

    /// Whether the floating window may be an OS window of its own.
    #[must_use]
    pub fn allow_native(mut self, allow: bool) -> Self {
        self.allow_native = allow;
        self
    }

    /// What an OS window holding the panel is for.
    #[must_use]
    pub fn window_kind(mut self, kind: WindowKind) -> Self {
        self.window_kind = kind;
        self
    }

    /// The panel is a document canvas.
    #[must_use]
    pub fn canvas(mut self) -> Self {
        self.canvas = true;
        self
    }

    /// The panel's body is the app's own egui code (`host_body`).
    #[must_use]
    pub fn host_body(mut self) -> Self {
        self.host_body = true;
        self
    }

    /// The grid the panel lays its content in.
    #[must_use]
    pub fn grid(mut self, grid: DockGrid) -> Self {
        self.grid = Some(grid);
        self
    }
}
