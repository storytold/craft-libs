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

use crate::contract::{Bind, BodyCx, NoValue, Outcome, Remember as _, Sampler as _, SaveKey, Show as _, ShowContainer as _};

use crate::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, FooterHooks, StackDialog, dialog_id};

use crate::display::{Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::notices::{Notice, NoticeLayout};

use crate::runtime::env::{Env, Label};

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

#[cfg(feature = "testing")]
mod stories;

craft_ui::words! {
    /// The words of the parameter dialog beside the common OK and Cancel.
    pub struct ParamDialogWords {
        reset: "paramDialog.reset" = "Reset";
    }
}

/// Where the preview pane sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamLayout {
    /// The preview pane to the right of the rows, under the zoom row.
    #[default]
    Beside,
    /// The preview pane under the rows.
    Below,
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

/// A dialog from a parameter list: one row per [`Param`], the Preview check box with a preview pane when the host
/// gives a picture, and OK, Cancel, extra buttons and a footer note.
///
/// - Keys: Tab walks the fields, Enter is OK unless a field owns it, Escape cancels. Holding Alt turns Cancel into
///   Reset, which restores the values from when the dialog opened (or `reset_to`) and keeps the dialog open.
/// - Values: the `values` binding holds them; without one the dialog keeps its own between passes. The first pass
///   fills the parameters' defaults and, with `remember`, the values saved by [`ParamResult::remember`].
/// - Preview: the host renders from the reported values while `preview_on` and hands the picture to `preview`, the
///   unfiltered one while `comparing` (the Preview check box held down). Drag pans the picture, the zoom buttons
///   scale it. A dialog with a preview has its buttons in a column beside the body, with the Preview check box and
///   the footer note under them.
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
    /// The values Reset sets, instead of the values from when the dialog opened. Keys the target lacks keep their
    /// value.
    #[prop]
    reset_to: Option<DialogFields>,
    /// Where the preview pane sits.
    #[prop]
    layout: ParamLayout,
    /// Whether the title row has a close action.
    #[prop]
    closable: bool,
    /// The picture the preview pane shows; without one the dialog has no preview.
    #[prop]
    preview: Option<SizedTexture>,
    /// Whether a click on the preview centres it on the point clicked (`PreviewPane::click_centres`); a dialog whose
    /// preview clicks pick a colour turns it off. The default is true.
    #[prop]
    click_centres: bool,
    /// The canvas the rows' droppers sample (`ParamRows::canvas`). While the dialog shows, its view stays usable under
    /// the modal layer (`Sampler::keep_view_open`).
    #[prop]
    canvas: Option<egui::Id>,
    /// Extra footer buttons after OK and Cancel (Color Table's Load… and Save…), each an id and a label. They keep the
    /// dialog open; a press comes back as `ParamDialogOutput::footer_action`.
    #[prop]
    extra_buttons: Vec<(String, Label)>,
    /// The patterns the dialog's [`ParamKind::Pattern`] rows offer.
    #[prop]
    patterns: Cow<'a, [PresetRow<String>]>,
    /// A muted line at the footer's start edge (JPEG Options' size estimate).
    footer_note: Option<Label>,
    /// A section under the rows that only reads the values (a monitor note).
    #[prop(func = "Fn(&mut Ui, &DialogFields)")]
    below: Option<BelowRows<'a>>,
    shaped: Option<Shaped<'a>>,
    /// The host's answers to the requests of earlier passes; the dialog writes them into its values first.
    #[prop]
    answers: Vec<HostAnswer>,
    /// The values a [`HostedParamDialog`] opened before the first pass: the first pass leaves them and Reset restores
    /// them.
    values_opened: Option<DialogFields>,
    /// OK keeps the dialog open and reports its answer in `ParamDialogOutput::held` ([`HostedParamDialog::held_ok`]).
    hold: bool,
    /// The host accepted the held OK: this pass closes the dialog as OK.
    finish_held: bool,
    /// Why the host refused the held OK, and the values it refused: the notice under the rows shows while the values
    /// are those.
    refusal: Option<(Label, DialogFields)>,
    /// The picture point this pass centres the preview pane's view on ([`HostedParamDialog::centre_preview`]).
    centre: Option<Pos2>,
    /// The colours a child picker previews in the colour rows, by row key ([`HostedParamDialog::preview_color`]).
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
            alt_resets: true,
            outer_gesture: false,
            history: false,
            collection_body: false,
            reset_to: None,
            layout: ParamLayout::default(),
            closable: true,
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

    /// Marks the bound values as opened by the host with `opening`, so the first pass doesn't open them again and
    /// Reset restores `opening`.
    pub(crate) fn values_opened(mut self, opening: DialogFields) -> Self {
        self.values_opened = Some(opening);
        self
    }

    /// Makes OK keep the dialog open and report its answer in `ParamDialogOutput::held`.
    fn hold_ok(mut self, on: bool) -> Self {
        self.hold = on;
        self
    }

    /// Closes the dialog as OK in this pass: the host accepted the held answer.
    fn finish_held(mut self, on: bool) -> Self {
        self.finish_held = on;
        self
    }

    /// Shows `reason` under the rows while the values are `refused`.
    fn refusal(mut self, refusal: Option<(Label, DialogFields)>) -> Self {
        self.refusal = refusal;
        self
    }

    /// Centres the preview pane's view on the picture point `point` in this pass; without a picture it does nothing.
    fn centre_on(mut self, point: Option<Pos2>) -> Self {
        self.centre = point;
        self
    }

    /// The colours the colour rows show in place of their values in this pass, by row key.
    fn preview_colors(mut self, colors: Vec<(String, Srgba)>) -> Self {
        self.preview_colors = colors;
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
            alt_resets,
            outer_gesture,
            history,
            collection_body,
            reset_to,
            layout,
            closable,
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
            .answers(answers)
            .reset_with(move |current, opened| {
                let mut values = current.clone();
                for (name, value) in reset_to.as_ref().unwrap_or(opened).iter() {
                    values.set(name, value.clone());
                }
                values
            });
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
        if preview.is_some() {
            form = form.preview(None);
        }
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
            }
            if let Some(canvas) = canvas {
                cx.keep_view_open(canvas);
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
                let mut shown = ParamRows::new(title.clone(), &params, Bind::bound(values)).patterns(&patterns).preview_colors(&preview_colors);
                if let Some(canvas) = canvas {
                    shown = shown.canvas(canvas);
                }
                rows = Some(shown.show(ui));
            };
            match (&preview, layout) {
                (Some(texture), ParamLayout::Beside) => {
                    Row::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        Stack::new().show(ui, |ui, _| shown_preview = show_preview(ui, &preview_label, texture, click_centres, &mut live));
                    });
                }
                (Some(texture), ParamLayout::Below) => {
                    Stack::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        shown_preview = show_preview(ui, &preview_label, texture, click_centres, &mut live);
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

    /// A muted line at the footer's start edge, left of the buttons (a file size estimate, say).
    pub fn footer_note(mut self, note: impl Into<Label>) -> Self {
        self.footer_note = Some(note.into());
        self
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

/// The zoom row and the pane.
fn show_preview(ui: &mut Ui, label: &Label, texture: &SizedTexture, click_centres: bool, live: &mut Live) -> ShownPreview {
    let zoom = PreviewZoom::new(&live.view).show(ui);
    let pane = PreviewPane::new(label.clone(), *texture, &live.view).click_centres(click_centres).show(ui);
    let centre = pane.shown.centre(texture.size);
    live.view = zoom.view.or(pane.view).unwrap_or(pane.shown);
    ShownPreview { clicked: pane.clicked, centre: Some(centre) }
}

/// Builds the dialog over the host-owned values of a [`HostedParamDialog`] for one pass.
type Build = dyn for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send;

/// A [`ParamDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns: the host keeps the values, and `build` makes
/// the dialog over them each pass.
///
/// - The host keeps the dialog's open state, so a dialog opened again under the same key opens.
/// - The preview picture comes from [`set_preview`](HostedParamDialog::set_preview) between passes, since `build` cannot
///   capture a picture that changes; [`is_previewing`](HostedParamDialog::is_previewing) and
///   [`is_comparing`](HostedParamDialog::is_comparing) say what to render.
/// - With [`held_ok`](HostedParamDialog::held_ok), OK files its answer for the host to check
///   ([`take_ok`](HostedParamDialog::take_ok)) and the dialog waits, OK disabled, for
///   [`answer_ok`](HostedParamDialog::answer_ok).
///
/// ```ignore
/// host.open(HostedParamDialog::new("radial-blur", DialogFields::new(), |fields| {
///     ParamDialog::new("radial-blur", "Radial Blur", params.clone()).values(Bind::bound(fields))
/// }));
/// // each pass: if dialog.is_previewing() { dialog.set_preview(Some(render(dialog.fields()))) }
/// ```
pub struct HostedParamDialog {
    id: DialogId,
    key: String,
    instance: Option<DialogId>,
    fields: DialogFields,
    /// The values were opened (remembered values and defaults loaded) by the first show, OK or set.
    opened: bool,
    build: Box<Build>,
    requests: HostRequests,
    /// Whether the dialog is open: bound to the dialog's `opened` prop, so the dialog's own open state, kept under
    /// its id, cannot hold a dialog closed that the host opened again.
    open: bool,
    /// The values the dialog opened with, passed to the form as the values Reset restores.
    opening: DialogFields,
    /// The preview picture the host last gave (`set_preview`).
    preview: Option<SizedTexture>,
    /// What the dialog showed in its last pass.
    shown: ShownState,
    /// Whether OK files its answer for the host instead of closing ([`held_ok`](HostedParamDialog::held_ok)).
    hold: bool,
    /// An OK the host holds back until it answers `answer_ok`.
    held: Option<Held>,
    /// The host's refusal of the last held OK, shown until a value changes.
    refused: Option<Refused>,
    /// The picture point the next pass centres the preview on ([`centre_preview`](HostedParamDialog::centre_preview)).
    centre: Option<Pos2>,
}

/// What a [`HostedParamDialog`] showed in its last pass: whether the preview is on and held for comparing, and the
/// picture point at the centre of the preview pane.
#[derive(Clone, Copy, Debug, Default)]
struct ShownState {
    previewing: bool,
    comparing: bool,
    centre: Option<Pos2>,
}

/// An OK a [`HostedParamDialog`] holds back until the host answers it.
struct Held {
    result: ParamResult,
    /// `take_ok` has handed the result to the host.
    taken: bool,
    /// The host accepted it: the next pass closes the dialog with it.
    accepted: bool,
}

/// Why the host refused a held OK, and the values it refused.
struct Refused {
    reason: Label,
    fields: DialogFields,
}

impl HostedParamDialog {
    /// A hosted dialog for the command `key`, opening with `fields`; `build` makes the dialog over the values.
    pub fn new(
        key: impl Into<String>,
        fields: DialogFields,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> Self {
        let key = key.into();
        HostedParamDialog {
            id: dialog_id(key.as_str()),
            key,
            instance: None,
            fields,
            opened: false,
            opening: DialogFields::default(),
            build: Box::new(build),
            requests: HostRequests::new(),
            open: true,
            preview: None,
            shown: ShownState::default(),
            hold: false,
            held: None,
            refused: None,
            centre: None,
        }
    }

    /// A hosted command form: the same as [`new`](HostedParamDialog::new), with `build` making a
    /// [`ParamDialog::command`].
    pub fn command(
        key: impl Into<String>,
        fields: DialogFields,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> Self {
        Self::new(key, fields, build)
    }

    /// Names this dialog among others made with the same key (one per document, say), so they keep separate state.
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
        self.preview = picture;
    }

    /// Whether the Preview check box was on in the last pass: the host renders the preview while it is.
    pub fn is_previewing(&self) -> bool {
        self.shown.previewing
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

    /// The OK filed by a held dialog, once.
    pub fn take_ok(&mut self) -> Option<ParamResult> {
        let held = self.held.as_mut().filter(|held| !held.taken)?;
        held.taken = true;
        Some(held.result.clone())
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label under the rows until a value changes. Without a held OK it does nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        match answer {
            Ok(()) => {
                if let Some(held) = self.held.as_mut() {
                    held.accepted = true;
                }
            }
            Err(reason) => {
                if let Some(held) = self.held.take() {
                    self.refused = Some(Refused { reason, fields: held.result.fields });
                }
            }
        }
    }

    /// Holds `result` for the host: the OK of a held dialog, from the button or from `confirm`.
    fn file_held(&mut self, mut result: ParamResult) {
        result.id = self.id;
        self.held = Some(Held { result, taken: false, accepted: false });
        self.refused = None;
    }

    /// The footer button id that was pressed, once (`ParamDialog::extra_buttons`).
    pub fn take_footer_action(&mut self) -> Option<String> {
        self.requests.take_footer_action()
    }

    /// The folder listing a files row asked for, once.
    pub fn take_listing(&mut self) -> Option<ListingRequest> {
        self.requests.take_listing()
    }

    /// Answers the listing request `key` with the `files` of its folder.
    pub fn answer_listing(&mut self, key: impl Into<String>, files: Vec<std::path::PathBuf>) {
        self.requests.answer_listing(key, files);
    }

    /// Sets the value under `key` as an edit would; the dialog shows it in its next pass. Unchecked: the app writes
    /// a value its own custom row holds (Blur Gallery's pins dragged on the canvas), which has no JSON form to check.
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

    /// The host cancelled its picker, file dialog or dropper: the requests not taken yet are dropped.
    pub fn cancel_requests(&mut self) {
        self.requests.cancel();
    }

    /// Opens the values once, as a `ParamDialog`'s first pass does, before the first show, OK or set reads them.
    fn open(&mut self, ctx: &Context) {
        if std::mem::replace(&mut self.opened, true) {
            return;
        }
        let ParamDialog { params, remember, patterns, .. } = (self.build)(&mut self.fields);
        let (params, patterns) = (params.into_owned(), patterns.into_owned());
        let saved = remember.and_then(|key| MemoryStage::peek_saved::<Option<DialogFields>>(ctx, &key));
        ParamDialog::open_values(&params, &patterns, saved.as_ref(), &mut self.fields);
        self.opening = self.fields.clone();
    }
}

/// Checks `value` against the row `field` of `params` and sets it in `fields`, as the user editing the row would; `Err`
/// says why the row refused (`FieldRefusal`). The field is a row's key or its unit key; a Files row takes a
/// `{files, folder, useFolder}` object and sets its two companion keys with it. Every dialog built on `Param`s sets
/// its fields through this one rule.
pub fn set_param_field(params: &[Param], fields: &mut DialogFields, field: &str, value: Value) -> Result<(), String> {
    let Some(param) = params.iter().find(|param| param.key == field || param.unit_key() == Some(field)) else {
        return Err(FieldRefusal::Unknown.message(field));
    };
    if !param.visible(fields) || !param.enabled(fields) {
        return Err(FieldRefusal::Inactive.message(field));
    }
    let writes = if param.key != field {
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

impl StackDialog for HostedParamDialog {
    type Result = ParamResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ParamResult>> {
        let measuring = crate::runtime::measure::active(ui.ctx());
        let answers = if measuring { Vec::new() } else { self.requests.take_answers() };
        self.open(ui.ctx());
        let finish = self.held.as_ref().is_some_and(|held| held.accepted);
        // A measuring run keeps the centre: the real run of the pass takes it.
        let centre = if measuring { self.centre } else { self.centre.take() };
        let mut dialog = (self.build)(&mut self.fields)
            .values_opened(self.opening.clone())
            .answers(answers)
            .opened(Bind::bound(&mut self.open))
            .hold_ok(self.hold)
            .finish_held(finish)
            .refusal(self.refused.as_ref().map(|refused| (refused.reason.clone(), refused.fields.clone())))
            .preview_colors(self.requests.previewed_colors().to_vec())
            .centre_on(centre);
        // OK waits for the host's answer: a second OK cannot file over the held one.
        if self.held.is_some() {
            dialog = dialog.ok_enabled(false);
        }
        if let Some(picture) = self.preview {
            dialog = dialog.preview(picture);
        }
        let shown = dialog.show(ui);
        if let Some(mut rows) = shown.rows {
            self.requests.absorb(&mut rows);
        }
        if let Some(point) = shown.preview_click {
            self.requests.request_preview_click(point);
        }
        // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if let Some(action) = shown.footer_action {
            self.requests.request_footer_action(action);
        }
        self.shown = ShownState { previewing: shown.preview_on, comparing: shown.comparing, centre: shown.preview_centre };
        if self.refused.as_ref().is_some_and(|refused| refused.fields != self.fields) {
            self.refused = None;
        }
        if let Some(result) = shown.held {
            self.file_held(result);
        }
        match (shown.result, shown.closed) {
            (Some(mut result), _) => {
                result.id = self.id;
                Some(DialogResult::Ok(result))
            }
            (None, Some(CloseReason::Activated)) if finish => {
                Some(self.held.take().map_or(DialogResult::Cancel, |held| DialogResult::Ok(held.result)))
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

    /// Answers OK as the button does: none while OK is disabled. The result keeps what a remembering dialog saves. A
    /// held dialog files the answer for the host, as its button does, and answers none.
    fn confirm(&mut self, ctx: &Context) -> Option<ParamResult> {
        self.open(ctx);
        if self.held.is_some() {
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
    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        self.open(ctx);
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
