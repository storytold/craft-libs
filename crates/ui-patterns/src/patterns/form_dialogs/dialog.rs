//! `ParamDialog`: a dialog from a parameter list, with an optional preview, and `HostedParamDialog`, the same dialog
//! owned by a `DialogHost`. A command form is a `ParamDialog` without a preview.

use super::fields::{DialogFields, FieldValue};
use crate::runtime::requests::{ColorRequest, HostAnswer, PathRequest};

use super::form::FormDialog;
use super::host_requests::HostRequests;
use super::param::{Param, ParamKind, open_fields, set_color_field};
use super::preview::{PreviewPane, PreviewView, PreviewZoom};

use super::rows::{ParamRows, ParamRowsOutput};

use crate::choice::Checkbox;
use crate::contract::{Bind, BodyCx, NoValue, Outcome, Props as _, Remember as _, SaveKey, Show as _, ShowContainer as _};

use crate::dialog::{Dialog, DialogId, DialogResult, DialogStack, FooterHooks, StackDialog, dialog_id};

use crate::layout_components::{Row, Stack};

use crate::runtime::env::{Env, Label};

use crate::runtime::memory::{MemoryStage, unique};

use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};

use egui::load::SizedTexture;
use egui::{Response, Ui};

use std::borrow::Cow;

#[cfg(feature = "testing")]
mod stories;

craft_ui::words! {
    /// The words of the parameter dialog beside the common OK and Cancel.
    pub struct ParamDialogWords {
        reset: "paramDialog.reset" = "Reset";
        preview: "paramDialog.preview" = "Preview";
    }
}

/// Where the preview pane sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamLayout {
    /// The preview pane to the right of the rows, under the Preview check box and the zoom row.
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
    remember: Option<(SaveKey, DialogFields)>,
}

impl ParamResult {
    /// Stores the values of a dialog built with [`ParamDialog::remember`] for the next open. The app calls it after
    /// its command succeeds, so a failed command leaves the previous values; without a remember key it does nothing.
    pub fn remember(&self, ui: &Ui) {
        if let Some((key, values)) = &self.remember {
            let mut stage = MemoryStage::open(ui.ctx(), unique(("craft_ui::param_result", key.as_str())));
            *stage.saved::<Option<DialogFields>>(key) = Some(values.clone());
            stage.write_back();
        }
    }
}

/// What a parameter dialog keeps while it is open: the Preview check box and the view of the preview pane.
#[derive(Clone)]
struct Live {
    on: bool,
    view: PreviewView,
}

impl Default for Live {
    fn default() -> Self {
        Live { on: true, view: PreviewView::default() }
    }
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
    /// Whether the Preview check box is on: the host renders the preview while it is.
    pub preview_on: bool,
}

impl Outcome for ParamDialogOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A dialog from a parameter list: one row per [`Param`], the Preview check box with a preview pane when the host
/// gives a picture, and OK, Cancel and a footer extra.
///
/// - Keys: Tab walks the fields, Enter is OK unless a field owns it, Escape cancels. Holding Alt turns Cancel into
///   Reset, which restores the values from when the dialog opened (or `reset_to`) and keeps the dialog open.
/// - Values: the `values` binding holds them; without one the dialog keeps its own between passes. The first pass
///   fills the parameters' defaults and, with `remember`, the values saved by [`ParamResult::remember`].
/// - Preview: the host renders from the reported values while `preview_on` and hands the picture to `preview`. Drag
///   pans the picture, the zoom buttons scale it.
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
    /// Controls before the buttons in the footer (Color Table's Load and Save).
    #[prop(func = "FnMut(&mut Ui)")]
    footer_extra: Option<FooterExtra<'a>>,
    /// A section under the rows that only reads the values (a monitor note).
    #[prop(func = "Fn(&mut Ui, &DialogFields)")]
    below: Option<BelowRows<'a>>,
    shaped: Option<Shaped<'a>>,
    /// The host's answers to the requests of earlier passes; the dialog writes them into its values first.
    #[prop]
    answers: Vec<HostAnswer>,
}

/// What `ParamDialog::shaped` applies to the built [`Dialog`].
struct Shaped<'a>(Box<dyn FnOnce(Dialog<'a>) -> Dialog<'a> + 'a>);

/// What `ParamDialog::footer_extra` draws before the buttons.
pub struct FooterExtra<'a>(Box<dyn FnMut(&mut Ui) + 'a>);

impl<'a> FooterExtra<'a> {
    /// The extra that runs `add`.
    pub fn new(add: impl FnMut(&mut Ui) + 'a) -> Self {
        FooterExtra(Box::new(add))
    }
}

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
            collection_body: false,
            reset_to: None,
            layout: ParamLayout::default(),
            closable: true,
            preview: None,
            footer_extra: None,
            below: None,
            shaped: None,
            answers: Vec::new(),
        }
    }

    /// Changes the built [`Dialog`] before it shows, for the `Dialog` settings a form track needs and `ParamDialog`
    /// has no setter for (`Dialog::overlay`, say). It runs after the settings `ParamDialog` makes itself.
    pub fn shaped(mut self, shape: impl FnOnce(Dialog<'a>) -> Dialog<'a> + 'a) -> Self {
        self.shaped = Some(Shaped(Box::new(shape)));
        self
    }

    /// The part of `fields` a remembering dialog saves: numbers, angles, choices and check boxes of the remembered
    /// rows and their unit choices; colours, grids, points, channels and free text start from their defaults.
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
            collection_body,
            reset_to,
            layout,
            closable,
            preview,
            footer_extra,
            below,
            shaped,
            answers,
        } = self;
        let words = Env::of(ui).words::<ParamDialogWords>();

        let mut form = match bound {
            Some(bind) => FormDialog::new(title.clone(), bind),
            None => FormDialog::own(title.clone()),
        };
        form = form
            .ok_enabled(ok_enabled)
            .alt_resets(alt_resets)
            .outer_gesture(outer_gesture)
            .collection_body(collection_body)
            .closable(closable)
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
        if let Some(open) = open {
            form = form.opened(open);
        }
        if let Some(FooterExtra(extra)) = footer_extra {
            form = form.footer_extra(extra);
        }
        if let Some(Shaped(shape)) = shaped {
            form = form.shaped(shape);
        }

        let mut live = Live::default();
        let mut rows = None;
        let shown = form.show(ui, |ui, cx, values| {
            if cx.first() {
                if let Some(key) = &remember
                    && let Some(saved) = cx.saved::<Option<DialogFields>>(key).clone()
                {
                    for (name, value) in Self::remembered(&params, &saved).iter() {
                        values.set(name, value.clone());
                    }
                }
                open_fields(&params, values);
            }
            for answer in cx.take_answers() {
                match answer {
                    HostAnswer::Color { key, color } | HostAnswer::Eyedropper { key, color } => set_color_field(&params, values, &key, color),
                    HostAnswer::Path { key, path } => values.set(&key, FieldValue::Text(path.to_string_lossy().into_owned())),
                }
            }
            live = cx.memory::<Live>().clone();
            let mut show_rows = |ui: &mut Ui, values: &mut DialogFields| {
                rows = Some(ParamRows::new(title.clone(), &params, Bind::bound(values)).show(ui));
            };
            match (&preview, layout) {
                (Some(texture), ParamLayout::Beside) => {
                    Row::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        Stack::new().show(ui, |ui, _| show_preview(ui, &words, texture, &mut live));
                    });
                }
                (Some(texture), ParamLayout::Below) => {
                    Stack::new().show(ui, |ui, _| {
                        show_rows(ui, values);
                        show_preview(ui, &words, texture, &mut live);
                    });
                }
                (None, _) => show_rows(ui, values),
            }
            if let Some(BelowRows(draw)) = &below {
                draw(ui, values);
            }
            *cx.memory::<Live>() = live.clone();
            true
        });

        let result = shown.accepted.map(|fields| {
            let kept = remember.map(|key| (key, Self::remembered(&params, &fields)));
            ParamResult { key, fields, remember: kept }
        });
        ParamDialogOutput {
            response: shown.response,
            closed: shown.closed,
            result,
            rows,
            values: shown.value,
            preview_on: preview.is_some() && live.on,
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
        }
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
            .buttons(self.buttons(accept, false))
            .settle_buttons(move |cx, buttons| {
                let footing = *cx.memory::<Footing>();
                settle.settle(buttons, footing.changed);
                if let (Some(ok), Some(accept)) = (footing.ok, buttons.first_mut()) {
                    accept.set_enabled(ok);
                }
            })
            .on_pick(move |button, cx| {
                if pick.resetting() && button.role() == DialogRole::Other {
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
            _ if self.resetting => Some(ResetPick::Reset),
            _ => Some(ResetPick::Cancel),
        }
    }
}

/// The Preview check box, the zoom row and the pane.
fn show_preview(ui: &mut Ui, words: &ParamDialogWords, texture: &SizedTexture, live: &mut Live) {
    Checkbox::new(Bind::bound(&mut live.on)).label(words.preview.clone()).show(ui);
    let zoom = PreviewZoom::new(&live.view).show(ui);
    let pane = PreviewPane::new(words.preview.clone(), *texture, &live.view).show(ui);
    live.view = zoom.view.or(pane.view).unwrap_or(pane.shown);
}

/// Builds the dialog over the host-owned values of a [`HostedParamDialog`] for one pass.
type Build = dyn for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send;

/// A [`ParamDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns: the host keeps the values, and `build` makes
/// the dialog over them each pass.
///
/// ```ignore
/// host.open(HostedParamDialog::new("radial-blur", DialogFields::new(), |fields| {
///     ParamDialog::new("radial-blur", "Radial Blur", params.clone()).values(Bind::bound(fields)).preview(texture)
/// }));
/// ```
pub struct HostedParamDialog {
    id: DialogId,
    key: String,
    instance: Option<DialogId>,
    fields: DialogFields,
    build: Box<Build>,
    requests: HostRequests,
}

impl HostedParamDialog {
    /// A hosted dialog for the command `key`, opening with `fields`; `build` makes the dialog over the values.
    pub fn new(
        key: impl Into<String>,
        fields: DialogFields,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> Self {
        let key = key.into();
        HostedParamDialog { id: dialog_id(key.as_str()), key, instance: None, fields, build: Box::new(build), requests: HostRequests::new() }
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

    /// The host cancelled its picker, file dialog or dropper: the requests not taken yet are dropped.
    pub fn cancel_requests(&mut self) {
        self.requests.cancel();
    }
}

impl StackDialog for HostedParamDialog {
    type Result = ParamResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ParamResult>> {
        let answers = if crate::runtime::measure::active(ui.ctx()) { Vec::new() } else { self.requests.take_answers() };
        let shown = (self.build)(&mut self.fields).answers(answers).show(ui);
        if let Some(mut rows) = shown.rows {
            self.requests.absorb(&mut rows);
        }
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(result)),
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
}
