//! `Env` and scopes, app values and `content_env`.

mod folder;
mod label;
mod shell;
mod units;

pub use label::{Label, Message, Word};
#[doc(hidden)]
pub use shell::ShellEntry;
pub use shell::{BodyMode, RevealEdge, ShellCx, ShellRegions, SlotKey, WindowKey};
pub use units::{LengthUnits, PointSize, UnitKind, UnitWords, Units};

use crate::runtime::commands::CommandChain;
use crate::runtime::egui::memory::FastMap;
use crate::runtime::egui::{pass, repaint, scale, stack, style};
use crate::runtime::frame::PerViewport;
use crate::runtime::windows;
use crate::style::{EnvKey, Ink, SurfaceKind, UiTokens};
use craft_behavior::{CanvasTransform, MotionMode};
use craft_host::Host;
use craft_host::target::{Os, ShellInfo};
pub use craft_i18n::Direction;
pub use craft_i18n::{AgeStyle, Count, Counted, WordRows, WordSet, WordTemplate, Words};
use craft_i18n::{CalendarDate, DateFormat, Format};
use craft_theme::ControlSize;
pub use craft_ui_macros::EnvValue;
use egui::{Context, Id, Ui, ViewportId};
use std::ops::Deref;
use std::sync::Arc;

mod app_values;
mod host_words;
mod overrides;
mod root;
mod tokens;
mod values;
mod view;

pub use app_values::{AppValues, EnvValue, StyleValue};
use host_words::{host_dates, host_format, host_words};
pub use overrides::{Overrides, ThemeScope, WordsOverride, with_env};
pub use root::{ResolvedRoot, RootEnv};
pub(crate) use tokens::TokenInputs;
#[cfg(feature = "testing")]
pub(crate) use tokens::root_token_entries;
use tokens::tokens_for;
pub use values::{ArenaId, MotionPolicy, Scale};
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

/// The values of an env.
#[derive(Clone)]
#[non_exhaustive]
pub struct Scoped {
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
    /// The text and icon ink of the surface the scope sits on, set by a container for its body.
    pub ink: Ink,
    /// The surface the scope's content sits on: the innermost container surface that paints a fill, set by that
    /// container for its body. The window's background at the root and where a layer starts.
    pub(crate) surface: SurfaceKind,
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
    /// The line height per point of text of the leading face of the interface stack, as the frame began: control
    /// heights depend on it, so the key covers it.
    #[doc(hidden)]
    pub line_ratio: f32,
    /// The key of everything above that the style cache depends on.
    #[doc(hidden)]
    pub key: EnvKey,
    /// Ambient control size; the root default is `Md`.
    pub size: ControlSize,
    /// AND of every scope and gate above.
    pub enabled: bool,
    /// OR of every scope above.
    pub read_only: bool,
    /// OR of every scope above: wheel takers inside register gated, taking the wheel only while focused or armed
    /// (a card in a scrolling overview). A layer opened from inside starts ungated.
    pub wheel_gated: bool,
    /// The flow direction: layout order, arrow keys and roving. A container whose axis keeps its order
    /// (`Props::ltr_axis`) sets it to left to right for what is inside.
    pub direction: Direction,
    /// The paragraph direction of text: the interface's, which a container that keeps its axis order leaves alone.
    /// The bidi order inside a label follows its script.
    pub text_direction: Direction,
    /// The motion mode in effect.
    pub motion: MotionPolicy,
    /// False inside the body of a component with an `Auto` id.
    pub ids_stable: bool,
    /// Root and viewport only: the pixels per point, read back once per pass.
    pub scale: Scale,
    /// The innermost arena: components shown under it join it.
    pub arena: Option<ArenaId>,
    /// The roving group of the innermost body with `ContainerProps.roving`: components shown under it put
    /// their stops in it instead of the scope's Tab order.
    pub(crate) roving: Option<Id>,
    /// From `ContainerProps.transform`.
    pub transform: Option<CanvasTransform>,
    /// The `Grid` or spread `Row` body whose cursor places the children in flow.
    pub(crate) grid: Option<Id>,
    /// The body that encloses the scope is a content-sized column: it takes its width from its widest child that is
    /// not fill-width, so a fill-width child takes the width the others settle on and never sets it.
    pub(crate) content_column: bool,
    /// The axes (horizontal, vertical) the enclosing body sizes from its content: a child that fills there takes its
    /// measured size, not the room.
    pub(crate) content: (bool, bool),
    /// The width a `Row` body was offered this pass, before its `End` or `Center` lead narrowed the block: the room
    /// its direct children share. Only the row's own children read it.
    pub(crate) row_room: Option<f32>,
    /// The line of an unwrapped `Row` body: the height of its tallest child in this pass's measuring run, or the line
    /// the body shares with its container's inline header or end region when that is taller. Its direct children
    /// placed in flow that are lines sit at the line's vertical centre, blocks and columns at its top. `None` while a
    /// row that shares no line is measured.
    pub(crate) row_line: Option<f32>,
    /// The `Stack` or `Row` body that encloses the scope and the axis it runs along, when a child that fills that
    /// axis shares the room its siblings leave (set only where the body is given the axis).
    pub(crate) fill_body: Option<(Id, craft_behavior::Axis)>,
    /// What each child of `fill_body` that fills its main axis takes along it this pass: the room less the measured
    /// extents of the other children and the gaps, over the number of children that fill. `None` while the body is
    /// measured, and without a measuring run of this pass.
    pub(crate) fill_share: Option<f32>,
    /// The shrink step of the unwrapped `Row` body that encloses the scope: its direct leaf children report their
    /// widths to it, or show in the widths it gave them.
    pub(crate) shrink: Option<Shrink>,
    /// The width a body that sizes its width from its content was offered this pass, less its padding: the room its
    /// direct children share across a column or along a row, wider than the body's own band before it measured them.
    pub(crate) offered: Option<f32>,
    /// The innermost scrolling body that encloses the scope: a collection under it owns no scroll of its own, and
    /// page and end moves a component leaves untaken go to it.
    pub(crate) scroll_owner: Option<Id>,
    /// The axes (horizontal, vertical) the innermost scrolling body scrolls on: the room along them is unbounded, so
    /// a child that fills there takes its content size.
    pub(crate) unbounded: (bool, bool),
    /// The view of the innermost scroll area, for the children that run only in view (`contract::Lazy`).
    pub(crate) lazy: Option<crate::runtime::lazy::View>,
    /// The id of the collection item the scope shows: its root is not a Tab stop (17.3).
    pub(crate) item: Option<Id>,
    /// The roving group of the collection row the scope shows: the stops of its components and parts join it, so
    /// Tab leaves the collection and Forward enters the row (17.5).
    pub(crate) row: Option<Id>,
    /// What the collection's item step recorded for the item it shows this pass.
    pub(crate) item_facts: Option<ItemFacts>,
    /// The container instances above that receive scoped commands.
    pub(crate) commands: CommandChain,
    pub(crate) host: Option<Arc<Host>>,
    #[doc(hidden)]
    pub values: AppValues,
}

/// The shrink step of an unwrapped `Row` body, as its direct leaf children shown in flow see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shrink {
    /// The first sweep of a measuring run of the row: each leaf reports its widths to the row (`layout::report_shrink`).
    Count(Id),
    /// Each leaf shows at most the width the measuring run of the row gave it (`layout::shrink_cap`).
    Apply(Id),
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
}

impl Scoped {
    /// The height, icon side, padding and gap of a control of `size`, with the line ratio of this scope.
    pub(crate) fn control(&self, size: craft_theme::ControlSize) -> craft_behavior::ControlMetrics {
        crate::style::control_metrics(&self.tokens, size, self.line_ratio)
    }

    /// Ends the layout room of the enclosing body: its flow grid and content column.
    pub(crate) fn start_room(&mut self) {
        self.grid = None;
        self.content_column = false;
        self.content = (false, false);
        self.row_room = None;
        self.row_line = None;
        self.fill_body = None;
        self.fill_share = None;
        self.shrink = None;
        self.offered = None;
    }

    /// The facts the collection recorded for the item `id`.
    pub(crate) fn facts_of(&self, id: Id) -> Option<&ItemFacts> {
        self.item_facts.as_ref().filter(|facts| facts.item == id)
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
        let inputs = TokenInputs::of(root, resolved);
        let tokens = tokens_for(ctx, &inputs);
        let motion = MotionPolicy::new(motion_mode(ctx, resolved.motion), tokens.tokens().motion);
        let os = windows::os_of(ctx);
        let scale = Scale(pixels_per_point);
        let line_ratio = crate::fonts::stack_facts(ctx).line_ratio;
        let shell = Arc::new(windows::shell_of(ctx, pass::current_viewport(ctx)));
        let active = shell.window.active;
        let keyboard = Arc::new(crate::runtime::keys::layout_of(ctx));
        Env {
            scoped: Arc::new(Scoped {
                key: EnvKey::new(tokens.key(), root.direction, scale, os, active, root.values.style_hash()).with_line(line_ratio),
                line_ratio,
                tokens,
                token_inputs: Arc::new(inputs),
                os,
                active,
                shell,
                keyboard,
                ink: Ink::default(),
                surface: SurfaceKind::Background,
                size: ControlSize::Md,
                enabled: true,
                read_only: false,
                wheel_gated: false,
                direction: root.direction,
                text_direction: root.direction,
                motion,
                ids_stable: true,
                scale,
                arena: None,
                roving: None,
                transform: None,
                grid: None,
                content_column: false,
                content: (false, false),
                row_room: None,
                row_line: None,
                fill_body: None,
                fill_share: None,
                shrink: None,
                offered: None,
                scroll_owner: None,
                unbounded: (false, false),
                lazy: None,
                item: None,
                row: None,
                item_facts: None,
                commands: CommandChain::default(),
                words: host_words(ctx, host.as_ref()),
                format: host_format(ctx, host.as_ref()),
                dates: host_dates(ctx, host.as_ref()),
                today: root.today,
                tab_toggles_chrome: root.tab_toggles_chrome,
                host,
                values: root.values.clone(),
            }),
        }
    }

    /// The env of `ui`: the nearest entry up the `Ui` stack, else the running viewport's root env, so the `Ui`
    /// of an egui `Area` or `Window` reads the app's root values. For app panels (layer 8) and caller closures only.
    ///
    /// While apps still use `add_enabled_ui`, it also ANDs `ui.is_enabled()`.
    pub fn of(ui: &Ui) -> Env {
        let ctx = ui.ctx();
        let viewport = pass::current_viewport(ctx);
        let found = PerViewport::<ViewportEnvs>::with(ctx, viewport, |envs| {
            stack::nearest(ui, |node| envs.entries.get(&node).cloned()).or_else(|| envs.root.clone())
        });
        let env = found.unwrap_or_else(|| Env::from_root(ctx, &RootEnv::fallback(ctx), None, scale::pixels_per_point(ctx)));
        if style::is_enabled(ui) { env } else { env.derive_local(|s| s.enabled = false) }
    }

    /// The running viewport's root env: what code with no `Ui` (the ghost layer) paints with.
    #[doc(hidden)]
    pub fn of_root(ctx: &Context) -> Env {
        let viewport = pass::current_viewport(ctx);
        let found = PerViewport::<ViewportEnvs>::with(ctx, viewport, |envs| envs.root.clone());
        found.unwrap_or_else(|| Env::from_root(ctx, &RootEnv::fallback(ctx), None, scale::pixels_per_point(ctx)))
    }

    /// The `Host` of the running viewport's root env.
    pub(crate) fn host_of(ctx: &Context) -> Option<Arc<Host>> {
        PerViewport::<ViewportEnvs>::with(ctx, pass::current_viewport(ctx), |envs| envs.root.as_ref().and_then(|env| env.host.clone()))
    }

    /// A clone of the innermost app value, else `T::root()`. Large values hold an `Arc`.
    pub fn get<T: EnvValue>(&self) -> T {
        self.values.get::<T>()
    }

    /// The inherited group only; layer-local fields start fresh where the capture is installed.
    pub fn capture(&self) -> EnvCapture {
        EnvCapture(self.derive(|s| {
            s.start_room();
            s.arena = None;
            s.roving = None;
            s.transform = None;
            s.scroll_owner = None;
            s.wheel_gated = false;
            s.unbounded = (false, false);
            s.lazy = None;
            s.item = None;
            s.row = None;
            s.item_facts = None;
            s.ink = Ink::default();
            s.surface = SurfaceKind::Background;
        }))
    }

    /// What component code sees of this env.
    pub fn view(&self) -> EnvView<'_> {
        EnvView { scoped: &self.scoped }
    }

    /// A child entry: the one way an env changes.
    pub(crate) fn derive(&self, f: impl FnOnce(&mut Scoped)) -> Env {
        let mut scoped = (*self.scoped).clone();
        f(&mut scoped);
        scoped.key = EnvKey::new(scoped.tokens.key(), scoped.direction, scoped.scale, scoped.os, scoped.active, scoped.values.style_hash())
            .with_line(scoped.line_ratio);
        Env { scoped: Arc::new(scoped) }
    }

    /// A child entry that changes layer-local fields only (arena, item, row, roving, grid, scroll owner,
    /// transform, ink): the key carries over, because none of its inputs change.
    /// This env for a body that starts a layout room of its own (a layer, a pane): the flow grid and the content
    /// column of the room around it do not reach in.
    pub(crate) fn new_room(&self) -> Env {
        self.derive_local(|scoped| {
            scoped.start_room();
            scoped.lazy = None;
        })
    }

    pub(crate) fn derive_local(&self, f: impl FnOnce(&mut Scoped)) -> Env {
        let mut scoped = (*self.scoped).clone();
        f(&mut scoped);
        Env { scoped: Arc::new(scoped) }
    }

    /// This env's container scope on top of `root`, the root env of the viewport the scope now runs in: the scale,
    /// active state, window facts and OS are the viewport's, everything else stays.
    pub(crate) fn rebase(&self, root: &Env) -> Env {
        self.derive(|s| {
            s.scale = root.scale;
            s.active = root.active;
            s.shell = root.shell.clone();
            s.os = root.os;
        })
    }

    /// Installs `env` for `ui` and everything below it; `restore` puts back what it replaced.
    pub(crate) fn install_for(ui: &Ui, env: Env) -> Installed {
        let viewport = pass::current_viewport(ui.ctx());
        let node = stack::node_id(ui);
        let previous = PerViewport::<ViewportEnvs>::with(ui.ctx(), viewport, |envs| envs.entries.insert(node, env));
        Installed { viewport, node, previous }
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
struct ViewportEnvs {
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
