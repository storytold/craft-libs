//! The Filter Gallery workspace (Filter › Filter Gallery…), Photoshop's layout.
//!
//! [`FilterGalleryWorkspace`] is a [`FullWindowDialog`] with the title strip "Filter Gallery (layer, 66%)": the
//! preview canvas at the start with a [`FullWindowZoomBar`] under it, and two columns at the end edge: the
//! category folders in the inner one ([`FilterTree`]) and the settings in the outer one, with OK and Cancel at
//! its top (`SideFooter::top`), the Filter select and the effect's parameters ([`EffectParameters`]), the
//! effect-layer stack ([`EffectStack`]) and the preview's status. No tool strip and no options bar; a drag on
//! the preview pans (the Hand tool is the canvas's only tool).
//!
//! The session is the stack ([`FilterGallerySession`]) with [`EditHistory`] undo. The app's
//! [`FilterGallerySource`] holds its catalog ([`FilterCatalog`]), which names the filters and their parameters,
//! the thumbnail source that renders the folders' pictures, and the preview of the visible part
//! ([`FilterGalleryWorkspace::visible`]), rendered as a job whenever the stack, the zoom, the visible part or the
//! source change. The app hosts the workspace as a `HostedWorkspace`; OK runs `filter.filterGallery` with the
//! effects; a smart filter opened again passes its recorded effects.
//!
//! Files: `catalog` and `session`; `tree`, `stack`, `parameters` and this file's body.
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`FullWindowZoomBar`]: craft_ui::dialog::FullWindowZoomBar
//! [`EditHistory`]: craft_behavior::EditHistory

mod catalog;
pub(crate) mod parameters;
mod session;
pub(crate) mod stack;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub(crate) mod tree;
mod words;

pub use catalog::{FilterCatalog, FilterCategory, FilterGallerySource, FilterIndex, GalleryFilter, GalleryThumb};
pub use parameters::{EffectParameters, EffectParametersOutput};
pub use session::{EffectChange, EffectLayer, FilterGallerySession, LayerId, SessionField};
pub use stack::{EffectStack, EffectStackOutput};
pub use tree::{FilterTree, FilterTreeLayout, FilterTreeOutput};
pub use words::FilterGalleryWords;

use std::cell::RefCell;
use std::sync::Arc;

use craft_behavior::edit_history::Step;
use craft_behavior::kurbo::Rect;
use craft_behavior::{CanvasView, EditEvent, EditHistory, Mixed, PixelSize};
use craft_theme::Srgba;
use craft_ui::canvas::{NavigationOptions, PixelRect, ViewCommands};
use craft_ui::contract::{Bind, Commands as _, Pattern, Pending, Remember as _, ShowContainer as _};
use craft_ui::dialog::{
    FieldRefusal, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView, FullWindowWorkspace,
    FullWindowZoomBar, SideFooter, WorkspaceCx, WorkspacePass,
};
use craft_ui::patterns::form_dialogs::FieldValue;
use craft_ui::primitives::Checker;
use craft_ui::primitives::textures::{ThumbnailOptions, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::{ColorRequest, PathRequest};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{Context, Ui};
use serde_json::{Value, json};

use crate::ui::wide_angle::PreviewDone;

/// What opens the workspace.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FilterGalleryOpen {
    /// The layer's size in document pixels.
    pub size: PixelSize,
    /// The layer's name: the title's subject.
    pub subject: String,
    /// The stack to start from: one layer of the last filter used, or a smart filter's recorded
    /// effects.
    pub session: FilterGallerySession,
    /// The user's navigation options (wheel, flick, animated zoom); Photoshop's by default.
    pub navigation: NavigationOptions,
    /// The user's transparency grid under the preview; none draws the picture on the canvas fill.
    pub checkerboard: Option<Checker>,
}

impl FilterGalleryOpen {
    /// Opens on a layer of `size` document pixels named `subject`, starting from `session`; navigation and checkerboard take their defaults.
    pub fn new(size: PixelSize, subject: impl Into<String>, session: FilterGallerySession) -> Self {
        Self { size, subject: subject.into(), session, navigation: NavigationOptions::default(), checkerboard: None }
    }

    /// The user's navigation options.
    #[must_use]
    pub fn navigation(mut self, navigation: NavigationOptions) -> Self {
        self.navigation = navigation;
        self
    }

    /// The user's transparency grid under the preview.
    #[must_use]
    pub fn checkerboard(mut self, checkerboard: Checker) -> Self {
        self.checkerboard = Some(checkerboard);
        self
    }
}

/// What the workspace reports, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FilterGalleryEvent {
    /// The stack changed (recorded): the app may note it.
    Stack(EffectChange),
    /// Another effect layer selected.
    Select(usize),
    /// A colour parameter's swatch was pressed: the app opens its picker and answers with `set_color`.
    PickColor(ColorRequest),
    /// A Browse button was pressed on the layer `layer`: the app opens its file picker and answers with
    /// `set_layer_fields`, which follows the layer if it moved meanwhile.
    #[non_exhaustive]
    PickPath {
        /// The layer whose parameter asked.
        layer: LayerId,
        /// The parameter's key and what the picker offers.
        request: PathRequest,
    },
}

/// The undo step name of every stack change: the workspace's name.
fn history_name(words: &FilterGalleryWords) -> Arc<str> {
    words.name.clone().as_str().into()
}

/// The Filter Gallery workspace, hosted as a [`FullWindowWorkspace`] over the app's [`FilterGallerySource`]: its
/// pass reports [`FilterGalleryEvent`]s, and OK answers the session.
///
/// Keys: the zoom commands zoom the preview, never the canvas behind; ⌘Z and ⇧⌘Z step the stack's history (a field
/// being edited steps its own text first); Enter OK, Esc Cancel.
///
/// Automation fields: `effects` (the stack as `[{filter, params, visible}]`), `select` (the selected layer's
/// index) and `zoom` (`"fit"` or the zoom, 1 = 100 %), which it lists and sets; and the actions `add` and `delete`
/// (`true`), `toggle` (a layer's index: its eye flips), `move` (`{from, to}`), `filter` (a filter key for the
/// selected layer) and `params` (values merged into the selected layer's parameters). Each change is one undo step
/// and is reported by the next pass.
#[derive(Pattern)]
pub struct FilterGalleryWorkspace {
    session: FilterGallerySession,
    history: EditHistory<FilterGallerySession>,
    subject: String,
    size: PixelSize,
    // View state outside the history.
    selected: usize,
    /// The selection the app has been told of (the open one, then each `Select` event).
    reported: usize,
    /// The user's navigation options and transparency grid.
    navigation: NavigationOptions,
    checkerboard: Option<Checker>,
    /// Counts the changes of the stack: part of the preview job's key.
    revision: u64,
    /// The canvas view of the last pass: what `visible` reads.
    shown: Option<CanvasView>,
    /// The zoom on screen as a percentage and as screen pixels per document pixel, of the last pass.
    percent: f64,
    zoom: f64,
    /// The preview's view: the body returns it each pass and `show` stores it after the dialog.
    view: FullWindowView,
    /// The preview job of the last pass, for the status line.
    status: Pending<PreviewDone, String>,
    /// Events from calls outside `show`, reported by the next pass.
    queued: Vec<FilterGalleryEvent>,
}

impl FilterGalleryWorkspace {
    /// A workspace on the stack and layer that `open` describes.
    pub fn new(open: FilterGalleryOpen) -> Self {
        let selected = open.session.effects.len().saturating_sub(1);
        Self {
            history: EditHistory::new(&open.session),
            selected,
            reported: selected,
            navigation: open.navigation,
            checkerboard: open.checkerboard,
            session: open.session,
            subject: open.subject,
            size: open.size,
            revision: 0,
            shown: None,
            percent: 100.0,
            zoom: 1.0,
            view: FullWindowView::new(),
            status: Pending::default(),
            queued: Vec::new(),
        }
    }

    /// The effect-layer stack as it is now.
    pub fn session(&self) -> &FilterGallerySession {
        &self.session
    }

    /// The index of the selected effect layer.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The part of the document the preview shows, in document pixels widened by the filters' reach, and the
    /// zoom (screen pixels per document pixel): what the app's job renders.
    pub fn visible(&self) -> (PixelRect, f64) {
        let image = Rect::new(0.0, 0.0, f64::from(self.size.width), f64::from(self.size.height));
        let part = self.shown.map_or(image, |view| view.visible().intersect(image)).expand();
        (PixelRect::new(part.x0 as i32, part.y0 as i32, part.x1 as i32, part.y1 as i32), self.zoom)
    }

    /// The color the app picked for a [`FilterGalleryEvent::PickColor`] request: written into the request's
    /// `key` of the selected effect (one undo step).
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        let index = self.selected.min(self.session.effects.len().saturating_sub(1));
        let before = self.session.clone();
        let Some(effect) = self.session.effects.get_mut(index) else { return };
        let mut fields = effect.fields.clone();
        fields.set(key, FieldValue::Color(color.to_rgb_array()));
        if fields == effect.fields {
            return;
        }
        effect.fields = fields.clone();
        self.record(before, &Arc::from(key));
        self.queued.push(FilterGalleryEvent::Stack(EffectChange::with_fields(index, fields)));
    }

    /// The answer to a [`FilterGalleryEvent::PickPath`]: writes `entries` into the fields of the layer `layer` as one
    /// undo step. Returns `false`, changing nothing, once the layer is deleted.
    pub fn set_layer_fields(&mut self, layer: LayerId, entries: impl IntoIterator<Item = (String, FieldValue)>) -> bool {
        let Some(at) = self.session.position(layer) else { return false };
        let before = self.session.clone();
        let Some(effect) = self.session.effects.get_mut(at) else { return false };
        let mut fields = effect.fields.clone();
        let mut name = None;
        for (key, value) in entries {
            name.get_or_insert_with(|| Arc::<str>::from(key.as_str()));
            fields.set(key, value);
        }
        let Some(name) = name.filter(|_| fields != effect.fields) else { return true };
        effect.fields = fields.clone();
        self.record(before, &name);
        self.queued.push(FilterGalleryEvent::Stack(EffectChange::with_fields(at, fields)));
        true
    }

    /// Undoes or redoes, keeping the selection on a layer that remains. The pass reports the selection as a `Select`
    /// event when it moved.
    fn step(&mut self, step: Step) {
        let next_layer = self.session.next_layer;
        if self.history.step(&mut self.session, step) {
            self.session.next_layer = next_layer.max(self.session.next_layer);
            self.selected = self.selected.min(self.session.effects.len().saturating_sub(1));
            self.revision += 1;
        }
    }

    /// Records the session as one undo step named `name`, after `before`.
    fn record(&mut self, before: FilterGallerySession, name: &Arc<str>) {
        self.revision += 1;
        self.history.record(&EditEvent::set(Mixed::Same(before), self.session.clone(), name.clone(), None));
    }

    /// Applies `change` through `catalog`, records it as one undo step named `name` and queues its report for the
    /// next pass to give: the one path of every stack change, from the columns and from automation. Returns whether
    /// the stack changed.
    fn change(&mut self, change: EffectChange, catalog: &dyn FilterCatalog, name: &Arc<str>) -> bool {
        let before = self.session.clone();
        let mut selected = self.selected;
        if !self.session.apply(change.clone(), catalog, &mut selected) {
            return false;
        }
        self.selected = selected;
        self.record(before, name);
        self.queued.push(FilterGalleryEvent::Stack(change));
        true
    }

    /// The selected layer's index, kept on a layer that exists.
    fn clamped_selection(&self) -> usize {
        self.selected.min(self.session.effects.len().saturating_sub(1))
    }

    /// Sets the automation field `field` to `value` over `catalog`; `Err` says why it refused.
    fn set_field(&mut self, catalog: &dyn FilterCatalog, name: &Arc<str>, field: &str, value: Value) -> Result<(), String> {
        if field == "zoom" {
            match (value.as_str(), value.as_f64()) {
                (Some("fit"), _) => self.view.apply(ViewCommands::FitOnScreen),
                (_, Some(zoom)) if zoom.is_finite() && zoom > 0.0 => self.view.set_device_zoom(zoom),
                _ => return Err(FieldRefusal::Rejects(&value).message(field)),
            };
            return Ok(());
        }
        match self.session.field_change(catalog, self.clamped_selection(), field, &value)? {
            Some(SessionField::Select(at)) => self.selected = at,
            Some(SessionField::Change(change)) => {
                self.change(change, catalog, name);
            }
            None => {}
        }
        Ok(())
    }
}

impl FullWindowWorkspace for FilterGalleryWorkspace {
    type Source = dyn FilterGallerySource;
    type Event = FilterGalleryEvent;
    type Result = FilterGallerySession;

    const KEY: &'static str = "filter.filterGallery";

    /// Shows the workspace for this pass: the title (the selected effect's filter name as the subject's prefix,
    /// PhotoCraft's "{effect} (layer, 66%)"), the side columns at the end edge (settings with the top footer
    /// outermost, then the folders with the source's thumbnails), the zoom bar (progress while the preview renders,
    /// else the render's time), and the canvas over the source's preview with the Hand tool. Changes go through
    /// [`FilterGallerySession::apply`] and are recorded in the history; a change, a zoom, a pan or a new source
    /// starts a new preview job. A swatch press reports [`FilterGalleryEvent::PickColor`].
    ///
    /// OK closes at once: the gallery never holds its OK (`cx.hold_ok` is not read).
    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<FilterGalleryEvent, FilterGallerySession> {
        let WorkspaceCx { source, revision: source_revision, open, .. } = cx;
        let (catalog, thumb_source) = (source.catalog(), source.thumb_source());
        let thumbs = Thumbnails::<GalleryThumb>::in_context(ui.ctx(), "filterGallery.thumbs", ThumbnailOptions::default());
        let env = Env::of(ui);
        let words = env.words::<FilterGalleryWords>();
        let common = env.words::<CommonWords>();
        let index = FilterIndex::of(catalog);
        let mut events = std::mem::take(&mut self.queued);
        self.selected = self.clamped_selection();
        let name = self.session.effects.get(self.selected).map_or_else(|| words.name.clone(), |effect| index.label(&effect.filter));
        let title = FullWindowTitle::new(name).subject(self.subject.clone()).zoom(self.percent).label(&env);
        let buttons = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let history_name = history_name(&words);
        let navigation = self.navigation;
        let checkerboard = self.checkerboard;
        let shared = RefCell::new(&mut *self);

        let mut dialog = FullWindowDialog::new(title)
            .opened(open)
            .buttons(buttons)
            .side_footer(SideFooter::top())
            .sides(|ui, _| {
                let mut workspace = shared.borrow_mut();
                let current = workspace.selected;
                let mut changes = Vec::new();
                let mut stack_pick = None;
                let mut pick_color = None;
                let mut pick_path = None;
                FullWindowSide::new(words.settings.clone()).headerless().show(ui, |ui, _| {
                    if let Some(effect) = workspace.session.effects.get(current) {
                        let parameters = EffectParameters::new(current, effect, &index).show(ui);
                        changes.extend(parameters.change);
                        pick_color = parameters.pick_color;
                        pick_path = parameters.path;
                    }
                    let stack = EffectStack::new(&workspace.session.effects, current, &index).show(ui);
                    stack_pick = stack.selected;
                    changes.extend(stack.change);
                    FullWindowStatus::of(&workspace.status).show(ui);
                });
                if let Some(request) = pick_color {
                    workspace.queued.push(FilterGalleryEvent::PickColor(request));
                }
                if let Some(request) = pick_path
                    && let Some(layer) = workspace.session.effects.get(current).map(|effect| effect.id)
                {
                    workspace.queued.push(FilterGalleryEvent::PickPath { layer, request });
                }
                for change in changes {
                    workspace.change(change, catalog, &history_name);
                }
                if let Some(pick) = stack_pick {
                    workspace.selected = pick.min(workspace.session.effects.len().saturating_sub(1));
                }
                let current_filter = workspace.session.effects.get(workspace.selected).map(|effect| effect.filter.clone()).unwrap_or_default();
                let mut tree_pick = None;
                FullWindowSide::new(words.filters.clone()).headerless().show(ui, |ui, _| {
                    tree_pick = FilterTree::new(&index, &current_filter, &thumbs, thumb_source).show(ui).picked;
                });
                if let Some(filter) = tree_pick {
                    let at = workspace.selected;
                    workspace.change(EffectChange::set_filter(at, filter), catalog, &history_name);
                }
            })
            .show(ui, |ui, cx| {
                let mut workspace = shared.borrow_mut();
                let workspace = &mut **workspace;
                while let Some(step) = cx.take_step(workspace.history.can_undo(), workspace.history.can_redo()) {
                    workspace.step(step);
                }
                let mut view = workspace.view;
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    view.apply(command);
                }
                let (visible, zoom) = workspace.visible();
                // A new source (a document edit under the open gallery) previews again, and a late preview of the old
                // pixels lands under a key no pass asks for.
                let key = (workspace.revision, source_revision, visible, zoom.to_bits());
                let effects = &workspace.session.effects;
                let status = cx.pending(key, || source.render(effects, visible, zoom)).clone();
                let mut bar = FullWindowZoomBar::new(Bind::bound(&mut view)).progress(FullWindowStatus::of(&status).progress());
                if let Pending::Ready(done) = &status {
                    let milliseconds = done.timing.as_millis().to_string();
                    bar = bar.note(Label::from(words.preview_ms.fill(&[("n", milliseconds.as_str())])));
                }
                bar.show(ui);
                let mut canvas = FullWindowCanvas::new(Bind::bound(&mut view), source.preview()).navigation(navigation);
                if let Some(checker) = checkerboard {
                    canvas = canvas.checkerboard(checker);
                }
                let canvas = canvas.show(ui).response.id;
                (view, status, canvas)
            });
        // Once per pass, after the real run: the view it settled and the render status are stored.
        let mut canvas = None;
        dialog.settle(|_, (view, status, shown_canvas)| {
            canvas = Some(*shown_canvas);
            let view = std::mem::take(view);
            let mut workspace = shared.borrow_mut();
            workspace.status = status.clone();
            workspace.shown = Some(view.view());
            workspace.percent = view.zoom_percent();
            workspace.zoom = view.device_zoom();
            workspace.view = view;
        });

        let workspace = shared.into_inner();
        events.append(&mut workspace.queued);
        if workspace.selected != workspace.reported {
            workspace.reported = workspace.selected;
            events.push(FilterGalleryEvent::Select(workspace.selected));
        }
        let result = dialog.output.result(|| workspace.session.clone());
        WorkspacePass::new(dialog.output.response, events).result(result).canvas(canvas)
    }

    fn json_fields(&self, _source: &Self::Source) -> Option<Value> {
        let zoom = if self.view.is_fit() { json!("fit") } else { json!(self.view.device_zoom()) };
        let mut fields = self.session.json_fields(self.clamped_selection());
        if let Value::Object(map) = &mut fields {
            map.insert("zoom".to_owned(), zoom);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, ctx: &Context, source: &Self::Source, field: &str, value: Value) -> Result<(), String> {
        let name = history_name(&Env::of_root(ctx).words::<FilterGalleryWords>());
        self.set_field(source.catalog(), &name, field, value)
    }

    fn confirm(&mut self, _: &Context, _: &Self::Source) -> Option<FilterGallerySession> {
        Some(self.session.clone())
    }
}
