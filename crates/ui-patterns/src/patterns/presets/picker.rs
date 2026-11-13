//! `PresetPicker`: a closed field that shows the current preset and opens a `PresetBrowser` in a popover.
//!
//! The closed field is a `SelectFace` over the field well, as `Select` shows it; the popover is a `Popover` container
//! with the browser. A pick sets the value and closes. A host can put content above the browser (`head`), keep the popover open on a click
//! (`keep_open`), give the browser a footer, set how far the thumbnails zoom, open the popover at a point without the
//! field (`open_at`) and let the user resize it (`resizable`).

#[cfg(feature = "testing")]
mod stories;

use std::sync::Arc;

use super::browser::{BrowserTools, PresetBrowser, ThumbPainter};
use super::event::{PresetEvent, PresetOutput};
use super::footer::PresetFooterOptions;
use super::model::{CellShape, PresetRow, ThumbInfo};
use super::state::{DefaultOpen, PresetState, PresetView};
use super::words::PresetWords;
use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, SaveKey, Show as _, ShowContainer as _, Tooltip, WordSetCx as _,
};
use crate::layout_components::Stack;
use crate::list::RowTest;
use crate::pickers::SelectFace;
use crate::popover::Popover;
use crate::primitives::ContentPainterCx;
use crate::runtime::env::{EnvView, Label};
use crate::runtime::memory::StageAccess;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, FocusLook, StyleCx, SurfaceLook};
use crate::tile_grid::TileZoom;
use craft_behavior::{EditEvent, EditEvents, HitLevel, HitPriority, Intent, Metrics, Mixed, PartKey, PressPhase, Sense, Value};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::hash::Hash;

/// The face under the whole field.
const FACE: PartKey = PartKey::new("face");
/// The thumbnail of the face, when it asks for an editor instead of opening the list.
const THUMB: PartKey = PartKey::new("thumb");

type PickerHeadFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// The content a host puts above the browser in the popover.
pub struct PickerHead<'a>(PickerHeadFn<'a>);

impl<'a> PickerHead<'a> {
    /// The head that runs `content`.
    pub fn new(content: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        PickerHead(Box::new(content))
    }
}

/// What the closed field shows of the current preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PickerField {
    /// The thumbnail alone; the name when the preset has no thumbnail, and an empty thumbnail
    /// with no preset or a mixed value.
    #[default]
    Thumbnail,
    /// The name alone.
    Name,
    /// The thumbnail, then the name.
    ThumbnailAndName,
}

/// The style of a preset picker's field: the field well and the focus edge; the face carries the padding.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PresetPickerStyle {
    /// The field well.
    #[style(surface)]
    surface: SurfaceLook,
    /// No padding of its own: the face carries the field's.
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
}

/// What a preset picker reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the picker's events"]
pub struct PresetPickerOutput<K> {
    /// The field's response and the edit of its value, with the popover's picks.
    pub edit: EditResponse<Option<K>>,
    /// The preset picked in this pass, which the binding now holds.
    pub picked: Option<K>,
    /// What the browser in the popover reported besides the pick and the open groups: renames, deletes, the
    /// context menu's rows and the footer's buttons.
    pub events: Vec<PresetEvent<K>>,
    /// The pick closed the popover (a double click or Enter); false for a click while `keep_open` holds it open.
    pub closed: bool,
    /// How the popover closed in this pass (a pick is `Activated`, Escape is `Escape`); `None` while it stays open
    /// or stays closed.
    pub closed_as: Option<CloseReason>,
    /// Whether the popover is open after this pass.
    pub open: bool,
    /// The thumbnail of the face was pressed (with `edit_face`): the host opens its editor.
    pub edit_requested: bool,
    /// The row `PresetPicker::create_row` added was chosen in this pass; the popover has closed.
    pub create: bool,
    /// The row `PresetPicker::detach_row` added was chosen in this pass; the popover has closed.
    pub detach: bool,
}

impl<K: Value> Outcome for PresetPickerOutput<K> {
    type Value = Option<K>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// A field that shows the current preset and opens its presets. A click, Enter, Space or Alt+Down opens the popover
/// with the browser; a click or Enter picks a preset and closes, and Escape closes and keeps the value.
///
/// - **Value.** The binding holds the picked preset's key, or none while the value is no preset of `rows`; the field
///   then shows `placeholder`, "Custom" by default.
/// - **State.** `state` is the browser's view settings; bind it with `Bind::bound`.
/// - **Thumbnails.** Each row's `preview` paints its thumbnail, in the field and in the browser.
/// - **Popover.** A `Popover` container: it hangs from the field, or from a point with [`open_at`](Self::open_at),
///   and with [`resizable`](Self::resizable) its edge grips resize it and the size is saved.
///
/// ```ignore
/// let out = PresetPicker::new("Contour", Bind::bound(&mut shape), &rows, Bind::bound(&mut state)).field(PickerField::ThumbnailAndName).show(ui);
/// ```
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(PresetPicker<'static, u32>))]
pub struct PresetPicker<'a, K: Value + Eq + Hash + 'static> {
    label: Label,
    value: Bind<'a, Option<K>>,
    rows: &'a [PresetRow<K>],
    state: Bind<'a, PresetState<K>>,
    /// What the closed field shows.
    #[prop]
    field: PickerField,
    /// The shape of the browser's tiles.
    #[prop]
    cell_shape: CellShape,
    /// How the browser shows the presets.
    #[prop]
    view: PresetView,
    /// Which groups of the browser are open before the user toggles any.
    #[prop]
    default_open: DefaultOpen,
    /// A row above the presets with the search field.
    #[prop]
    toolbar: bool,
    /// The search field in the toolbar; off, the toolbar keeps the host's tools and the search is cleared.
    #[prop]
    search: bool,
    /// The host's controls at the reading end of the browser's toolbar.
    #[prop(func = "FnMut(&mut Ui)")]
    tools: Option<BrowserTools<'a>>,
    /// The id of the browser in the popover, which `PresetBrowser::request_rename` names: a host that starts a rename
    /// from its own control (a menu's Rename) gives one.
    #[prop]
    browser_id: Option<egui::Id>,
    /// Which presets and groups the browser may rename.
    #[prop(func = "Fn(&K) -> bool")]
    can_rename: Option<RowTest<'a, K>>,
    /// Which presets and groups the row menu may delete; its row is disabled for the others.
    #[prop(func = "Fn(&K) -> bool")]
    can_delete: Option<RowTest<'a, K>>,
    /// Which presets the row menu's Move to may move.
    #[prop(func = "Fn(&K) -> bool")]
    can_move: Option<RowTest<'a, K>>,
    /// Whether a secondary click or the context-menu key opens the browser's row menu.
    #[prop]
    row_menu: bool,
    /// Shown, muted, while the value is no preset of the rows.
    #[prop(into)]
    placeholder: Option<Label>,
    /// Shown instead of the current preset's name.
    #[prop(into)]
    display_name: Option<Label>,
    /// A short line under the field's thumbnail (a brush's current size).
    #[prop(into)]
    caption: Option<Label>,
    /// The field's tooltip.
    #[prop(into)]
    tooltip: Option<Tooltip>,
    /// Content above the browser in the popover.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    head: Option<PickerHead<'a>>,
    /// A click on a preset picks it and keeps the popover open; a double click or Enter picks it and closes.
    #[prop(flag)]
    keep_open: bool,
    /// The browser's footer.
    #[prop]
    footer: Option<PresetFooterOptions<'a>>,
    /// The zoom the thumbnails span and how it steps; the footer's slider uses the same.
    #[prop]
    thumb_range: TileZoom,
    /// The thumbnail of the closed field asks the host for an editor (`PresetPickerOutput::edit_requested`) and the
    /// rest of the field opens the list.
    #[prop(flag)]
    edit_face: bool,
    /// The label of the action row after the presets that creates a preset.
    #[prop(into)]
    create_row: Option<Label>,
    /// The label of the action row after the presets that detaches the value from its preset.
    #[prop(into)]
    detach_row: Option<Label>,
    /// Paints the popover's thumbnails (a swatch, a gradient ramp, a brush tip) in place of each row's own `preview`.
    #[prop(func = "Fn(&ContentPainterCx, Rect, ThumbInfo<'_, K>)")]
    preview: Option<ThumbPainter<'a, K>>,
    /// The columns of the popover's grid: the popover is as wide as this many tiles, at least its field.
    #[prop]
    columns: u16,
    /// Edge grips resize the popover, and the size the user leaves it at is saved under this key.
    #[prop]
    resizable: Option<SaveKey>,
    opened_at: Option<(Pos2, Bind<'a, bool>)>,
}

/// The picker's state while its field is shown: the popover's open state (without `open_at`).
#[derive(Clone, Default)]
struct Session {
    open: bool,
}

impl<'a, K: Value + Eq + Hash + 'static> PresetPicker<'a, K> {
    /// A picker named `label` over `value`, choosing among the presets of `rows`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<K>>, rows: &'a [PresetRow<K>], state: Bind<'a, PresetState<K>>) -> Self {
        PresetPicker {
            label: label.into(),
            value,
            rows,
            state,
            field: PickerField::default(),
            cell_shape: CellShape::default(),
            view: PresetView::Grid,
            default_open: DefaultOpen::All,
            toolbar: false,
            search: true,
            tools: None,
            browser_id: None,
            can_rename: None,
            can_delete: None,
            can_move: None,
            row_menu: true,
            placeholder: None,
            display_name: None,
            caption: None,
            tooltip: None,
            head: None,
            keep_open: false,
            footer: None,
            thumb_range: TileZoom::Ladder,
            edit_face: false,
            create_row: None,
            detach_row: None,
            preview: None,
            columns: 6,
            resizable: None,
            opened_at: None,
        }
    }

    /// Opens the popover at `at`, a point in the parent's layer space, while `open` is true, and shows no field: the
    /// app opens it (a right-click on the canvas) and every close writes `false`.
    pub fn open_at(mut self, at: Pos2, open: Bind<'a, bool>) -> Self {
        self.opened_at = Some((at, open));
        self
    }

    /// The rows the picker chooses among.
    pub(super) fn preset_rows(&self) -> &'a [PresetRow<K>] {
        self.rows
    }

    /// The key the binding holds, when it holds one.
    pub(super) fn held_key(&mut self, cx: &mut impl StageAccess) -> Option<K> {
        match self.value.reborrow().read(cx) {
            Some(Mixed::Same(held)) => held,
            _ => None,
        }
    }

    /// Shows the field and, while open, the popover.
    pub fn show(self, ui: &mut Ui) -> PresetPickerOutput<K> {
        let PresetPicker {
            label,
            mut value,
            rows,
            mut state,
            field,
            cell_shape,
            view,
            default_open,
            toolbar,
            search,
            mut tools,
            browser_id,
            can_rename,
            can_delete,
            can_move,
            row_menu,
            placeholder,
            display_name,
            caption,
            tooltip,
            mut head,
            keep_open,
            footer,
            thumb_range,
            edit_face,
            create_row,
            detach_row,
            preview,
            columns,
            resizable,
            mut opened_at,
        } = self;
        let name: Arc<str> = Arc::from(label.as_str());
        let shown = Stack::new().show(ui, |ui, cx| {
            let mut session = cx.memory::<Session>().clone();
            // Without the field (`open_at`) the face does not run, so no field shows in the tree; the popover hangs
            // from the point instead of from the face.
            let mut open = session.open;
            let (popover, face_edit, edit_requested) = match opened_at.as_mut() {
                Some((at, bind)) => (Popover::new(label.clone(), bind.reborrowed()).at_point(*at), None, false),
                None => {
                    let face = PickerFace {
                        common: Common::new(),
                        value: value.reborrowed(),
                        rows,
                        field,
                        cell_shape,
                        placeholder: placeholder.clone(),
                        display_name: display_name.clone(),
                        caption: caption.clone(),
                        edit_face,
                        open,
                        toggle: false,
                        edit_requested: false,
                    }
                    .label(label.clone());
                    let face = match tooltip.clone() {
                        Some(tooltip) => face.tooltip(tooltip),
                        None => face,
                    };
                    let FaceOutput { edit, toggle, edit_requested } = face.show(ui);
                    if toggle {
                        open = !open;
                    }
                    let popover = Popover::new(label.clone(), Bind::bound(&mut open)).opened_by(&edit.response);
                    (popover, Some(edit), edit_requested)
                }
            };
            let current = match value.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => None,
            };
            // The actions follow the presets as rows of the browser's collection, set off by a divider, as the
            // preset menu lists them.
            let actions: Vec<(Label, bool)> =
                [create_row.clone().map(|label| (label, true)), detach_row.clone().map(|label| (label, false))].into_iter().flatten().collect();
            let action_labels = actions.iter().map(|(label, _)| label.clone());
            let mut browse = |ui: &mut Ui| {
                let mut browser = PresetBrowser::new(label.clone(), state.reborrowed(), rows)
                    .cell_shape(cell_shape)
                    .view_lock(view)
                    .default_open(default_open)
                    .toolbar(toolbar)
                    .search(search)
                    .row_menu(row_menu)
                    .thumb_range(thumb_range)
                    .columns(columns)
                    .actions(action_labels.clone());
                if let Some(options) = footer.as_ref() {
                    browser = browser.footer(options.clone());
                }
                if let Some(tools) = tools.as_mut() {
                    browser = browser.tools(|ui| tools.show(ui));
                }
                if let Some(id) = browser_id {
                    browser = browser.browser_id(id);
                }
                if let Some(key) = current.clone() {
                    browser = browser.selected(key);
                }
                if let Some(test) = can_rename.as_ref() {
                    browser = browser.can_rename(|key| test.passes(key));
                }
                if let Some(test) = can_delete.as_ref() {
                    browser = browser.can_delete(|key| test.passes(key));
                }
                if let Some(test) = can_move.as_ref() {
                    browser = browser.can_move(|key| test.passes(key));
                }
                if let Some(painter) = preview.as_ref() {
                    browser = browser.thumb(|cx, rect, info| painter.paint(cx, rect, info));
                }
                browser.show(ui)
            };
            let popover = popover.collection_body();
            let popover = match resizable.clone() {
                Some(key) => popover.resizable(true).remember(key),
                None => popover,
            };
            let popup = popover.show(ui, |ui, body| {
                let out: PresetOutput<K> = Stack::new()
                    .fill()
                    .header(|ui, cx| {
                        if let Some(head) = head.as_mut() {
                            (head.0)(ui, cx);
                        }
                    })
                    .show(ui, |ui, _| browse(ui))
                    .inner?;
                let closing = out.events.iter().any(|event| match event {
                    PresetEvent::Select { .. } => !keep_open,
                    PresetEvent::Activate { .. } => true,
                    PresetEvent::Action(at) => *at < actions.len(),
                    _ => false,
                });
                if closing {
                    body.request_close_as(CloseReason::Activated);
                }
                Some(out)
            });
            let closed_as = popup.output.closed;
            let open_after = popup.output.open;
            let browsed = popup.inner.flatten();

            let mut edit = face_edit.unwrap_or_else(|| EditResponse::lifted(popup.output.response.clone(), EditEvents::new(), None));
            let mut out = PresetPickerOutput {
                edit: EditResponse::lifted(edit.response.clone(), EditEvents::new(), None),
                picked: None,
                events: Vec::new(),
                closed: false,
                closed_as,
                open: open_after,
                edit_requested,
                create: false,
                detach: false,
            };
            let mut pick: Option<(K, bool)> = None;
            if let Some(browsed) = browsed {
                for event in browsed.events {
                    match event {
                        PresetEvent::Select { key, .. } => pick = Some((key, !keep_open)),
                        PresetEvent::Activate { key, .. } => pick = Some((key, true)),
                        PresetEvent::Open(_) => {}
                        PresetEvent::Action(at) => match actions.get(at) {
                            Some(&(_, true)) => out.create = true,
                            Some(&(_, false)) => out.detach = true,
                            None => {}
                        },
                        other => out.events.push(other),
                    }
                }
            }
            let mut events = EditEvents::new();
            if let Some((key, close)) = pick {
                events.push(EditEvent::set(Mixed::Same(current.clone()), Some(key.clone()), Arc::clone(&name), None));
                out.picked = Some(key);
                out.closed = close;
            }
            for event in events.iter() {
                value.reborrow().write(cx, event);
            }
            edit.events.extend(events);
            out.edit = edit;
            if opened_at.is_none() {
                session.open = open_after;
            }
            *cx.memory::<Session>() = session;
            out
        });
        match shown.inner {
            Some(out) => out,
            None => PresetPickerOutput {
                edit: EditResponse::lifted(shown.output, EditEvents::new(), None),
                picked: None,
                events: Vec::new(),
                closed: false,
                closed_as: None,
                open: false,
                edit_requested: false,
                create: false,
                detach: false,
            },
        }
    }
}

/// The picker's closed field: the face of the current preset. It owns the picker's value for the field's edit
/// response; the popover's picks are written by `PresetPicker`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(unregistered)]
struct PickerFace<'a, K: Value + Eq + Hash + 'static> {
    #[common]
    common: Common,
    value: Bind<'a, Option<K>>,
    rows: &'a [PresetRow<K>],
    field: PickerField,
    cell_shape: CellShape,
    placeholder: Option<Label>,
    display_name: Option<Label>,
    caption: Option<Label>,
    edit_face: bool,
    open: bool,
    toggle: bool,
    edit_requested: bool,
}

/// What the field reports.
#[derive(Debug)]
struct FaceOutput<K> {
    edit: EditResponse<Option<K>>,
    /// A click on the face, Enter or Space toggles the popover; Alt+Down opens it.
    toggle: bool,
    edit_requested: bool,
}

impl<K: Value> Outcome for FaceOutput<K> {
    type Value = Option<K>;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

impl<K: Value + Eq + Hash + 'static> PickerFace<'_, K> {
    fn preset(&self, key: &K) -> Option<&PresetRow<K>> {
        self.rows.iter().find(|row| !row.is_group() && row.key == *key)
    }

    /// What the field shows of `value` and the preset it shows, if any: the preset's name (or `display_name`), else
    /// the mixed word for a mixed value or the placeholder.
    fn shown<'s>(&'s self, value: Option<&Option<K>>, words: &PresetWords, env: &EnvView<'_>) -> (Label, Option<&'s PresetRow<K>>) {
        let Some(value) = value else { return (env.resolve(&words.mixed), None) };
        match value.as_ref().and_then(|key| self.preset(key)) {
            Some(row) => (self.display_name.as_ref().map_or_else(|| Label::from(row.label.clone()), |name| env.resolve(name)), Some(row)),
            None => (env.resolve(self.placeholder.as_ref().unwrap_or(&words.custom)), None),
        }
    }

    /// `face` with the caption under its thumbnail, when the picker has one.
    fn captioned<'f>(&self, face: SelectFace<'f>) -> SelectFace<'f> {
        match &self.caption {
            Some(caption) => face.thumb_caption(caption.clone()),
            None => face,
        }
    }

    /// `face` with an empty thumbnail of the cell shape, and the name unless the field shows the thumbnail alone.
    fn empty_thumb<'f>(&self, face: SelectFace<'f>) -> SelectFace<'f> {
        self.captioned(face.thumb(self.cell_shape.aspect(), |_, _| {}).name(self.field != PickerField::Thumbnail))
    }

    /// The face of a field showing `text`: with a `row`, its thumbnail and name as `field` says; without, the muted
    /// look of a mixed value or a placeholder, which a thumbnail-only field shows as an empty thumbnail. The measured
    /// candidates and the painted face are both built here.
    fn face_of(&self, text: Label, row: Option<&PresetRow<K>>, paint: bool) -> SelectFace<'static> {
        let face = SelectFace::new(text);
        match (row, self.field) {
            (None, PickerField::Thumbnail) => self.empty_thumb(face.placeholder()),
            (None, _) => face.placeholder(),
            (Some(row), field) => match (&row.preview, field) {
                (_, PickerField::Name) | (None, _) => face,
                (Some(preview), field) => {
                    let draw = paint.then(|| preview.paint.clone());
                    let face = face
                        .thumb(preview.aspect, move |cx, rect| {
                            if let Some(draw) = &draw {
                                draw(cx, rect);
                            }
                        })
                        .name(field == PickerField::ThumbnailAndName);
                    self.captioned(face)
                }
            },
        }
    }
}

impl<K: Value + Eq + Hash + 'static> Component for PickerFace<'_, K> {
    const ROLE: Role = Role::COMBO_BOX;
    type Style = PresetPickerStyle;
    type Output = FaceOutput<K>;

    fn style(cx: &StyleCx<'_>, _: PickerFaceStyleKey<K>) -> PresetPickerStyle {
        PresetPickerStyle { surface: cx.dropdown(), metrics: Metrics::new(), focus: cx.focus_look() }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    /// The widest face that can show, so the field keeps its width whichever preset is picked: the face of each
    /// preset, the mixed word and the placeholder, and an empty thumbnail face for the height.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let words = cx.words::<PresetWords>();
        let empty = SelectFace::new(Label::Empty);
        let empty = if self.field == PickerField::Name { empty } else { self.empty_thumb(empty) };
        let rows = self.rows.iter().filter(|row| !row.is_group());
        let faces = rows.map(|row| {
            let (text, _) = self.shown(Some(&Some(row.key.clone())), &words, &cx.env);
            self.face_of(text, Some(row), false)
        });
        let words_shown = [self.shown(None, &words, &cx.env), self.shown(Some(&None), &words, &cx.env)];
        let notes = words_shown.into_iter().map(|(text, row)| self.face_of(text, row, false));
        std::iter::once(empty).chain(faces).chain(notes).map(|face| cx.face(face)).fold(Vec2::ZERO, Vec2::max)
    }

    fn parts(&mut self, rect: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(FACE, rect)
            .sense(Sense::CLICK | Sense::HOVER | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Body, 0))
            .pointer_only("Enter, Space and Alt+Down open the presets");
        if self.edit_face {
            let (thumb, _) = cx.flow.take_start(cx.content_rect.height());
            cx.part(THUMB, thumb)
                .sense(Sense::CLICK | Sense::HOVER)
                .priority(HitPriority::new(HitLevel::Body, 1))
                .pointer_only("the host's edit command opens the editor");
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Option<K>>> {
        Some(self.value.reborrow())
    }

    /// A click on the face, Enter and Space toggle the popover; Alt+Down opens it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let clicked = matches!(cx.part(FACE).press.phase, PressPhase::Release { click: true, .. });
        let keyed = cx.activation().is_some_and(|activation| activation.by_key);
        let opens = !self.open && cx.intents().take(Intent::Open);
        self.toggle = clicked || keyed || opens;
        if self.edit_face && matches!(cx.part(THUMB).press.phase, PressPhase::Release { click: true, .. }) {
            self.edit_requested = true;
        }
    }

    /// The current preset's name as the field's value, and whether the popover is open after this pass's toggle.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<PresetWords>();
        let (text, _) = self.shown(cx.value(), &words, &cx.env);
        access.value(text.as_str().to_owned()).expanded(self.open != self.toggle);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<PresetWords>();
        let (text, row) = self.shown(cx.value(), &words, &cx.env);
        ui.face(FACE, self.face_of(text, row, true));
    }

    fn finish(self, cx: Cx<'_, Self>) -> FaceOutput<K> {
        FaceOutput { edit: cx.edit_response(), toggle: self.toggle, edit_requested: self.edit_requested }
    }
}
