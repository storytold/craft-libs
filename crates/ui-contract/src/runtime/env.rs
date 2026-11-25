//! `Env` and scopes, app values and `content_env`.

mod folder;
mod label;
mod shell;
mod units;

pub use label::{Label, Message, Word};
#[doc(hidden)]
pub use shell::ShellEntry;
pub use shell::{BodyMode, PageBandRoom, RevealEdge, ShellCx, ShellRegions, SlotKey, WindowKey};
pub use units::{ColumnSize, LengthUnits, PointSize, UnitKind, UnitWords, Units};

use crate::runtime::commands::CommandChain;
use crate::runtime::egui::memory::{self, FastMap};
use crate::runtime::egui::{pass, repaint, scale, stack, style};
use crate::runtime::frame::PerViewport;
use crate::runtime::windows;
use crate::style::{EnvKey, GlyphRole, Ink, SurfaceKind, UiTokens};
use craft_behavior::{CanvasTransform, MotionMode};
use craft_host::target::{Os, ShellInfo};
use craft_host::{Host, Jobs};
pub use craft_i18n::Direction;
pub use craft_i18n::{AgeStyle, Count, Counted, WordRows, WordSet, WordTemplate, Words};
use craft_i18n::{CalendarDate, DateFormat, Format};
use craft_theme::ControlSize;
pub use craft_ui_macros::EnvValue;
use egui::{Context, Id, Ui, ViewportId};
use std::ops::Deref;
use std::sync::Arc;

/// A value a higher layer defines and the env only carries: the layer that defines it reads it back by its type.
pub(crate) type Held = Arc<dyn std::any::Any + Send + Sync>;

mod app_values;
mod host_words;
mod overrides;
mod root;
mod scrub;
pub(crate) mod system;
pub(crate) mod tokens;
mod values;
mod view;

pub use app_values::{AppValues, EnvValue, StyleValue};
pub use craft_platform::appearance::SystemReader;
use host_words::{host_dates, host_format, host_words};
pub use overrides::{Overrides, ThemeScope, WheelScope, WordsOverride, in_room, with_env};
pub use root::{ResolvedRoot, RootEnv};
pub use scrub::ScrubDrags;
pub use system::{SystemState, system_appearance, system_text_scale};
pub(crate) use tokens::TokenInputs;
#[cfg(feature = "testing")]
pub(crate) use tokens::root_token_resolves;
use tokens::tokens_for;
pub use values::{AltCodePages, ArenaId, ColumnAlign, InBar, MotionPolicy, PaneEdge, RowPressed, Scale, WheelSpeed};
pub use view::EnvView;

/// The inherited context of a `Ui`: one `Arc` clone per read.
#[derive(Clone)]
pub struct Env {
    scoped: Arc<Scoped>,
}

impl Deref for Env {
    type Target = Scoped;

    fn deref(&self) -> &Scoped {
        &self.scoped
    }
}

/// A container's footer that its body places (`FooterPlace::Body`): the container and the size the footer measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FooterSlotOf {
    pub owner: Id,
    pub size: egui::Vec2,
}

/// The flow facts of the body around a memoized view that its content is laid out by: whether that body is a
/// content-sized or a given column, the axes it sizes from its content, the axis it shares among the children that fill
/// it, whether the view runs in a least measure, the
/// axes a scrolling body leaves unbounded, whether it fills a beside label's cell and whether its leaves fit the view's
/// room (`Scoped::view_fits`). A measuring run of a container or a row lays the view out under other facts than its
/// real run, so they are a fact of the view's place, which each kind of run compares on its own (`lazy::Place`,
/// `lazy::Measured`), not of its key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flow {
    content_column: bool,
    given_column: bool,
    content: (bool, bool),
    shares: Option<craft_behavior::Axis>,
    least: bool,
    unbounded: (bool, bool),
    cell_fills: bool,
    fits: bool,
    egui_cell: bool,
}

/// The values of an env.
#[derive(Clone)]
#[non_exhaustive]
pub struct Scoped {
    /// What a scope rarely changes, shared by the scopes below until one changes it (`inherited_mut`).
    inherited: Arc<Inherited>,
    /// The text and icon ink of the surface the scope sits on, set by a container for its body.
    pub ink: Ink,
    /// The surface the scope's content sits on: the innermost container surface that paints a fill, set by that
    /// container for its body. The window's background at the root and where a layer starts.
    pub(crate) surface: SurfaceKind,
    /// The key of everything above that the style cache depends on.
    #[doc(hidden)]
    pub key: EnvKey,
    /// The key of what a memoized view shown in the scope inherits beyond `key` (`scope_key`), once a view asked.
    scope_key: std::sync::OnceLock<u128>,
    /// Ambient control size; the root default is `Md`.
    pub size: ControlSize,
    /// How the icon-only buttons of the scope show their glyph; the root default and a layer's is `Control`.
    pub glyphs: GlyphRole,
    /// AND of every scope and gate above.
    pub enabled: bool,
    /// OR of every scope above.
    pub read_only: bool,
    /// How wheel takers inside take the wheel (`WheelScope`). A layer opened from inside starts plain.
    pub wheel: WheelScope,
    /// The flow direction: layout order, arrow keys and roving. A container whose axis keeps its order
    /// (`Props::ltr_axis`) sets it to left to right for what is inside.
    pub direction: Direction,
    /// The paragraph direction of text: the interface's, which a container that keeps its axis order leaves alone.
    /// The bidi order inside a label follows its script.
    pub text_direction: Direction,
    /// False inside the body of a component with an `Auto` id.
    pub ids_stable: bool,
    /// The innermost arena: components shown under it join it.
    pub arena: Option<ArenaId>,
    /// The roving group of the innermost body with `ContainerProps.roving`: components shown under it put
    /// their stops in it instead of the scope's Tab order.
    pub(crate) roving: Option<Id>,
    /// From `ContainerProps.transform`.
    pub transform: Option<CanvasTransform>,
    /// The `Grid` or spread `Row` body whose cursor places the children in flow.
    pub(crate) grid: Option<Id>,
    /// The `Grid` body of `grid` sets its cells at the top of their row (`CellAlign::Top`), not in its middle.
    pub(crate) grid_top: bool,
    /// The body that encloses the scope is a content-sized column: it takes its width from its widest child that is
    /// not fill-width, so a fill-width child takes the width the others settle on and never sets it. Such a column
    /// exists only where the width comes from content: under a content-sized mount, a row or a grid.
    pub(crate) content_column: bool,
    /// The body that encloses the scope is a `Stack` given its width: a content-sized container in it passes that
    /// width on to its own body, so a child there that fills takes it (fill propagates down the column).
    pub(crate) given_column: bool,
    /// The axes (horizontal, vertical) the enclosing body sizes from its content: a child that fills there takes its
    /// measured size, not the room.
    pub(crate) content: (bool, bool),
    /// The width a `Row` body was offered this pass, before its `End` or `Center` lead narrowed the block: the room
    /// its direct children share. Only the row's own children read it.
    pub(crate) row_room: Option<f32>,
    /// How far the ranked units of the first `Row` body below give way before its other drops: the row drops them
    /// until the kept units fit in its ideal width less this, an end region that joins its body's fit
    /// (`EndRegion::fit`) giving up the body's shortfall. That row's children do not read it.
    pub(crate) rank_short: Option<f32>,
    /// The line of the unwrapped `Row` body whose direct children the scope places in flow (`RowLine`).
    pub(crate) row_line: Option<RowLine>,
    /// Where the first child the scope places in flow reports its first line: a container's header or body
    /// (`layout::FirstLines`). `None` in the children of a `Row` body, which reports its line itself.
    pub(crate) first_line: Option<Id>,
    /// The body whose flow places the scope's direct children: the flow start a child reads (`layout::flow_start`)
    /// and the one a child placed in flow closes.
    pub(crate) flow_body: Option<Id>,
    /// The `Stack` body that places the scope's direct children in flow and the gap it leaves between them; it holds
    /// only while that body is `flow_body`.
    pub(crate) stack_gap: Option<(Id, f32)>,
    /// A rule precedes the start of the body this scope opens: its container draws one under its header, or draws
    /// nothing of its own and itself follows one. The body's layout opens its flow start with it.
    pub(crate) after_rule: bool,
    /// The body is an unwrapped `Row` placed from its start whose container is a row of a vertical flow: its first
    /// child in flow starts where the rows of that flow start (`label::in_column`).
    pub(crate) column_row: bool,
    /// The layout of the bracketed `Row` body (`ContainerProps::bracket`) whose labelled rows report their controls to
    /// it: set for the row's body and kept through the containers inside it up to a labelled one.
    pub(crate) bracket: Option<Id>,
    /// The container whose footer stands in its body (`FooterPlace::Body`) and the size that footer measured: a
    /// `FooterSlot` shown under it takes that size and is where the footer is drawn.
    pub(crate) footer_slot: Option<FooterSlotOf>,
    /// The `Stack` or `Row` body that encloses the scope and the axis it runs along, when a child that fills that
    /// axis shares the room its siblings leave (set only where the body is given the axis).
    pub(crate) fill_body: Option<(Id, craft_behavior::Axis)>,
    /// What each child of `fill_body` that fills its main axis takes along it this pass: the room less the measured
    /// extents of the other children and the gaps, over the number of children that fill. `None` while the body is
    /// measured, and without a measuring run of this pass.
    pub(crate) fill_share: Option<f32>,
    /// The memoized view whose content the scope's direct children run in, with the axis the body around that view
    /// shares (its `fill_body`'s): a child given that axis to fill tells the view (`layout::fill_size`), which then is
    /// a child that fills that body (`contract::lazy`).
    pub(crate) fill_view: Option<(Id, craft_behavior::Axis)>,
    /// The shrink step of the unwrapped `Row` or `Stack` body that encloses the scope, as its direct children in flow
    /// take part in it.
    pub(crate) shrink: Option<Shrink>,
    /// The sweep of the content-sized `Stack` column that encloses the scope, as its direct children in flow see it:
    /// a held fill-width child reports the width it measured at, and after the column's second sweep measures and
    /// shows at the width the column settled on (`ColumnSweep`).
    pub(crate) column: Option<ColumnSweep>,
    /// The scope runs in the least measure of a body (`Shrink::Least`): every leaf in flow takes its least width. It
    /// holds through the bodies nested inside.
    pub(crate) least: bool,
    /// The width a body that sizes its width from its content was offered this pass, less its padding: the room its
    /// direct children share across a column or along a row, wider than the body's own band before it measured them.
    pub(crate) offered: Option<f32>,
    /// The line length of paragraphs: the wrap width (`ContainerProps::wrap_width`) of the innermost container that
    /// declares one and takes its width from its content. A wrapping text in a wider room breaks its lines there. The
    /// bodies nested inside keep it; a layer or a pane starts without one.
    pub(crate) measure: Option<f32>,
    /// The innermost scrolling body that encloses the scope: a collection under it owns no scroll of its own, and
    /// page and end moves a component leaves untaken go to it.
    pub(crate) scroll_owner: Option<ScrollOwner>,
    /// The enclosing region's fallback collection for unused moves.
    pub(crate) fallback_collection: Option<Id>,
    /// The axes (horizontal, vertical) the innermost scrolling body scrolls on: the room along them is unbounded, so
    /// a child that fills there takes its content size, except the children that fill the main axis of a body that
    /// shares it: they share what the view shows, at least their content size (`layout::view_room`).
    pub(crate) unbounded: (bool, bool),
    /// The view of the innermost scroll area, for the children that run only in view (`contract::Lazy`).
    pub(crate) lazy: Option<crate::runtime::lazy::View>,
    /// The id of the collection item the scope shows: its root is not a Tab stop (17.3).
    pub(crate) item: Option<Id>,
    /// The roving group of the collection row the scope shows: the stops of its components and parts join it, so
    /// Tab leaves the collection and Forward enters the row (17.5).
    pub(crate) row: Option<Id>,
    /// What the collection's item step recorded for the item it shows this pass.
    pub(crate) item_facts: Option<Arc<ItemFacts>>,
    /// The inside label of the side label whose control the scope shows (`FieldLayout::Inside`). Only that control
    /// sees it: a component's children and a container's regions start without one.
    pub(crate) inside_label: Option<Held>,
    /// The scope is the body of a beside label in an inspector (`LabelScheme::Inspector`): the control shown there
    /// fills the width the label leaves. Only that control sees it, as with `inside_label`.
    pub(crate) cell_fills: bool,
    /// The scope lays out in a cell of an `egui::Grid` that egui code made below the body around it: the grid places
    /// each child from its cell's left edge in any direction, and gives the column the width the child takes from the
    /// next frame. `Env::of` sets it where the stack between a `Ui` and its entry holds a grid's `Ui`; a scope that
    /// passes `Env::of` on (`with_env`, `Lazy`, a memoized view) keeps it, and a body (`start_room`) or a component's
    /// children (`child_env`) end it.
    pub(crate) egui_cell: bool,
}

/// The values of an env that a scope rarely changes: the resolved tokens, the platform, the language and the app's
/// values. `Scoped` reads them through `Deref`.
#[derive(Clone)]
#[non_exhaustive]
pub struct Inherited {
    /// The resolved tokens of the scope; read in `style` only.
    #[doc(hidden)]
    pub tokens: Arc<UiTokens>,
    /// What the tokens were resolved from (the light or dark mode among them): the app's, until a scope that
    /// resolves its tokens in another mode sets them.
    pub(crate) token_inputs: Arc<TokenInputs>,
    /// The operating system.
    #[doc(hidden)]
    pub os: Os,
    /// Whether the window is active.
    #[doc(hidden)]
    pub active: bool,
    /// The platform and the running viewport's window facts.
    pub(crate) shell: Arc<ShellInfo>,
    /// The keyboard layout as the platform reports it and typing has shown it.
    pub(crate) keyboard: Arc<craft_keymap::Layout>,
    /// The built-in text of the scope.
    #[doc(hidden)]
    pub words: Arc<Words>,
    /// How the scope writes numbers: the host's catalog format, else the system region's.
    pub(crate) format: Arc<Format>,
    /// How the scope writes dates: the host's catalog format, else the system region's.
    pub(crate) dates: Arc<DateFormat>,
    /// Today's date as the host supplied it.
    pub(crate) today: Option<CalendarDate>,
    /// `RootEnv::tab_toggles_chrome`.
    pub(crate) tab_toggles_chrome: bool,
    /// `RootEnv::escape_leaves_fields`.
    pub(crate) escape_leaves_fields: bool,
    /// `RootEnv::shortcuts_repeat`: the key stage reads only it, so no view's key covers it.
    pub(crate) shortcuts_repeat: bool,
    /// `RootEnv::cursor_follows_zoom`: painted cursor images follow the interface zoom.
    pub(crate) cursor_follows_zoom: bool,
    /// `RootEnv::tooltips`: tooltips show.
    pub(crate) tooltips: bool,
    /// `RootEnv::canvas_pattern`: the canvas surround shows the theme's pattern.
    #[doc(hidden)]
    pub canvas_pattern: bool,
    /// The line height per point of text of the leading face of the interface stack, as the frame began: control
    /// heights depend on it, so the key covers it.
    #[doc(hidden)]
    pub line_ratio: f32,
    /// The motion mode in effect.
    pub motion: MotionPolicy,
    /// Root and viewport only: the pixels per point, read back once per pass.
    pub scale: Scale,
    /// The container instances above that receive scoped commands.
    pub(crate) commands: CommandChain,
    pub(crate) host: Option<Arc<Host>>,
    #[doc(hidden)]
    pub values: AppValues,
    /// The context the env was built in: the accessors of tracked inputs note their reads into its records.
    pub(crate) ctx: Context,
}

impl Deref for Scoped {
    type Target = Inherited;

    fn deref(&self) -> &Inherited {
        &self.inherited
    }
}

/// The shrink step of an unwrapped `Row` or a `Stack` body, as its direct children shown in flow see it. In a row
/// each leaf, and each body that does not fill the row, is a unit the row narrows to fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shrink {
    /// The first sweep of a measuring run of the row: each unit reports its widths to the row (`layout::report_shrink`),
    /// a levelled unit its width at each level (`layout::levelled_unit`).
    Count(Id),
    /// The sweep of a short row that asks each body unit for its least width (`layout::report_least`): it measures its
    /// content unbounded along x, every leaf at its least, and leaves no record.
    Least(Id),
    /// Each unit shows at most the width the measuring run of the row gave it (`layout::shrink_cap`), a levelled unit
    /// at the level the row gave it.
    Apply(Id),
    /// A `Stack` body or a wrapping `Row`: each leaf shows at most the room (a whole line), never below its least.
    Fit,
}

/// The sweep of a content-sized `Stack` column, as its direct children shown in flow see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ColumnSweep {
    /// The first sweep of the measuring run of the column `Id`: each held fill-width child reports the width it
    /// measured at (`layout::report_held`).
    Count(Id),
    /// The width the content-sized `Stack` column `Id` settled on, in its second sweep and in the real run after one:
    /// a held fill-width child measures and shows across it, and a held picture measures in it.
    Column(Id, f32),
}

/// The line of an unwrapped `Row` body, as its direct children placed in flow see it: a child of one line sits with
/// its centre on the line's centre, and a child of more than one line, or with no line of its own, at its top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowLine {
    /// The row body, which the children of a measuring run report their lines to.
    pub(crate) body: Id,
    /// How far below the row's top the line's centre lies: the lowest centre among its one-line children in this
    /// pass's measuring run, or that of the line the body shares with its container's inline header or end region
    /// when lower. `None` while a row that shares no line is measured.
    pub(crate) centre: Option<f32>,
    /// The row's height once its children sit on the line, at least the shared line's. `None` with `centre`.
    pub(crate) height: Option<f32>,
    /// Whether this pass's measuring run of the row settled the line (the row's own record): only then does a
    /// container that fills its height take the line (`line_height`). While the row is measured, the line it shares
    /// gives `centre` and `height` but is not settled.
    pub(crate) settled: bool,
    /// Whether the children placed in flow under the line take their slots on it and report their lines to the row:
    /// the row's direct children. The content of a levelled unit stands on the line only to fill it (`fill_size`,
    /// `line_height`).
    pub(crate) places: bool,
}

/// What a collection records for one of its items in a pass: the row flags the item folds into its state (step 12)
/// and its place in the set (step 16). The collection owns the record; a component reads it from its env.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct ItemFacts {
    pub(crate) item: Id,
    #[doc(hidden)]
    pub flags: crate::style::Flags,
    pub(crate) position: crate::runtime::access::SetPosition,
    /// The keys of the item's cells in display order, so a cell reports the position of its column.
    pub(crate) columns: Vec<craft_behavior::ColumnKey>,
    /// The collection's toggle columns, shown or not: a row's label never takes one of them.
    #[doc(hidden)]
    pub toggles: Vec<craft_behavior::ColumnKey>,
    /// The keys of the toggle columns that have one (`ToggleColumn::key`), which their cells show in the tooltip.
    #[doc(hidden)]
    pub toggle_keys: Vec<(craft_behavior::ColumnKey, craft_keymap::Chord)>,
    /// Each column's alignment and whether its figures are tabular, from its `ColumnSpec`: a row places its cells'
    /// text by them.
    #[doc(hidden)]
    pub aligns: Vec<(craft_behavior::ColumnKey, ColumnAlign, bool)>,
    /// Where the collection holds a press on the row (`RowState::pressed`).
    #[doc(hidden)]
    pub pressed: Option<RowPressed>,
    /// The row's menu is open (`RowState::menu_open`).
    #[doc(hidden)]
    pub menu_open: bool,
}

/// The innermost body that owns the scroll of a scope (`Scoped::scroll_owner`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScrollOwner {
    pub(crate) id: Id,
    /// The owner shows its body through a view of its own (a canvas with a view, `ContainerProps::scroll_owner`),
    /// not through a scroll region: a stop in the body is on a document, with its key role.
    pub(crate) view: bool,
}

impl Scoped {
    /// How far from a handle, point or edge a press still takes it, in screen points: the theme's `size.hit.target`.
    pub fn hit_reach(&self) -> f32 {
        self.tokens.size.hit.target
    }

    /// Whether the scope is the body of a container with a view of its own, as the innermost scroll owner: its stops
    /// take the document key role (`KeyRole::document`).
    pub(crate) fn in_view_body(&self) -> bool {
        self.scroll_owner.is_some_and(|owner| owner.view)
    }

    /// The height, icon side, padding, gap and field widths of a control of `size`, with the line ratio of this scope.
    pub(crate) fn control(&self, size: craft_theme::ControlSize) -> craft_behavior::ControlMetrics {
        crate::style::control_metrics(&self.tokens, size, self.line_ratio)
    }

    /// Whether the enclosing body gives a child that fills the width its width: it does not size from its content and
    /// is no content column. Elsewhere such a child measures from its content and the column holds it at the width
    /// the column settles on.
    pub(crate) const fn gives_width(&self) -> bool {
        !self.content.0 && !self.content_column
    }

    /// Starts a layer from its opener's scope: the inherited group stays, the layer-local fields start fresh, and the
    /// layer is no line of a bar it was opened from. A layer's content measures and runs in this.
    ///
    /// The layer is outside its opener's flow: an inside label and a filling cell belong to the beside label's own
    /// control, and the opener's least measure is not the layer's. A tooltip of a control in an inspector cell keeps
    /// its text width, not the layer's cap.
    pub(crate) fn start_layer(&mut self) {
        self.start_room();
        self.inside_label = None;
        self.cell_fills = false;
        self.least = false;
        self.bracket = None;
        self.measure = None;
        self.arena = None;
        self.roving = None;
        self.transform = None;
        self.scroll_owner = None;
        self.fallback_collection = None;
        self.wheel = WheelScope::Plain;
        self.unbounded = (false, false);
        self.lazy = None;
        self.glyphs = GlyphRole::Control;
        self.item = None;
        self.row = None;
        self.item_facts = None;
        self.ink = Ink::default();
        self.surface = SurfaceKind::Background;
        self.footer_slot = None;
        self.inherited_mut().values.set(InBar(false));
    }

    /// Ends the layout room of the enclosing body: its flow grid and content column. The axes the body sizes from its
    /// content stay (`content`): like `cell_fills` and `least`, they say what a child that fills takes there, its
    /// measured size or the room, and the place of a memoized view holds them (`flow`). `has_room` checks the same set,
    /// and `Env::of` ends it for a `Ui` below the one an entry was installed for.
    pub(crate) fn start_room(&mut self) {
        self.grid = None;
        self.grid_top = false;
        self.content_column = false;
        self.given_column = false;
        self.row_room = None;
        self.row_line = None;
        self.first_line = None;
        self.flow_body = None;
        self.column_row = false;
        self.fill_body = None;
        self.fill_share = None;
        self.fill_view = None;
        self.shrink = None;
        self.column = None;
        self.offered = None;
        self.egui_cell = false;
    }

    /// Whether any of the facts `start_room` ends is set, besides `egui_cell`, which `Env::of` sets.
    fn has_room(&self) -> bool {
        self.grid.is_some()
            || self.grid_top
            || self.content_column
            || self.given_column
            || self.row_room.is_some()
            || self.row_line.is_some()
            || self.first_line.is_some()
            || self.flow_body.is_some()
            || self.column_row
            || self.fill_body.is_some()
            || self.fill_share.is_some()
            || self.fill_view.is_some()
            || self.shrink.is_some()
            || self.column.is_some()
            || self.offered.is_some()
    }

    /// Starts the layout room of a memoized view's content (`start_room`), keeping whether the enclosing body is a
    /// content-sized or a given column and whether the view lies in an egui grid's cell, as `start_room` keeps the
    /// content axes. The view's content then measures and
    /// fills as it would shown without the view: a child that fills takes its measured size where the body sizes from
    /// content, never the open room. Where a leaf shown here would fit its room (`view_fits`), the leaves of the view's
    /// content fit the room the view is given (`Shrink::Fit`).
    ///
    /// In a body that shares an axis among the children that fill it (`fill_body`), a direct child of the content
    /// given that axis to fill tells the view `view` (`fill_view`): the view is then one of the children that fill the
    /// body, given its share as its room, as the child shown without the view would be. A view that is a unit of a
    /// shrinking `Row` that does not share its width sizes from its content along x, as the row counted it: a child
    /// that fills there takes its measured width, not the row's flow region a view the row does not narrow is shown in.
    pub(crate) fn start_view_room(&mut self, view: Id) {
        let fits = self.view_fits();
        let row_unit = matches!(self.shrink, Some(Shrink::Count(_) | Shrink::Least(_) | Shrink::Apply(_)));
        let shares = self.shares();
        let (content_column, given_column, egui_cell) = (self.content_column, self.given_column, self.egui_cell);
        // The view stands on the line of the row around it only to fill it, as a levelled unit does: a fill-width row in
        // it (a bar in a shell slot) takes the line's height and centres on it, as shown without the view; its children
        // take no slots on the line (photocraft#16405).
        let line = self.row_line.map(|line| RowLine { places: false, ..line });
        self.start_room();
        (self.content_column, self.given_column, self.egui_cell) = (content_column, given_column, egui_cell);
        self.row_line = line;
        self.fill_view = shares.map(|axis| (view, axis));
        if fits {
            self.shrink = Some(Shrink::Fit);
        }
        if row_unit && shares != Some(craft_behavior::Axis::X) {
            self.content.0 = true;
        }
    }

    /// The axis the enclosing body shares among its direct children that fill it (`fill_body`).
    pub(crate) const fn shares(&self) -> Option<craft_behavior::Axis> {
        match self.fill_body {
            Some((_, axis)) => Some(axis),
            None => None,
        }
    }

    /// Whether a memoized view shown in flow here fits its content to its room, as a leaf here would: in a `Stack` body
    /// and a wrapping `Row`, whose leaves show at most its room (a whole line), and as one of the units a shrinking
    /// `Row` narrows. In a `Grid` cell or a `Ui` with no body around it a leaf keeps its measured width, and so does the
    /// view's content.
    pub(crate) const fn view_fits(&self) -> bool {
        matches!(self.shrink, Some(Shrink::Count(_) | Shrink::Least(_) | Shrink::Apply(_) | Shrink::Fit))
    }

    /// The key of everything a memoized view shown in this scope inherits beyond `key` and `flow` (the style inputs and
    /// the flow facts, which its place compares): the scope's state (enabled, read-only, size, glyphs, ink, surface,
    /// wheel, directions), the containers around it (arena, roving group, transform, collection item and row, inside
    /// label, footer slot, line length, scroll owner), the command chain, motion, the host's formats and date, and every
    /// app value. Computed once per scope, on the first view that asks; the bytes it fed are counted then.
    pub(crate) fn scope_key(&self) -> u128 {
        *self.scope_key.get_or_init(|| {
            use crate::contract::{MemoKey, MemoProps as _};
            let mut key = MemoKey::of_view(&self.ctx);
            key.feed_hash(&(self.enabled, self.read_only, self.size, self.glyphs, self.ink, self.surface, self.wheel));
            key.feed_hash(&(self.direction, self.text_direction, self.ids_stable));
            key.feed_hash(&(self.arena, self.roving, self.item, self.row, self.scroll_owner.map(|owner| (owner.id, owner.view))));
            self.transform.map(|transform| (transform.scale, transform.offset)).key(&mut key);
            self.footer_slot.map(|slot| (slot.owner, slot.size)).key(&mut key);
            self.measure.key(&mut key);
            // State a key cannot follow, rebuilt each pass: a view under it runs in every pass.
            if self.item_facts.is_some() || self.inside_label.is_some() {
                key.changes_every_pass();
            }
            for id in self.commands.ids() {
                id.key(&mut key);
            }
            key.feed_hash(&(
                std::mem::discriminant(&self.motion.mode),
                self.canvas_pattern,
                self.tab_toggles_chrome,
                self.escape_leaves_fields,
                self.today,
            ));
            (&self.format, &self.dates, &self.host).key(&mut key);
            let value_bytes = self.values.key(&mut key);
            #[cfg(feature = "testing")]
            crate::runtime::measure::count_memo_key_bytes(&self.ctx, key.bytes() + value_bytes);
            #[cfg(not(feature = "testing"))]
            let _ = value_bytes;
            key.finish()
        })
    }

    /// The flow facts of the enclosing body a memoized view shown in this scope is laid out by (`Flow`).
    pub(crate) const fn flow(&self) -> Flow {
        Flow {
            content_column: self.content_column,
            given_column: self.given_column,
            content: self.content,
            shares: self.shares(),
            least: self.least,
            unbounded: self.unbounded,
            cell_fills: self.cell_fills,
            fits: self.view_fits(),
            egui_cell: self.egui_cell,
        }
    }

    /// The facts the collection recorded for the item `id`.
    pub(crate) fn facts_of(&self, id: Id) -> Option<&ItemFacts> {
        self.item_facts.as_deref().filter(|facts| facts.item == id)
    }

    /// The inherited values, to change: shared with no other scope from here on.
    pub(crate) fn inherited_mut(&mut self) -> &mut Inherited {
        Arc::make_mut(&mut self.inherited)
    }

    /// Adds `values` over the scope's app values.
    pub(crate) fn extend_values(&mut self, values: &AppValues) {
        if !values.is_empty() {
            self.inherited_mut().values.extend(values);
        }
    }
}

/// The inherited group of an env, captured to open a popup or viewport: the layer-local fields are reset.
#[derive(Clone)]
pub struct EnvCapture(Env);

impl EnvCapture {
    /// The env a new egui layer or viewport starts with.
    pub(crate) fn env(&self) -> Env {
        self.0.clone()
    }
}

impl Env {
    /// The word set `L` in the scope's language.
    pub fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.words.set::<L>()
    }

    /// How the scope writes numbers: locale digits, separators and grouping.
    pub fn format(&self) -> &Format {
        &self.format
    }

    /// How the scope writes dates: the host's catalog format, else the system region's.
    pub fn dates(&self) -> Arc<DateFormat> {
        Arc::clone(&self.dates)
    }

    /// Today's date as the host supplied it with `RootEnv::today`; `None` when it gave none.
    pub fn today(&self) -> Option<CalendarDate> {
        self.today
    }

    /// The scope's number format and unit words, to read and write numbers with their unit.
    pub fn units(&self) -> Units {
        Units::new(self.format.clone(), self.words::<UnitWords>(), self.words.clone(), self.values.get::<PointSize>())
    }

    /// How long ago `seconds` was, in the scope's language and digits: "Just now", "3 minutes ago", "Yesterday".
    pub fn relative_age(&self, seconds: u64, style: AgeStyle) -> String {
        craft_i18n::relative_age(&self.words, &self.format, seconds, style)
    }

    /// The root env of a frame: the app's values with the pixels per point read back from egui.
    pub(crate) fn from_root(ctx: &Context, root: &RootEnv, host: Option<Arc<Host>>, pixels_per_point: f32) -> Env {
        let resolved = root.resolved(ctx);
        // An id that is not a valid `AppId` names no theme pins, so it resolves as no app.
        let app = host.as_ref().and_then(|host| craft_theme::AppId::parse(host.app().id).ok());
        let inputs = TokenInputs::of(root, resolved, app);
        let tokens = tokens_for(ctx, &inputs);
        let motion = MotionPolicy::new(motion_mode(ctx, resolved.motion), tokens.tokens().motion);
        let os = windows::os_of(ctx);
        let scale = Scale(pixels_per_point);
        let line_ratio = crate::fonts::stack_facts(ctx).line_ratio;
        let shell = Arc::new(windows::shell_of(ctx, pass::current_viewport(ctx)));
        let active = shell.window.active;
        let keyboard = Arc::new(crate::runtime::keys::layout_of(ctx));
        // The window facts, the safe rect and the layout are tracked inputs: a view that read them runs when they change.
        let safe = crate::runtime::egui::areas::safe_rect_raw(ctx);
        crate::runtime::inputs::observe(ctx, crate::runtime::inputs::Input::Shell, ((*shell).clone(), safe));
        crate::runtime::inputs::observe(ctx, crate::runtime::inputs::Input::Keyboard, (*keyboard).clone());
        let words = host_words(ctx, host.as_ref());
        let key = EnvKey::new(tokens.key(), root.direction, scale, os, active, root.values.style_hash())
            .with_line(line_ratio)
            .with_words(words.generation());
        Env {
            scoped: Arc::new(Scoped {
                inherited: Arc::new(Inherited {
                    line_ratio,
                    tokens,
                    token_inputs: Arc::new(inputs),
                    os,
                    active,
                    shell,
                    keyboard,
                    motion,
                    scale,
                    commands: CommandChain::default(),
                    words,
                    format: host_format(ctx, host.as_ref()),
                    dates: host_dates(ctx, host.as_ref()),
                    today: root.today,
                    tab_toggles_chrome: root.tab_toggles_chrome,
                    escape_leaves_fields: root.escape_leaves_fields,
                    shortcuts_repeat: root.shortcuts_repeat,
                    cursor_follows_zoom: root.cursor_follows_zoom,
                    tooltips: root.tooltips,
                    canvas_pattern: root.canvas_pattern,
                    host,
                    values: root.values.clone(),
                    ctx: ctx.clone(),
                }),
                key,
                scope_key: std::sync::OnceLock::new(),
                ink: Ink::default(),
                surface: SurfaceKind::Background,
                size: ControlSize::Md,
                glyphs: GlyphRole::Control,
                enabled: true,
                read_only: false,
                wheel: WheelScope::Plain,
                direction: root.direction,
                text_direction: root.direction,
                ids_stable: true,
                arena: None,
                roving: None,
                transform: None,
                grid: None,
                grid_top: false,
                content_column: false,
                given_column: false,
                content: (false, false),
                row_room: None,
                rank_short: None,
                row_line: None,
                first_line: None,
                flow_body: None,
                stack_gap: None,
                after_rule: false,
                column_row: false,
                bracket: None,
                footer_slot: None,
                fill_body: None,
                fill_share: None,
                fill_view: None,
                shrink: None,
                column: None,
                least: false,
                offered: None,
                measure: None,
                scroll_owner: None,
                fallback_collection: None,
                unbounded: (false, false),
                lazy: None,
                item: None,
                row: None,
                item_facts: None,
                inside_label: None,
                cell_fills: false,
                egui_cell: false,
            }),
        }
    }

    /// The env of `ui`: the nearest entry up the `Ui` stack, else the running viewport's root env, so the `Ui`
    /// of an egui `Area` or `Window` reads the app's root values. For app panels (layer 8) and caller closures only.
    ///
    /// What a body sets up for the children it places in flow (`Scoped::start_room`: its `Grid` or spread row's cells,
    /// its unwrapped row's line and shrink step, its flow start, its content column) is for its direct children, which
    /// it shows in the `Ui` it installed its env for. A `Ui` egui code makes below that one (an `egui::Grid`,
    /// `ui.horizontal`, `ui.push_id` in a caller's closure) lays out its own children: an entry found for an ancestor
    /// of `ui` comes without those facts, so a component there takes no cell, line or shrink step of the body around it.
    /// Where that stack holds an `egui::Grid`'s `Ui`, the env says the component lays out in the grid's cell
    /// (`Scoped::egui_cell`).
    ///
    /// While apps still use `add_enabled_ui`, it also ANDs `ui.is_enabled()`, outside sizing passes.
    pub fn of(ui: &Ui) -> Env {
        let ctx = ui.ctx();
        let viewport = pass::current_viewport(ctx);
        // The root env stands for an entry above the whole stack.
        let (found, grid) = PerViewport::<ViewportEnvs>::with(ctx, viewport, |envs| {
            let stack::Found { value, grid } = stack::nearest(ui, |node| envs.entries.get(&node).cloned());
            (value.or_else(|| envs.root.clone().map(|env| (env, false))), grid)
        });
        let (env, own) = found.unwrap_or_else(|| (Env::from_root(ctx, &RootEnv::fallback(ctx), None, scale::pixels_per_point(ctx)), false));
        let egui_cell = env.egui_cell || grid;
        let env = if own || !(env.scoped.has_room() || egui_cell != env.egui_cell) {
            env
        } else {
            env.derive_local(|scoped| {
                scoped.start_room();
                scoped.egui_cell = egui_cell;
            })
        };
        // egui's `invisible()` also disables the `Ui`, and a sizing `Ui` is invisible, not disabled: in a sizing pass
        // the env keeps the enabled state the real run has, so a memoized view keys alike in both.
        if style::is_enabled(ui) || ui.is_sizing_pass() { env } else { env.derive_local(|s| s.enabled = false) }
    }

    /// The running viewport's root env: what code with no `Ui` (the ghost layer) paints with.
    #[doc(hidden)]
    pub fn of_root(ctx: &Context) -> Env {
        let viewport = pass::current_viewport(ctx);
        let found = PerViewport::<ViewportEnvs>::with(ctx, viewport, |envs| envs.root.clone());
        found.unwrap_or_else(|| Env::from_root(ctx, &RootEnv::fallback(ctx), None, scale::pixels_per_point(ctx)))
    }

    /// How far from a handle, point or edge a press still takes it, in screen points: the theme's `size.hit.target` as
    /// the running viewport's root env has it. `ToolContext::reach` reads the same
    /// value; code with no tool context (a menu capture hit-testing a canvas point) reads it here.
    pub fn root_hit_reach(ctx: &Context) -> f32 {
        Self::of_root(ctx).hit_reach()
    }

    /// The `Host` of the running viewport's root env.
    pub(crate) fn host_of(ctx: &Context) -> Option<Arc<Host>> {
        PerViewport::<ViewportEnvs>::with(ctx, pass::current_viewport(ctx), |envs| envs.root.as_ref().and_then(|env| env.host.clone()))
    }

    /// Where `ctx`'s background work runs: the host's jobs, else the jobs `install` was given, else
    /// `craft_host::default_jobs()`.
    pub(crate) fn jobs_of(ctx: &Context) -> Arc<dyn Jobs> {
        if let Some(host) = Self::host_of(ctx) {
            return Arc::clone(host.jobs_shared());
        }
        memory::with::<InstalledJobs, _>(ctx, |installed| installed.0.clone()).unwrap_or_else(craft_host::default_jobs)
    }

    /// A clone of the innermost app value, else `T::root()`. Large values hold an `Arc`.
    pub fn get<T: EnvValue>(&self) -> T {
        self.values.get::<T>()
    }

    /// The inherited group only; layer-local fields start fresh where the capture is installed.
    pub fn capture(&self) -> EnvCapture {
        EnvCapture(self.derive(Scoped::start_layer))
    }

    /// What component code sees of this env.
    pub fn view(&self) -> EnvView<'_> {
        EnvView { scoped: &self.scoped }
    }

    /// A child entry: the one way an env changes.
    pub(crate) fn derive(&self, f: impl FnOnce(&mut Scoped)) -> Env {
        let mut scoped = (*self.scoped).clone();
        f(&mut scoped);
        scoped.scope_key = std::sync::OnceLock::new();
        scoped.key = EnvKey::new(scoped.tokens.key(), scoped.direction, scoped.scale, scoped.os, scoped.active, scoped.values.style_hash())
            .with_line(scoped.line_ratio)
            .with_words(scoped.words.generation());
        Env { scoped: Arc::new(scoped) }
    }

    /// This env for a body that starts a layout room of its own (a layer, a pane): the flow grid and the content
    /// column of the room around it do not reach in. When opened inside a bar, it leaves the bar (`InBar(false)`) and
    /// recomputes the env key so styles do not reuse the bar's cache entries.
    pub(crate) fn new_room(&self) -> Env {
        let reset = |scoped: &mut Scoped| {
            scoped.start_room();
            scoped.bracket = None;
            scoped.lazy = None;
            scoped.measure = None;
        };
        if self.get::<InBar>().0 {
            self.derive(|scoped| {
                reset(scoped);
                // A pane opened from a bar is not the bar's line.
                scoped.inherited_mut().values.set(InBar(false));
            })
        } else {
            self.derive_local(reset)
        }
    }

    /// A child entry that changes layer-local fields only (arena, item, row, roving, grid, scroll owner,
    /// transform, ink): the key carries over, because none of its inputs change.
    pub(crate) fn derive_local(&self, f: impl FnOnce(&mut Scoped)) -> Env {
        let mut scoped = (*self.scoped).clone();
        f(&mut scoped);
        scoped.scope_key = std::sync::OnceLock::new();
        Env { scoped: Arc::new(scoped) }
    }

    /// Reuse a sweep's scope while it is uniquely held. A captured child env keeps its snapshot. The scope key is
    /// computed again by the first view that asks after the edit.
    pub(crate) fn local_mut(&mut self) -> &mut Scoped {
        let scoped = Arc::make_mut(&mut self.scoped);
        scoped.scope_key = std::sync::OnceLock::new();
        scoped
    }

    /// This env's container scope on top of `root`, the root env of the viewport the scope now runs in: the scale,
    /// active state, window facts and OS are the viewport's, everything else stays.
    pub(crate) fn rebase(&self, root: &Env) -> Env {
        self.derive(|s| {
            let inherited = s.inherited_mut();
            inherited.scale = root.scale;
            inherited.active = root.active;
            inherited.shell = root.shell.clone();
            inherited.os = root.os;
        })
    }

    /// Installs `env` for `ui` and everything below it; `restore` puts back what it replaced.
    pub(crate) fn install_for(ui: &Ui, env: Env) -> Installed {
        let viewport = pass::current_viewport(ui.ctx());
        let node = stack::node_id(ui);
        let previous = PerViewport::<ViewportEnvs>::with(ui.ctx(), viewport, |envs| envs.entries.insert(node, env));
        Installed { viewport, node, previous }
    }

    /// Runs `body` in `ui` with `env` installed for it, and puts back what the install replaced, also when `body`
    /// panics. A body runs its direct children this way in a `Ui` it makes for them.
    pub(crate) fn run_in<R>(ui: &mut Ui, env: Env, body: impl FnOnce(&mut Ui) -> R) -> R {
        let ctx = ui.ctx().clone();
        let _restore = Restore { ctx: &ctx, installed: Some(Env::install_for(ui, env)) };
        body(ui)
    }

    /// Puts back the entry `install_for` replaced.
    pub(crate) fn restore(ctx: &Context, installed: Installed) {
        PerViewport::<ViewportEnvs>::with(ctx, installed.viewport, |envs| match installed.previous {
            Some(previous) => {
                envs.entries.insert(installed.node, previous);
            }
            None => {
                envs.entries.remove(&installed.node);
            }
        });
    }

    /// The root env of the running viewport, once its begin hook ran.
    pub(crate) fn root_of(ctx: &Context) -> Option<Env> {
        PerViewport::<ViewportEnvs>::with(ctx, ctx.viewport_id(), |envs| envs.root.clone())
    }

    /// The root env `viewport`'s last begin hook installed, for the input hook, which runs before the pass.
    pub(crate) fn root_for(ctx: &Context, viewport: ViewportId) -> Option<Env> {
        PerViewport::<ViewportEnvs>::with(ctx, viewport, |envs| envs.root.clone())
    }

    /// Records `env` as the root env of `viewport`, which `Env::of` falls back to for a `Ui` with no entry above it.
    pub(crate) fn set_root(ctx: &Context, viewport: ViewportId, env: Env) {
        PerViewport::<ViewportEnvs>::with(ctx, viewport, |envs| envs.root = Some(env));
    }
}

/// One viewport's env entries by the `Ui` they are installed for, and its root env. The lock is held only to
/// install or look up an entry or clone an `Arc`.
#[derive(Default)]
pub(crate) struct ViewportEnvs {
    entries: FastMap<Id, Env>,
    root: Option<Env>,
}

/// What `install_for` replaced.
pub(crate) struct Installed {
    viewport: ViewportId,
    node: Id,
    previous: Option<Env>,
}

/// Restores the replaced entry when dropped, so a panic in a scope's body leaves no entry behind.
struct Restore<'a> {
    ctx: &'a Context,
    installed: Option<Installed>,
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        if let Some(installed) = self.installed.take() {
            Env::restore(self.ctx, installed);
        }
    }
}

/// The mode the root runs animations at: the app's, and off while egui's animation time is zero.
fn motion_mode(ctx: &Context, motion: MotionMode) -> MotionMode {
    if repaint::animation_time(ctx) > 0.0 { motion } else { motion.lowered_by(MotionMode::Off) }
}

/// The jobs `install` was given, for a context with no host.
#[derive(Default)]
struct InstalledJobs(Option<Arc<dyn Jobs>>);

/// Keeps `jobs` as where `ctx`'s background work runs while no host is installed; `None` leaves
/// `craft_host::default_jobs()`.
pub(crate) fn install_jobs(ctx: &Context, jobs: Option<Arc<dyn Jobs>>) {
    memory::with::<InstalledJobs, _>(ctx, |installed| installed.0 = jobs);
}
