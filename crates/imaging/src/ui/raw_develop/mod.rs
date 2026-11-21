//! The RAW Develop workspace: a full-window editor of the Camera Raw settings over the app's develop.
//!
//! [`RawDevelopWorkspace`] is a [`FullWindowWorkspace`] the app opens as a `HostedWorkspace`. The window has the
//! filmstrip at the start edge while several raws are being opened, the preview canvas with its zoom bar, and at the
//! end edge the scope ([`RawScopeStrip`], or [`RawFloatingScope`] while it floats) over the [`RawDevelopPanel`]. The
//! app's [`RawDevelopSource`] develops the preview; the workspace keeps the settings, their history, the scope's state
//! and the color samplers.
//!
//! Files: `mod.rs` the workspace, `source.rs` the app's side, `filmstrip.rs` the raw tiles, `commands.rs` the keys,
//! `words.rs` the words of the workspace and of the components below, `controls.rs` the panel, `mixer.rs` the Color
//! Mixer, `scopes.rs` the scope strips, `preview.rs` the samplers' readouts and the clipping overlay.
//!
//! [`FullWindowWorkspace`]: craft_ui::dialog::FullWindowWorkspace

mod commands;
pub(crate) mod controls;
pub(crate) mod filmstrip;
mod mixer;
mod preview;
pub(crate) mod scopes;
mod source;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod words;

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use craft_behavior::edit_history::Step;
use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{CancelReason, EditEvent, EditHistory, Mixed, PixelSize};
use craft_color_ui::ui::{ColorSamplerMarks, ColorSamplerTool, ColorSamplers, SamplerCache, SamplerEvent};
use craft_theme::TypeRole;
use craft_ui::canvas::{NavigationOptions, PixelRect, RasterSource, ToolHost, ViewCommands, ZoomTool};
use craft_ui::contract::{Bind, Pattern, Remember as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{
    DialogResult, FieldRefusal, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView,
    FullWindowWorkspace, FullWindowZoomBar, WorkspaceCx, WorkspacePass, serde_fields, set_serde_field,
};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::primitives::Checker;
use craft_ui::runtime::commands::{CommandSet as _, Commands as _, FirePhase};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft_ui::runtime::pending::Pending;
use craft_ui::runtime::windows::WindowSpec;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{Context, Id, Ui};

pub use commands::RawDevelopCommands;
pub use controls::{RawDevelopPanel, RawDevelopPanelOutput};
pub use filmstrip::{RawFilmItem, RawFilmstrip};
pub use scopes::{RawFloatingScope, RawScopeStrip, RawScopeStripOutput};
pub use source::{RawDevelopSource, RawPreviewRequest};
pub use words::RawDevelopWords;

pub use crate::raw::Demosaic;
use crate::raw::RawDevelopSettings;
use crate::ui::live_preview::settings_hash;
use crate::ui::scopes::{ClippingDiagnostic, ClippingMode, ScopeCommand, ScopePanel, ScopePanelState, ScopeSettings};

/// The saved slot of the scope's settings, shared by every open of the workspace.
const SCOPE_KEY: SaveKey = SaveKey::new("craft_imaging.raw_develop.scope");

/// Where the workspace's result goes: the footer's buttons follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawTarget {
    /// A filter on a layer: OK and Cancel.
    Filter,
    /// A smart filter: OK and Cancel.
    SmartFilter,
    /// Raws being opened: Open, Cancel and Done.
    OpenRaw,
}

impl RawTarget {
    /// The button that is the default outcome.
    fn accept(self) -> RawFinish {
        match self {
            RawTarget::OpenRaw => RawFinish::Open,
            RawTarget::Filter | RawTarget::SmartFilter => RawFinish::Ok,
        }
    }
}

/// What opens the workspace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RawDevelopOpen {
    /// The raw's or the layer's name: the title's subject.
    pub subject: String,
    /// The picture's size in document pixels.
    pub size: PixelSize,
    /// Where the result goes.
    pub target: RawTarget,
    /// The settings to start from.
    pub settings: RawDevelopSettings,
    /// The settings Reset restores.
    pub original: RawDevelopSettings,
    /// The demosaic, while raws are being opened.
    pub demosaic: Option<Demosaic>,
    /// The filmstrip tiles, one per raw.
    pub filmstrip: Vec<RawFilmItem>,
    /// The raw shown.
    pub current: usize,
    /// The user's navigation options.
    pub navigation: NavigationOptions,
    /// The user's transparency grid.
    pub checkerboard: Option<Checker>,
}

impl RawDevelopOpen {
    /// Opens on `subject` of `size` with `settings`, for `target`. Reset restores `settings` until
    /// [`original`](Self::original) says otherwise.
    pub fn new(subject: impl Into<String>, size: PixelSize, target: RawTarget, settings: RawDevelopSettings) -> Self {
        Self {
            subject: subject.into(),
            size,
            target,
            original: settings.clone(),
            settings,
            demosaic: None,
            filmstrip: Vec::new(),
            current: 0,
            navigation: NavigationOptions::default(),
            checkerboard: None,
        }
    }

    /// Sets the settings Reset restores.
    #[must_use]
    pub fn original(mut self, original: RawDevelopSettings) -> Self {
        self.original = original;
        self
    }

    /// Sets the demosaic the raw is opened with.
    #[must_use]
    pub fn demosaic(mut self, demosaic: Demosaic) -> Self {
        self.demosaic = Some(demosaic);
        self
    }

    /// Sets the filmstrip's tiles and the raw shown.
    #[must_use]
    pub fn filmstrip(mut self, items: Vec<RawFilmItem>, current: usize) -> Self {
        self.filmstrip = items;
        self.current = current;
        self
    }

    /// Sets the user's navigation options.
    #[must_use]
    pub fn navigation(mut self, navigation: NavigationOptions) -> Self {
        self.navigation = navigation;
        self
    }

    /// Sets the user's transparency grid.
    #[must_use]
    pub fn checkerboard(mut self, checker: Checker) -> Self {
        self.checkerboard = Some(checker);
        self
    }
}

/// What the workspace reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RawDevelopEvent {
    /// The settings changed.
    Settings(Box<EditEvent<RawDevelopSettings>>),
    /// A filmstrip pick: the app keeps the current raw's settings and calls `show_item`.
    Show(usize),
    /// Load Settings asks the app for a file.
    LoadSettings,
    /// Save Settings asks the app for a file.
    SaveSettings,
}

/// How the workspace finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawFinish {
    /// OK of a filter or a smart filter.
    Ok,
    /// Open of raws being opened.
    Open,
    /// Done of raws being opened.
    Done,
}

/// What OK answers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RawDevelopResult {
    /// The button that finished it.
    pub finish: RawFinish,
    /// The settings at the end.
    pub settings: RawDevelopSettings,
    /// The demosaic, while raws are being opened.
    pub demosaic: Option<Demosaic>,
}

/// The tool the canvas runs; the Hand tool is the canvas's own navigation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RawTool {
    #[default]
    Hand,
    Zoom,
    Sampler,
}

/// What a footer button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Footer {
    Finish(RawFinish),
    Save,
    Load,
    Cancel,
}

/// A change the passes ask of the workspace, applied after the dialog has been shown.
enum Action {
    /// An edit of the settings from the panel or the scope's tone zones.
    Edit(Box<EditEvent<RawDevelopSettings>>),
    /// The Auto button.
    Auto,
    /// A filmstrip pick.
    Show(usize),
    /// An undo or redo.
    Step(Step),
}

/// The RAW Develop workspace, titled "RAW Develop (name)" or "RAW Develop Filter (layer)".
///
/// Keys: Y shows the picture before the settings while held; S, H and Z pick the Color Sampler, Hand and Zoom tools; U
/// and O toggle the clipping warnings ([`ScopeCommand`]); ⌘Z and ⇧⌘Z step the history; Enter OK; Esc Cancel.
///
/// A pass applies only its own input: the edits, steps and picks it reports come from that pass's events, and what the
/// automation surface queued (`set_json_field`) is reported once, by the first pass that runs after it.
#[derive(Pattern)]
pub struct RawDevelopWorkspace {
    subject: String,
    size: PixelSize,
    target: RawTarget,
    settings: RawDevelopSettings,
    /// The settings the workspace, or the raw now shown, started from: a curve left as stored passes OK, so an older
    /// Smart Filter stays editable.
    stored: RawDevelopSettings,
    original: RawDevelopSettings,
    demosaic: Option<Demosaic>,
    history: EditHistory<RawDevelopSettings>,
    filmstrip: Vec<RawFilmItem>,
    current: usize,
    navigation: NavigationOptions,
    checkerboard: Option<Checker>,
    view: FullWindowView,
    before: bool,
    tool: RawTool,
    zoom_tool: Option<ZoomTool>,
    canvas: Option<Id>,
    hover: Option<[f64; 2]>,
    /// The scope's state: `None` until the first pass fills it from the saved slot.
    scope: Option<ScopePanelState>,
    /// The scope settings as last written to the saved slot.
    scope_written: Option<ScopeSettings>,
    floating: bool,
    samplers: ColorSamplers,
    sampler_tool: ColorSamplerTool,
    sampler_cache: SamplerCache,
    /// The Alt-drag of Exposure, Whites or Blacks that shows the clipping overlay while it lasts.
    diagnostic: Option<ClippingDiagnostic>,
    status: Option<Label>,
    last_done: Option<(Duration, Option<Label>)>,
    queued: Vec<RawDevelopEvent>,
    /// A finish the automation surface asked for; the next pass reports it.
    requested: Option<RawFinish>,
}

impl RawDevelopWorkspace {
    /// A workspace over `open`.
    pub fn new(open: RawDevelopOpen) -> Self {
        Self {
            subject: open.subject,
            size: open.size,
            target: open.target,
            history: EditHistory::new(&open.settings),
            stored: open.settings.clone(),
            settings: open.settings,
            original: open.original,
            demosaic: open.demosaic,
            filmstrip: open.filmstrip,
            current: open.current,
            navigation: open.navigation,
            checkerboard: open.checkerboard,
            view: FullWindowView::new(),
            before: false,
            tool: RawTool::default(),
            zoom_tool: None,
            canvas: None,
            hover: None,
            scope: None,
            scope_written: None,
            floating: false,
            samplers: ColorSamplers::new(),
            sampler_tool: ColorSamplerTool::new(),
            sampler_cache: SamplerCache::default(),
            diagnostic: None,
            status: None,
            last_done: None,
            queued: Vec::new(),
            requested: None,
        }
    }

    /// The settings as they are now.
    pub fn session(&self) -> &RawDevelopSettings {
        &self.settings
    }

    /// Loads `settings` as one history step, named by the words of the app's root.
    pub fn load(&mut self, ctx: &Context, settings: RawDevelopSettings) {
        let name = Arc::from(Env::of_root(ctx).words::<RawDevelopWords>().load_step.as_str());
        self.apply(settings, name);
    }

    /// Shows raw `current` with its `settings`, the `original` Reset restores and its `demosaic`. The history starts
    /// over.
    pub fn show_item(&mut self, current: usize, settings: RawDevelopSettings, original: RawDevelopSettings, demosaic: Option<Demosaic>) {
        self.history = EditHistory::new(&settings);
        self.stored = settings.clone();
        self.settings = settings;
        self.original = original;
        self.demosaic = demosaic;
        self.current = current;
        self.diagnostic = None;
    }

    /// Sets the picture of filmstrip tile `index` and whether the raw is edited.
    pub fn set_item_picture(&mut self, index: usize, picture: egui::load::SizedTexture, edited: bool) {
        if let Some(item) = self.filmstrip.get_mut(index) {
            item.picture = Some(picture);
            item.edited = edited;
        }
    }

    /// Shows `message` in the status line.
    pub fn status(&mut self, message: Option<Label>) {
        self.status = message;
    }

    /// Replaces the settings by `next` as one history step named `name`, and reports it. Nothing changes when `next`
    /// is the settings already.
    fn apply(&mut self, next: RawDevelopSettings, name: Arc<str>) -> bool {
        if next == self.settings {
            return false;
        }
        let event = EditEvent::set(Mixed::Same(self.settings.clone()), next.clone(), name, None);
        self.history.record(&event);
        self.settings = next;
        self.queued.push(RawDevelopEvent::Settings(Box::new(event)));
        true
    }

    /// Takes in one edit event of the panel or the scope strip: records it and moves the settings to what it says.
    fn take_edit(&mut self, event: EditEvent<RawDevelopSettings>, alt: bool) -> RawDevelopEvent {
        self.history.record(&event);
        match &event {
            EditEvent::Update { value, .. } => {
                self.diagnostic = alt.then(|| diagnostic_of(&self.settings, value)).flatten().or(self.diagnostic);
                self.settings = value.clone();
            }
            EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } => {
                self.settings = after.clone();
                self.diagnostic = None;
            }
            EditEvent::Cancel { before, .. } => {
                if let Some(Mixed::Same(before)) = before {
                    self.settings = before.clone();
                }
                self.diagnostic = None;
            }
            _ => {}
        }
        RawDevelopEvent::Settings(Box::new(event))
    }

    /// Steps the history and reports the step as a change of the settings named `name`.
    fn step(&mut self, step: Step, name: Arc<str>) -> Option<RawDevelopEvent> {
        let before = self.settings.clone();
        if !self.history.step(&mut self.settings, step) {
            return None;
        }
        self.diagnostic = None;
        let event = EditEvent::set(Mixed::Same(before), self.settings.clone(), name, None);
        Some(RawDevelopEvent::Settings(Box::new(event)))
    }

    /// The result the button `finish` answers.
    fn result(&self, finish: RawFinish) -> RawDevelopResult {
        RawDevelopResult { finish, settings: self.settings.clone(), demosaic: self.demosaic }
    }

    /// The part of the picture the preview shows, in document pixels, and the zoom (screen pixels per document
    /// pixel): what the develop renders. The whole picture until the first pass has settled the view.
    fn visible(&self) -> (PixelRect, f64) {
        let image = DocRect::new(0.0, 0.0, f64::from(self.size.width), f64::from(self.size.height));
        let part = if self.view.size().width > 0.0 { self.view.view().visible().intersect(image) } else { image };
        let rect = PixelRect::new(part.x0.floor() as i32, part.y0.floor() as i32, part.x1.ceil() as i32, part.y1.ceil() as i32);
        (rect, self.view.device_zoom())
    }

    /// Ends the gesture the active tool holds on the canvas, through the host that runs it.
    fn end_gesture(&mut self, ctx: &Context, reason: CancelReason) {
        let Some(canvas) = self.canvas else { return };
        let size = [self.size.width, self.size.height];
        match self.tool {
            RawTool::Hand => {}
            RawTool::Zoom => {
                if let Some(zoom) = self.zoom_tool.as_mut() {
                    ToolHost::new().cancel(ctx, canvas, zoom, reason);
                }
            }
            RawTool::Sampler => {
                let mut pass = self.sampler_tool.pass(&self.samplers, size);
                ToolHost::new().cancel(ctx, canvas, &mut pass, reason);
            }
        }
    }

    /// Applies what the Color Sampler tool asked for to the points.
    fn take_sampler_events(&mut self) {
        let size = [self.size.width, self.size.height];
        for event in self.sampler_tool.take_events() {
            // A refused point (the eleventh, one off the picture) is the tool's toast, not a state change.
            let _ = match event {
                SamplerEvent::Add { at, .. } => self.samplers.add(at, size).map(|_| ()),
                SamplerEvent::Move { index, to, .. } => self.samplers.move_point(index, to, size),
                SamplerEvent::Remove { index, .. } => self.samplers.remove(index),
                _ => Ok(()),
            };
        }
    }

    /// Fills the scope's state from the saved slot on the first pass, with today's rules: samplers are never
    /// restored, the vectorscope starts off and the selected-region option is off without a selection.
    fn open_scope(&mut self, ctx: &Context, source: &dyn RawDevelopSource) {
        if self.scope.is_some() {
            return;
        }
        let saved: ScopeSettings = saved_value(ctx, &SCOPE_KEY);
        let mut settings = saved.clone();
        settings.vectorscope = false;
        settings.selected_region = settings.selected_region && source.has_selection();
        self.scope = Some(ScopePanelState { settings, range: None });
        self.scope_written = Some(saved);
    }

    /// Writes the scope's settings to the saved slot when they changed.
    fn save_scope(&mut self, ctx: &Context) {
        let Some(state) = &self.scope else { return };
        if self.scope_written.as_ref() != Some(&state.settings) {
            set_saved_value(ctx, &SCOPE_KEY, state.settings.clone());
            self.scope_written = Some(state.settings.clone());
        }
    }

    /// Runs Auto: the settings the source picks, as one step named `name`.
    fn run_auto(&mut self, source: &dyn RawDevelopSource, name: Arc<str>) {
        let next = source.auto(&self.settings);
        self.apply(next, name);
    }

    /// Sets the zoom the `view` field names: `"fit"` or a percent.
    fn set_view(&mut self, value: &serde_json::Value) -> Result<(), String> {
        match value {
            serde_json::Value::String(text) if text == "fit" => {
                self.view.apply(ViewCommands::FitOnScreen);
                Ok(())
            }
            serde_json::Value::Number(percent) => match percent.as_f64().filter(|p| p.is_finite() && *p > 0.0) {
                Some(percent) => {
                    self.view.set_device_zoom(percent / 100.0);
                    Ok(())
                }
                None => Err(FieldRefusal::Rejects(value).message("view")),
            },
            _ => Err(FieldRefusal::Rejects(value).message("view")),
        }
    }
}

/// The clipping overlay an Alt-drag of a tone control shows: Exposure and Whites the highlights, Blacks the shadows.
fn diagnostic_of(before: &RawDevelopSettings, after: &RawDevelopSettings) -> Option<ClippingDiagnostic> {
    if after.blacks != before.blacks {
        Some(ClippingDiagnostic::Shadows)
    } else if after.whites != before.whites || after.exposure != before.exposure {
        Some(ClippingDiagnostic::Highlights)
    } else {
        None
    }
}

/// The boolean an action field asks for.
fn flag(field: &str, value: &serde_json::Value) -> Result<bool, String> {
    value.as_bool().ok_or_else(|| FieldRefusal::Rejects(value).message(field))
}

impl FullWindowWorkspace for RawDevelopWorkspace {
    type Source = dyn RawDevelopSource;
    type Event = RawDevelopEvent;
    type Result = RawDevelopResult;

    const KEY: &'static str = "filter.cameraRaw";

    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<Self::Event, Self::Result> {
        let WorkspaceCx { source, revision, open, hold_ok, ok_waits, .. } = cx;
        let env = Env::of(ui);
        let words = env.words::<RawDevelopWords>();
        let common = env.words::<CommonWords>();
        let ctx = ui.ctx().clone();
        let alt = craft_ui::runtime::pointer::mods(&ctx).alt;
        self.open_scope(&ctx, source);

        let name = if self.target == RawTarget::OpenRaw { words.name.clone() } else { words.name_filter.clone() };
        let title: Label = FullWindowTitle::new(name).subject(self.subject.clone()).zoom(self.view.zoom_percent()).label(&env);
        // A held OK files its answer for the app and keeps the workspace open until the app answers; the buttons that finish wait meanwhile.
        let finishing = |button: FooterButton| button.stays_open(hold_ok).enabled(!ok_waits);
        let mut plan =
            vec![(Footer::Finish(self.target.accept()), finishing(FooterButton::new(self.target_label(&words, &common), DialogRole::Accept)))];
        if self.target == RawTarget::OpenRaw {
            plan.push((Footer::Finish(RawFinish::Done), finishing(FooterButton::new(words.done.clone(), DialogRole::Other))));
        }
        plan.push((Footer::Save, FooterButton::new(words.save_settings.clone(), DialogRole::Other).stays_open(true)));
        plan.push((Footer::Load, FooterButton::new(words.load_settings.clone(), DialogRole::Other).stays_open(true)));
        plan.push((Footer::Cancel, FooterButton::new(common.cancel.clone(), DialogRole::Cancel)));
        let (kinds, buttons): (Vec<Footer>, Vec<FooterButton>) = plan.into_iter().unzip();

        let settings_hash_key = settings_hash(&[format!("{:?}", (&self.settings, self.demosaic)).as_bytes()]);
        let can_step = (self.history.can_undo(), self.history.can_redo());
        let actions = RefCell::new(Vec::<Action>::new());
        let ws = RefCell::new(&mut *self);

        let shown = FullWindowDialog::new(title)
            .own_window(WindowSpec::dialog())
            .remember(SaveKey::new("craft_imaging.raw_develop.window"))
            .opened(open)
            .command_scope(RawDevelopCommands::SCOPE)
            .buttons(buttons)
            .sides(|ui, _| {
                let mut guard = ws.borrow_mut();
                let w = &mut **guard;
                let settings = w.settings.clone();
                let demosaic = w.demosaic;
                let original = w.original.clone();
                let (before, floating_now) = (w.before, w.floating);
                let (at, has_selection) = (w.hover, source.has_selection());
                FullWindowSide::new(words.settings.clone()).show(ui, |ui, _| {
                    let mut floating = floating_now;
                    if let Some(state) = w.scope.as_mut()
                        && !floating_now
                    {
                        let region = state.settings.selected_region;
                        let mut out = None;
                        source.scope(before, region, at, &mut |frame| {
                            let strip = RawScopeStrip::new(&mut *state, frame.as_ref(), Bind::given(settings.clone()))
                                .floating(Bind::bound(&mut floating))
                                .has_selection(has_selection)
                                .show(ui);
                            out = Some(strip);
                        });
                        // A source that shows no frame still gets its strip.
                        let strip = out.unwrap_or_else(|| {
                            RawScopeStrip::new(&mut *state, None, Bind::given(settings.clone()))
                                .floating(Bind::bound(&mut floating))
                                .has_selection(has_selection)
                                .show(ui)
                        });
                        actions.borrow_mut().extend(strip.edit.events.into_iter().map(|event| Action::Edit(Box::new(event))));
                    }
                    let mut panel = RawDevelopPanel::new(Bind::given(settings.clone()).reset(original.clone()));
                    if let Some(demosaic) = demosaic {
                        panel = panel.demosaic(demosaic);
                    }
                    let out = panel.show(ui);
                    let mut queued = actions.borrow_mut();
                    queued.extend(out.edit.events.into_iter().map(|event| Action::Edit(Box::new(event))));
                    if out.auto {
                        queued.push(Action::Auto);
                    }
                    if let Some(next) = out.demosaic {
                        w.demosaic = Some(next);
                    }
                    w.floating = floating;
                });
            })
            .show(ui, |ui, cx| {
                let mut guard = ws.borrow_mut();
                let w = &mut **guard;
                while let Some(step) = cx.take_step(can_step.0, can_step.1) {
                    w.end_gesture(&ctx, CancelReason::UndoDuringGesture);
                    actions.borrow_mut().push(Action::Step(step));
                }
                let mut view = w.view;
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    view.apply(command);
                }
                w.view = view;
                while let Some((command, fired)) = cx.take_command::<RawDevelopCommands>() {
                    match (command, fired.phase) {
                        (RawDevelopCommands::Before, FirePhase::Press) => w.before = true,
                        (RawDevelopCommands::Before, FirePhase::Release) => w.before = false,
                        (RawDevelopCommands::SamplerTool, FirePhase::Press) => {
                            w.end_gesture(&ctx, CancelReason::Removed);
                            w.tool = RawTool::Sampler;
                        }
                        (RawDevelopCommands::Hand, FirePhase::Press) => {
                            w.end_gesture(&ctx, CancelReason::Removed);
                            w.tool = RawTool::Hand;
                        }
                        (RawDevelopCommands::Zoom, FirePhase::Press) => {
                            w.end_gesture(&ctx, CancelReason::Removed);
                            w.tool = RawTool::Zoom;
                        }
                        _ => {}
                    }
                }

                // One develop for each settings, demosaic, visible part and zoom: Before and a samplers change start none.
                let (visible, zoom) = w.visible();
                let key = (revision, settings_hash_key, format!("{visible:?}"), zoom.to_bits());
                let request = RawPreviewRequest::new(revision, w.settings.clone(), w.demosaic, visible, zoom);
                let status = cx.pending(key, || source.develop(&request)).clone();
                if let Pending::Ready(done) = &status {
                    w.last_done = Some((done.timing, done.backend.clone()));
                }
                let progress = FullWindowStatus::of(&status).progress();
                let note: Option<Label> = w.status.clone().or_else(|| {
                    w.last_done.as_ref().map(|(timing, backend)| {
                        let milliseconds = timing.as_millis().to_string();
                        match backend {
                            Some(backend) => {
                                Label::from(words.preview_ms_backend.fill(&[("n", milliseconds.as_str()), ("backend", backend.as_str())]))
                            }
                            None => Label::from(words.preview_ms.fill(&[("n", milliseconds.as_str())])),
                        }
                    })
                });

                let raster: &dyn RasterSource = if w.before { source.before() } else { source.after() };
                let state_settings = w.scope.as_ref().map(|state| state.settings.clone()).unwrap_or_default();
                let mode = state_settings.clipping_mode(w.diagnostic);
                let mode = (mode != ClippingMode::None).then_some(mode);
                let readouts = if w.samplers.points().is_empty() {
                    None
                } else {
                    let rev = settings_hash(&[&revision.to_le_bytes(), &settings_hash_key.to_le_bytes()]);
                    Some(preview::sampler_readouts(&w.samplers, source, w.before, rev, state_settings.lab_readouts, &mut w.sampler_cache))
                };
                let carried = w.sampler_tool.carried();
                let size = [w.size.width, w.size.height];
                let navigation = w.navigation;
                let checkerboard = w.checkerboard;
                let tool = w.tool;
                let show_strip = w.filmstrip.len() > 1;
                let mut current = w.current;
                let mut picked = None;
                let mut hover = None;
                let mut canvas_id = None;

                Stack::new().fill().command_scope(ScopeCommand::SCOPE).show(ui, |ui, scope_cx| {
                    while let Some((command, _)) = scope_cx.take_command::<ScopeCommand>() {
                        if let Some(state) = w.scope.as_mut() {
                            ScopePanel::run_command(state, command);
                        }
                    }
                    Row::new().fill().show(ui, |ui, _| {
                        if show_strip {
                            let _ = RawFilmstrip::new(&w.filmstrip, Bind::bound(&mut current)).show(ui);
                            if current != w.current {
                                picked = Some(current);
                            }
                        }
                        Stack::new().fill().show(ui, |ui, _| {
                            let mut bar = FullWindowZoomBar::new(Bind::bound(&mut view)).progress(progress);
                            if let Some(note) = note.clone() {
                                bar = bar.note(note);
                            }
                            bar.show(ui);
                            if let Some(readouts) = &readouts {
                                Row::new().show(ui, |ui, _| {
                                    for readout in readouts.iter() {
                                        let text = match readout.lab {
                                            Some([l, a, b]) => Label::from(words.sampler_readout_lab.fill(&[
                                                ("n", readout.number.to_string().as_str()),
                                                ("l", l.to_string().as_str()),
                                                ("a", a.to_string().as_str()),
                                                ("b", b.to_string().as_str()),
                                            ])),
                                            None => Label::from(words.sampler_readout.fill(&[
                                                ("n", readout.number.to_string().as_str()),
                                                ("r", readout.rgb[0].to_string().as_str()),
                                                ("g", readout.rgb[1].to_string().as_str()),
                                                ("b", readout.rgb[2].to_string().as_str()),
                                            ])),
                                        };
                                        Text::new(text).role(TypeRole::Caption).show(ui);
                                    }
                                });
                            }
                            let samplers = &w.samplers;
                            let clipping_label = words.clipping.clone();
                            let samplers_label = words.samplers.clone();
                            // The mask the source hands out is held for the pass; the overlay borrows it.
                            let clipping = mode.and_then(|mode| preview::clipping_state(source, mode).map(|held| (held, mode)));
                            let controls = move |ui: &mut Ui, rect: egui::Rect| {
                                if let Some((held, mode)) = &clipping {
                                    preview::clipping_overlay(clipping_label.clone(), held, *mode).show_at(ui, rect);
                                }
                                let marks = ColorSamplerMarks::new(samplers_label.clone(), samplers);
                                let marks = match carried {
                                    Some(carried) => marks.carried(carried),
                                    None => marks,
                                };
                                marks.show_at(ui, rect);
                            };
                            let canvas = FullWindowCanvas::new(Bind::bound(&mut view), raster).navigation(navigation).controls(controls);
                            let canvas = match checkerboard {
                                Some(checker) => canvas.checkerboard(checker),
                                None => canvas,
                            };
                            let out = match tool {
                                RawTool::Hand => canvas.show(ui),
                                RawTool::Zoom => {
                                    let zoom = w.zoom_tool.get_or_insert_with(|| ZoomTool::new(navigation));
                                    zoom.options = navigation;
                                    canvas.tool(zoom).show(ui)
                                }
                                RawTool::Sampler => {
                                    let mut pass = w.sampler_tool.pass(&w.samplers, size);
                                    canvas.tool(&mut pass).show(ui)
                                }
                            };
                            canvas_id = Some(out.response.id);
                            hover = out.hover.map(|point| [point.x, point.y]);
                        });
                    });
                });
                w.view = view;
                w.canvas = canvas_id;
                w.hover = hover;
                w.take_sampler_events();
                if let Some(index) = picked {
                    actions.borrow_mut().push(Action::Show(index));
                }

                // The floating scope is a child dialog of the workspace; it shows the same state the strip does.
                if w.floating {
                    let settings = w.settings.clone();
                    let (before, at, has_selection) = (w.before, w.hover, source.has_selection());
                    let mut floating = w.floating;
                    if let Some(state) = w.scope.as_mut() {
                        let region = state.settings.selected_region;
                        let mut out = None;
                        source.scope(before, region, at, &mut |frame| {
                            out = Some(
                                RawFloatingScope::new(&mut *state, frame.as_ref(), Bind::bound(&mut floating), Bind::given(settings.clone()))
                                    .has_selection(has_selection)
                                    .show(ui),
                            );
                        });
                        if let Some(float) = out {
                            actions.borrow_mut().extend(float.edit.events.into_iter().map(|event| Action::Edit(Box::new(event))));
                        }
                    }
                    w.floating = floating;
                }
            });

        let output = shown.output;
        let mut events = std::mem::take(&mut self.queued);
        for step in actions.borrow().iter().filter_map(|action| if let Action::Step(step) = action { Some(*step) } else { None }) {
            events.extend(self.step(step, Arc::from(words.name.as_str())));
        }
        for action in actions.into_inner() {
            match action {
                Action::Edit(event) => events.push(self.take_edit(*event, alt)),
                Action::Auto => self.run_auto(source, Arc::from(words.auto.as_str())),
                Action::Show(index) => events.push(RawDevelopEvent::Show(index)),
                Action::Step(_) => {}
            }
        }
        events.append(&mut self.queued);
        self.save_scope(&ctx);

        let footer = output.picked.and_then(|index| kinds.get(index)).copied();
        match footer {
            Some(Footer::Save) => events.push(RawDevelopEvent::SaveSettings),
            Some(Footer::Load) => events.push(RawDevelopEvent::LoadSettings),
            _ => {}
        }
        let finish = match footer {
            Some(Footer::Finish(finish)) => finish,
            _ => self.target.accept(),
        };
        let (result, held) = match (self.requested.take(), hold_ok) {
            (Some(finish), true) => (None, Some(self.result(finish))),
            (Some(finish), false) => (Some(DialogResult::Ok(self.result(finish))), None),
            // Esc and Cancel close the workspace either way.
            (None, true) => (output.closed.map(|_| DialogResult::Cancel), matches!(footer, Some(Footer::Finish(_))).then(|| self.result(finish))),
            (None, false) => (output.result(|| self.result(finish)), None),
        };
        WorkspacePass::new(output.response, events).held(held).result(result).canvas(self.canvas)
    }

    fn json_fields(&self, source: &Self::Source) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.settings)?;
        if let serde_json::Value::Object(map) = &mut fields {
            map.insert("before".to_owned(), serde_json::Value::Bool(self.before));
            map.insert("demosaic".to_owned(), serde_json::to_value(self.demosaic).unwrap_or(serde_json::Value::Null));
            let view = if self.view.is_fit() { serde_json::Value::from("fit") } else { serde_json::Value::from(self.view.zoom_percent()) };
            map.insert("view".to_owned(), view);
            let scope = self.scope.as_ref().map(|state| state.settings.clone()).unwrap_or_default();
            let lab = scope.lab_readouts;
            map.insert("scope".to_owned(), serde_json::to_value(scope).unwrap_or(serde_json::Value::Null));
            let read = |at: [f64; 2]| preview::readout_json(source, at, self.before, lab);
            map.insert("pointerReadout".to_owned(), self.hover.map_or(serde_json::Value::Null, read));
            map.insert("samplerReadouts".to_owned(), self.samplers.points().iter().copied().map(read).collect());
        }
        Some(fields)
    }

    fn set_json_field(&mut self, ctx: &Context, source: &Self::Source, field: &str, value: serde_json::Value) -> Result<(), String> {
        let words = Env::of_root(ctx).words::<RawDevelopWords>();
        match field {
            "auto" => {
                if flag(field, &value)? {
                    self.run_auto(source, Arc::from(words.auto.as_str()));
                }
                Ok(())
            }
            "undo" | "redo" => {
                if flag(field, &value)? {
                    let step = if field == "undo" { Step::Undo } else { Step::Redo };
                    self.end_gesture(ctx, CancelReason::UndoDuringGesture);
                    let events = self.step(step, Arc::from(words.name.as_str()));
                    self.queued.extend(events);
                }
                Ok(())
            }
            "done" => {
                let on = flag(field, &value)?;
                if self.target != RawTarget::OpenRaw {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                if on {
                    self.requested = Some(RawFinish::Done);
                }
                Ok(())
            }
            "saveSettings" | "loadSettings" => {
                if flag(field, &value)? {
                    self.queued.push(if field == "saveSettings" { RawDevelopEvent::SaveSettings } else { RawDevelopEvent::LoadSettings });
                }
                Ok(())
            }
            "before" => {
                self.before = flag(field, &value)?;
                Ok(())
            }
            "demosaic" => {
                if self.demosaic.is_none() {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                self.demosaic = Some(serde_json::from_value(value.clone()).map_err(|_| FieldRefusal::Rejects(&value).message(field))?);
                Ok(())
            }
            "view" => self.set_view(&value),
            "scope" => {
                let settings: ScopeSettings = serde_json::from_value(value.clone()).map_err(|_| FieldRefusal::Rejects(&value).message(field))?;
                self.scope.get_or_insert_with(ScopePanelState::default).settings = settings;
                self.save_scope(ctx);
                Ok(())
            }
            _ => {
                let mut next = self.settings.clone();
                set_serde_field(&mut next, field, value)?;
                next.validate_changes(&self.settings)?;
                // The step is named as the panel names the row that edits the field; a field no one row edits takes
                // the workspace's name.
                let name: Arc<str> = Arc::from(controls::field_word(&words, field).unwrap_or(&words.name).as_str());
                self.apply(next, name);
                Ok(())
            }
        }
    }

    fn confirm(&mut self, ctx: &Context, source: &Self::Source) -> Option<Self::Result> {
        let _ = (ctx, source);
        self.settings.validate_changes(&self.stored).ok()?;
        Some(self.result(self.target.accept()))
    }
}

impl RawDevelopWorkspace {
    /// The label of the default footer button: OK for a filter, Open for raws being opened.
    fn target_label(&self, words: &RawDevelopWords, common: &CommonWords) -> Label {
        match self.target {
            RawTarget::OpenRaw => words.open.clone(),
            RawTarget::Filter | RawTarget::SmartFilter => common.ok.clone(),
        }
    }
}
