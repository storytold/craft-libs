//! The Filter Gallery workspace (Filter › Filter Gallery…), Photoshop's layout.
//!
//! [`FilterGalleryWorkspace`] is a [`FullWindowDialog`] with the title strip "Filter Gallery (layer, 66%)": the
//! preview canvas at the start with a [`FullWindowZoomBar`] under it, and two columns at the end edge: the
//! category folders in the inner one ([`FilterTree`]) and the settings in the outer one, with OK and Cancel at
//! its top (`SideFooter::top`), the Filter select and the effect's parameters ([`EffectParameters`]), the
//! effect-layer stack ([`EffectStack`]) and the preview's status. No tool strip and no options bar; a drag on
//! the preview pans (the Hand tool is the canvas's only tool).
//!
//! The session is the stack ([`FilterGallerySession`]) with [`EditHistory`] undo. The app's catalog
//! ([`FilterCatalog`]) names the filters and their parameters, its thumbnail source renders the folders'
//! pictures, and its [`FilterGallerySource`] renders the preview of the visible part
//! ([`FilterGalleryWorkspace::visible`]) as a job whenever the stack, the zoom or the visible part change. OK
//! runs `filter.filterGallery` with the effects; a smart filter opened again passes its recorded effects.
//!
//! Files: `catalog` and `session`; `tree`, `stack`, `parameters` and this file's body.
//!
//! [`FullWindowDialog`]: craft_ui::dialog::FullWindowDialog
//! [`FullWindowZoomBar`]: craft_ui::dialog::FullWindowZoomBar
//! [`EditHistory`]: craft_behavior::EditHistory

mod catalog;
mod parameters;
mod session;
mod stack;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod tree;
mod words;

pub use catalog::{FilterCatalog, FilterCategory, FilterGallerySource, FilterIndex, GalleryFilter, GalleryThumb};
pub use parameters::{EffectParameters, EffectParametersOutput};
pub use session::{EffectChange, EffectLayer, FilterGallerySession};
pub use stack::{EffectStack, EffectStackOutput};
pub use tree::{FilterTree, FilterTreeOutput};
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
    DialogOutput, FullWindowCanvas, FullWindowDialog, FullWindowSide, FullWindowStatus, FullWindowTitle, FullWindowView, FullWindowZoomBar,
    SideFooter,
};
use craft_ui::patterns::form_dialogs::FieldValue;
use craft_ui::primitives::Checker;
use craft_ui::primitives::textures::{ThumbSource, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::Ui;

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
}

/// What [`FilterGalleryWorkspace::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FilterGalleryOutput {
    /// What happened this pass, in order.
    pub events: Vec<FilterGalleryEvent>,
    /// A color parameter's swatch pressed: the app opens its picker and answers into the selected
    /// effect's fields ([`FilterGalleryWorkspace::set_color`]).
    pub pick_color: Option<ColorRequest>,
    /// The dialog: its response, why it closed and which footer button was picked (0 is OK, 1 is Cancel).
    pub dialog: DialogOutput,
}

impl FilterGalleryOutput {
    /// OK was chosen.
    pub fn accepted(&self) -> bool {
        self.dialog.picked == Some(OK)
    }
}

/// The footer button that accepts.
const OK: usize = 0;

/// What one pass of the two closures shares.
struct Pass<'w> {
    workspace: &'w mut FilterGalleryWorkspace,
    events: Vec<FilterGalleryEvent>,
    pick_color: Option<ColorRequest>,
}

/// The Filter Gallery workspace.
///
/// Keys: the zoom commands zoom the preview, never the canvas behind; Undo and Redo reach the stack's history
/// through [`step`](Self::step); Enter OK, Esc Cancel.
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

    /// Shows the workspace for this pass: the title (the selected effect's filter name as the subject's prefix,
    /// PhotoCraft's "{effect} (layer, 66%)"), the side columns at the end edge (settings with the top footer
    /// outermost, then the folders), the zoom bar (progress while the preview renders, else the render's time),
    /// and the canvas over `source`'s preview with the Hand tool. Changes go through
    /// [`FilterGallerySession::apply`] and are recorded in the history; a change, a zoom or a pan starts a new
    /// preview job of `source`.
    ///
    /// `thumb_source` renders the pictures `thumbs` asks for (one per filter, the filter applied to a small
    /// crop of the layer).
    pub fn show(
        &mut self,
        ui: &mut Ui,
        catalog: &dyn FilterCatalog,
        thumbs: &Thumbnails<GalleryThumb>,
        thumb_source: &dyn ThumbSource<GalleryThumb>,
        source: &dyn FilterGallerySource,
    ) -> FilterGalleryOutput {
        let env = Env::of(ui);
        let words = env.words::<FilterGalleryWords>();
        let common = env.words::<CommonWords>();
        let index = FilterIndex::of(catalog);
        let events = std::mem::take(&mut self.queued);
        self.selected = self.selected.min(self.session.effects.len().saturating_sub(1));
        let name = self.session.effects.get(self.selected).map_or_else(|| words.name.clone(), |effect| index.label(&effect.filter));
        let title = FullWindowTitle::new(name).subject(self.subject.clone()).zoom(self.percent).label(&env);
        let buttons = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let history_name: Arc<str> = words.name.clone().as_str().into();
        let navigation = self.navigation;
        let checkerboard = self.checkerboard;
        let pass = RefCell::new(Pass { workspace: self, events, pick_color: None });

        let mut dialog = FullWindowDialog::new(title)
            .buttons(buttons)
            .side_footer(SideFooter::top())
            .sides(|ui, _| {
                let mut pass = pass.borrow_mut();
                let current = pass.workspace.selected;
                let mut changes = Vec::new();
                let mut stack_pick = None;
                let mut pick_color = None;
                FullWindowSide::new(words.settings.clone()).headerless().show(ui, |ui, _| {
                    if let Some(effect) = pass.workspace.session.effects.get(current) {
                        let parameters = EffectParameters::new(current, effect, &index).show(ui);
                        changes.extend(parameters.change);
                        pick_color = parameters.pick_color;
                    }
                    let stack = EffectStack::new(&pass.workspace.session.effects, current, &index).show(ui);
                    stack_pick = stack.selected;
                    changes.extend(stack.change);
                    FullWindowStatus::of(&pass.workspace.status).show(ui);
                });
                pass.pick_color = pick_color;
                for change in changes {
                    pass.apply(change, catalog, &history_name);
                }
                if let Some(pick) = stack_pick {
                    pass.workspace.selected = pick.min(pass.workspace.session.effects.len().saturating_sub(1));
                }
                let current_filter =
                    pass.workspace.session.effects.get(pass.workspace.selected).map(|effect| effect.filter.clone()).unwrap_or_default();
                let mut tree_pick = None;
                FullWindowSide::new(words.filters.clone()).headerless().show(ui, |ui, _| {
                    tree_pick = FilterTree::new(&index, &current_filter, thumbs, thumb_source).show(ui).picked;
                });
                if let Some(filter) = tree_pick {
                    let at = pass.workspace.selected;
                    pass.apply(EffectChange::set_filter(at, filter), catalog, &history_name);
                }
            })
            .show(ui, |ui, cx| {
                let mut pass = pass.borrow_mut();
                let workspace = &mut *pass.workspace;
                let mut view = workspace.view;
                while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                    view.apply(command);
                }
                let (visible, zoom) = workspace.visible();
                let key = (workspace.revision, visible, zoom.to_bits());
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
                canvas.show(ui);
                (view, status)
            });
        // Once per pass, after the real run: the view it settled and the render status are stored.
        dialog.settle(|_, (view, status)| {
            let view = std::mem::take(view);
            let workspace = &mut *pass.borrow_mut().workspace;
            workspace.status = status.clone();
            workspace.shown = Some(view.view());
            workspace.percent = view.zoom_percent();
            workspace.zoom = view.device_zoom();
            workspace.view = view;
        });

        let Pass { workspace, mut events, pick_color } = pass.into_inner();
        events.append(&mut workspace.queued);
        if workspace.selected != workspace.reported {
            workspace.reported = workspace.selected;
            events.push(FilterGalleryEvent::Select(workspace.selected));
        }
        FilterGalleryOutput { events, pick_color, dialog: dialog.output }
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

    /// The color the app picked for a [`FilterGalleryOutput::pick_color`] request: written into the request's
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

    /// Undo or redo from the app's Edit menu; returns whether it stepped. The next pass clamps the
    /// selection and reports it as a `Select` event when it moved.
    pub fn step(&mut self, step: Step) -> bool {
        if !self.history.step(&mut self.session, step) {
            return false;
        }
        self.selected = self.selected.min(self.session.effects.len().saturating_sub(1));
        self.revision += 1;
        true
    }

    /// Records the session as one undo step named `name`, after `before`.
    fn record(&mut self, before: FilterGallerySession, name: &Arc<str>) {
        self.revision += 1;
        self.history.record(&EditEvent::set(Mixed::Same(before), self.session.clone(), name.clone(), None));
    }
}

impl Pass<'_> {
    /// Applies `change` through the catalog, records it and reports it.
    fn apply(&mut self, change: EffectChange, catalog: &dyn FilterCatalog, name: &Arc<str>) {
        let workspace = &mut *self.workspace;
        let before = workspace.session.clone();
        let mut selected = workspace.selected;
        if workspace.session.apply(change.clone(), catalog, &mut selected) {
            workspace.selected = selected;
            workspace.record(before, name);
            self.events.push(FilterGalleryEvent::Stack(change));
        }
    }
}
