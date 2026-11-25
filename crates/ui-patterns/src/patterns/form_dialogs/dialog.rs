//! `ParamDialog`: a dialog from a parameter list, with an optional preview, and `HostedParamDialog`, the same dialog
//! owned by a `DialogHost`. A command form is a `ParamDialog` without a preview.

use super::fields::{DialogFields, FieldValue};
use crate::patterns::presets::{
    NameCheck, NamedPresetEvent, NamedPresetRow, PickerField, PresetError, PresetKind, PresetPicker, PresetRow, PresetSource,
};
use crate::runtime::requests::{ColorRequest, EyedropperRequest, HostAnswer, ListingRequest, PathRequest};

use super::form::{FormDialog, FormDialogWords};
use super::host_requests::HostRequests;
use super::param::{Param, ParamKind, open_fields, set_color_field};
use super::preview::{PagerStep, PaneOverlay, PanePager, PreviewPane, PreviewState, PreviewView, PreviewZoom};

use super::rows::{ParamRows, ParamRowsOutput};

use crate::contract::{
    Bind, BodyCx, CanvasGesture, CollectionEvent, NoValue, Outcome, Remember as _, Sampler as _, SaveKey, Show as _, ShowCollection as _,
    ShowContainer as _,
};

use crate::dialog::{
    Alert, ButtonPlace, Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, FooterHooks, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id,
};

use crate::display::{Shade, Text};
use crate::form::SliderFieldLayout;
use crate::layout::RowAlign;
use crate::layout_components::{Row, Stack};
use crate::notices::{Notice, NoticeLayout};
use crate::style::DataColor;
use crate::tabs::{Tab, TabKind, TabStrip};

use crate::primitives::ContentPainterCx;
use crate::runtime::env::{Env, Label};
use crate::text::RangeAlert;
use craft_behavior::{CanvasTransform, DisplayOrder, EditEvent, ItemSource, Mixed, SelectionChange};

use crate::runtime::memory::{Lifetime, MemoryStage, with_session};

use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};

use crate::contract::{CaptionColumn, LabelScheme};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use egui::{Context, Pos2, Rect, Response, Ui};
use serde_json::Value;

use std::borrow::Cow;
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

craft_ui::words! {
    /// The words of the parameter dialog beside the common OK and Cancel.
    pub struct ParamDialogWords {
        reset: "paramDialog.reset" = "Reset";
        presets: "paramDialog.presets" = "Preset";
    }
}

/// How a parameter dialog arranges its rows, its preview pane and its buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum ParamLayout {
    /// The preview pane to the right of the rows, under the zoom row. A dialog that previews has its buttons in a
    /// column beside the body, with the Preview check box under them.
    #[default]
    Beside,
    /// The preview pane under the rows, under the zoom row. A dialog that previews has its buttons in a column beside
    /// the body, with the Preview check box under them: Photoshop's filter dialogs.
    Below,
    /// One column: each slider row stacked (the label and the number field with its unit on one line, a full-width
    /// slider under them), then the preview pane with the zoom row under it, and the buttons across the foot with the
    /// Preview check box at their start: PhotoCraft's filter dialogs. The other rows keep the form's label and control
    /// columns.
    Stacked,
    /// A document dialog, as production's: the captions at the row start with their control right after them, the
    /// option groups shaded, the preview pane under the rows and the buttons across the foot.
    Document,
}

impl ParamLayout {
    /// Where a dialog that previews stands its buttons: in a column beside the body, or across the foot.
    fn button_place(self) -> ButtonPlace {
        if matches!(self, ParamLayout::Stacked | ParamLayout::Document) { ButtonPlace::Foot } else { ButtonPlace::Side }
    }

    /// How the range rows arrange their slider and number field.
    fn sliders(self) -> SliderFieldLayout {
        match self {
            ParamLayout::Stacked => SliderFieldLayout::Stacked,
            ParamLayout::Beside | ParamLayout::Below | ParamLayout::Document => SliderFieldLayout::Inline,
        }
    }

    /// The label scheme the rows take, when the layout sets one.
    fn labels(self) -> Option<LabelScheme> {
        (self == ParamLayout::Document).then_some(LabelScheme::Inline)
    }
}

/// The layout a [`ParamDialog`] takes when it names none: set once for an app on the root's values
/// (`RootEnv::values`), so every dialog of the app lays out alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, crate::contract::MemoProps)]
pub struct DialogLayout(pub ParamLayout);

impl crate::runtime::env::EnvValue for DialogLayout {
    fn root() -> Self {
        DialogLayout::default()
    }
}

/// One tab of a tabbed [`ParamDialog`]: a title and the rows it shows.
///
/// A read-only tab shows its rows' values without controls; its note is an inline `Notice` (`Tone::Info`) above them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ParamTab {
    /// The tab's title.
    pub title: Label,
    /// The tab's rows.
    pub params: Vec<Param>,
    /// A note shown above the rows of a read-only tab.
    pub note: Option<Label>,
    /// Whether the rows show their values without controls.
    pub read_only: bool,
}

impl ParamTab {
    /// A tab titled `title` with the rows `params`.
    pub fn new(title: impl Into<Label>, params: Vec<Param>) -> Self {
        ParamTab { title: title.into(), params, note: None, read_only: false }
    }

    /// The tab as a read-only one, with `note` above its rows.
    #[must_use]
    pub fn read_only(self, note: Option<Label>) -> Self {
        ParamTab { note, read_only: true, ..self }
    }
}

/// The tab titles of a tabbed dialog as an item source, keyed by position.
struct Titles<'t>(&'t [ParamTab]);

impl DisplayOrder<usize> for Titles<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|tab| Cow::Borrowed(tab.title.as_str()))
    }
}

impl ItemSource<usize> for Titles<'_> {}

/// The tab a tabbed dialog without a `tab` binding shows while it is open.
#[derive(Clone, Default)]
struct OpenTab(usize);

/// The shared rows, then the rows of every tab: their values open, validate and answer together, whichever tab shows.
/// The rows of a read-only tab are disabled here, so automation cannot set what the user cannot edit.
fn every_param<'p>(params: Cow<'p, [Param]>, tabs: &[ParamTab]) -> Cow<'p, [Param]> {
    if tabs.is_empty() {
        return params;
    }
    let mut every = params.into_owned();
    every.extend(tabs.iter().flat_map(|tab| tab.params.iter().cloned().map(move |param| if tab.read_only { param.disabled() } else { param })));
    Cow::Owned(every)
}

/// Where a tabbed dialog's own `params` go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SharedRows {
    /// Above the tab strip.
    #[default]
    Above,
    /// Under the shown tab's rows.
    Below,
}

/// The host's drawing over a hosted preview pane's picture, kept by the host between passes ([`PaneOverlay`] runs once).
pub type SharedOverlay = Arc<dyn Fn(&ContentPainterCx, CanvasTransform, Rect) + Send + Sync>;

/// What a parameter dialog answers with at OK.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ParamResult {
    /// The dialog's key: the command the values are for.
    pub key: String,
    /// The values the user confirmed, keyed by parameter.
    pub fields: DialogFields,
    /// The id of the dialog that filed the result, instance ids included, so a family's step names the dialog that
    /// answered. `HostedParamDialog` sets it; a `ParamDialog` shown on its own has no host id and leaves `Id::NULL`.
    pub id: DialogId,
    remember: Option<(SaveKey, Lifetime, DialogFields)>,
}

impl ParamResult {
    /// Stores the values of a dialog built with [`ParamDialog::remember`] for the next open. The app calls it after
    /// its command succeeds, so a failed command leaves the previous values; without a remember key it does nothing.
    pub fn remember(&self, ctx: &Context) {
        if let Some((key, lifetime, values)) = &self.remember {
            let values = Some(values.clone());
            match lifetime {
                Lifetime::Run => crate::runtime::memory::set_run_value::<Option<DialogFields>>(ctx, key, values),
                _ => crate::runtime::memory::set_saved_value::<Option<DialogFields>>(ctx, key, values),
            }
        }
    }
}

/// What a parameter dialog keeps while it is open: the view of the preview pane.
#[derive(Clone, Default)]
struct Live {
    view: PreviewView,
}

/// What a parameter dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ParamDialogOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer, the pass the dialog closed as OK.
    pub result: Option<ParamResult>,
    /// The rows' output of this pass, with the colour, path and eyedropper requests; `None` when the dialog is closed.
    pub rows: Option<ParamRowsOutput>,
    /// The values after the pass when they differ from the binding's or the dialog opened in this pass: a `values`
    /// binding that does not write through applies them, the opening snapshot included.
    pub values: Option<DialogFields>,
    /// Whether the Preview check box is on: the host renders the preview while it is. Holding the box to compare
    /// leaves it as it was until the release.
    pub preview_on: bool,
    /// Whether the Preview check box is on and held: the host shows the unfiltered picture in the preview pane while
    /// it is.
    pub comparing: bool,
    /// The point of the preview picture a click landed on this pass (`PreviewOutput::clicked`): an eyedropper on the
    /// preview samples there.
    pub preview_click: Option<Pos2>,
    /// The id of the extra footer button or the custom row's button pressed this pass (`ParamDialog::extra_buttons`,
    /// `CustomContext::request_action`).
    pub action: Option<String>,
    /// The preset edits of this pass: Apply, Create, Rename and Delete from the presets row (`ParamDialog::presets`).
    pub presets: Vec<NamedPresetEvent<String>>,
    /// The help link a row's button reported in this pass.
    pub help_link: Option<String>,
    /// The step the preview pager took this pass (`ParamDialog::preview_pager`).
    pub preview_step: Option<PagerStep>,
    /// The picture point at the centre of the view the preview pane drew this pass; `None` without a preview.
    pub(crate) preview_centre: Option<Pos2>,
}

impl Outcome for ParamDialogOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A dialog from a parameter list: one row per [`Param`], the Preview check box when it
/// [`previews`](ParamDialog::previews), with a preview pane when the host gives a picture, and OK, Cancel, extra
/// buttons and a footer note.
///
/// - Keys: Tab walks the fields, Enter is OK unless a field owns it, Escape cancels. Holding Alt turns Cancel into
///   Reset, which restores the values from when the dialog opened, with `reset_to`'s over them, and keeps the dialog
///   open. Keys added since the open (a host's own data without a row) go.
/// - Values: the `values` binding holds them; without one the dialog keeps its own between passes. The first pass
///   fills the parameters' defaults and, with `remember`, the values saved by [`ParamResult::remember`].
/// - Preview: a dialog that `previews` has the Preview check box; the host renders from the reported values while
///   `preview_on` (on the canvas, or as the picture it hands to `preview`), the unfiltered picture while `comparing`
///   (the Preview check box held down). Drag pans the picture, the zoom buttons scale it. The
///   [`layout`](ParamDialog::layout) places the pane and the buttons.
/// - Footer: [`extra_buttons`](ParamDialog::extra_buttons) keep the dialog open and come back as
///   `ParamDialogOutput::action`, the one action channel a custom row's `CustomContext::request_action` reports in
///   too; [`footer_note`](ParamDialog::footer_note) is a muted line at the start edge.
/// - Tabs: with [`tabs`](ParamDialog::tabs) a `TabStrip` shows the tab titles and the shown tab's rows; the dialog's
///   own `params` are the shared rows, above the strip or under the tab's rows ([`SharedRows`]). Every tab's values
///   open, validate and answer together. The `tab` binding says which tab shows.
/// - Read-only: [`read_only`](ParamDialog::read_only) shows the values without controls, has one button and no Alt
///   reset, and answers with no values whichever way it closes.
/// - A command form is a dialog with no preview and no remember key.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct ParamDialog<'a> {
    key: String,
    title: Label,
    params: Cow<'a, [Param]>,
    /// The tabs of a tabbed dialog; `params` are then the shared rows. A `TabStrip` (`TabKind::Text`) shows the titles
    /// and the shown tab's rows. Every tab's values open, validate and answer together.
    #[prop]
    tabs: Vec<ParamTab>,
    /// Where the shared rows go: above the tab strip, or under the shown tab's rows.
    #[prop]
    shared_rows: SharedRows,
    /// Binds the shown tab to `tab`.
    #[prop]
    tab: Option<Bind<'a, usize>>,
    /// Whether the dialog shows the values only: an OK-only footer and no Alt reset.
    #[prop]
    read_only: bool,
    /// Passed on to `Dialog::title_note`.
    #[prop(into)]
    title_note: Option<Label>,
    /// Binds the values to `values`.
    #[prop(name = "values")]
    bound: Option<Bind<'a, DialogFields>>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Saves the values under `key` when the app calls [`ParamResult::remember`], and opens with them.
    #[prop]
    remember: Option<SaveKey>,
    /// How long the values `remember` keeps live; without `remember` it does nothing.
    #[prop]
    remember_for: Lifetime,
    /// The named presets row above the rows.
    #[prop]
    presets: Option<ParamPresets<'a>>,
    /// Whether OK can be pressed; Enter does nothing while it cannot.
    #[prop]
    ok_enabled: bool,
    /// Why OK is disabled: the tooltip of the disabled button.
    #[prop]
    ok_reason: Option<Label>,
    /// The accepting button's label when it is not OK.
    #[prop]
    ok_label: Option<Label>,
    /// The second button's label when it is not Cancel (Close, say).
    #[prop]
    cancel_label: Option<Label>,
    /// The title of the range alert a hosted dialog words for a clamped value ([`HostedParamDialog::take_range_alert`]);
    /// the dialog's title when not set. An app names itself here.
    #[prop(into)]
    alert_title: Option<Label>,
    /// Whether holding Alt turns the second button into Reset; the default is true.
    #[prop]
    alt_resets: bool,
    /// Whether the dialog holds one outer gesture over its fields, from open to close.
    #[prop]
    outer_gesture: bool,
    /// Whether Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z step the dialog's own values ([`FormDialog::history`]).
    #[prop]
    history: bool,
    /// Whether the body is one collection that owns the scroll (a browse list): the dialog's own body scroll is off.
    #[prop]
    collection_body: bool,
    /// The values Reset sets over the values from when the dialog opened: keys it lacks take their opening value, and
    /// keys added since the open go.
    #[prop]
    reset_to: Option<DialogFields>,
    /// How the rows, the preview pane and the buttons are arranged.
    #[prop]
    layout: Option<ParamLayout>,
    /// In a document dialog ([`ParamLayout::Document`]), the captions share one column at the row start and the
    /// controls line up after it, a group of controls (a custom row) with its caption beside it rather than over it:
    /// production's Rotate Pages (Direction:, Pages: and Rotate: in a column). Off by default: each caption is as
    /// wide as its text, and a group row's caption heads it ([`CaptionColumn`]).
    #[prop]
    caption_column: bool,
    /// Whether the title row has a close action.
    #[prop]
    closable: bool,
    /// While no value differs from the one the dialog opened with, a Close button stands in place of OK and Cancel.
    #[prop]
    close_unchanged: bool,
    /// Whether the dialog has the Preview check box: the host previews its values while the box is on.
    #[prop]
    previews: bool,
    /// The Preview check box's value; without it the dialog keeps it, and every open starts with the box on.
    #[prop]
    preview_on: Option<Bind<'a, bool>>,
    /// The picture the preview pane shows in a dialog that [`previews`](ParamDialog::previews); without one there is
    /// no pane.
    #[prop]
    preview: Option<PreviewState>,
    /// The pager row under the preview pane; its step comes back as `ParamDialogOutput::preview_step`.
    #[prop]
    preview_pager: Option<PanePager>,
    /// The host's drawing over the picture of a `Ready` preview.
    #[prop(func = "FnOnce(&ContentPainterCx, CanvasTransform, Rect)")]
    preview_overlay: Option<PaneOverlay<'a>>,
    /// Multiplies the `Ready` preview picture's colours (`PreviewPane::tint`).
    #[prop]
    preview_tint: Option<DataColor>,
    /// Whether a click on the preview centres it on the point clicked (`PreviewPane::click_centres`); a dialog whose
    /// preview clicks pick a colour turns it off. The default is true. While it is on and the pane shows, a primary
    /// press on the dialog's [`canvas`](ParamDialog::canvas) passes the modal layer too: the image is the pane's
    /// locator.
    #[prop]
    click_centres: bool,
    /// The canvas the rows' droppers sample (`ParamRows::canvas`). While the dialog shows, its view stays usable under
    /// the modal layer (`Sampler::keep_view_open`), and while its preview pane centres on clicks a primary press on it
    /// passes (`Sampler::pass_canvas_gesture`): the app centres the pane on the point under it.
    #[prop]
    canvas: Option<egui::Id>,
    /// Extra footer buttons after OK and Cancel (Color Table's Load… and Save…), each an id and a button. They keep the
    /// dialog open; a press comes back as `ParamDialogOutput::action`.
    #[prop]
    extra_buttons: Vec<(String, FooterButton)>,
    /// The patterns the dialog's [`ParamKind::Pattern`] rows offer. A hosted dialog's build gives a source over
    /// owned inputs.
    #[prop]
    patterns: Option<PresetSource<'a, String>>,
    /// A muted line at the footer's start edge, left of the buttons (a file size estimate, say).
    #[prop(into)]
    footer_note: Option<Label>,
    /// A section under the rows that only reads the values (a monitor note).
    #[prop(func = "Fn(&mut Ui, &DialogFields)")]
    below: Option<BelowRows<'a>>,
    shaped: Option<Shaped<'a>>,
    /// The host's answers to the requests of earlier passes; the dialog writes them into its values first.
    #[prop]
    answers: Vec<HostAnswer>,
    /// Marks the bound values as opened by the host with `opening`, so the first pass doesn't open them again and
    /// Reset restores `opening`.
    #[prop(vis = "pub(crate)")]
    values_opened: Option<DialogFields>,
    /// Centres the preview pane's view on the picture point `point` in this pass; without a picture it does nothing.
    #[prop(name = "centre_on", vis = "pub(crate)")]
    centre: Option<Pos2>,
    /// The colours the colour rows show in place of their values in this pass, by row key.
    #[prop(vis = "pub(crate)")]
    preview_colors: Vec<(String, Srgba)>,
    /// OK files the values in `held` and the dialog stays open; a refusal shows until the values change.
    #[prop(vis = "pub(crate)")]
    held: Option<&'a mut HeldAnswer<DialogFields>>,
    /// The lines the host shows under the rows, after `below` ([`HostedParamDialog::set_notes`]).
    #[prop(vis = "pub(crate)")]
    host_notes: Vec<Label>,
}

/// What `ParamDialog::shaped` applies to the built [`Dialog`].
struct Shaped<'a>(Box<dyn FnOnce(Dialog<'a>) -> Dialog<'a> + 'a>);

type BelowRowsFn<'a> = Box<dyn Fn(&mut Ui, &DialogFields) + 'a>;

/// The named presets a [`ParamDialog`] offers above its rows: the preset rows, the values each applies, and the names
/// the user cannot rename or delete.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ParamPresets<'a> {
    /// The listed presets, by name.
    pub rows: &'a [PresetRow<String>],
    /// The values each preset applies, by name.
    pub values: &'a [(String, DialogFields)],
    /// The names of the built-in presets, which `check_name` refuses.
    pub built_in: &'a [String],
}

impl<'a> ParamPresets<'a> {
    /// The presets `rows`, applying `values`, of which `built_in` cannot be renamed or deleted.
    pub fn new(rows: &'a [PresetRow<String>], values: &'a [(String, DialogFields)], built_in: &'a [String]) -> Self {
        ParamPresets { rows, values, built_in }
    }
}

/// The footer rows of a [`ParamDialog`] between passes: whether each was on when the last pass ended, which its
/// button label shows, and the key of the row whose button was pressed, which the next pass flips.
#[derive(Clone, Debug, Default)]
struct FooterRows {
    on: Vec<bool>,
    flip: Option<String>,
}

/// What `ParamDialog::below` draws under the rows.
pub struct BelowRows<'a>(BelowRowsFn<'a>);

impl<'a> BelowRows<'a> {
    /// The section that runs `draw`.
    pub fn new(draw: impl Fn(&mut Ui, &DialogFields) + 'a) -> Self {
        BelowRows(Box::new(draw))
    }
}

impl<'a> ParamDialog<'a> {
    /// A dialog for `params`, titled `title`, answering for the command `key`.
    pub fn new(key: impl Into<String>, title: impl Into<Label>, params: impl Into<Cow<'a, [Param]>>) -> Self {
        ParamDialog {
            key: key.into(),
            title: title.into(),
            params: params.into(),
            tabs: Vec::new(),
            shared_rows: SharedRows::default(),
            tab: None,
            read_only: false,
            title_note: None,
            bound: None,
            open: None,
            remember: None,
            remember_for: Lifetime::default(),
            presets: None,
            ok_enabled: true,
            ok_reason: None,
            ok_label: None,
            cancel_label: None,
            alert_title: None,
            alt_resets: true,
            outer_gesture: false,
            history: false,
            collection_body: false,
            reset_to: None,
            layout: None,
            caption_column: false,
            closable: true,
            close_unchanged: false,
            previews: false,
            preview_on: None,
            preview: None,
            preview_pager: None,
            preview_overlay: None,
            preview_tint: None,
            click_centres: true,
            canvas: None,
            extra_buttons: Vec::new(),
            patterns: None,
            footer_note: None,
            below: None,
            shaped: None,
            answers: Vec::new(),
            values_opened: None,
            centre: None,
            preview_colors: Vec::new(),
            held: None,
            host_notes: Vec::new(),
        }
    }

    /// Changes the built [`Dialog`] before it shows, for the `Dialog` settings a form track needs and `ParamDialog`
    /// has no setter for (`Dialog::overlay`, say). It runs after the settings `ParamDialog` makes itself.
    pub fn shaped(mut self, shape: impl FnOnce(Dialog<'a>) -> Dialog<'a> + 'a) -> Self {
        self.shaped = Some(Shaped(Box::new(shape)));
        self
    }

    /// The answer OK gives with `fields`: with `remember`, it keeps the part of them the dialog saves.
    fn result(key: String, params: &[Param], remember: Option<SaveKey>, remember_for: Lifetime, fields: DialogFields) -> ParamResult {
        let remember = remember.map(|save| (save, remember_for, Self::remembered(params, &fields)));
        ParamResult { key, fields, remember, id: DialogId::NULL }
    }

    /// Opens `values` for `params`: the `saved` values a remembering dialog keeps over them, then the defaults of
    /// the keys still missing, then the snapshot Reset restores. A saved pattern that is not among `patterns` any more
    /// is left out, so its row starts at its default.
    fn open_values(params: &[Param], patterns: &[PresetRow<String>], saved: Option<&DialogFields>, values: &mut DialogFields) {
        if let Some(saved) = saved {
            let offered = |value: &FieldValue| matches!(value, FieldValue::Text(id) if patterns.iter().any(|row| row.kind == PresetKind::Item && row.key == *id));
            let pattern_row = |name: &str| params.iter().any(|param| param.key == name && matches!(param.kind, ParamKind::Pattern { .. }));
            for (name, value) in Self::remembered(params, saved).iter() {
                if !pattern_row(name) || offered(value) {
                    values.set(name, value.clone());
                }
            }
        }
        open_fields(params, values);
    }

    /// The part of `fields` a remembering dialog saves: numbers, angles, choices, check boxes, colours, blend modes and
    /// patterns of the remembered rows and their unit choices; grids, points, channels, free text, files and custom
    /// rows start from their defaults. A colour row whose default follows the document opts out with
    /// [`Param::not_remembered`].
    pub fn remembered(params: &[Param], fields: &DialogFields) -> DialogFields {
        let kept = |param: &Param| {
            param.is_remembered()
                && param.footer_on().is_none()
                && matches!(
                    param.kind,
                    ParamKind::Range { .. }
                        | ParamKind::Angle { .. }
                        | ParamKind::Int { .. }
                        | ParamKind::Number { .. }
                        | ParamKind::Bool { .. }
                        | ParamKind::Choice { .. }
                        | ParamKind::Radio { .. }
                        | ParamKind::Color { .. }
                        | ParamKind::Blend { .. }
                        | ParamKind::Pattern { .. }
                )
        };
        let units = params.iter().filter(|param| param.is_remembered()).filter_map(Param::unit_key);
        let keys: Vec<&str> = params.iter().filter(|param| kept(param)).map(|param| param.key.as_str()).chain(units).collect();
        let mut values = fields.clone();
        values.retain_fitting(&keys, |key, value| params.iter().any(|param| param.accepts(key, value)));
        values
    }

    /// A form for the command `key`, titled `title`: one row per parameter, OK and Cancel, no preview and no remember
    /// key. Batch, Image Processor, Open EXR Parts and Fit Image are command forms. It has every option of the
    /// dialog.
    pub fn command(key: impl Into<String>, title: impl Into<Label>, params: impl Into<Cow<'a, [Param]>>) -> Self {
        Self::new(key, title, params)
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> ParamDialogOutput {
        let ParamDialog {
            key,
            title,
            params,
            tabs,
            shared_rows,
            mut tab,
            read_only,
            title_note,
            bound,
            open,
            remember,
            remember_for,
            presets,
            ok_enabled,
            ok_reason,
            ok_label,
            cancel_label,
            alert_title: _,
            alt_resets,
            outer_gesture,
            history,
            collection_body,
            reset_to,
            layout,
            caption_column,
            closable,
            close_unchanged,
            previews,
            preview_on,
            preview,
            preview_pager,
            mut preview_overlay,
            preview_tint,
            click_centres,
            canvas,
            extra_buttons,
            patterns,
            footer_note,
            below,
            shaped,
            answers,
            values_opened,
            centre,
            preview_colors,
            held,
            host_notes,
        } = self;
        let env = Env::of(ui);
        let layout = layout.unwrap_or_else(|| env.view().get::<DialogLayout>().0);
        let preview_label = env.words::<FormDialogWords>().preview.clone();
        // A read-only dialog has one button, so there is no second one for Alt to turn into Reset.
        let alt_resets = alt_resets && !read_only;
        // The footer `FormDialog` builds for this pass, made the same way, so its `pick` reads `FormOutput::picked`.
        let footer = ResetFooter::new(ui, cancel_label.clone(), alt_resets);
        let tabbed = !tabs.is_empty();
        let every = every_param(Cow::Borrowed(&*params), &tabs);
        let read_only_ok =
            read_only.then(|| FooterButton::new(ok_label.clone().unwrap_or_else(|| env.words::<CommonWords>().ok.clone()), DialogRole::Accept));

        let mut form = match bound {
            Some(bind) => FormDialog::new(title.clone(), bind),
            None => FormDialog::own(title.clone()),
        };
        form = form
            .ok_enabled(ok_enabled)
            .alt_resets(alt_resets)
            .outer_gesture(outer_gesture)
            .history(history)
            // The tab strip is a collection in the body, as a browse list is.
            .collection_body(collection_body || tabbed)
            .closable(closable)
            .close_unchanged(close_unchanged)
            .answers(answers);
        if let Some(held) = held {
            form = form.held(held);
        }
        if let Some(reset_to) = reset_to {
            form = form.reset_with(move |_, opened, _| reset_target(opened, Some(&reset_to)));
        }
        if let Some(reason) = ok_reason {
            form = form.ok_reason(reason);
        }
        if let Some(label) = ok_label {
            form = form.ok_label(label);
        }
        if let Some(label) = cancel_label {
            form = form.cancel_label(label);
        }
        let host_opened = values_opened.is_some();
        if let Some(opening) = values_opened {
            form = form.opening(opening);
        }
        if let Some(open) = open {
            form = form.opened(open);
        }
        if previews {
            form = form.preview(preview_on).button_place(layout.button_place());
        }
        // The pane shows only in a dialog that previews.
        let preview = preview.filter(|_| previews);
        // The footer rows' buttons follow the extra buttons. A button's label cannot change inside the pass that
        // draws it, so it shows the state the previous pass left.
        let footer_rows: Vec<&Param> = params.iter().filter(|param| param.footer_on().is_some()).collect();
        let footer_id = crate::runtime::memory::unique(("paramDialog.footer", key.as_str()));
        let footer_states: Vec<bool> =
            if footer_rows.is_empty() { Vec::new() } else { with_session(ui.ctx(), footer_id, |footer: &mut FooterRows| footer.on.clone()) };
        let footer_buttons = footer_rows.iter().enumerate().map(|(at, param)| {
            let on = footer_states.get(at).copied().unwrap_or(false);
            let label = param.footer_on().filter(|_| on).unwrap_or_else(|| param.label.clone());
            FooterButton::new(label, DialogRole::Other).stays_open(true)
        });
        let buttons: Vec<FooterButton> = extra_buttons.iter().map(|(_, button)| button.clone()).chain(footer_buttons).collect();
        if !buttons.is_empty() {
            form = form.extra_buttons(buttons);
        }
        let presets_label = Env::of(ui).words::<ParamDialogWords>().presets.clone();
        let mut preset_events = Vec::new();
        if let Some(note) = footer_note {
            form = form.footer_extra(move |ui| {
                Text::new(note.clone()).shade(Shade::Muted).show(ui);
            });
        }
        // `FormDialog` has one slot for `Dialog` settings: the read-only footer, then the title note, then the caller's
        // shape, last so it can still change what the dialog set itself.
        if read_only_ok.is_some() || title_note.is_some() || shaped.is_some() {
            form = form.shaped(move |mut dialog| {
                if let Some(ok) = read_only_ok {
                    dialog = dialog.buttons([ok]);
                }
                if let Some(note) = title_note {
                    dialog = dialog.title_note(note);
                }
                if let Some(Shaped(shape)) = shaped {
                    dialog = shape(dialog);
                }
                dialog
            });
        }

        let mut live = Live::default();
        let mut rows = None;
        let mut shown_preview = ShownPreview::default();
        let body = DialogRows {
            title: &title,
            shared: &params,
            tabs: &tabs,
            place: shared_rows,
            patterns: patterns.as_ref(),
            preview_colors: &preview_colors,
            sliders: layout.sliders(),
            // A tabbed document dialog (Document Properties) is an aligned form: no colons, one caption column, the
            // controls filling the rest of the row.
            labels: if layout == ParamLayout::Document && !tabs.is_empty() { Some(LabelScheme::Inspector) } else { layout.labels() },
            caption_column: caption_column && layout == ParamLayout::Document,
            canvas,
            read_only,
        };
        let shown = form.show(ui, |ui, cx, values| {
            if cx.first() && !host_opened {
                let saved = remember.as_ref().and_then(|key| remembered_values(ui.ctx(), key, remember_for));
                let offered = patterns.as_ref().map(|source| source.rows(ui.ctx(), &Env::of(ui))).unwrap_or_default();
                Self::open_values(&every, &offered, saved.as_ref(), values);
            }
            for answer in cx.take_answers() {
                let answered = match &answer {
                    HostAnswer::Color { key, .. } | HostAnswer::Eyedropper { key, .. } | HostAnswer::Path { key, .. } => {
                        every.iter().find(|param| param.key == *key)
                    }
                    HostAnswer::Listing { .. } => None,
                };
                let held = answered.and_then(|param| values.get(&param.key).cloned());
                match answer {
                    HostAnswer::Color { key, color } | HostAnswer::Eyedropper { key, color } => set_color_field(&every, values, &key, color),
                    HostAnswer::Path { key, path } => values.set(&key, FieldValue::Text(path.to_string_lossy().into_owned())),
                    HostAnswer::Listing { key, files } => {
                        let files = files.into_iter().map(|file| FieldValue::Text(file.to_string_lossy().into_owned())).collect();
                        if let Some(param) = every.iter().find(|param| param.key == key) {
                            values.set(param.folder_files_key(), FieldValue::List(files));
                        }
                    }
                }
                if let Some(param) = answered {
                    param.drop_dependents(held.as_ref(), values);
                }
            }
            // A footer row pressed in the previous pass flips once; a measuring run leaves it for the real one.
            if !footer_rows.is_empty()
                && !crate::runtime::measure::active(ui.ctx())
                && let Some(flip) = with_session(ui.ctx(), footer_id, |footer: &mut FooterRows| footer.flip.take())
            {
                let on = values.bool(&flip).unwrap_or(false);
                values.set(flip, FieldValue::Bool(!on));
            }
            if let Some(presets) = &presets {
                show_presets(ui, &presets_label, presets, values, &mut preset_events);
            }
            if let Some(canvas) = canvas {
                cx.keep_view_open(canvas);
            }
            // The canvas shows the picture the pane does: while a click on the pane centres it, a press on the image is
            // the pane's locator press, which the app answers with `HostedParamDialog::centre_preview`.
            if let Some(canvas) = canvas.filter(|_| click_centres && preview.is_some()) {
                cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
            }
            live = cx.memory::<Live>().clone();
            // Only a `Ready` preview has a picture to centre on.
            if let (Some(point), Some(PreviewState::Ready(texture))) = (centre, &preview) {
                live.view = live.view.centred_on(point, texture.size);
            }
            // The `tab` binding, when there is one, says which tab shows; the dialog's memory does otherwise.
            let last = tabs.len().saturating_sub(1);
            let bound_tab = match tab.as_mut().map(|bind| bind.reborrow().read(&mut **cx)) {
                Some(Some(Mixed::Same(at))) => Some(at),
                _ => None,
            };
            let held_tab = bound_tab.unwrap_or_else(|| cx.memory::<OpenTab>().0).min(last);
            let mut shown_tab = held_tab;
            let mut show_rows = |ui: &mut Ui, values: &mut DialogFields| rows = body.show(ui, &mut shown_tab, values);
            // A measuring run paints nothing, so it leaves the overlay, which runs once, to the real run.
            let overlay = if crate::runtime::measure::active(ui.ctx()) { None } else { preview_overlay.take() };
            let mut extras = PaneExtras { pager: preview_pager.clone(), overlay, tint: preview_tint };
            match (&preview, layout) {
                (Some(state), ParamLayout::Beside) => {
                    Row::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        Stack::new().show(ui, |ui, _| {
                            shown_preview = show_preview(ui, &preview_label, state, &mut extras, click_centres, ZoomRow::Above, &mut live);
                        });
                    });
                }
                (Some(state), ParamLayout::Document) => {
                    Stack::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        shown_preview = show_thumbnail_preview(ui, &preview_label, state, &mut extras, click_centres, &host_notes);
                    });
                }
                (Some(state), layout) => {
                    let zoom = match layout {
                        ParamLayout::Document => ZoomRow::Hidden,
                        ParamLayout::Stacked => ZoomRow::Below,
                        _ => ZoomRow::Above,
                    };
                    Stack::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        shown_preview = show_preview(ui, &preview_label, state, &mut extras, click_centres, zoom, &mut live);
                    });
                }
                (None, _) => show_rows(ui, values),
            }
            if let Some(bind) = tab.as_mut().filter(|_| bound_tab != Some(shown_tab)) {
                let event = EditEvent::set(bound_tab.map_or(Mixed::Mixed, Mixed::Same), shown_tab, title.as_str().into(), None);
                let _ = bind.reborrow().write(&mut **cx, &event);
            }
            cx.memory::<OpenTab>().0 = shown_tab;
            if let Some(BelowRows(draw)) = &below {
                draw(ui, values);
            }
            // A document dialog's notes stand beside its small picture.
            if !(preview.is_some() && layout == ParamLayout::Document) {
                for note in &host_notes {
                    Text::new(note.clone()).show(ui);
                }
            }
            *cx.memory::<Live>() = live.clone();
            if !footer_rows.is_empty() {
                let states: Vec<bool> = footer_rows.iter().map(|param| values.bool(&param.key).unwrap_or(false)).collect();
                with_session(ui.ctx(), footer_id, |footer: &mut FooterRows| footer.on = states);
            }
            // OK and Enter wait while a row's text does not parse.
            rows.as_ref().is_none_or(|rows| !rows.unparsed)
        });

        // `picked` is this pass's press: a second pass of the frame has none, so a press is reported once.
        let pick = footer.pick(shown.picked);
        let pressed = match pick {
            Some(ResetPick::Extra(at)) => extra_buttons.get(at).map(|(id, _)| id.clone()),
            _ => None,
        };
        if let Some(ResetPick::Extra(at)) = pick
            && let Some(param) = at.checked_sub(extra_buttons.len()).and_then(|at| footer_rows.get(at))
        {
            with_session(ui.ctx(), footer_id, |footer: &mut FooterRows| footer.flip = Some(param.key.clone()));
            crate::runtime::frame::request_repaint(ui.ctx());
        }
        // Footer and row buttons share one channel. The rows' own slot is emptied, so a host that absorbs the rows
        // files the press once.
        let action = rows.as_mut().and_then(|rows: &mut ParamRowsOutput| rows.action.take()).or(pressed);
        let help_link = rows.as_ref().and_then(|rows| rows.help_link.clone());
        // A read-only dialog answers with no values, whichever way it closes.
        let result = if read_only {
            shown.closed.is_some().then(|| Self::result(key, &every, None, remember_for, DialogFields::new()))
        } else {
            shown.accepted.map(|fields| Self::result(key, &every, remember, remember_for, fields))
        };
        ParamDialogOutput {
            response: shown.response,
            closed: shown.closed,
            result,
            rows,
            values: shown.value,
            preview_on: shown.preview_on,
            comparing: shown.comparing,
            preview_click: shown_preview.clicked,
            action,
            presets: preset_events,
            help_link,
            preview_step: shown_preview.step,
            preview_centre: shown_preview.centre,
        }
    }
}

/// The values Reset restores: those from when the dialog opened, with `reset_to`'s over them. The one rule for the
/// Alt footer and for a host's reset.
fn reset_target(opened: &DialogFields, reset_to: Option<&DialogFields>) -> DialogFields {
    let mut values = opened.clone();
    for (name, value) in reset_to.into_iter().flat_map(DialogFields::iter) {
        values.set(name, value.clone());
    }
    values
}

/// The presets row above the rows: a pick writes the preset's values over `values` as one edit, and the presets made,
/// renamed or deleted come back in `events` for the host to store.
fn show_presets(ui: &mut Ui, label: &Label, presets: &ParamPresets<'_>, values: &mut DialogFields, events: &mut Vec<NamedPresetEvent<String>>) {
    let current = presets.values.iter().find(|(_, preset)| preset_applies(preset, values)).map(|(name, _)| name.clone());
    let source = PresetSource::listed(presets.rows);
    let picker = PresetPicker::new(label.clone(), Bind::given(current), &source, Bind::kept()).field(PickerField::Name);
    let taken: Vec<&str> = presets.built_in.iter().map(String::as_str).chain(presets.rows.iter().map(|row| row.label.as_str())).collect();
    let check = NameCheck::new(taken, |taken, name| {
        let name = name.to_lowercase();
        taken.iter().any(|other| other.to_lowercase() == name).then_some(PresetError::NameTaken)
    });
    let shown = NamedPresetRow::new(label.clone(), picker).check_name(check).built_in(|name: &String| presets.built_in.contains(name)).show(ui);
    for event in shown.events {
        match event {
            NamedPresetEvent::Apply(name) => {
                if let Some((_, preset)) = presets.values.iter().find(|(preset, _)| *preset == name) {
                    apply_preset(preset, values);
                }
            }
            event @ (NamedPresetEvent::Create { .. } | NamedPresetEvent::Rename { .. } | NamedPresetEvent::Delete(_)) => events.push(event),
            _ => {}
        }
    }
}

/// Whether `values` hold every value `preset` sets.
fn preset_applies(preset: &DialogFields, values: &DialogFields) -> bool {
    preset.iter().all(|(key, value)| values.get(key) == Some(value))
}

/// Writes the values of `preset` over `values`.
fn apply_preset(preset: &DialogFields, values: &mut DialogFields) {
    for (key, value) in preset.iter() {
        values.set(key, value.clone());
    }
}

/// The values `ctx` remembers under `key` for `lifetime`: the saved tier for `Saved`, the session tier for `Run`.
fn remembered_values(ctx: &Context, key: &SaveKey, lifetime: Lifetime) -> Option<DialogFields> {
    match lifetime {
        Lifetime::Run => crate::runtime::memory::run_value::<Option<DialogFields>>(ctx, key),
        _ => MemoryStage::peek_saved::<Option<DialogFields>>(ctx, key),
    }
}

/// Applies the host's `answers` to `values` for `params`: a colour, path or eyedropper answer writes its row, and a
/// row that resets other rows drops them.
pub(crate) fn apply_answers(params: &[Param], values: &mut DialogFields, answers: Vec<HostAnswer>) {
    for answer in answers {
        let answered = match &answer {
            HostAnswer::Color { key, .. } | HostAnswer::Eyedropper { key, .. } | HostAnswer::Path { key, .. } => {
                params.iter().find(|param| param.key == *key)
            }
            HostAnswer::Listing { .. } => None,
        };
        let held = answered.and_then(|param| values.get(&param.key).cloned());
        match answer {
            HostAnswer::Color { key, color } | HostAnswer::Eyedropper { key, color } => set_color_field(params, values, &key, color),
            HostAnswer::Path { key, path } => values.set(&key, FieldValue::Text(path.to_string_lossy().into_owned())),
            HostAnswer::Listing { key, files } => {
                let files = files.into_iter().map(|file| FieldValue::Text(file.to_string_lossy().into_owned())).collect();
                if let Some(param) = params.iter().find(|param| param.key == key) {
                    values.set(param.folder_files_key(), FieldValue::List(files));
                }
            }
        }
        if let Some(param) = answered {
            param.drop_dependents(held.as_ref(), values);
        }
    }
}

/// What the footer of an installed Reset footer reads from the body.
#[derive(Clone, Copy, Default)]
struct Footing {
    changed: bool,
    ok: Option<bool>,
}

/// What a footer with a Reset alternative answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResetPick {
    /// The accepting button.
    Accept,
    /// The second button while Alt is held: the dialog resets its values and stays open.
    Reset,
    /// The second button while Alt is not held: the dialog closes without an answer.
    Cancel,
    /// The extra button at this index in the order [`extra`](ResetFooter::extra) was given.
    Extra(usize),
}

/// The two footer buttons of a dialog whose Cancel turns into Reset while Alt is held. Reset restores the values from
/// when the dialog opened and keeps the dialog open (`FooterButton::stays_open`). The dialog takes them through
/// `Dialog::buttons`, so close reasons, `close_guard`, Enter and `DialogOutput::picked` work as for any other buttons;
/// [`pick`](ResetFooter::pick) reads which one was picked. `ParamDialog` builds its footer with it, and so does any
/// other dialog with this behaviour.
///
/// ```ignore
/// let footer = ResetFooter::new(ui, None, true);
/// let accept = FooterButton::new(common.ok.clone(), DialogRole::Accept);
/// let dialog = footer.install(Dialog::new("Levels"), accept, |cx| { /* restore the values and what the body keeps */ });
/// let shown = dialog.show(ui, |ui, cx| {
///     /* rows */
///     ResetFooter::report(cx, values_changed, true);
/// });
/// ```
#[derive(Clone, Debug)]
pub struct ResetFooter {
    resetting: bool,
    cancel: Label,
    reset: Label,
    extra: Vec<FooterButton>,
}

impl ResetFooter {
    /// The footer for this pass: `cancel` names the second button when it is not Cancel; with `alt_resets` false
    /// the second button stays Cancel whatever is held.
    pub fn new(ui: &Ui, cancel: Option<Label>, alt_resets: bool) -> Self {
        let env = Env::of(ui);
        ResetFooter {
            resetting: alt_resets && crate::runtime::pointer::mods(ui.ctx()).alt,
            cancel: cancel.unwrap_or_else(|| env.words::<CommonWords>().cancel.clone()),
            reset: env.words::<ParamDialogWords>().reset.clone(),
            extra: Vec::new(),
        }
    }

    /// Buttons after the second one, which [`install`](ResetFooter::install) gives the dialog with the two.
    #[must_use]
    pub fn extra(mut self, buttons: impl IntoIterator<Item = FooterButton>) -> Self {
        self.extra = buttons.into_iter().collect();
        self
    }

    /// Whether the second button is Reset in this pass.
    pub fn resetting(&self) -> bool {
        self.resetting
    }

    /// `accept` and the second button, for `Dialog::buttons`. The second is Reset (role `Other`, stays open, enabled
    /// while `changed`) or Cancel.
    pub fn buttons(&self, accept: FooterButton, changed: bool) -> [FooterButton; 2] {
        let second = if self.resetting {
            FooterButton::new(self.reset.clone(), DialogRole::Other).stays_open(true).enabled(changed)
        } else {
            FooterButton::new(self.cancel.clone(), DialogRole::Cancel)
        };
        [accept, second]
    }

    /// Sets the Reset button's enabled state to `changed` in `buttons` as `buttons` made them; the rule for
    /// `Dialog::settle_buttons`, for a dialog whose rows decide `changed` in the same pass.
    pub fn settle(&self, buttons: &mut [FooterButton], changed: bool) {
        if let (true, Some(second)) = (self.resetting, buttons.get_mut(1)) {
            second.set_enabled(changed);
        }
    }

    /// `dialog` (a `Dialog` or a `FullWindowDialog`) with this footer: `accept` and the second button, their enabled state settled from what the body
    /// passed to [`report`](ResetFooter::report) in the same pass, and `restore` run in the footer step when the Reset
    /// button is picked. `restore` gets the dialog's body context, so it reaches the values and the body's memory;
    /// the dialog shows the restored values in its next pass.
    pub fn install<'a, D: FooterHooks<'a>>(self, dialog: D, accept: FooterButton, restore: impl FnMut(&mut BodyCx<'_>) + 'a) -> D {
        let mut restore = restore;
        let pick = self.clone();
        let settle = self.clone();
        dialog
            .buttons(self.buttons(accept, false).into_iter().chain(self.extra.iter().cloned()))
            .settle_buttons(move |cx, buttons| {
                let footing = *cx.memory::<Footing>();
                settle.settle(buttons, footing.changed);
                if let (Some(ok), Some(accept)) = (footing.ok, buttons.first_mut()) {
                    accept.set_enabled(ok);
                }
            })
            .on_pick(move |button, cx| {
                if pick.resetting() && button.role() == DialogRole::Other && *button.label() == pick.reset {
                    restore(cx);
                    crate::runtime::frame::request_repaint(cx.memory.ctx());
                }
            })
    }

    /// What the footer of an [installed](ResetFooter::install) dialog reads in the same pass: whether the values
    /// differ from the opening ones (Reset is enabled then) and whether OK can be pressed.
    pub fn report(cx: &mut BodyCx<'_>, changed: bool, ok: bool) {
        *cx.memory::<Footing>() = Footing { changed, ok: Some(ok) };
        Dialog::report_changed(cx, changed);
    }

    /// Which button `DialogOutput::picked` names, for the buttons of this footer.
    pub fn pick(&self, picked: Option<usize>) -> Option<ResetPick> {
        match picked? {
            0 => Some(ResetPick::Accept),
            1 if self.resetting => Some(ResetPick::Reset),
            1 => Some(ResetPick::Cancel),
            at => Some(ResetPick::Extra(at - 2)),
        }
    }
}

/// The rows a dialog shows: its own rows and, in a tabbed dialog, its tabs, all drawn the same way.
struct DialogRows<'p> {
    /// The dialog's title, which names its own rows and its tab strip.
    title: &'p Label,
    /// The dialog's own rows: the shared rows of a tabbed dialog.
    shared: &'p [Param],
    tabs: &'p [ParamTab],
    place: SharedRows,
    patterns: Option<&'p PresetSource<'p, String>>,
    preview_colors: &'p [(String, Srgba)],
    sliders: SliderFieldLayout,
    /// The label scheme the layout sets for the rows.
    labels: Option<LabelScheme>,
    /// The rows' captions share one column ([`ParamDialog::caption_column`]).
    caption_column: bool,
    canvas: Option<egui::Id>,
    /// The dialog shows values only.
    read_only: bool,
}

impl DialogRows<'_> {
    /// The rows `params` over `values`, their group named `label`; `read_only` for the rows of a read-only tab.
    fn group(&self, ui: &mut Ui, label: &Label, params: &[Param], read_only: bool, values: &mut DialogFields) -> ParamRowsOutput {
        let mut rows = ParamRows::new(label.clone(), params, Bind::bound(values))
            .preview_colors(self.preview_colors)
            .sliders(self.sliders)
            .read_only(self.read_only || read_only);
        if let Some(source) = self.patterns {
            rows = rows.patterns(source);
        }
        if let Some(scheme) = self.labels {
            rows = rows.labels(scheme);
        }
        if let Some(canvas) = self.canvas {
            rows = rows.canvas(canvas);
        }
        if self.caption_column {
            return crate::runtime::env::with_env(
                ui,
                |scope| {
                    scope.values.set(CaptionColumn::SHARED);
                },
                |ui| rows.show(ui),
            );
        }
        rows.show(ui)
    }

    /// The body's rows. A tabbed dialog shows its shared rows where `place` puts them, the tab strip (with more than one
    /// tab), whose pick moves `shown`, and the rows of the tab `shown` under its note. One output for all the rows drawn.
    fn show(&self, ui: &mut Ui, shown: &mut usize, values: &mut DialogFields) -> Option<ParamRowsOutput> {
        let (title, shared, tabs) = (self.title, self.shared, self.tabs);
        if tabs.is_empty() {
            return Some(self.group(ui, title, shared, false, values));
        }
        let mut out = None;
        if self.place == SharedRows::Above && !shared.is_empty() {
            merge_rows(&mut out, self.group(ui, title, shared, false, values));
        }
        if tabs.len() > 1 {
            let last = tabs.len() - 1;
            let strip = TabStrip::new(title.clone())
                .kind(TabKind::Text)
                .shown(*shown)
                .show_items(ui, &Titles(tabs), &mut |key, _| Tab::new(tabs.get(*key).map_or_else(Label::default, |tab| tab.title.clone())));
            for event in strip.response.events {
                if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                    && let Some(key) = keys.into_iter().last()
                {
                    *shown = key.min(last);
                }
            }
        }
        if let Some(tab) = tabs.get(*shown) {
            if let Some(note) = tab.note.as_ref().filter(|_| tab.read_only) {
                Notice::new(Tone::Info, note.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            merge_rows(&mut out, self.group(ui, &tab.title, &tab.params, tab.read_only, values));
        }
        if self.place == SharedRows::Below && !shared.is_empty() {
            merge_rows(&mut out, self.group(ui, title, shared, false, values));
        }
        out
    }
}

/// Adds `next`, the output of rows shown after those of `out`, to `out`.
fn merge_rows(out: &mut Option<ParamRowsOutput>, next: ParamRowsOutput) {
    *out = Some(match out.take() {
        Some(earlier) => earlier.merged(next),
        None => next,
    });
}

/// What a dialog's preview pane shows beside its picture: the pager row under it, the host's drawing over the picture
/// and the tint.
struct PaneExtras<'p> {
    pager: Option<PanePager>,
    /// Taken by the run that paints: it runs once.
    overlay: Option<PaneOverlay<'p>>,
    tint: Option<DataColor>,
}

/// What the preview pane reported in a pass.
#[derive(Clone, Copy, Default)]
struct ShownPreview {
    /// The picture point a click landed on.
    clicked: Option<Pos2>,
    /// The picture point at the centre of the view the pane drew.
    centre: Option<Pos2>,
    /// The step a press on the pager made.
    step: Option<PagerStep>,
}

/// Where the zoom row stands: over the preview pane, under it or nowhere.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ZoomRow {
    Above,
    Below,
    /// No zoom row: the pane zooms with the wheel alone, as a document dialog's does.
    Hidden,
}

/// A document dialog's preview: a small picture of the page at the start, and beside it the pager ("Preview: page 1 of
/// 3") with the host's `notes` under it. There is no well across the dialog and no zoom row.
fn show_thumbnail_preview(
    ui: &mut Ui,
    label: &Label,
    state: &PreviewState,
    extras: &mut PaneExtras<'_>,
    click_centres: bool,
    notes: &[Label],
) -> ShownPreview {
    let picture = match state {
        PreviewState::Ready(texture) => Some(texture.size),
        PreviewState::Pending(_) | PreviewState::Failed(_) => None,
    };
    let mut shown = ShownPreview::default();
    Row::new().show(ui, |ui, _| {
        let mut pane = PreviewPane::state(label.clone(), state.clone(), &PreviewView::fixed()).click_centres(click_centres).thumbnail(true);
        if let Some(tint) = extras.tint {
            pane = pane.tint(tint);
        }
        // `ParamDialog::show` leaves no overlay in a measuring run, which paints nothing.
        if let Some(overlay) = extras.overlay.take() {
            pane = pane.overlay(overlay.into_paint());
        }
        let pane = pane.show(ui);
        let mut step = None;
        Stack::new().show(ui, |ui, _| {
            step = extras.pager.as_ref().and_then(|pager| pager.clone().show_in(ui, true));
            for note in notes {
                Text::new(note.clone()).show(ui);
            }
        });
        shown = ShownPreview { clicked: pane.clicked, centre: picture.map(|size| pane.shown.centre(size)), step };
    });
    shown
}

/// The pane, the zoom row where `zoom_row` puts it and the pager centred under them. Only a `Ready` picture has a zoom
/// row above the pane, and a centre: a pending or failed preview has nothing to zoom (the row under it shows always).
fn show_preview(
    ui: &mut Ui,
    label: &Label,
    state: &PreviewState,
    extras: &mut PaneExtras<'_>,
    click_centres: bool,
    zoom_row: ZoomRow,
    live: &mut Live,
) -> ShownPreview {
    let picture = match state {
        PreviewState::Ready(texture) => Some(texture.size),
        PreviewState::Pending(_) | PreviewState::Failed(_) => None,
    };
    let above = (picture.is_some() && zoom_row == ZoomRow::Above).then(|| PreviewZoom::new(&live.view).show(ui));
    let mut pane = PreviewPane::state(label.clone(), state.clone(), &live.view).click_centres(click_centres);
    if let Some(tint) = extras.tint {
        pane = pane.tint(tint);
    }
    // `ParamDialog::show` leaves no overlay in a measuring run, which paints nothing.
    if let Some(overlay) = extras.overlay.take() {
        pane = pane.overlay(overlay.into_paint());
    }
    let pane = pane.show(ui);
    // Under the pane the row is there before the picture is: a dialog sized from its first pass keeps room for it.
    let below = (zoom_row == ZoomRow::Below).then(|| PreviewZoom::new(&live.view).show(ui));
    let step = extras
        .pager
        .as_ref()
        .and_then(|pager| Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| pager.clone().show(ui)).inner.flatten());
    live.view = above.or(below).and_then(|zoom| zoom.view).or(pane.view).unwrap_or(pane.shown.clone());
    ShownPreview { clicked: pane.clicked, centre: picture.map(|size| pane.shown.centre(size)), step }
}

/// Builds the dialog over the host-owned values of a [`HostedParamDialog`] for one pass.
type Build = dyn for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send;

/// A [`ParamDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns: the host keeps the values, and `build` makes
/// the dialog over them each pass.
///
/// - The host keeps the Preview check box of a dialog that [`previews`](ParamDialog::previews): it starts on, so
///   [`is_previewing`](HostedParamDialog::is_previewing) holds from construction, before the first pass.
/// - The preview picture comes from [`set_preview`](HostedParamDialog::set_preview) between passes, since `build` cannot
///   capture a picture that changes; [`is_previewing`](HostedParamDialog::is_previewing) and
///   [`is_comparing`](HostedParamDialog::is_comparing) say what to render.
/// - What the host's jobs find about the values comes the same way: the lines under the rows from
///   [`set_notes`](HostedParamDialog::set_notes), a row's label and whether it shows from
///   [`set_param_label`](HostedParamDialog::set_param_label) and [`set_param_shown`](HostedParamDialog::set_param_shown),
///   and whether OK can be pressed from
///   [`set_ready`](HostedParamDialog::set_ready). What the values alone decide stays in `build`.
/// - With [`held_ok`](HostedParamDialog::held_ok), OK files its answer for the host to check
///   ([`take_ok`](HostedParamDialog::take_ok)) and the dialog waits, OK disabled, for
///   [`answer_ok`](HostedParamDialog::answer_ok).
///
/// ```ignore
/// host.open(HostedParamDialog::new(ctx, "radial-blur", DialogFields::new(), |fields| {
///     ParamDialog::new("radial-blur", "Radial Blur", params.clone()).values(Bind::bound(fields))
/// }));
/// // each pass: if dialog.is_previewing() { dialog.set_preview(Some(render(dialog.fields()))) }
/// ```
pub struct HostedParamDialog {
    id: DialogId,
    key: String,
    instance: Option<DialogId>,
    fields: DialogFields,
    build: Box<Build>,
    requests: HostRequests,
    /// The last range alert a row raised, with its title and the env the dialog showed in (which words it), until the
    /// host takes it.
    range_alert: Option<(Label, RangeAlert, Env)>,
    /// The values the dialog opened with, passed to the form as the values Reset restores.
    opening: DialogFields,
    /// Whether the dialog has the Preview check box ([`ParamDialog::previews`] of `build`).
    previews: bool,
    /// The Preview check box's value: bound to the dialog's `preview_on` prop, so it is known before the first pass.
    preview_on: bool,
    /// The preview picture the host last gave (`set_preview`).
    picture: Option<PreviewState>,
    /// The pager row under the preview pane the host last gave (`set_preview_pager`).
    pager: Option<PanePager>,
    /// The host's drawing over the preview picture (`set_preview_overlay`).
    overlay: Option<SharedOverlay>,
    /// The tint of the preview picture (`set_preview_tint`).
    tint: Option<DataColor>,
    /// The lines under the rows the host last gave (`set_notes`).
    notes: Vec<Label>,
    /// The row labels and shown states the host last gave (`set_param_label`, `set_param_shown`).
    rows: HostRows,
    /// Whether the host lets OK be pressed, and why not (`set_ready`).
    ready: Result<(), Label>,
    /// The tab a tabbed dialog shows: bound to the dialog's `tab` prop, so the host sets it before a pass.
    tab: usize,
    /// What the dialog showed in its last pass.
    shown: ShownState,
    /// Whether OK files its answer for the host instead of closing ([`held_ok`](HostedParamDialog::held_ok)).
    hold: bool,
    /// The OK the host holds back until it answers `answer_ok`, and its refusal, shown until a value changes.
    held: HeldAnswer<DialogFields>,
    /// The picture point the next pass centres the preview on ([`centre_preview`](HostedParamDialog::centre_preview)).
    centre: Option<Pos2>,
    /// The named presets the presets row lists ([`set_presets`](HostedParamDialog::set_presets)).
    presets: Option<HeldPresets>,
    /// The preset edits of the dialog's passes, until the host takes them.
    preset_events: Vec<NamedPresetEvent<String>>,
}

/// The presets a hosted dialog lists: the rows, the values each applies and the built-in names.
struct HeldPresets {
    rows: Vec<PresetRow<String>>,
    values: Vec<(String, DialogFields)>,
    built_in: Vec<String>,
}

/// What a [`HostedParamDialog`] showed in its last pass: whether the Preview check box is held for comparing, and the
/// picture point at the centre of the preview pane.
#[derive(Clone, Copy, Debug, Default)]
struct ShownState {
    comparing: bool,
    centre: Option<Pos2>,
}

/// What a [`HostedParamDialog`]'s host says about rows `build` makes, by row key: the label, and whether the row shows.
#[derive(Default)]
struct HostRows {
    labels: Vec<(String, Label)>,
    shown: Vec<(String, bool)>,
}

impl HostRows {
    /// Puts `value` under `key`, replacing what `key` had.
    fn put<T>(list: &mut Vec<(String, T)>, key: String, value: T) {
        match list.iter_mut().find(|(k, _)| *k == key) {
            Some((_, old)) => *old = value,
            None => list.push((key, value)),
        }
    }

    /// Gives the rows of `dialog`, shared and in every tab, the host's labels and shown states.
    fn apply(&self, dialog: &mut ParamDialog<'_>) {
        if self.labels.is_empty() && self.shown.is_empty() {
            return;
        }
        let apply = |params: &mut [Param]| {
            for param in params {
                if let Some((_, label)) = self.labels.iter().find(|(key, _)| *key == param.key) {
                    param.label = label.clone();
                }
                if let Some((_, shown)) = self.shown.iter().find(|(key, _)| *key == param.key) {
                    param.shown = *shown;
                }
            }
        };
        apply(dialog.params.to_mut());
        for tab in &mut dialog.tabs {
            apply(&mut tab.params);
        }
    }
}

impl HostedParamDialog {
    /// A hosted dialog for the command `key`, opening with `fields`; `build` makes the dialog over the values. The
    /// values open now, as a `ParamDialog`'s first pass opens its own: the remembered values `ctx` keeps over
    /// `fields`, then the defaults of the keys still missing. Automation reads and sets these values before any pass.
    pub fn new(
        ctx: &Context,
        key: impl Into<String>,
        mut fields: DialogFields,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> Self {
        let key = key.into();
        let ParamDialog { params, tabs, remember, remember_for, patterns, previews, .. } = build(&mut fields);
        // No pass runs here: the rows are built in the root scope, whose words name them; only their keys are read.
        let patterns = patterns.map(|source| source.rows(ctx, &Env::of_root(ctx))).unwrap_or_default();
        let params = every_param(params, &tabs).into_owned();
        let saved = remember.and_then(|key| remembered_values(ctx, &key, remember_for));
        ParamDialog::open_values(&params, &patterns, saved.as_ref(), &mut fields);
        let id = dialog_id(key.as_str());
        HostedParamDialog {
            id,
            key,
            instance: Some(id),
            opening: fields.clone(),
            fields,
            build: Box::new(build),
            requests: HostRequests::new(),
            range_alert: None,
            previews,
            preview_on: true,
            picture: None,
            pager: None,
            overlay: None,
            tint: None,
            notes: Vec::new(),
            rows: HostRows::default(),
            ready: Ok(()),
            tab: 0,
            shown: ShownState::default(),
            hold: false,
            held: HeldAnswer::new(),
            centre: None,
            presets: None,
            preset_events: Vec::new(),
        }
    }

    /// A hosted command form: the same as [`new`](HostedParamDialog::new), with `build` making a
    /// [`ParamDialog::command`].
    pub fn command(
        ctx: &Context,
        key: impl Into<String>,
        fields: DialogFields,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> Self {
        Self::new(ctx, key, fields, build)
    }

    /// Names this dialog among others made with the same key (one per document, say), so they keep separate state.
    /// Without it the dialog is the one of its key: dialogs of different keys stay open together, and the same key
    /// opened again replaces it. Its result is taken with [`DialogHost::take_keyed`](crate::dialog::DialogHost::take_keyed)
    /// or by id.
    #[must_use]
    pub fn instance(mut self, instance: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id = dialog_id((self.key.as_str(), instance));
        self.instance = Some(self.id);
        self
    }

    /// The values the dialog shows.
    pub fn fields(&self) -> &DialogFields {
        &self.fields
    }

    /// The Browse button that was pressed, once.
    pub fn pick_path(&mut self) -> Option<PathRequest> {
        self.requests.take_path()
    }

    /// The point of the preview picture the last click landed on (`ParamDialogOutput::preview_click`), once.
    pub fn take_preview_click(&mut self) -> Option<Pos2> {
        self.requests.take_preview_click()
    }

    /// Answers the Browse request `key` with the chosen path.
    pub fn answer_path(&mut self, key: impl Into<String>, path: std::path::PathBuf) {
        self.requests.answer_path(key, path);
    }

    /// Answers the eyedropper request `key` with the sampled colour.
    pub fn answer_eyedropper(&mut self, key: impl Into<String>, color: craft_theme::Srgba) {
        self.requests.answer_eyedropper(key, color);
    }

    /// Sets what the preview pane shows from the next pass on: the progress line, the failure or the picture; `None`
    /// shows none.
    pub fn set_preview(&mut self, preview: Option<PreviewState>) {
        self.picture = preview;
    }

    /// Sets the pager row under the preview pane from the next pass on; `None` shows none.
    pub fn set_preview_pager(&mut self, pager: Option<PanePager>) {
        self.pager = pager;
    }

    /// Sets the host's drawing over the preview picture from the next pass on; `None` draws none.
    pub fn set_preview_overlay(&mut self, overlay: Option<SharedOverlay>) {
        self.overlay = overlay;
    }

    /// Sets the tint that multiplies the `Ready` preview picture's colours from the next pass on; `None` shows it
    /// untinted.
    pub fn set_preview_tint(&mut self, tint: Option<DataColor>) {
        self.tint = tint;
    }

    /// Sets the lines shown under the rows, after the build's `below` section, from the next pass on: what the host's
    /// jobs found about the values (pages still being checked, pages a picture covers), which `build` cannot capture
    /// as they change. An empty list shows none.
    pub fn set_notes(&mut self, notes: Vec<Label>) {
        self.notes = notes;
    }

    /// Sets the label of the row `key` from the next pass on, over the one `build` gives: when the words come from the
    /// host's jobs (the pages a box applies to), which `build` cannot capture as they change.
    pub fn set_param_label(&mut self, key: impl Into<String>, label: impl Into<Label>) {
        HostRows::put(&mut self.rows.labels, key.into(), label.into());
    }

    /// Sets whether the row `key` shows from the next pass on, over `build`'s [`Param::shown`]: when the host's jobs
    /// decide it (a box offered only when a check finds something). A hidden row takes no space and refuses a set
    /// through automation, as one hidden by [`Param::visible_when`]; its value stays in the fields and the result.
    pub fn set_param_shown(&mut self, key: impl Into<String>, shown: bool) {
        HostRows::put(&mut self.rows.shown, key.into(), shown);
    }

    /// Sets whether the host lets OK be pressed from the next pass on: `Err` disables OK, Enter and `confirm`, with
    /// the label as the disabled button's tooltip, whatever `build` allows; `Ok` leaves OK to `build`. A host whose
    /// jobs decide OK (a check that runs over the document) sets it as their results come in.
    pub fn set_ready(&mut self, ready: Result<(), Label>) {
        self.ready = ready;
    }

    /// The step the preview pager took, once.
    pub fn take_preview_step(&mut self) -> Option<PagerStep> {
        self.requests.take_preview_step()
    }

    /// Opens the dialog on the tab `tab` of a tabbed dialog ([`ParamDialog::tabs`]).
    #[must_use]
    pub fn tab(mut self, tab: usize) -> Self {
        self.tab = tab;
        self
    }

    /// Shows the tab `tab` from the next pass on.
    pub fn set_tab(&mut self, tab: usize) {
        self.tab = tab;
    }

    /// The tab shown in the last pass; before the first pass, the tab it opens on.
    pub fn shown_tab(&self) -> usize {
        self.tab
    }

    /// Whether the dialog previews and its Preview check box is on: the host renders the preview while it is. The box
    /// starts on, so this holds from construction.
    pub fn is_previewing(&self) -> bool {
        self.previews && self.preview_on
    }

    /// Whether the Preview check box was held down in the last pass, to compare with the unfiltered picture.
    pub fn is_comparing(&self) -> bool {
        self.shown.comparing
    }

    /// The eyedropper toggle that was pressed, once.
    pub fn take_eyedropper(&mut self) -> Option<EyedropperRequest> {
        self.requests.take_eyedropper()
    }

    /// The Preview check box starts off.
    #[must_use]
    pub fn preview_off(mut self) -> Self {
        self.preview_on = false;
        self
    }

    /// The ids of the actions the custom rows requested since the last call (`CustomContext::request_action`).
    pub fn take_actions(&mut self) -> Vec<String> {
        self.requests.take_actions()
    }

    /// Makes OK file its answer for the host to take instead of closing: the host checks it and answers with
    /// [`answer_ok`](HostedParamDialog::answer_ok). OK stays disabled until then.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// [`held_ok`](HostedParamDialog::held_ok) for a dialog already open.
    pub fn hold_ok(&mut self) {
        self.hold = true;
    }

    /// The OK filed by a held dialog, once.
    pub fn take_ok(&mut self) -> Option<ParamResult> {
        let fields = self.held.take()?;
        Some(self.result_of(fields))
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label under the rows until a value changes. Without a held OK it does nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// `Some`: the host refuses the current values, and OK stays off with `reason` shown under the rows until the values
    /// change; `None` ends the refusal.
    pub fn refuse(&mut self, reason: Option<Label>) {
        match reason {
            Some(reason) => self.held.refuse(reason, self.fields.clone()),
            None => self.held.clear_refusal(),
        }
    }

    /// Sets the named presets the dialog's presets row lists (`ParamDialog::presets`): the rows, the values each
    /// applies and the built-in names, which `check_name` refuses.
    pub fn set_presets(&mut self, rows: Vec<PresetRow<String>>, values: Vec<(String, DialogFields)>, built_in: Vec<String>) {
        self.presets = Some(HeldPresets { rows, values, built_in });
    }

    /// The preset edits of the last pass (Create, Rename, Delete), once.
    pub fn take_preset_events(&mut self) -> Vec<NamedPresetEvent<String>> {
        std::mem::take(&mut self.preset_events)
    }

    /// Whether an Escape ended a row's sampling in the last pass, once; the next Escape cancels as usual.
    pub fn take_sampling_ended(&mut self) -> bool {
        self.requests.take_sampling_ended()
    }

    /// The help link a row's button reported in the last pass, once.
    pub fn take_help_link(&mut self) -> Option<String> {
        self.requests.take_help_link()
    }

    /// What the Reset footer restores: the opening values with the dialog's `reset_to` over them.
    fn reset_goal(&self) -> DialogFields {
        let mut fields = self.fields.clone();
        let reset_to = (self.build)(&mut fields).reset_to;
        reset_target(&self.opening, reset_to.as_ref())
    }

    /// The answer OK gives with `fields`: the key, the part a remembering dialog saves and this dialog's id, as the
    /// dialog's own build says them.
    /// A read-only dialog answers with no values, as its OK does.
    fn result_of(&mut self, fields: DialogFields) -> ParamResult {
        let ParamDialog { key, params, tabs, remember, remember_for, read_only, .. } = (self.build)(&mut self.fields);
        let params = every_param(params, &tabs).into_owned();
        let mut result = if read_only {
            ParamDialog::result(key, &params, None, remember_for, DialogFields::new())
        } else {
            ParamDialog::result(key, &params, remember, remember_for, fields)
        };
        result.id = self.id;
        result
    }

    /// The id of the footer button or custom row button that was pressed, once (`ParamDialog::extra_buttons`).
    pub fn take_action(&mut self) -> Option<String> {
        self.requests.take_action()
    }

    /// The row keys an edit changed since the last take: an edit in a row or a set through automation. The host's own
    /// [`set_field`](HostedParamDialog::set_field), its answers and Reset are no edit of a row.
    pub fn take_edited(&mut self) -> Vec<String> {
        self.requests.take_edited()
    }

    /// The folder listing a files row asked for, once.
    pub fn take_listing(&mut self) -> Option<ListingRequest> {
        self.requests.take_listing()
    }

    /// The range alert of a value typed past a row's range and clamped, worded and titled
    /// ([`ParamDialog::alert_title`]), once: the host opens it.
    pub fn take_range_alert(&mut self) -> Option<Alert<'static>> {
        let (title, alert, env) = self.range_alert.take()?;
        Some(Alert::range(title, &alert, &env))
    }

    /// Answers the listing request `key` with the `files` of its folder.
    pub fn answer_listing(&mut self, key: impl Into<String>, files: Vec<std::path::PathBuf>) {
        self.requests.answer_listing(key, files);
    }

    /// Sets the value under `key`; the dialog shows it in its next pass. Unchecked: the app writes a value its own
    /// custom row holds (Blur Gallery's pins dragged on the canvas), which has no JSON form to check. It is the app's
    /// own write, not an edit of a row, so it removes no keys a row [`resets`](Param::resets).
    pub fn set_field(&mut self, key: impl Into<String>, value: FieldValue) {
        self.fields.set(key, value);
    }

    /// Removes the value under `key`, as if it had never been set; the dialog shows that in its next pass.
    pub fn clear_field(&mut self, key: &str) {
        self.fields.remove(key);
    }

    /// Centres the preview pane on `point` of the picture in the next pass, as a click on the pane does. Without a
    /// preview picture in that pass the point is dropped.
    pub fn centre_preview(&mut self, point: Pos2) {
        self.centre = Some(point);
    }

    /// The point of the picture at the centre of the preview pane in the last pass; `None` before the pane has shown.
    pub fn preview_centre(&self) -> Option<Pos2> {
        self.shown.centre
    }

    /// Shows `color` as the live preview of the colour row `key` from the next pass on; `None` ends it. The dialog's
    /// values are left as they are.
    pub fn preview_color(&mut self, key: &str, color: Option<craft_theme::Srgba>) {
        self.requests.preview_color(key, color);
    }

    /// The colours the colour rows preview in place of their values, by row key ([`preview_color`](Self::preview_color)).
    pub fn previewed_colors(&self) -> &[(String, Srgba)] {
        self.requests.previewed_colors()
    }

    /// The host cancelled its picker, file dialog or dropper: the requests not taken yet are dropped.
    pub fn cancel_requests(&mut self) {
        self.requests.cancel();
    }
}

/// Checks `value` against the row `field` of `params` and sets it in `fields`, as the user editing the row would; `Err`
/// says why the row refused (`FieldRefusal`). The field is a row's key, its unit key or a custom row's companion key
/// ([`Param::companions`]); a Files row takes a `{files, folder, useFolder}` object and sets its two companion keys with
/// it. A custom row with a set-time rule ([`Param::on_set`]) writes its key and companions through that rule instead.
/// Every dialog built on `Param`s sets its fields through this one rule.
pub fn set_param_field(params: &[Param], fields: &mut DialogFields, field: &str, value: Value) -> Result<(), String> {
    let companion = |param: &Param| matches!(&param.kind, ParamKind::Custom(row) if row.has_companion(field));
    let Some(param) = params.iter().find(|param| param.key == field || param.unit_key() == Some(field) || companion(param)) else {
        return Err(FieldRefusal::Unknown.message(field));
    };
    if !param.visible(fields) || !param.enabled(fields) {
        return Err(FieldRefusal::Inactive.message(field));
    }
    let held = fields.get(&param.key).cloned();
    let rule = match &param.kind {
        ParamKind::Custom(row) => row.on_set(),
        _ => None,
    };
    match rule {
        Some(set) => set(fields, field, value)?,
        None => write_param_field(param, fields, field, value, companion(param))?,
    }
    param.drop_dependents(held.as_ref(), fields);
    Ok(())
}

/// `set_param_field`'s write of `value` under `field` when the row has no set-time rule: the row's own key, its
/// unit, a Files row's keys or a custom row's companion (`companion`), each checked with `Param::accepts`.
fn write_param_field(param: &Param, fields: &mut DialogFields, field: &str, value: Value, companion: bool) -> Result<(), String> {
    let writes = if companion {
        Some(vec![(field.to_owned(), (!value.is_null()).then_some(FieldValue::Json(value.clone())))])
    } else if param.key != field {
        value.as_str().map(|unit| vec![(field.to_owned(), Some(FieldValue::Text(unit.to_owned())))])
    } else if matches!(param.kind, ParamKind::Files { .. }) {
        read_files(param, &value)
    } else {
        read_value(&param.kind, value.clone()).map(|read| vec![(field.to_owned(), Some(read))])
    };
    // `Param::accepts` checks every key a value sets, the Files row's companions included; `None` clears a key.
    let writes = writes.filter(|writes| writes.iter().all(|(key, read)| read.as_ref().is_none_or(|read| param.accepts(key, read))));
    let Some(writes) = writes else {
        return Err(FieldRefusal::Rejects(&value).message(field));
    };
    for (key, read) in writes {
        match read {
            Some(read) => fields.set(key, read),
            None => {
                fields.remove(&key);
            }
        }
    }
    Ok(())
}

/// The keys a Files row's `{files, folder, useFolder}` object sets: the row's list, its folder (cleared by a `null` or
/// missing `folder`) and its Files/Folder mode; `None` when `value` is not that object.
fn read_files(param: &Param, value: &Value) -> Option<Vec<(String, Option<FieldValue>)>> {
    let files = read_value(&param.kind, value.clone())?;
    let folder = match value.get("folder") {
        None | Some(Value::Null) => None,
        Some(folder) => Some(FieldValue::Text(folder.as_str()?.to_owned())),
    };
    let use_folder = FieldValue::Bool(value.get("useFolder")?.as_bool()?);
    Some(vec![(param.key.clone(), Some(files)), (param.folder_key(), folder), (param.use_folder_key(), Some(use_folder))])
}

/// `value` read as the value a row of `kind` keeps under its own key; `None` when it isn't one, or the row keeps none.
fn read_value(kind: &ParamKind, value: Value) -> Option<FieldValue> {
    Some(match kind {
        ParamKind::Range { .. } | ParamKind::Angle { .. } | ParamKind::Number { .. } | ParamKind::OptionalNumber { .. } => {
            FieldValue::Number(value.as_f64()?)
        }
        ParamKind::Int { .. } | ParamKind::Channel { .. } => FieldValue::Int(value.as_i64()?),
        ParamKind::Bool { .. } => FieldValue::Bool(value.as_bool()?),
        ParamKind::Choice { .. } | ParamKind::Radio { .. } | ParamKind::Text { .. } => FieldValue::Text(value.as_str()?.to_owned()),
        ParamKind::Color { .. } => FieldValue::Color(serde_json::from_value(value).ok()?),
        ParamKind::Point { .. } => FieldValue::Point(serde_json::from_value(value).ok()?),
        ParamKind::Grid { .. } => FieldValue::Grid(serde_json::from_value(value).ok()?),
        ParamKind::Blend { .. } | ParamKind::Pattern { .. } => FieldValue::Text(value.as_str()?.to_owned()),
        ParamKind::Files { .. } => {
            let files = value.get("files")?.as_array()?;
            FieldValue::List(files.iter().map(|file| file.as_str().map(|path| FieldValue::Text(path.to_owned()))).collect::<Option<_>>()?)
        }
        ParamKind::Custom(_) => FieldValue::Json(value),
        ParamKind::ReadOnlyText { .. } | ParamKind::Section => return None,
    })
}

impl HoldsOk for HostedParamDialog {
    type Answer = ParamResult;

    fn held_ok(self) -> Self {
        HostedParamDialog::held_ok(self)
    }

    fn take_ok(&mut self) -> Option<ParamResult> {
        HostedParamDialog::take_ok(self)
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        HostedParamDialog::answer_ok(self, answer);
    }
}

impl StackDialog for HostedParamDialog {
    type Result = ParamResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ParamResult>> {
        let measuring = crate::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        // A measuring run keeps the centre: the real run of the pass takes it.
        let centre = if measuring { self.centre } else { self.centre.take() };
        let mut dialog = (self.build)(&mut self.fields);
        self.rows.apply(&mut dialog);
        self.previews = dialog.previews;
        dialog = dialog
            .preview_on(Bind::bound(&mut self.preview_on))
            .values_opened(self.opening.clone())
            .answers(answers)
            .preview_colors(self.requests.previewed_colors().to_vec());
        // A refusal the host keeps holds OK off with its reason whatever the values are.
        if let Some((reason, _)) = self.held.refusal().filter(|_| self.held.refusal_blocks()) {
            dialog = dialog.ok_enabled(false).ok_reason(reason.clone());
        }
        if let Some(presets) = &self.presets {
            dialog = dialog.presets(ParamPresets::new(&presets.rows, &presets.values, &presets.built_in));
        }
        // The held answer also carries a refusal the host keeps on a dialog without a held OK: its notice shows the reason
        // under the rows, and OK is off meanwhile, so nothing is filed.
        if self.hold || self.held.refusal_blocks() {
            dialog = dialog.held(&mut self.held);
        }
        if let Some(point) = centre {
            dialog = dialog.centre_on(point);
        }
        if let Err(reason) = &self.ready {
            dialog = dialog.ok_enabled(false).ok_reason(reason.clone());
        }
        if !self.notes.is_empty() {
            dialog = dialog.host_notes(self.notes.clone());
        }
        if let Some(picture) = self.picture.clone() {
            dialog = dialog.preview(picture);
        }
        if let Some(pager) = self.pager.clone() {
            dialog = dialog.preview_pager(pager);
        }
        if let Some(overlay) = self.overlay.clone() {
            dialog = dialog.preview_overlay(move |cx, transform, rect| overlay(cx, transform, rect));
        }
        if let Some(tint) = self.tint {
            dialog = dialog.preview_tint(tint);
        }
        dialog = dialog.tab(Bind::bound(&mut self.tab));
        let alert_title = dialog.alert_title.clone().unwrap_or_else(|| dialog.title.clone());
        let shown = dialog.show(ui);
        if let Some(mut rows) = shown.rows {
            self.requests.absorb(&mut rows);
            if let Some(alert) = rows.out_of_range {
                self.range_alert = Some((alert_title, alert, Env::of(ui)));
            }
        }
        if let Some(point) = shown.preview_click {
            self.requests.request_preview_click(point);
        }
        // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if let Some(action) = shown.action {
            self.requests.request_action(action);
        }
        if let Some(step) = shown.preview_step {
            self.requests.request_preview_step(step);
        }
        self.shown = ShownState { comparing: shown.comparing, centre: shown.preview_centre };
        self.preset_events.extend(shown.presets);
        match (shown.result, shown.closed) {
            (Some(mut result), _) => {
                result.id = self.id;
                Some(DialogResult::Ok(result))
            }
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }

    fn instance(&self) -> Option<DialogId> {
        self.instance
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    fn answer_color(&mut self, key: &str, color: craft_theme::Srgba) {
        self.requests.answer_color(key, color);
    }

    /// Sets the fields on a clone of the values and commits the clone only when every field is accepted. The rows are
    /// those the user sees: the host's labels and shown states over every tab's rows; a read-only dialog refuses all.
    fn set_json_fields(&mut self, _: &Context, fields: &[(String, Value)]) -> Result<(), String> {
        let (params, read_only) = {
            let mut dialog = (self.build)(&mut self.fields);
            self.rows.apply(&mut dialog);
            let ParamDialog { params, tabs, read_only, .. } = dialog;
            (every_param(params, &tabs).into_owned(), read_only)
        };
        let mut staged = self.fields.clone();
        let mut preview = self.preview_on;
        for (field, value) in fields {
            if read_only {
                return Err(FieldRefusal::Inactive.message(field));
            }
            match (field.as_str(), &self.presets) {
                ("preview", _) if self.previews => {
                    preview = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(value).message(field))?;
                }
                ("preset", Some(presets)) => {
                    let preset = value.as_str().and_then(|name| presets.values.iter().find(|(preset, _)| preset == name));
                    let (_, preset) = preset.ok_or_else(|| FieldRefusal::Rejects(value).message(field))?;
                    apply_preset(preset, &mut staged);
                }
                _ => set_param_field(&params, &mut staged, field, value.clone())?,
            }
        }
        self.requests.request_set(&params, &self.fields, &staged);
        self.fields = staged;
        self.preview_on = preview;
        Ok(())
    }

    /// Whether the Reset footer's restore would change the values.
    fn can_reset(&self) -> bool {
        self.fields != self.reset_goal()
    }

    /// Runs the restore the Reset footer runs: the opening values with `reset_to` over them.
    fn reset(&mut self, _: &Context) -> bool {
        let target = self.reset_goal();
        let changed = self.fields != target;
        self.fields = target;
        changed
    }

    /// Answers OK as the button does: none while OK is disabled. The result keeps what a remembering dialog saves. A
    /// held dialog files the answer for the host, as its button does, and answers none; once the host accepted the
    /// held answer, it answers that.
    fn confirm(&mut self, _: &Context) -> Option<ParamResult> {
        if self.held.is_accepted() {
            let fields = self.held.finish()?;
            return Some(self.result_of(fields));
        }
        if self.held.is_waiting() {
            return None;
        }
        // OK is off while the dialog says so, a refusal the host keeps holds it, or the host is not ready.
        let ok_enabled = (self.build)(&mut self.fields).ok_enabled && !self.held.refusal_blocks();
        if !ok_enabled || self.ready.is_err() {
            return None;
        }
        if self.hold {
            self.held.file(self.fields.clone());
            return None;
        }
        let fields = self.fields.clone();
        Some(self.result_of(fields))
    }

    /// The values by parameter key, as [`DialogFields`] serializes them, with the Preview check box under `preview`
    /// for a dialog that previews and the preset the values equal under `preset` for one with presets.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = serde_json::to_value(&self.fields).ok()?;
        if let Some(map) = fields.as_object_mut() {
            if self.previews {
                map.insert("preview".to_owned(), Value::Bool(self.preview_on));
            }
            if let Some(presets) = &self.presets {
                let current = presets.values.iter().find(|(_, preset)| preset_applies(preset, &self.fields));
                map.insert("preset".to_owned(), current.map_or(Value::Null, |(name, _)| Value::String(name.clone())));
            }
        }
        Some(fields)
    }

    /// Keeps `value` when the row `field` names (a parameter or its unit choice) is shown, takes input, and takes
    /// the value ([`set_param_field`]).
    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        self.set_json_fields(ctx, &[(field.to_owned(), value)])
    }

    /// Shows a child picker's live colour in the colour row `key`, as [`HostedParamDialog::preview_color`] does.
    fn preview_color(&mut self, key: &str, color: Option<craft_theme::Srgba>) {
        self.requests.preview_color(key, color);
    }
}
