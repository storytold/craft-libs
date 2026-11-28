//! The frame plugin: the per-pass and per-frame steps, with the inputs the app sets and the outputs it drains.

use crate::runtime::commands::{self, CommandInput, CommandSet as _, Fired};
use crate::runtime::egui::input::{self as raw_input, Arrived, HeldInput, RawInput};
use crate::runtime::egui::{clipboard, pass, scale, style};
use crate::runtime::env::{AltCodePages, Env, RootEnv, WheelSpeed, WindowKey};
use crate::runtime::pass_state;
use crate::runtime::{access, cursor, drag, focus, ime, keys, layout_memory, measure, memory, motion, overlay, pointer, toast, wheel, windows};
use craft_behavior::{DocKey, GestureId};
use craft_host::Host;
use craft_host::target::Os;
use craft_keymap::Mods;
use craft_theme::{AppSizes, Appearance, Contrast, HitSizes, Mode, SUITE_DEFAULT, Size, TextSize, ThemeDef, builtins, resolve_sized};
use egui::{Context, Id, Ui, ViewportId};
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

/// What the app sets before each frame.
#[derive(Clone)]
#[non_exhaustive]
pub struct FrameInput {
    /// The app's root values.
    pub root: RootEnv,
    /// App-scope command state, the binding overrides and the gate, set from app state.
    pub commands: CommandInput,
    /// The app's `Host`, read through `EnvView::host`. The app keeps one `Arc<Host>` across frames and installs a
    /// new one only to switch the language or a service: the scope's words are built again for a new host, and a new
    /// language runs every memoized view again (the language is part of its place).
    pub host: Option<Arc<Host>>,
    /// The modifiers the app's own tablet or latched-key reader holds down beyond the window's input.
    pub latched: Mods,
    /// The window in which the app's own egui code shows a modal this frame (a dialog the app draws itself, not a
    /// framework layer). It blocks the app's other windows as a framework app-modal layer open in it does: their
    /// presses, keys, wheel and text go nowhere, a press there brings the modal's window forward, they refuse to close,
    /// and the modal's window takes the keyboard if one of them has it when the modal opens. A panel floated into a
    /// window of its own is one of them. `None` (the default) when the app shows no such modal; set every frame from
    /// the app's own state.
    pub app_modal: Option<ViewportId>,
    /// Whether the window system's input reaches the OS windows the runtime hosts.
    pub window_input: WindowInput,
}

/// Whether the window system's input reaches the OS windows the runtime hosts beside the root (a panel floated into
/// its own window). eframe gives the host only the root's raw input (`App::raw_input_hook`), so the host applies its
/// policy to the root there and gives it here for the rest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowInput {
    /// The window system's input reaches every hosted window.
    #[default]
    Live,
    /// An agent-driven session: a hosted window's events, hovered files and dropped files are dropped before any
    /// stage reads them, so only the runtime's injected input reaches it. Its window facts (size, focus, close
    /// requests) stay.
    Dropped,
}

impl FrameInput {
    /// A frame with `root` and no host.
    pub fn new(root: RootEnv) -> Self {
        FrameInput { root, commands: CommandInput::new(), host: None, latched: Mods::NONE, app_modal: None, window_input: WindowInput::Live }
    }

    /// Sets whether the window system's input reaches the hosted windows.
    pub fn window_input(mut self, window_input: WindowInput) -> Self {
        self.window_input = window_input;
        self
    }

    /// Sets the command input.
    pub fn commands(mut self, commands: CommandInput) -> Self {
        self.commands = commands;
        self
    }

    /// Sets the modifiers latched beyond the window's input.
    pub fn latched(mut self, mods: Mods) -> Self {
        self.latched = mods;
        self
    }

    /// Sets the window in which the app's own egui code shows a modal ([`FrameInput::app_modal`](Self::app_modal)).
    pub fn app_modal(mut self, viewport: Option<ViewportId>) -> Self {
        self.app_modal = viewport;
        self
    }

    /// Sets the host: the same `Arc<Host>` every frame, a new one to switch (see [`FrameInput::host`](Self::host)).
    pub fn host(mut self, host: Arc<Host>) -> Self {
        self.host = Some(host);
        self
    }
}

/// Which panels a window's chrome toggle hides (spec 1.8): the `window.toggle_panels` and
/// `window.toggle_panels_keep_tools` commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChromeToggle {
    /// Every panel of the window.
    Panels,
    /// Every panel of the window except the tools.
    PanelsKeepTools,
    /// Ends Tab and Shift+Tab hiding the panels; nothing when they are shown.
    ShowPanels,
    /// Reveals the edge while the window is body-only or its panels are hidden; `None` ends the reveal.
    Reveal(Option<super::env::RevealEdge>),
}

/// A step of the outer gesture of a container whose `ContainerProps::outer_edit` is on: one undo step over the edits of
/// its body's fields, from open to close. It carries no value; the fields' own events carry theirs, and a
/// `GestureHost` folds every gesture it handles while the outer one is open into it.
#[derive(Clone, Debug, PartialEq)]
pub enum OuterEdit {
    /// The container opened: the step begins, named by the container's label and document key.
    Begin {
        /// The outer gesture.
        id: GestureId,
        /// The resolved label of the container ("Image Size" for "Undo Image Size").
        label: Arc<str>,
        /// The container's document key.
        doc_key: Option<DocKey>,
    },
    /// The container closed as an OK: the edits since `Begin` stay, as one undo step.
    Commit {
        /// The outer gesture.
        id: GestureId,
    },
    /// The container closed any other way (Cancel, Escape, the close action, the app closing it): the edits since
    /// `Begin` are taken back.
    Cancel {
        /// The outer gesture.
        id: GestureId,
    },
}

/// What the runtime accumulated since the last `take_output`, across passes and discarded passes.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FrameOutput {
    /// The app-scope commands fired since the last call, in order.
    pub fired: Vec<Fired>,
    /// The commands components took since the last call, in order, whatever fired them (a key, a menu row, the
    /// palette): what ran inside the framework, for an app that records the commands it runs (an action recorder).
    pub delivered: Vec<Fired>,
    /// The key presses since the last call that ran nothing because their chord binds only commands disabled where the
    /// press landed, in order: each as the key would have fired its first such command. The app says why it did not
    /// run (`commands::resolve`'s hint), as it does for a disabled menu row.
    pub dimmed: Vec<Fired>,
    /// The app-scope commands a key press fired while only the app's own command state disabled them, in order.
    /// Refusals by the gate or the modal rule are not listed.
    pub declined: Vec<Fired>,
    /// Text for the live announcement region.
    pub announcements: Vec<String>,
    /// The chrome toggles the windows asked for since the last call, in order: each names its viewport and whether the
    /// tools stay. The shells take their own toggles in the pass that records them; the host reads this list only to
    /// know that a toggle happened.
    pub chrome_toggles: Vec<(ViewportId, ChromeToggle)>,
    /// The gestures whose component was evicted while they ran. Every `GestureHost` reads the list with
    /// `handle_output` and ends the gestures it owns as a cancel with `CancelReason::Removed`.
    pub removed_gestures: Vec<GestureId>,
    /// The steps of outer gestures since the last call, in order. Every `GestureHost` reads them with `handle_output`,
    /// before `removed_gestures`.
    pub outer_edits: Vec<OuterEdit>,
    /// Whether a component refused this pass's input since the last call (`frame::refuse`): the app plays its alert.
    pub refused: bool,
    /// The components whose live preview ended because they stopped showing; the host ends the preview it holds for
    /// each id.
    pub ended_previews: Vec<egui::Id>,
}

/// The per-frame and per-context state of the frame loop.
#[derive(Default)]
pub(crate) struct FrameState {
    input: Option<FrameInput>,
    /// The root pass number when `set_input` was last called, so test drivers know whether a frame's input is set.
    input_pass: Option<u64>,
    /// The window of `FrameInput::app_modal` and the root frame its modal first showed in: its place in the modal stack
    /// (`overlay::modal_viewport`). It stays while the app names the same window frame after frame.
    app_modal: Option<(ViewportId, u64)>,
    /// The pointer tolerances of the app's resolved tokens (`tokens.size.hit`) and the `sizes` they came from:
    /// the input hook writes the slop before any `Env` exists, and the press lifecycle reads the hit target.
    hit: Option<(Arc<AppSizes>, HitSizes)>,
    output: FrameOutput,
    /// The shells the viewports' last passes showed, for aiming a chrome toggle at the window that holds focus.
    shells: Vec<ShellRecord>,
    /// The chrome toggles no shell has taken yet.
    toggles: Vec<PendingToggle>,
    /// Passes between their begin and end hooks: nested viewport passes count.
    passes_open: u32,
    /// How many root-viewport passes the begin hook has run for: egui counts a pass when it ends.
    root_passes_hooked: u64,
    /// What drops the records of closed viewports as a frame begins.
    prunes: HashMap<TypeId, fn(&Context, &[ViewportId])>,
    sweeps: HashMap<TypeId, fn(&Context)>,
    /// What a layer above the runtime reads at the begin hook, after the pointer snapshot and before any component runs.
    begins: HashMap<TypeId, fn(&Context)>,
    frame_layers: Option<fn(&Context)>,
}

/// A window shell as its latest pass left it.
struct ShellRecord {
    viewport: ViewportId,
    window: WindowKey,
    /// The shell's root container, an instance of its window command scope.
    root: Id,
    pass: u64,
}

/// A chrome toggle waiting for its window's shell.
#[derive(Clone)]
struct PendingToggle {
    viewport: ViewportId,
    /// The window that held focus when the toggle was asked for; none when no shell of the viewport did.
    window: Option<WindowKey>,
    toggle: ChromeToggle,
    pass: u64,
}

/// How many passes a shell has to take its toggle before the toggle is dropped.
const TOGGLE_PASSES: u64 = 2;

/// Records of one runtime service, one per viewport, kept from the viewport's first pass until it closes: the root's
/// begin hook drops the records of the viewports egui no longer reports as a frame begins.
#[doc(hidden)]
pub struct PerViewport<T> {
    /// One or two entries: a linear search beats hashing.
    records: Vec<(ViewportId, T)>,
    pruning: bool,
}

impl<T> Default for PerViewport<T> {
    fn default() -> Self {
        PerViewport { records: Vec::new(), pruning: false }
    }
}

/// How a record that follows the passes of its viewport advanced to the running pass.
#[doc(hidden)]
pub enum PassStep {
    /// The viewport's previous pass left this record.
    Next,
    /// A pass or more went by without touching it.
    Skipped,
}

/// A record that resets when its viewport's pass number changes.
#[derive(Default)]
#[doc(hidden)]
pub struct Stamped<T> {
    pass: u64,
    value: T,
}

impl<T> Stamped<T> {
    /// The record as its viewport's latest pass left it.
    pub(crate) fn value(&self) -> &T {
        &self.value
    }
}

impl<T: Default> PerViewport<T> {
    fn record(&mut self, viewport: ViewportId) -> &mut T {
        let at = match self.records.iter().position(|(id, _)| *id == viewport) {
            Some(at) => at,
            None => {
                self.records.push((viewport, T::default()));
                self.records.len() - 1
            }
        };
        &mut self.records[at].1
    }
}

/// Where the records of a [`PerViewport`] type live: in the context's pass state, a field of it for the types the
/// runtime names there (`runtime::pass_state`, which overrides these), else by type beside the fields.
#[doc(hidden)]
pub trait Stored: Default + Send + 'static {
    /// Runs `f` on the records of this type.
    fn records<R>(ctx: &Context, f: impl FnOnce(&mut PerViewport<Self>) -> R) -> R {
        pass_state::with_records::<Self, R>(ctx, f)
    }

    /// Runs `f` on the stamped records of this type.
    fn stamped<R>(ctx: &Context, f: impl FnOnce(&mut PerViewport<Stamped<Self>>) -> R) -> R {
        pass_state::with_records::<Stamped<Self>, R>(ctx, f)
    }
}

impl<T: Stored> Stored for Stamped<T> {
    fn records<R>(ctx: &Context, f: impl FnOnce(&mut PerViewport<Self>) -> R) -> R {
        T::stamped(ctx, f)
    }
}

impl<T: Stored> PerViewport<T> {
    /// Runs `f` on the record of `viewport`, created on first use.
    pub(crate) fn with<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut T) -> R) -> R {
        let mut f = Some(f);
        let mut out = None;
        Self::visit(ctx, viewport, &mut |value| {
            if let Some(f) = f.take() {
                out = Some(f(value));
            }
        });
        #[expect(clippy::expect_used, reason = "visit calls its visitor once on the viewport's record")]
        out.expect("the viewport record was visited")
    }

    fn visit(ctx: &Context, viewport: ViewportId, f: &mut dyn FnMut(&mut T)) {
        // The prune registration runs outside the records' borrow: it reaches the context.
        let registered = T::records(ctx, |records| {
            let registered = std::mem::replace(&mut records.pruning, true);
            f(records.record(viewport));
            registered
        });
        if !registered {
            Self::register_prune(ctx);
        }
    }

    /// Runs `f` on the record of `viewport` when it has one; creates none.
    pub(crate) fn with_existing<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        T::records(ctx, |records| records.records.iter_mut().find(|(id, _)| *id == viewport).map(|(_, record)| f(record)))
    }

    /// Runs `f` on the record of every viewport that has one.
    pub(crate) fn each(ctx: &Context, mut f: impl FnMut(&mut T)) {
        T::records(ctx, |records| records.records.iter_mut().for_each(|(_, record)| f(record)));
    }

    fn register_prune(ctx: &Context) {
        fn prune<T: Stored>(ctx: &Context, live: &[ViewportId]) {
            T::records(ctx, |records| records.records.retain(|(viewport, _)| live.contains(viewport)));
        }
        register_prune(ctx, TypeId::of::<T>(), prune::<T>);
    }
}

impl<T: Stored> PerViewport<Stamped<T>> {
    /// Runs `f` on the record of `viewport` for its pass `pass_nr`; `reset` runs first when the pass changed.
    #[doc(hidden)]
    pub fn with_pass<R>(ctx: &Context, viewport: ViewportId, pass_nr: u64, reset: impl FnOnce(&mut T, PassStep), f: impl FnOnce(&mut T) -> R) -> R {
        let mut reset = Some(reset);
        let mut f = Some(f);
        let mut out = None;
        Self::visit_pass(
            ctx,
            viewport,
            pass_nr,
            &mut |value, step| {
                if let Some(reset) = reset.take() {
                    reset(value, step);
                }
            },
            &mut |value| {
                if let Some(f) = f.take() {
                    out = Some(f(value));
                }
            },
        );
        #[expect(clippy::expect_used, reason = "visit_pass visits the current record exactly once")]
        out.expect("the stamped viewport record was visited")
    }

    // Callers share stamp validation and reset ordering, including the parked-record lookup.
    fn visit_pass(ctx: &Context, viewport: ViewportId, pass_nr: u64, reset: &mut dyn FnMut(&mut T, PassStep), f: &mut dyn FnMut(&mut T)) {
        Self::visit(ctx, viewport, &mut |record| {
            if record.pass != pass_nr {
                reset(&mut record.value, if record.pass + 1 == pass_nr { PassStep::Next } else { PassStep::Skipped });
                record.pass = pass_nr;
            }
            f(&mut record.value);
        });
    }
}

/// Registers the prune of a service's per-viewport records to the open viewports ([`prune_closed`]), once per service
/// type.
pub(crate) fn register_prune(ctx: &Context, key: TypeId, prune: fn(&Context, &[ViewportId])) {
    pass_state::with::<FrameState, _>(ctx, |state| {
        state.prunes.entry(key).or_insert(prune);
    });
}

/// Drops the records of the viewports that have closed, at the start of a frame. A pass's list of viewports dates from
/// the start of its frame, so only there does it name every viewport with a record that is still open: later in the
/// frame it lacks the ones the frame opened.
fn prune_closed(ctx: &Context) {
    let live = pass::live_viewports(ctx);
    let prunes: Vec<fn(&Context, &[ViewportId])> = pass_state::with::<FrameState, _>(ctx, |state| state.prunes.values().copied().collect());
    for prune in prunes {
        prune(ctx, &live);
    }
}

/// Registers a sweep the end hook runs for the running viewport's pass, once per service type.
pub(crate) fn register_end_sweep(ctx: &Context, key: TypeId, sweep: fn(&Context)) {
    pass_state::with::<FrameState, _>(ctx, |state| {
        state.sweeps.entry(key).or_insert(sweep);
    });
}

/// Registers a hook the begin hook runs for each viewport's pass after the pointer snapshot, once per service type:
/// a service of a layer above the runtime reads its state there before any component runs.
pub(crate) fn register_begin_hook(ctx: &Context, key: TypeId, hook: fn(&Context)) {
    pass_state::with::<FrameState, _>(ctx, |state| {
        state.begins.entry(key).or_insert(hook);
    });
}

/// Before the pass: from eframe's `App::raw_input_hook`, or before `run_ui` in tests. A call during a pass,
/// or after a pass whose plugin hooks did not run, is a debug assertion.
pub fn set_input(ctx: &Context, input: FrameInput) {
    let scale = input.root.scale;
    let commands = input.commands.clone();
    let root_passes = pass::pass_nr(ctx, ViewportId::ROOT);
    let hooks_ran = pass_state::with::<FrameState, _>(ctx, |state| {
        debug_assert!(state.passes_open == 0, "set_input was called during a pass");
        let ran = state.root_passes_hooked == root_passes;
        if !state.hit.as_ref().is_some_and(|(sizes, _)| Arc::ptr_eq(sizes, &input.root.sizes)) {
            state.hit = Some((input.root.sizes.clone(), resolved_hit(&input.root.sizes)));
        }
        state.app_modal = match (input.app_modal, state.app_modal) {
            (Some(viewport), Some((shown, since))) if shown == viewport => Some((viewport, since)),
            (viewport, _) => viewport.map(|viewport| (viewport, pass::root_frame_nr(ctx))),
        };
        state.input = Some(input);
        state.input_pass = Some(root_passes);
        ran
    });
    commands::set_input(ctx, &commands);
    clipboard::begin_frame(ctx);
    windows::set_input(ctx);
    debug_assert!(hooks_ran, "the runtime plugin's hooks did not run for the previous pass: drive frames with `Context::run_ui` or `run_pass`");
    scale::set_interface_scale(ctx, scale.0);
}

/// Whether [`set_input`] has already run for the upcoming root pass.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn input_pending(ctx: &Context) -> bool {
    let root_passes = pass::pass_nr(ctx, ViewportId::ROOT);
    pass_state::with::<FrameState, _>(ctx, |state| state.input_pass == Some(root_passes))
}

/// The tokens the app's `sizes` give the default density: the built-in sets and the overrides apply in
/// `craft_theme`'s resolve, whichever theme carries them.
fn resolved_default(sizes: &AppSizes) -> craft_theme::Tokens {
    let appearance =
        Appearance { mode: Mode::Light, contrast: Contrast::Standard, size: Size::default(), text_size: TextSize::default(), reduced_motion: false };
    resolve_sized(suite_default_theme(), appearance, None, sizes).tokens
}

/// The tolerances the app's `sizes` give the default density.
#[doc(hidden)]
pub fn resolved_hit(sizes: &AppSizes) -> HitSizes {
    resolved_default(sizes).size.hit
}

/// The standard icon size the default density gives (`size.icon.md`), for a context built without an env.
#[doc(hidden)]
pub fn resolved_icon_side(sizes: &AppSizes) -> f32 {
    resolved_default(sizes).size.icon.md
}

/// The built-in theme the tolerances resolve under.
fn suite_default_theme() -> &'static ThemeDef {
    let themes = builtins();
    themes.iter().find(|theme| theme.id.as_str() == SUITE_DEFAULT).unwrap_or(&themes[0])
}

/// The tolerances of the running frame: the app's, else the default density's.
fn frame_hit(ctx: &Context) -> HitSizes {
    pass_state::with::<FrameState, _>(ctx, |state| state.hit.as_ref().map(|(_, hit)| *hit)).unwrap_or_else(|| resolved_hit(&AppSizes::default()))
}

/// What the raw-input step reads of the frame's root values (`RootEnv`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct RootInput {
    pub(crate) wheel: WheelSpeed,
    pub(crate) alt_codes: AltCodePages,
    pub(crate) ctrl_wheel_pinch: bool,
}

impl RootInput {
    fn of(root: &RootEnv) -> Self {
        RootInput { wheel: root.wheel, alt_codes: root.alt_codes, ctrl_wheel_pinch: root.ctrl_wheel_pinch }
    }
}

/// The raw-input values of the frame's root values; the defaults before the app sets any input.
pub(crate) fn root_input(ctx: &Context) -> RootInput {
    pass_state::with::<FrameState, _>(ctx, |state| state.input.as_ref().map(|input| RootInput::of(&input.root)))
        .unwrap_or_else(|| RootInput::of(&RootEnv::default()))
}

/// The window in which the app's own egui code shows a modal this frame (`FrameInput::app_modal`), and the root frame
/// that modal first showed in.
pub(crate) fn app_modal(ctx: &Context) -> Option<(ViewportId, u64)> {
    pass_state::with::<FrameState, _>(ctx, |state| state.app_modal)
}

/// The frame's window-input policy; `Live` before the app sets any input.
pub(crate) fn window_input(ctx: &Context) -> WindowInput {
    pass_state::with::<FrameState, _>(ctx, |state| state.input.as_ref().map(|input| input.window_input)).unwrap_or_default()
}

/// How far a press may move and still be a click, in points: the app's hit tolerances (`HitSizes::slop`). egui's own
/// click options stay the app's.
pub(crate) fn slop(ctx: &Context) -> f32 {
    frame_hit(ctx).slop
}

/// Called by `install`: the passes the context ran before the plugin existed have no hooks to wait for.
pub(crate) fn seed_hooked_passes(ctx: &Context) {
    let root_passes = pass::pass_nr(ctx, ViewportId::ROOT);
    pass_state::with::<FrameState, _>(ctx, |state| state.root_passes_hooked = root_passes);
}

/// Whether a pass is between its begin and end hooks.
pub(crate) fn in_pass(ctx: &Context) -> bool {
    pass_state::with::<FrameState, _>(ctx, |state| state.passes_open > 0)
}

/// Records that the running pass refused its input (the text part refusing a Tab); the next `take_output` reports it
/// in `FrameOutput::refused`. Public so an app's own widget refuses through the same channel.
pub fn refuse(ctx: &Context) {
    pass_state::with::<FrameState, _>(ctx, |state| state.output.refused = true);
}

/// Everything accumulated since the last call, across passes and discarded passes.
pub fn take_output(ctx: &Context) -> FrameOutput {
    let mut output = pass_state::with::<FrameState, _>(ctx, |state| std::mem::take(&mut state.output));
    output.fired = commands::take_fired(ctx);
    output.delivered = commands::take_delivered(ctx);
    output.dimmed = commands::take_dimmed(ctx);
    output.declined = commands::take_declined(ctx);
    // The menu, palette and automation fires of the chrome and nav commands reach the app as ordinary fires with no
    // window: the focused window's shell takes a toggle, and its focus service a pane move, in the next pass. A key fire
    // already acted in the key stage. Quit, from any source, asks the root window to close through its close path.
    let focused = crate::runtime::egui::viewports::reported(ctx).into_iter().find(|(_, facts)| facts.focused).map(|(viewport, _)| viewport);
    for fired in &output.fired {
        if fired.phase == commands::FirePhase::Press && commands::QuitCommand::from_ref(&fired.command).is_some() {
            windows::act(ctx, ViewportId::ROOT, windows::WindowAction::Close);
        }
        if fired.source == commands::FireSource::Key {
            continue;
        }
        if let (Some(command), Some(viewport)) = (commands::ChromeCommand::from_ref(&fired.command), focused) {
            queue_toggle(ctx, viewport, command.toggle());
        }
        if let (Some(command), Some(viewport)) = (commands::NavCommand::from_ref(&fired.command), focused) {
            keys::queue_nav(ctx, viewport, command);
        }
    }
    output
}

/// Sets the end hook's ghost-layer painter.
pub(crate) fn set_frame_layers(ctx: &Context, painter: Option<fn(&Context)>) {
    pass_state::with::<FrameState, _>(ctx, |state| state.frame_layers = painter);
}

/// Adds a gesture whose component was evicted to the next `FrameOutput`.
pub(crate) fn note_removed_gesture(ctx: &Context, id: GestureId) {
    pass_state::with::<FrameState, _>(ctx, |state| state.output.removed_gestures.push(id));
}

/// Adds a component whose live preview ended to the next `FrameOutput`.
pub(crate) fn note_ended_preview(ctx: &Context, id: Id) {
    pass_state::with::<FrameState, _>(ctx, |state| state.output.ended_previews.push(id));
}

/// Adds a step of an outer gesture to the next `FrameOutput`.
#[doc(hidden)]
pub fn note_outer_edit(ctx: &Context, edit: OuterEdit) {
    pass_state::with::<FrameState, _>(ctx, |state| state.output.outer_edits.push(edit));
}

/// Adds text to the next `FrameOutput`'s announcements.
pub(crate) fn announce(ctx: &Context, text: String) {
    pass_state::with::<FrameState, _>(ctx, |state| state.output.announcements.push(text));
}

/// Asks for another pass of the viewport the running pass belongs to. Asked in a memoized view's run, that view runs in
/// the next pass, and so do the views around it, whose replay would hold its old run. Asked outside every view, every
/// view replays: app data a view shows reaches it through its props.
#[doc(hidden)]
pub fn request_repaint(ctx: &Context) {
    crate::runtime::replay::note_unsettled(ctx);
    raw_input::request_repaint_of(ctx, pass::current_viewport(ctx));
}

/// Asks for one more pass of the viewport the running pass belongs to after `seconds` (a playback clock, a poll);
/// the earliest request of a frame wins. A component asks through `Animate::wake_at`, which also needs no context.
pub fn request_repaint_after(ctx: &Context, seconds: f64) {
    crate::runtime::egui::repaint::request_after(ctx, crate::runtime::egui::pass::current_viewport(ctx), seconds);
}

/// Whether the running pass can still be discarded: egui runs a bounded number of passes a frame, and a discard asked
/// in the last one is ignored, so what that pass shows paints.
pub fn discardable(ctx: &Context) -> bool {
    crate::runtime::egui::sizing::discardable(ctx)
}

/// Whether egui will run the running pass again, so nothing of it paints: egui's `Context::will_discard`, or a discard
/// pass that layout memory asked for in this pass and requests from egui only at the pass's end. Host code that does
/// work once per shown frame (a frame count, a settle count) skips it while this is true; `will_discard` alone is still
/// false during the app's UI in a pass that a changed size runs again.
pub fn will_rerun(ctx: &Context) -> bool {
    ctx.will_discard() || crate::runtime::layout_memory::discard_pending(ctx)
}

/// Discards the running pass, so nothing of it paints, and runs it again, when [`discardable`]: for a host that learns
/// only after a part ran that the part showed what it must not. A pass after a discard carries no input (events,
/// injected steps and press edges enter a frame's first pass only); held state persists.
pub fn discard_pass(ctx: &Context, reason: &'static str) {
    crate::runtime::egui::sizing::request_discard(ctx, crate::runtime::egui::sizing::Rerun::App(reason));
}

/// Records the shell of window `window` in `viewport`, whose root container is `root`, so a chrome toggle finds the
/// window that holds focus.
#[doc(hidden)]
pub fn note_shell(ctx: &Context, viewport: ViewportId, window: &WindowKey, root: Id) {
    let pass = pass::pass_nr(ctx, viewport);
    pass_state::with::<FrameState, _>(ctx, |state| {
        state.shells.retain(|shell| shell.viewport != viewport || (shell.window != *window && shell.pass + TOGGLE_PASSES > pass));
        state.shells.push(ShellRecord { viewport, window: window.clone(), root, pass });
    });
}

/// The root container of the shell the latest pass of `viewport` showed: where a key pressed with nothing focused
/// matches, so the window's commands take it.
pub fn shell_root(ctx: &Context, viewport: ViewportId) -> Option<Id> {
    pass_state::with::<FrameState, _>(ctx, |state| {
        state.shells.iter().filter(|shell| shell.viewport == viewport).max_by_key(|shell| shell.pass).map(|shell| shell.root)
    })
}

/// Adds a chrome toggle of `viewport` to the next `FrameOutput` and aims it at the window of the viewport that holds
/// focus, or at the first shell that asks when none does.
pub fn request_chrome_toggle(ctx: &Context, viewport: ViewportId, toggle: ChromeToggle) {
    queue_toggle(ctx, viewport, toggle);
    pass_state::with::<FrameState, _>(ctx, |state| state.output.chrome_toggles.push((viewport, toggle)));
}

fn queue_toggle(ctx: &Context, viewport: ViewportId, toggle: ChromeToggle) {
    let pass = pass::pass_nr(ctx, viewport);
    let candidates: Vec<(WindowKey, Id)> = pass_state::with::<FrameState, _>(ctx, |state| {
        state.shells.iter().filter(|shell| shell.viewport == viewport).map(|s| (s.window.clone(), s.root)).collect()
    });
    let window = candidates.into_iter().find(|(_, root)| commands::focus_within(ctx, *root)).map(|(window, _)| window);
    pass_state::with::<FrameState, _>(ctx, |state| {
        state.toggles.retain(|pending| pending.viewport != viewport || pending.pass + TOGGLE_PASSES > pass);
        state.toggles.push(PendingToggle { viewport, window, toggle, pass });
    });
}

/// Takes the chrome toggles of the window `window` in `viewport`: the ones aimed at it, and the ones no window held
/// focus for. A shell calls it once in each pass, so its window toggles in the pass the key went down.
#[doc(hidden)]
pub fn take_chrome_toggles(ctx: &Context, viewport: ViewportId, window: &WindowKey) -> Vec<ChromeToggle> {
    pass_state::with::<FrameState, _>(ctx, |state| {
        let wanted = |pending: &PendingToggle| pending.viewport == viewport && pending.window.as_ref().is_none_or(|aimed| aimed == window);
        measure::take_queued(ctx, &mut state.toggles, wanted).into_iter().map(|pending| pending.toggle).collect()
    })
}

/// The held input, the runtime's request to end the viewport's egui drag and the clock of its running pass.
#[derive(Default)]
struct ViewportRecord {
    held: HeldInput,
    press_cancelled: bool,
    /// `None` until the viewport's first pass begins.
    now: Option<f64>,
}

impl Stored for ViewportRecord {}

/// What a pass fixes for every step: its viewport, numbers and time.
#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct PassFacts {
    pub viewport: ViewportId,
    pub pass_nr: u64,
    pub frame_nr: u64,
    /// The pass's time in seconds.
    pub now: f64,
    /// Seconds since this viewport's previous pass. A frame's later passes share its pass clock, so they read 0; a
    /// viewport's first pass reads 0.
    pub elapsed: f64,
}

/// The facts of the innermost running pass of `ctx` on this thread, `None` outside every pass of it. The begin hook
/// reads them; steps read them, none writes them.
#[doc(hidden)]
pub fn running(ctx: &Context) -> Option<PassFacts> {
    pass_state::running(ctx)
}

/// The facts of the innermost running pass of `ctx`, or outside every pass of it, what the context says of its
/// current viewport.
pub(crate) fn facts(ctx: &Context) -> PassFacts {
    running(ctx).unwrap_or_else(|| {
        let viewport = ctx.viewport_id();
        PassFacts { viewport, pass_nr: pass::pass_nr(ctx, viewport), frame_nr: pass::frame_nr(ctx, viewport), now: clock(ctx), elapsed: 0.0 }
    })
}

/// The pass clock in seconds rounded down to a multiple of `step`, for content that changes with time (a rescan every
/// few seconds, a "5 minutes ago"): the running view is shown again at the next multiple. Between two steps a run
/// shows what the last one showed, so a memoized view that reads it replays until the next step and runs then. A
/// `step` of zero or less returns the clock and shows the view again in every pass.
pub fn stepped_clock(ctx: &Context, step: f64) -> f64 {
    let now = clock(ctx);
    if step <= 0.0 {
        crate::runtime::motion::note_unsettled(ctx);
        return now;
    }
    let at = (now / step).floor() * step;
    crate::runtime::motion::wake_at(ctx, at + step);
    at
}

/// The running pass's time in seconds, the same on every pass of a frame: for app code that times a wait. Content
/// that changes with time reads [`stepped_clock`], which shows it again when the value it read changes; this read
/// notes nothing, for timing events in the component that has the pointer or the focus.
pub fn clock(ctx: &Context) -> f64 {
    match running(ctx) {
        Some(facts) => facts.now,
        None => PerViewport::<ViewportRecord>::with(ctx, pass::current_viewport(ctx), |record| record.now.unwrap_or(0.0)),
    }
}

/// Records that the runtime cancelled the captured press of the running viewport; the next begin hook ends
/// egui's drag so hover reaches other widgets again.
pub(crate) fn note_cancelled_press(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<ViewportRecord>::with(ctx, viewport, |record| record.press_cancelled = true);
    crate::runtime::egui::repaint::request_of(ctx, crate::runtime::egui::pass::current_viewport(ctx));
}

/// Whether the one-press rule (H0) holds input of `viewport` for its next frame.
#[cfg(feature = "testing")]
pub(crate) fn holds_input(ctx: &Context, viewport: ViewportId) -> bool {
    PerViewport::<ViewportRecord>::with(ctx, viewport, |record| record.held.holds())
}

/// The events the one-press rule (H0) or the key stage held that the frame `frame` of `viewport` gets, read in its
/// input hook before H0 hands them on.
pub(crate) fn held_for(ctx: &Context, viewport: ViewportId, frame: u64) -> Vec<egui::Event> {
    PerViewport::<ViewportRecord>::with(ctx, viewport, |record| record.held.delivered_in(frame))
}

/// The mode the current frame's root theme resolves to on `ctx`.
fn effective_mode(ctx: &Context) -> Mode {
    let root = pass_state::with::<FrameState, _>(ctx, |state| state.input.as_ref().map(|input| input.root.clone()));
    match root {
        Some(root) => root.effective_mode(ctx),
        None => RootEnv::fallback(ctx).effective_mode(ctx),
    }
}

/// The input hook: egui runs it before it pushes the viewport, so every step reads only `RawInput` and
/// records keyed by `input.viewport_id`.
pub(crate) fn input_hook(ctx: &Context, input: &mut RawInput) {
    let viewport = input.viewport_id;
    // H0a: the next injected step joins the input and passes the stages below like real input.
    let injected = crate::runtime::egui::injected::input_hook(ctx, input);
    // H0c: the legacy pinch fold, the text cap and the Control-click mapping, on injected and real input alike, before
    // the one-press rule sees a mapped press and before the IME and key stages read the text.
    raw_input::prepare(ctx, input);
    // H0b: the OS colour scheme egui's integration reports reaches the system reader.
    crate::runtime::env::system::input_hook(ctx, &input.events);
    // H0d: match egui's font coverage transfer function to the pass's resolved theme mode before egui runs
    // `Fonts::begin_pass`, so dark-on-light and light-on-dark text use the right curve even when the app's mode
    // differs from the OS's.
    let mode = effective_mode(ctx);
    style::set_text_color_transfer(ctx, mode);
    // From H0 to H3 the events go with the input time each arrived at (`Arrived`): held events keep theirs, and the key
    // stage gives each shortcut its own. egui's events are what these stages leave.
    let mut events = Arrived::take(input);
    // H0: one press per pass.
    let frame = pass::frame_nr(ctx, viewport);
    let held_back = PerViewport::<ViewportRecord>::with(ctx, viewport, |record| raw_input::hold_back_presses(&mut events, frame, &mut record.held));
    // H3: the key stage strips what the runtime owns; egui decides on a repaint from the events left. The events sent
    // to the focus are counted first, before the IME and key stages take theirs.
    keys::count_sent(ctx, viewport, &events);
    keys::note_ime_presses(ctx, viewport, &events);
    ime::input_hook(ctx, &mut events, viewport);
    let (stripped, later) = keys::input_hook(ctx, input, &mut events);
    if !later.is_empty() {
        PerViewport::<ViewportRecord>::with(ctx, viewport, |record| record.held.hold_ahead(later));
    }
    Arrived::into_input(events, input);
    // H3b: a press outside the top menu family goes no further than the menu's dismissal.
    let absorbed = overlay::absorb_presses(ctx, input);
    // H4: release after an OS resize.
    let released = windows::input_hook(ctx, input);
    if injected || held_back || stripped || absorbed || released {
        raw_input::request_repaint_of(ctx, viewport);
    }
}

/// The begin hook, where the viewport is current.
pub(crate) fn begin_pass(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // The pass this hook begins is not on the running stack yet.
    let viewport = ctx.viewport_id();
    let root_pass = (viewport == ViewportId::ROOT).then(|| pass::pass_nr(&ctx, viewport));
    let frame_input = pass_state::with::<FrameState, _>(&ctx, |state| {
        state.passes_open += 1;
        if let Some(nr) = root_pass {
            state.root_passes_hooked = nr + 1;
        }
        state.input.clone()
    });
    // H5: read the clock once; the runtime cancelled a captured press since this viewport's last pass.
    let now = pass::now(&ctx);
    let (pass_nr, frame_nr) = (pass::pass_nr(&ctx, viewport), ctx.cumulative_frame_nr_for(viewport));
    let (elapsed, cancelled) = PerViewport::<ViewportRecord>::with(&ctx, viewport, |record| {
        // The events this frame's input hook held arrived at the frame's input time.
        record.held.stamp(now);
        let elapsed = record.now.replace(now).map_or(0.0, |previous| (now - previous).max(0.0));
        (elapsed, std::mem::take(&mut record.press_cancelled))
    });
    pass_state::hold(&ctx);
    pass_state::begin_running(&ctx, PassFacts { viewport, pass_nr, frame_nr, now, elapsed });
    if root_pass.is_some() && pass::first_of_frame(&ctx) {
        prune_closed(&ctx);
    }
    if cancelled {
        raw_input::stop_dragging(&ctx);
    }
    // H5: this viewport's window info; a press H3 dropped under a modal raises the modal's window.
    windows::begin(&ctx);
    // H5: the pointer snapshot, before any component runs.
    let input = frame_input.unwrap_or_else(|| FrameInput::new(RootEnv::fallback(&ctx)));
    pointer::begin(&ctx, frame_hit(&ctx), input.latched);
    let begins: Vec<fn(&Context)> = pass_state::with::<FrameState, _>(&ctx, |state| state.begins.values().copied().collect());
    for hook in begins {
        hook(&ctx);
    }
    drag::begin(&ctx);
    // H5: install the root `Env` and the theme styles' direction from `FrameInput.root`.
    let env = Env::from_root(&ctx, &input.root, input.host, scale::pixels_per_point(&ctx));
    style::set_theme_direction(&ctx, ui, env.direction.is_rtl());
    Env::set_root(&ctx, viewport, env.clone());
    drop(Env::install_for(ui, env));
    // H7: focus moves and the press classification, then the pass's intents, then the wheel split.
    focus::begin(&ctx, input.root.tab_all_controls || windows::os_of(&ctx) != Os::Mac, input.root.dialog_tab);
    ime::begin(&ctx, focus::focused_rec(&ctx).filter(|rec| rec.stop.text).map(|rec| rec.stop.root));
    // Lead-1: an Escape over a captured press cancels the press and reaches no component. Any other Escape the key
    // stage took ends egui's own drag, as egui's Escape would have before any widget runs.
    let escapes = keys::begin(&ctx, pointer::press_captured(&ctx) || drag::running(&ctx));
    if escapes.cancels_press {
        pointer::cancel_press(&ctx, craft_behavior::PressCancel::Escape);
        drag::cancel(&ctx, craft_behavior::CancelReason::Escape);
    } else if escapes.to_components > 0 {
        raw_input::stop_dragging(&ctx);
    }
    wheel::begin(&ctx, input.root.pointer_scoped_field_wheel);
    // H7: shortcuts the owner allows, through the gate.
    commands::begin(&ctx);
    access::begin_live_pass(&ctx);
}

/// The end hook.
pub(crate) fn end_pass(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    drag::end_pass(&ctx);
    cursor::end_pass(&ctx);
    commands::end(&ctx);
    access::end_live_pass(&ctx);
    crate::runtime::egui::access::order_children(&ctx);
    let painter = pass_state::with::<FrameState, _>(&ctx, |state| state.frame_layers);
    if let Some(paint_frame_layers) = painter {
        paint_frame_layers(&ctx);
    }
    toast::end_pass(&ctx);
    // Every taker of the pass has run, and the layers and the focus are the pass's still.
    keys::hand_on_cancel(&ctx);
    overlay::end_pass(&ctx);
    focus::end(&ctx);
    let sweeps: Vec<fn(&Context)> = pass_state::with::<FrameState, _>(&ctx, |state| state.sweeps.values().copied().collect());
    for sweep in sweeps {
        sweep(&ctx);
    }
    // Evict shown memory by id and queue the evicted slots (7.2).
    memory::evict(&ctx);
    layout_memory::end_hook(&ctx);
    motion::end_pass(&ctx);
    windows::end(&ctx);
    ime::end_pass(&ctx);
    pass_state::with::<FrameState, _>(&ctx, |state| state.passes_open = state.passes_open.saturating_sub(1));
    #[cfg(feature = "testing")]
    crate::runtime::egui::access::check_parents_acyclic(&ctx);
    pass_state::end_running(&ctx);
    pass_state::release(&ctx);
}
