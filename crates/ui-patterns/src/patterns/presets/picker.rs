//! `PresetPicker`: a closed field that shows the current preset and opens a `PresetBrowser` in a popover.
//!
//! The closed field is a `SelectFace` over the field well, as `Select` shows it. Hovering a preset in the popover
//! proposes it, a pick commits and closes, and closing without a pick restores the value. A host can put content
//! above the browser (`head`), keep the popover open on a click (`keep_open`), give the browser a footer and set
//! how far the thumbnails zoom.

#[cfg(feature = "testing")]
mod stories;

use super::browser::{PresetBrowser, ThumbPainter};
use super::event::PresetEvent;
use super::footer::PresetFooterOptions;
use super::model::{CellShape, PresetRow, ThumbInfo};
use super::state::{DefaultOpen, PresetState, PresetView};
use super::words::PresetWords;
use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Outcome, Overlay, OverlayUi, Overlays as _, PaintUi,
    PartsCx, Pointer as _, Props as _, Role, ShowContainer as _, WordSetCx as _,
};
use crate::layout_components::Stack;
use crate::list::RowTest;
use crate::pickers::{SelectFace, toggle_on_key};
use crate::primitives::ContentPainterCx;
use crate::runtime::env::{EnvView, Label};
use crate::runtime::memory::StageAccess;
use crate::style::{ComponentStyle, FocusLook, StyleCx, SurfaceLook};
use crate::tile_grid::TileZoom;
use craft_behavior::{CancelReason, GesturePhase, HitLevel, HitPriority, Metrics, PartKey, PressPhase, Sense, Value};
use egui::{Rect, Response, Ui, Vec2};
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
    /// The thumbnail alone; the name when the preset has none.
    #[default]
    Thumbnail,
    /// The name alone.
    Name,
    /// The thumbnail, then the name.
    ThumbnailAndName,
}

/// The style of a preset picker: the field well and the focus edge; the face carries the padding.
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
    /// The field's response and the edit of its value.
    pub edit: EditResponse<Option<K>>,
    /// The preset picked in this pass, which the binding now holds.
    pub picked: Option<K>,
    /// What the browser in the popover reported besides the pick and the open groups: renames, deletes, the
    /// context menu's rows and the footer's buttons.
    pub events: Vec<PresetEvent<K>>,
    /// The pick closed the popover (a double click or Enter); false for a click while `keep_open` holds it open.
    pub closed: bool,
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
/// with the browser; hovering a preset proposes it, a click or Enter picks it and closes, and Escape closes and
/// restores the value.
///
/// - **Value.** The binding holds the picked preset's key, or none while the value is no preset of `rows`; the field
///   then shows `placeholder`, "Custom" by default.
/// - **State.** `state` is the browser's view settings; bind it with `Bind::bound`.
/// - **Thumbnails.** Each row's `preview` paints its thumbnail, in the field and in the browser.
///
/// ```ignore
/// let out = PresetPicker::new("Contour", Bind::bound(&mut shape), &rows, Bind::bound(&mut state)).field(PickerField::ThumbnailAndName).show(ui);
/// ```
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(PresetPicker<'static, u32>))]
pub struct PresetPicker<'a, K: Value + Eq + Hash + 'static> {
    #[common]
    common: Common,
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
    /// Content above the browser in the popover.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    head: Option<PickerHead<'a>>,
    /// A click on a preset picks it and keeps the popover open; a double click or Enter picks it and closes. Hovering
    /// proposes nothing.
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
    picked: Option<K>,
    closed: bool,
    edit_requested: bool,
    create: bool,
    detach: bool,
    events: Vec<PresetEvent<K>>,
}

impl<'a, K: Value + Eq + Hash + 'static> PresetPicker<'a, K> {
    /// A picker named `label` over `value`, choosing among the presets of `rows`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Option<K>>, rows: &'a [PresetRow<K>], state: Bind<'a, PresetState<K>>) -> Self {
        PresetPicker {
            common: Common::new(),
            value,
            rows,
            state,
            field: PickerField::default(),
            cell_shape: CellShape::default(),
            view: PresetView::Grid,
            default_open: DefaultOpen::All,
            toolbar: false,
            can_rename: None,
            can_delete: None,
            can_move: None,
            row_menu: true,
            placeholder: None,
            display_name: None,
            head: None,
            keep_open: false,
            footer: None,
            thumb_range: TileZoom::Ladder,
            edit_face: false,
            create_row: None,
            detach_row: None,
            preview: None,
            picked: None,
            closed: false,
            edit_requested: false,
            create: false,
            detach: false,
            events: Vec::new(),
        }
        .label(label)
    }

    /// The rows the picker chooses among.
    pub(super) fn preset_rows(&self) -> &'a [PresetRow<K>] {
        self.rows
    }

    /// The key the binding holds, when it holds one.
    pub(super) fn held_key(&mut self, cx: &mut impl StageAccess) -> Option<K> {
        match self.value.reborrow().read(cx) {
            Some(craft_behavior::Mixed::Same(held)) => held,
            _ => None,
        }
    }

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

    /// The face of a field showing `text`: with a `row`, its thumbnail and name as `field` says; without, the muted
    /// look of a mixed value or a placeholder. The measured candidates and the painted face are both built here.
    fn face_of(&self, text: Label, row: Option<&PresetRow<K>>, paint: bool) -> SelectFace<'static> {
        let Some(row) = row else { return SelectFace::new(text).placeholder() };
        let face = SelectFace::new(text);
        match (&row.preview, self.field) {
            (_, PickerField::Name) | (None, _) => face,
            (Some(preview), field) => {
                let draw = paint.then(|| preview.paint.clone());
                face.thumb(preview.aspect, move |cx, rect| {
                    if let Some(draw) = &draw {
                        draw(cx, rect);
                    }
                })
                .name(field == PickerField::ThumbnailAndName)
            }
        }
    }
}

impl<K: Value + Eq + Hash + 'static> Component for PresetPicker<'_, K> {
    const ROLE: Role = Role::COMBO_BOX;
    type Style = PresetPickerStyle;
    type Output = PresetPickerOutput<K>;

    fn style(cx: &StyleCx<'_>, _: PresetPickerStyleKey<K>) -> PresetPickerStyle {
        PresetPickerStyle { surface: cx.field(), metrics: Metrics::new(), focus: cx.focus_look() }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    /// The widest face that can show, so the field keeps its width whichever preset is picked: the face of each
    /// preset, the mixed word and the placeholder, and an empty thumbnail face for the height.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let words = cx.words::<PresetWords>();
        let aspect = self.cell_shape.aspect();
        let empty = SelectFace::new(Label::Empty);
        let empty = if self.field == PickerField::Name { empty } else { empty.thumb(aspect, |_, _| {}).name(self.field != PickerField::Thumbnail) };
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
            .pointer_only("Enter, Space and Alt+Down open the presets")
            .toggles("browser")
            .on_press();
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

    /// A press on the face and Alt+Down open the popover through the face's `toggles`; Enter and Space toggle it
    /// (`toggle_on_key`). A preview that the popover closed on without a pick is cancelled.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("browser");
        toggle_on_key(cx, popup);
        if self.edit_face && matches!(cx.part(THUMB).press.phase, PressPhase::Release { click: true, .. }) {
            self.edit_requested = true;
        }
        if !popup.is_open(cx) && matches!(cx.gesture(), Some(GesturePhase::Begin | GesturePhase::Update)) {
            cx.cancel(CancelReason::Dismissed);
        }
    }

    /// The popover: the browser over the rows. The preset the pointer moves the lead to is proposed; a click or Enter
    /// on a preset proposes it, commits and closes the popover.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("browser");
        let name = cx.label();
        let current = cx.value().and_then(Clone::clone);
        let (rows, shape, view, default_open, toolbar) = (self.rows, self.cell_shape, self.view, self.default_open, self.toolbar);
        let (can_rename, can_delete, can_move, row_menu) =
            (self.can_rename.as_ref(), self.can_delete.as_ref(), self.can_move.as_ref(), self.row_menu);
        let (thumb_range, footer, keep_open) = (self.thumb_range, self.footer.as_ref(), self.keep_open);
        let state = &mut self.state;
        let head = &mut self.head;
        let preview = self.preview.as_ref();
        // The actions follow the presets as rows of the browser's collection, set off by a divider, as the preset menu
        // lists them.
        let actions: Vec<(Label, bool)> =
            [self.create_row.clone().map(|label| (label, true)), self.detach_row.clone().map(|label| (label, false))].into_iter().flatten().collect();
        let action_labels = actions.iter().map(|(label, _)| label.clone());
        let mut browse = |ui: &mut Ui| {
            let mut browser = PresetBrowser::new(name.clone(), state.reborrowed(), rows)
                .cell_shape(shape)
                .view_lock(view)
                .default_open(default_open)
                .toolbar(toolbar)
                .row_menu(row_menu)
                .thumb_range(thumb_range)
                .actions(action_labels.clone());
            if let Some(options) = footer {
                browser = browser.footer(options.clone());
            }
            if let Some(key) = current.clone() {
                browser = browser.selected(key);
            }
            if let Some(test) = can_rename {
                browser = browser.can_rename(|key| test.passes(key));
            }
            if let Some(test) = can_delete {
                browser = browser.can_delete(|key| test.passes(key));
            }
            if let Some(test) = can_move {
                browser = browser.can_move(|key| test.passes(key));
            }
            if let Some(painter) = preview {
                browser = browser.thumb(|cx, rect, info| painter.paint(cx, rect, info));
            }
            browser.show(ui)
        };
        let out = ui.show(popup, Overlay::popover(), |ui, _| {
            let Some(head) = head.as_mut() else { return Some(browse(ui)) };
            Stack::new().fill().header(|ui, cx| (head.0)(ui, cx)).show(ui, |ui, _| browse(ui)).inner
        });
        let Some(out) = out.flatten() else { return };
        let mut pick: Option<(K, bool)> = None;
        for event in out.events {
            match event {
                PresetEvent::Select { key, .. } => pick = Some((key, !keep_open)),
                PresetEvent::Activate { key, .. } => pick = Some((key, true)),
                PresetEvent::Open(_) => {}
                PresetEvent::Action(at) => {
                    let Some(&(_, create)) = actions.get(at) else { continue };
                    if create {
                        self.create = true;
                    } else {
                        self.detach = true;
                    }
                    popup.close(cx);
                }
                other => self.events.push(other),
            }
        }
        match (pick, out.hovered) {
            (Some((key, close)), _) => {
                cx.propose(Some(key.clone()));
                cx.commit();
                if close {
                    popup.close(cx);
                }
                self.picked = Some(key);
                self.closed = close;
            }
            (None, Some(key)) if !keep_open && current.as_ref() != Some(&key) => cx.propose(Some(key)),
            _ => {}
        }
    }

    /// The current preset's name as the field's value.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<PresetWords>();
        let (text, _) = self.shown(cx.value(), &words, &cx.env);
        access.value(text.as_str().to_owned());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<PresetWords>();
        let (text, row) = self.shown(cx.value(), &words, &cx.env);
        ui.face(FACE, self.face_of(text, row, true));
    }

    fn finish(self, cx: Cx<'_, Self>) -> PresetPickerOutput<K> {
        PresetPickerOutput {
            edit: cx.edit_response(),
            picked: self.picked,
            events: self.events,
            closed: self.closed,
            edit_requested: self.edit_requested,
            create: self.create,
            detach: self.detach,
        }
    }
}
