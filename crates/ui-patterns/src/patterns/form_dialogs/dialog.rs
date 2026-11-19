//! `ParamDialog`: a dialog from a parameter list, with an optional preview, and `HostedParamDialog`, the same dialog
//! owned by a `DialogHost`. A command form is a `ParamDialog` without a preview.

use super::fields::{DialogFields, FieldValue};
use crate::patterns::presets::{PresetKind, PresetRow};
use crate::runtime::requests::{ColorRequest, EyedropperRequest, HostAnswer, ListingRequest, PathRequest};

use super::form::{FormDialog, FormDialogWords};
use super::host_requests::HostRequests;
use super::param::{Param, ParamKind, open_fields, set_color_field};
use super::preview::{PreviewPane, PreviewView, PreviewZoom};

use super::rows::{ParamRows, ParamRowsOutput};

use crate::contract::{Bind, BodyCx, CanvasGesture, NoValue, Outcome, Remember as _, Sampler as _, SaveKey, Show as _, ShowContainer as _};

use crate::dialog::{
    Alert, ButtonPlace, Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, FooterHooks, HeldAnswer, HeldOk, StackDialog, dialog_id,
};

use crate::display::{Shade, Text};
use crate::form::SliderFieldLayout;
use crate::layout_components::{Row, Stack};
use crate::notices::{Notice, NoticeLayout};

use crate::runtime::env::{Env, Label};
use crate::text::RangeAlert;

use crate::runtime::memory::MemoryStage;

use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};

use craft_theme::Srgba;
use craft_theme::variant::Tone;
use egui::load::SizedTexture;
use egui::{Context, Pos2, Response, Ui};
use serde_json::Value;

use std::borrow::Cow;

#[cfg(feature = "suite")]
pub(crate) mod stories;

craft_ui::words! {
    /// The words of the parameter dialog beside the common OK and Cancel.
    pub struct ParamDialogWords {
        reset: "paramDialog.reset" = "Reset";
    }
}

/// How a parameter dialog arranges its rows, its preview pane and its buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

impl ParamLayout {
    /// Where a dialog that previews stands its buttons: in a column beside the body, or across the foot.
    fn button_place(self) -> ButtonPlace {
        if matches!(self, ParamLayout::Stacked) { ButtonPlace::Foot } else { ButtonPlace::Side }
    }

    /// How the range rows arrange their slider and number field.
    fn sliders(self) -> SliderFieldLayout {
        match self {
            ParamLayout::Stacked => SliderFieldLayout::Stacked,
            ParamLayout::Beside | ParamLayout::Below => SliderFieldLayout::Inline,
        }
    }
}

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
    remember: Option<(SaveKey, DialogFields)>,
}

impl ParamResult {
    /// Stores the values of a dialog built with [`ParamDialog::remember`] for the next open. The app calls it after
    /// its command succeeds, so a failed command leaves the previous values; without a remember key it does nothing.
    pub fn remember(&self, ctx: &Context) {
        if let Some((key, values)) = &self.remember {
            crate::runtime::memory::set_saved_value::<Option<DialogFields>>(ctx, key, Some(values.clone()));
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
    /// The id of the extra footer button pressed this pass (`ParamDialog::extra_buttons`).
    pub footer_action: Option<String>,
    /// The answer OK would have given, the pass OK was pressed on a dialog that holds its OK for the host.
    pub(crate) held: Option<ParamResult>,
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
///   `ParamDialogOutput::footer_action`; [`footer_note`](ParamDialog::footer_note) is a muted line at the start edge.
/// - A command form is a dialog with no preview and no remember key.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct ParamDialog<'a> {
    key: String,
    title: Label,
    params: Cow<'a, [Param]>,
    /// Binds the values to `values`.
    #[prop(name = "values")]
    bound: Option<Bind<'a, DialogFields>>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Saves the values under `key` when the app calls [`ParamResult::remember`], and opens with them.
    #[prop]
    remember: Option<SaveKey>,
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
    layout: ParamLayout,
    /// Whether the title row has a close action.
    #[prop]
    closable: bool,
    /// Whether the dialog has the Preview check box: the host previews its values while the box is on.
    #[prop]
    previews: bool,
    /// The Preview check box's value; without it the dialog keeps it, and every open starts with the box on.
    #[prop]
    preview_on: Option<Bind<'a, bool>>,
    /// The picture the preview pane shows in a dialog that [`previews`](ParamDialog::previews); without one there is
    /// no pane.
    #[prop]
    preview: Option<SizedTexture>,
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
    /// Extra footer buttons after OK and Cancel (Color Table's Load… and Save…), each an id and a label. They keep the
    /// dialog open; a press comes back as `ParamDialogOutput::footer_action`.
    #[prop]
    extra_buttons: Vec<(String, Label)>,
    /// The patterns the dialog's [`ParamKind::Pattern`] rows offer.
    #[prop]
    patterns: Cow<'a, [PresetRow<String>]>,
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
    /// Makes OK keep the dialog open and report its answer in `ParamDialogOutput::held`.
    #[prop(name = "hold_ok", vis = "pub(crate)")]
    hold: bool,
    /// Closes the dialog as OK in this pass: the host accepted the held answer.
    #[prop(vis = "pub(crate)")]
    finish_held: bool,
    /// Shows `reason` under the rows while the values are `refused`.
    #[prop(vis = "pub(crate)")]
    refusal: Option<(Label, DialogFields)>,
    /// Centres the preview pane's view on the picture point `point` in this pass; without a picture it does nothing.
    #[prop(name = "centre_on", vis = "pub(crate)")]
    centre: Option<Pos2>,
    /// The colours the colour rows show in place of their values in this pass, by row key.
    #[prop(vis = "pub(crate)")]
    preview_colors: Vec<(String, Srgba)>,
}

/// What `ParamDialog::shaped` applies to the built [`Dialog`].
struct Shaped<'a>(Box<dyn FnOnce(Dialog<'a>) -> Dialog<'a> + 'a>);

type BelowRowsFn<'a> = Box<dyn Fn(&mut Ui, &DialogFields) + 'a>;

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
            bound: None,
            open: None,
            remember: None,
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
            layout: ParamLayout::default(),
            closable: true,
            previews: false,
            preview_on: None,
            preview: None,
            click_centres: true,
            canvas: None,
            extra_buttons: Vec::new(),
            patterns: Cow::Borrowed(&[]),
            footer_note: None,
            below: None,
            shaped: None,
            answers: Vec::new(),
            values_opened: None,
            hold: false,
            finish_held: false,
            refusal: None,
            centre: None,
            preview_colors: Vec::new(),
        }
    }

    /// Changes the built [`Dialog`] before it shows, for the `Dialog` settings a form track needs and `ParamDialog`
    /// has no setter for (`Dialog::overlay`, say). It runs after the settings `ParamDialog` makes itself.
    pub fn shaped(mut self, shape: impl FnOnce(Dialog<'a>) -> Dialog<'a> + 'a) -> Self {
        self.shaped = Some(Shaped(Box::new(shape)));
        self
    }

    /// The answer OK gives with `fields`: with `remember`, it keeps the part of them the dialog saves.
    fn result(key: String, params: &[Param], remember: Option<SaveKey>, fields: DialogFields) -> ParamResult {
        let remember = remember.map(|save| (save, Self::remembered(params, &fields)));
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
            bound,
            open,
            remember,
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
            closable,
            previews,
            preview_on,
            preview,
            click_centres,
            canvas,
            extra_buttons,
            patterns,
            footer_note,
            below,
            shaped,
            answers,
            values_opened,
            hold,
            finish_held,
            refusal,
            centre,
            preview_colors,
        } = self;
        let preview_label = Env::of(ui).words::<FormDialogWords>().preview.clone();
        // The footer `FormDialog` builds for this pass, made the same way, so its `pick` reads `FormOutput::picked`.
        let footer = ResetFooter::new(ui, cancel_label.clone(), alt_resets);

        let mut form = match bound {
            Some(bind) => FormDialog::new(title.clone(), bind),
            None => FormDialog::own(title.clone()),
        };
        form = form
            .ok_enabled(ok_enabled)
            .alt_resets(alt_resets)
            .outer_gesture(outer_gesture)
            .history(history)
            .collection_body(collection_body)
            .closable(closable)
            .accept_stays_open(hold)
            .answers(answers);
        if let Some(reset_to) = reset_to {
            form = form.reset_with(move |_, opened| {
                let mut values = opened.clone();
                for (name, value) in reset_to.iter() {
                    values.set(name, value.clone());
                }
                values
            });
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
        if !extra_buttons.is_empty() {
            form = form.extra_buttons(extra_buttons.iter().map(|(_, label)| FooterButton::new(label.clone(), DialogRole::Other)));
        }
        if let Some(note) = footer_note {
            form = form.footer_extra(move |ui| {
                Text::new(note.clone()).shade(Shade::Muted).show(ui);
            });
        }
        if let Some(Shaped(shape)) = shaped {
            form = form.shaped(shape);
        }

        let mut live = Live::default();
        let mut rows = None;
        let mut shown_preview = ShownPreview::default();
        let mut latest = None::<DialogFields>;
        let shown = form.show(ui, |ui, cx, values| {
            if cx.first() && !host_opened {
                let saved = remember.as_ref().and_then(|key| cx.saved::<Option<DialogFields>>(key).clone());
                Self::open_values(&params, &patterns, saved.as_ref(), values);
            }
            for answer in cx.take_answers() {
                let answered = match &answer {
                    HostAnswer::Color { key, .. } | HostAnswer::Eyedropper { key, .. } | HostAnswer::Path { key, .. } => {
                        params.iter().find(|param| param.key == *key)
                    }
                    HostAnswer::Listing { .. } => None,
                };
                let held = answered.and_then(|param| values.get(&param.key).cloned());
                match answer {
                    HostAnswer::Color { key, color } | HostAnswer::Eyedropper { key, color } => set_color_field(&params, values, &key, color),
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
            if let Some(canvas) = canvas {
                cx.keep_view_open(canvas);
            }
            // The canvas shows the picture the pane does: while a click on the pane centres it, a press on the image is
            // the pane's locator press, which the app answers with `HostedParamDialog::centre_preview`.
            if let Some(canvas) = canvas.filter(|_| click_centres && preview.is_some()) {
                cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
            }
            // A measuring run closes nothing: the real run of the same pass makes the close.
            if finish_held && !crate::runtime::measure::active(ui.ctx()) {
                cx.request_close_as(CloseReason::Activated);
            }
            live = cx.memory::<Live>().clone();
            if let (Some(point), Some(texture)) = (centre, &preview) {
                live.view = live.view.centred_on(point, texture.size);
            }
            let mut show_rows = |ui: &mut Ui, values: &mut DialogFields| {
                let mut shown = ParamRows::new(title.clone(), &params, Bind::bound(values))
                    .patterns(&patterns)
                    .preview_colors(&preview_colors)
                    .sliders(layout.sliders());
                if let Some(canvas) = canvas {
                    shown = shown.canvas(canvas);
                }
                rows = Some(shown.show(ui));
            };
            match (&preview, layout) {
                (Some(texture), ParamLayout::Beside) => {
                    Row::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        Stack::new()
                            .show(ui, |ui, _| shown_preview = show_preview(ui, &preview_label, texture, click_centres, ZoomRow::Above, &mut live));
                    });
                }
                (Some(texture), layout) => {
                    let zoom = if layout == ParamLayout::Stacked { ZoomRow::Below } else { ZoomRow::Above };
                    Stack::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        shown_preview = show_preview(ui, &preview_label, texture, click_centres, zoom, &mut live);
                    });
                }
                (None, _) => show_rows(ui, values),
            }
            // The host's refusal of the held OK stays until a value changes.
            if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| *refused == *values) {
                Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            if let Some(BelowRows(draw)) = &below {
                draw(ui, values);
            }
            *cx.memory::<Live>() = live.clone();
            latest = Some(values.clone());
            true
        });

        // `picked` is this pass's press: a second pass of the frame has none, so a press is reported once.
        let pick = footer.pick(shown.picked);
        let footer_action = match pick {
            Some(ResetPick::Extra(at)) => extra_buttons.get(at).map(|(id, _)| id.clone()),
            _ => None,
        };
        let (result, held) = match (shown.accepted, pick, latest) {
            (Some(fields), _, _) => (Some(Self::result(key, &params, remember, fields)), None),
            (None, Some(ResetPick::Accept), Some(fields)) if hold => (None, Some(Self::result(key, &params, remember, fields))),
            _ => (None, None),
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
            footer_action,
            held,
            preview_centre: shown_preview.centre,
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
            resetting: alt_resets && crate::runtime::pointer::frame(ui.ctx()).mods.alt,
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

/// What the preview pane reported in a pass.
#[derive(Clone, Copy, Default)]
struct ShownPreview {
    /// The picture point a click landed on.
    clicked: Option<Pos2>,
    /// The picture point at the centre of the view the pane drew.
    centre: Option<Pos2>,
}

/// Where the zoom row stands: over the preview pane or under it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ZoomRow {
    Above,
    Below,
}

/// The zoom row and the pane.
fn show_preview(ui: &mut Ui, label: &Label, texture: &SizedTexture, click_centres: bool, zoom_row: ZoomRow, live: &mut Live) -> ShownPreview {
    let above = (zoom_row == ZoomRow::Above).then(|| PreviewZoom::new(&live.view).show(ui));
    let pane = PreviewPane::new(label.clone(), *texture, &live.view).click_centres(click_centres).show(ui);
    let below = (zoom_row == ZoomRow::Below).then(|| PreviewZoom::new(&live.view).show(ui));
    let centre = pane.shown.centre(texture.size);
    live.view = above.or(below).and_then(|zoom| zoom.view).or(pane.view).unwrap_or(pane.shown);
    ShownPreview { clicked: pane.clicked, centre: Some(centre) }
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
    picture: Option<SizedTexture>,
    /// What the dialog showed in its last pass.
    shown: ShownState,
    /// Whether OK files its answer for the host instead of closing ([`held_ok`](HostedParamDialog::held_ok)).
    hold: bool,
    /// The OK the host holds back until it answers `answer_ok`, and its refusal, shown until a value changes.
    held: HeldAnswer<ParamResult>,
    /// The picture point the next pass centres the preview on ([`centre_preview`](HostedParamDialog::centre_preview)).
    centre: Option<Pos2>,
}

/// What a [`HostedParamDialog`] showed in its last pass: whether the Preview check box is held for comparing, and the
/// picture point at the centre of the preview pane.
#[derive(Clone, Copy, Debug, Default)]
struct ShownState {
    comparing: bool,
    centre: Option<Pos2>,
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
        let ParamDialog { params, remember, patterns, previews, .. } = build(&mut fields);
        let (params, patterns) = (params.into_owned(), patterns.into_owned());
        let saved = remember.and_then(|key| MemoryStage::peek_saved::<Option<DialogFields>>(ctx, &key));
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
            shown: ShownState::default(),
            hold: false,
            held: HeldAnswer::new(),
            centre: None,
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

    /// The colour parameter whose button was pressed, once.
    pub fn pick_color(&mut self) -> Option<ColorRequest> {
        self.requests.take_color()
    }

    /// The Browse button that was pressed, once.
    pub fn pick_path(&mut self) -> Option<PathRequest> {
        self.requests.take_path()
    }

    /// The point of the preview picture the last click landed on (`ParamDialogOutput::preview_click`), once.
    pub fn take_preview_click(&mut self) -> Option<Pos2> {
        self.requests.take_preview_click()
    }

    /// Answers the colour request `key`: the dialog writes the colour into its values in its next pass.
    pub fn answer_color(&mut self, key: impl Into<String>, color: craft_theme::Srgba) {
        self.requests.answer_color(key, color);
    }

    /// Answers the Browse request `key` with the chosen path.
    pub fn answer_path(&mut self, key: impl Into<String>, path: std::path::PathBuf) {
        self.requests.answer_path(key, path);
    }

    /// Answers the eyedropper request `key` with the sampled colour.
    pub fn answer_eyedropper(&mut self, key: impl Into<String>, color: craft_theme::Srgba) {
        self.requests.answer_eyedropper(key, color);
    }

    /// Sets the picture the preview pane shows from the next pass on; `None` shows none.
    pub fn set_preview(&mut self, picture: Option<SizedTexture>) {
        self.picture = picture;
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
        self.held.take()
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label under the rows until a value changes. Without a held OK it does nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// Holds `result` for the host: the OK of a held dialog, from the button or from `confirm`.
    fn file_held(&mut self, mut result: ParamResult) {
        result.id = self.id;
        self.held.file(result);
    }

    /// The footer button id that was pressed, once (`ParamDialog::extra_buttons`).
    pub fn take_footer_action(&mut self) -> Option<String> {
        self.requests.take_footer_action()
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
/// it. Every dialog built on `Param`s sets its fields through this one rule.
pub fn set_param_field(params: &[Param], fields: &mut DialogFields, field: &str, value: Value) -> Result<(), String> {
    let companion = |param: &Param| matches!(&param.kind, ParamKind::Custom(row) if row.has_companion(field));
    let Some(param) = params.iter().find(|param| param.key == field || param.unit_key() == Some(field) || companion(param)) else {
        return Err(FieldRefusal::Unknown.message(field));
    };
    if !param.visible(fields) || !param.enabled(fields) {
        return Err(FieldRefusal::Inactive.message(field));
    }
    let writes = if companion(param) {
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
    let held = fields.get(&param.key).cloned();
    for (key, read) in writes {
        match read {
            Some(read) => fields.set(key, read),
            None => {
                fields.remove(&key);
            }
        }
    }
    param.drop_dependents(held.as_ref(), fields);
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

impl StackDialog for HostedParamDialog {
    type Result = ParamResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ParamResult>> {
        let measuring = crate::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        let finish = self.held.is_accepted();
        // A measuring run keeps the centre: the real run of the pass takes it.
        let centre = if measuring { self.centre } else { self.centre.take() };
        let mut dialog = (self.build)(&mut self.fields);
        self.previews = dialog.previews;
        dialog = dialog
            .preview_on(Bind::bound(&mut self.preview_on))
            .values_opened(self.opening.clone())
            .answers(answers)
            .hold_ok(self.hold)
            .finish_held(finish)
            .preview_colors(self.requests.previewed_colors().to_vec());
        if let Some((reason, refused)) = self.held.refusal() {
            dialog = dialog.refusal((reason.clone(), refused.fields.clone()));
        }
        if let Some(point) = centre {
            dialog = dialog.centre_on(point);
        }
        // OK waits for the host's answer: a second OK cannot file over the held one.
        if self.held.is_waiting() {
            dialog = dialog.ok_enabled(false);
        }
        if let Some(picture) = self.picture {
            dialog = dialog.preview(picture);
        }
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
        if let Some(action) = shown.footer_action {
            self.requests.request_footer_action(action);
        }
        self.shown = ShownState { comparing: shown.comparing, centre: shown.preview_centre };
        if self.held.refusal().is_some_and(|(_, refused)| refused.fields != self.fields) {
            self.held.clear_refusal();
        }
        if let Some(result) = shown.held {
            self.file_held(result);
        }
        match (shown.result, shown.closed) {
            (Some(mut result), _) => {
                result.id = self.id;
                Some(DialogResult::Ok(result))
            }
            (None, Some(CloseReason::Activated)) if finish => Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
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

    /// Answers OK as the button does: none while OK is disabled. The result keeps what a remembering dialog saves. A
    /// held dialog files the answer for the host, as its button does, and answers none; once the host accepted the
    /// held answer, it answers that.
    fn confirm(&mut self, _: &Context) -> Option<ParamResult> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() {
            return None;
        }
        let ParamDialog { key, params, remember, ok_enabled, .. } = (self.build)(&mut self.fields);
        if !ok_enabled {
            return None;
        }
        let params = params.into_owned();
        let mut result = ParamDialog::result(key, &params, remember, self.fields.clone());
        if self.hold {
            self.file_held(result);
            return None;
        }
        result.id = self.id;
        Some(result)
    }

    /// The values by parameter key, as [`DialogFields`] serializes them.
    fn json_fields(&self) -> Option<Value> {
        serde_json::to_value(&self.fields).ok()
    }

    /// Keeps `value` when the row `field` names (a parameter or its unit choice) is shown, takes input, and takes
    /// the value ([`set_param_field`]).
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        let params = (self.build)(&mut self.fields).params.into_owned();
        let before = self.fields.clone();
        set_param_field(&params, &mut self.fields, field, value)?;
        for param in params.iter().filter(|param| matches!(param.kind, ParamKind::Files { .. })) {
            if let Some(request) = param.listing_after(&before, &self.fields) {
                self.requests.request_listing(request);
            }
        }
        Ok(())
    }

    /// Shows a child picker's live colour in the colour row `key`, as [`HostedParamDialog::preview_color`] does.
    fn preview_color(&mut self, key: &str, color: Option<craft_theme::Srgba>) {
        self.requests.preview_color(key, color);
    }
}
