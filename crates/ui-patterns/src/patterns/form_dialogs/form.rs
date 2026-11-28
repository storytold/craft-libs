//! `FormDialog<V>`: the one typed-value form loop. A dialog that edits one value `V` through a `Bind` keeps the values
//! it opened with, writes the bind as the body edits, settles the footer (OK and Reset) in the pass the body ran and
//! answers with the accepted value. `ParamDialog` is a `FormDialog<DialogFields>`.

use super::dialog::ResetFooter;
use super::host_requests::HostRequests;
use crate::choice::Checkbox;
use crate::contract::{Bind, BodyCx, Props as _, Remember as _, Show as _, ShowContainer as _, pressed};
use crate::runtime::commands::Commands as _;
use crate::runtime::requests::HostAnswer;

use crate::dialog::{ButtonPlace, Dialog, DialogOutput, HeldAnswer};

use crate::runtime::env::{Env, Label};

use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;

use crate::sections::{DialogRole, FooterButton};

use craft_behavior::edit_history::EditHistory;
use craft_behavior::{CancelReason, EditEvent, Mixed, Value};

use egui::{Response, Ui};

use std::cell::{Cell, RefCell};

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

craft_ui::words! {
    /// The words a form dialog adds beside the common OK and Cancel.
    pub struct FormDialogWords {
        preview: "formDialog.preview" = "Preview";
        /// The one closing button of a form without OK ([`FormButtons::Close`]).
        close: "formDialog.close" = "Close";
    }
}

/// The buttons a form dialog's footer starts with, before its [extra buttons](FormDialog::extra_buttons).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormButtons {
    /// OK and Cancel (Reset while Alt is held).
    #[default]
    OkCancel,
    /// No OK: one button with `DialogRole::Cancel`, labelled by [`cancel_label`](FormDialog::cancel_label) or Close.
    /// The work is in the extra buttons, whose [`FormOutput::picked`] indexes still start at 2. Enter picks the one
    /// enabled extra button with `DialogRole::Accept` when there is one, and does nothing otherwise. Nothing is filed
    /// as a held OK, and [`FormCx::accept`] does nothing.
    Close,
}

/// The dialog's own undo history of its value while it is open: made on the opening pass, stepped by Undo and Redo.
#[derive(Clone)]
struct History<V>(Option<EditHistory<V>>);

impl<V> Default for History<V> {
    fn default() -> Self {
        History(None)
    }
}

/// The values the dialog opened with, taken after the body's first pass.
#[derive(Clone)]
struct Opened<V>(Option<V>);

impl<V> Default for Opened<V> {
    fn default() -> Self {
        Opened(None)
    }
}

/// What the footer of an owner-less Preview check box keeps while the dialog is open: the box is off.
#[derive(Clone, Default)]
struct PreviewOff(bool);

/// The value a binding holds this pass; `None` while it holds none (a kept or mixed binding).
pub(crate) fn read_bind<T: Value>(bind: &mut Bind<'_, T>) -> Option<T> {
    match bind.reborrow().get() {
        Some(Mixed::Same(value)) => Some(value),
        _ => None,
    }
}

/// Writes `value` through a binding as one step. A given binding keeps its value: the app takes the dialog's reports.
pub(crate) fn write_bind<T: Value>(bind: &mut Bind<'_, T>, value: T) {
    let before = bind.reborrow().get().unwrap_or(Mixed::Mixed);
    let _ = bind.reborrow().apply(&EditEvent::set(before, value, Arc::from(""), None));
}

/// What a form body sees: the dialog's body context, whether this is the pass the dialog opened in, the answers the
/// host gave since the last pass and the requests the body files for the host.
pub struct FormCx<'a, 'b> {
    cx: &'a mut BodyCx<'b>,
    first: bool,
    answers: Vec<HostAnswer>,
    requests: &'a mut HostRequests,
    accept: bool,
}

impl FormCx<'_, '_> {
    /// Whether this is the pass the dialog opened in: the body fills its defaults then.
    pub fn first(&self) -> bool {
        self.first
    }

    /// The host's answers to earlier requests, once: the body writes them into its value.
    pub fn take_answers(&mut self) -> Vec<HostAnswer> {
        std::mem::take(&mut self.answers)
    }

    /// The requests this pass's body files for the host (colour, path, eyedropper).
    pub fn requests(&mut self) -> &mut HostRequests {
        self.requests
    }

    /// The body's activation is the OK press: a double click or Enter on a grid cell, say. After the body, the dialog
    /// takes it as a press of OK while the values can be accepted. Under [`held`](FormDialog::held) it files the values
    /// and stays open for the app's answer (nothing while an answer is out); with
    /// [`accept_stays_open`](FormDialog::accept_stays_open) it reports the pick and stays open; otherwise it closes as
    /// `CloseReason::Activated` with the value. A measuring run and a form without OK ([`FormButtons::Close`]) take
    /// none.
    pub fn accept(&mut self) {
        self.accept = true;
    }
}

impl<'b> Deref for FormCx<'_, 'b> {
    type Target = BodyCx<'b>;

    fn deref(&self) -> &BodyCx<'b> {
        self.cx
    }
}

impl DerefMut for FormCx<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cx
    }
}

/// What a form dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FormOutput<V> {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The index of the footer button picked in this pass, in the order `ResetFooter::buttons` gives them, then the
    /// [`extra_buttons`](FormDialog::extra_buttons) from 2 on. A [`FormCx::accept`] that leaves the dialog open is
    /// OK's pick, 0. Without OK ([`FormButtons::Close`]) Close is 1 and the extras keep their indexes.
    pub picked: Option<usize>,
    /// The value, the pass the dialog closed as OK while it could be accepted.
    pub accepted: Option<V>,
    /// The value after the pass when it differs from the bind's at the start of the pass or the dialog opened in this
    /// pass: a bind that does not write through applies it.
    pub value: Option<V>,
    /// What the body asked of the host in this pass.
    pub requests: HostRequests,
    /// Whether the [`preview`](FormDialog::preview) check box is on after the pass; false without one. Holding the
    /// box to compare leaves it as it was until the release.
    pub preview_on: bool,
    /// Whether the Preview check box is on and held: the host shows the unfiltered picture while it is.
    pub comparing: bool,
}

type FooterExtraFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

type OnResetFn<'a> = Box<dyn FnMut(&mut BodyCx<'_>) + 'a>;

type ResetWithFn<'a, V> = Box<dyn Fn(&V, &V, &mut BodyCx<'_>) -> V + 'a>;

/// A dialog over one value `V`.
///
/// - The bind holds the value. [`own`](FormDialog::own) binds the dialog's memory instead, and the value starts from
///   `V::default()` each time the dialog opens.
/// - Keys: Enter is OK unless the body owns it or [`enter_confirms`](FormDialog::enter_confirms) is off, Escape cancels. Holding Alt turns Cancel into Reset, which restores
///   the value from when the dialog opened (and runs [`on_reset`](FormDialog::on_reset) for what the body keeps).
/// - The body returns whether the value can be accepted; OK is enabled while it can and `ok_enabled` is true. The
///   body's own activation (a double click on a cell) is OK through [`FormCx::accept`].
/// - [`buttons`](FormDialog::buttons) with [`FormButtons::Close`] leaves out OK, for a form whose work is in its extra
///   buttons.
/// - With [`history`](FormDialog::history), Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z step the dialog's own value; a drag or a
///   typing run is one step.
/// - The footer goes across the foot, the Preview check box and the footer controls at its start edge; with
///   [`button_place`](FormDialog::button_place) `ButtonPlace::Side` the buttons stand in a column beside the body, with the Preview check
///   box and the footer controls under them.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern)]
#[pattern(probe(FormDialog<'static, bool>))]
pub struct FormDialog<'a, V: Value + Default> {
    title: Label,
    bind: Bind<'a, V>,
    fresh: bool,
    open: Option<Bind<'a, bool>>,
    ok_label: Option<Label>,
    cancel_label: Option<Label>,
    buttons: FormButtons,
    ok_enabled: bool,
    ok_reason: Option<Label>,
    alt_resets: bool,
    outer_gesture: bool,
    history: bool,
    collection_body: bool,
    closable: bool,
    close_unchanged: bool,
    accept_stays_open: bool,
    enter_confirms: bool,
    button_place: ButtonPlace,
    footer_extra: Option<FooterExtraFn<'a>>,
    /// The Preview check box when shown, over its owner's bind or (`None`) the footer's memory.
    preview: Option<Option<Bind<'a, bool>>>,
    extra_buttons: Vec<FooterButton>,
    shaped: Option<Box<dyn FnOnce(Dialog<'a>) -> Dialog<'a> + 'a>>,
    answers: Vec<HostAnswer>,
    reset_with: Option<ResetWithFn<'a, V>>,
    on_reset: Option<OnResetFn<'a>>,
    /// The host's opening values: each pass given them keeps them as the values Reset restores.
    opening: Option<V>,
    /// Where OK files the values instead of closing; a refusal shows until the values change.
    held: Option<&'a mut HeldAnswer<V>>,
}

impl<'a, V: Value + Default> FormDialog<'a, V> {
    /// A dialog titled `title` over the value in `bind`.
    pub fn new(title: impl Into<Label>, bind: Bind<'a, V>) -> Self {
        FormDialog {
            title: title.into(),
            bind,
            fresh: false,
            open: None,
            ok_label: None,
            cancel_label: None,
            buttons: FormButtons::OkCancel,
            ok_enabled: true,
            ok_reason: None,
            alt_resets: true,
            outer_gesture: false,
            history: false,
            collection_body: false,
            closable: true,
            close_unchanged: false,
            accept_stays_open: false,
            enter_confirms: true,
            button_place: ButtonPlace::Foot,
            footer_extra: None,
            preview: None,
            extra_buttons: Vec::new(),
            shaped: None,
            answers: Vec::new(),
            reset_with: None,
            opening: None,
            on_reset: None,
            held: None,
        }
    }

    /// OK files the values in `held` and the dialog stays open; a refusal shows through `Dialog::refusal` until the
    /// values change, and an accepted answer closes it.
    pub fn held(mut self, held: &'a mut HeldAnswer<V>) -> Self {
        self.held = Some(held);
        self
    }

    /// A dialog that keeps its value in its own memory while it is open.
    pub fn own(title: impl Into<Label>) -> Self {
        FormDialog { fresh: true, ..FormDialog::new(title, Bind::kept()) }
    }

    /// Binds the open state to `open`.
    pub fn opened(mut self, open: Bind<'a, bool>) -> Self {
        self.open = Some(open);
        self
    }

    /// The accepting button's label when it is not OK.
    pub fn ok_label(mut self, label: impl Into<Label>) -> Self {
        self.ok_label = Some(label.into());
        self
    }

    /// The second button's label when it is not Cancel (Close under [`FormButtons::Close`]).
    pub fn cancel_label(mut self, label: impl Into<Label>) -> Self {
        self.cancel_label = Some(label.into());
        self
    }

    /// The buttons the footer starts with; [`FormButtons::OkCancel`] by default.
    pub fn buttons(mut self, buttons: FormButtons) -> Self {
        self.buttons = buttons;
        self
    }

    /// Whether OK can be pressed; Enter does nothing while it cannot.
    pub fn ok_enabled(mut self, on: bool) -> Self {
        self.ok_enabled = on;
        self
    }

    /// Why OK is disabled: the tooltip of the disabled button.
    pub fn ok_reason(mut self, reason: impl Into<Label>) -> Self {
        self.ok_reason = Some(reason.into());
        self
    }

    /// Whether holding Alt turns the second button into Reset; the default is true.
    pub fn alt_resets(mut self, on: bool) -> Self {
        self.alt_resets = on;
        self
    }

    /// Whether the dialog holds one outer gesture over its fields, from open to close.
    pub fn outer_gesture(mut self, on: bool) -> Self {
        self.outer_gesture = on;
        self
    }

    /// Whether the dialog keeps an undo history of its value while it is open: Undo and Redo step it, after a focused
    /// text field's own steps. A change becomes a step once no gesture runs in the body, so a drag or a typing run is
    /// one step. Off by default.
    pub fn history(mut self, on: bool) -> Self {
        self.history = on;
        self
    }

    /// Whether the body is one collection that owns the scroll.
    pub fn collection_body(mut self, on: bool) -> Self {
        self.collection_body = on;
        self
    }

    /// Whether the title row has a close action.
    pub fn closable(mut self, on: bool) -> Self {
        self.closable = on;
        self
    }

    /// While no value differs from the one the dialog opened with, a Close button stands in place of OK and Cancel (as
    /// a Properties dialog does); the first edit brings them back.
    pub fn close_unchanged(mut self, on: bool) -> Self {
        self.close_unchanged = on;
        self
    }

    /// Whether OK keeps the dialog open when it is picked, until the host answers with a close (an export asks for a
    /// path first, say). The body closes it with `FormCx::request_close_as`; the Reset footer is unchanged.
    pub fn accept_stays_open(mut self, on: bool) -> Self {
        self.accept_stays_open = on;
        self
    }

    /// Whether Enter, left untaken by the focused part, is OK; the default is true. Off, only a click (or the
    /// button's own keys) gives OK, as Export As starts only from its Export button.
    pub fn enter_confirms(mut self, on: bool) -> Self {
        self.enter_confirms = on;
        self
    }

    /// Controls before the buttons in the footer.
    pub fn footer_extra(mut self, extra: impl FnMut(&mut Ui) + 'a) -> Self {
        self.footer_extra = Some(Box::new(extra));
        self
    }

    /// The Preview check box, first in the footer: the host previews the value while it is on, and the unfiltered
    /// picture while it is held ([`FormOutput::comparing`]). With `on`, the box reads and writes that bind (a kept
    /// bind starts on). Without it, every open starts with the box on, and the dialog keeps it off while it stays open.
    pub fn preview(mut self, on: Option<Bind<'a, bool>>) -> Self {
        self.preview = Some(on);
        self
    }

    /// Where the buttons stand (`Dialog::button_place`): across the foot by default; `ButtonPlace::Side` puts them in a
    /// column beside the body, with the Preview check box and the footer controls under them, as Photoshop's
    /// button-column dialogs.
    pub fn button_place(mut self, place: ButtonPlace) -> Self {
        self.button_place = place;
        self
    }

    /// Footer buttons after OK and Cancel, which keep the dialog open: the caller acts on
    /// [`FormOutput::picked`].
    pub fn extra_buttons(mut self, buttons: impl IntoIterator<Item = FooterButton>) -> Self {
        self.extra_buttons = buttons.into_iter().map(|button| button.stays_open(true)).collect();
        self
    }

    /// Changes the built [`Dialog`] before it shows, for the settings this type has no setter for.
    pub fn shaped(mut self, shape: impl FnOnce(Dialog<'a>) -> Dialog<'a> + 'a) -> Self {
        self.shaped = Some(Box::new(shape));
        self
    }

    /// The host's answers to earlier requests; the body reads them with [`FormCx::take_answers`].
    pub fn answers(mut self, answers: Vec<HostAnswer>) -> Self {
        self.answers = answers;
        self
    }

    /// The value Reset sets, given the current value, the one from when the dialog opened and the body's context (the
    /// dialog's own memory); without it Reset sets the opening value.
    pub fn reset_with(mut self, target: impl Fn(&V, &V, &mut BodyCx<'_>) -> V + 'a) -> Self {
        self.reset_with = Some(Box::new(target));
        self
    }

    /// The opening values the host keeps: Reset restores them and the edited check compares with them. Each pass given
    /// them stores them, so a host that moves its baseline (a seed after the open) passes the new one. Without them the
    /// first pass's values are the opening values.
    pub fn opening(mut self, opening: V) -> Self {
        self.opening = Some(opening);
        self
    }

    /// Runs when Reset is picked, for what the body keeps in its own memory.
    pub fn on_reset(mut self, restore: impl FnMut(&mut BodyCx<'_>) + 'a) -> Self {
        self.on_reset = Some(Box::new(restore));
        self
    }

    /// Shows the dialog: `body` edits the value and says whether it can be accepted.
    pub fn show(self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &mut FormCx<'_, '_>, &mut V) -> bool) -> FormOutput<V> {
        let FormDialog {
            title,
            bind,
            fresh,
            open,
            ok_label,
            cancel_label,
            buttons,
            ok_enabled,
            ok_reason,
            alt_resets,
            outer_gesture,
            history,
            collection_body,
            closable,
            close_unchanged,
            accept_stays_open,
            enter_confirms,
            button_place,
            footer_extra,
            preview,
            extra_buttons,
            shaped,
            mut answers,
            reset_with,
            on_reset,
            opening,
            held: hold,
        } = self;
        // A measuring run files and closes nothing: the real run that follows does.
        let measuring_run = crate::runtime::measure::active(ui.ctx());
        // The app accepted the held OK: this pass finishes it. A second pass of the frame finds the answer already taken.
        let finish = !measuring_run && hold.as_deref().is_some_and(|held| held.is_accepted());
        // The refusal and the values it refused are read before the body, which may edit the values; the notice is decided
        // after the body, in the same pass, against the values the body left. A blocking (host) refusal shows whatever the
        // values are.
        let refusal =
            hold.as_deref().and_then(|held| held.refusal().map(|(reason, refused)| (reason.clone(), refused.clone(), held.refusal_blocks())));
        let reason_cell = Rc::new(Cell::new(None::<Label>));
        let waiting = hold.as_deref().is_some_and(|held| held.is_waiting());
        let ok_enabled = ok_enabled && !waiting;
        // A held OK files its values and leaves the dialog open until the app answers.
        let accept_stays_open_button = accept_stays_open || hold.is_some();
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        let name: Arc<str> = title.as_str().into();
        let bind = Rc::new(RefCell::new(bind));

        // A form without OK has its one closing button and the extras; Enter goes to the enabled Accept-role extra.
        let no_ok = buttons == FormButtons::Close;
        let enter_extra = extra_buttons.iter().any(|button| button.role() == DialogRole::Accept && button.is_enabled());
        let close = no_ok.then(|| {
            let label = cancel_label.clone().unwrap_or_else(|| env.words::<FormDialogWords>().close.clone());
            std::iter::once(FooterButton::new(label, DialogRole::Cancel)).chain(extra_buttons.iter().cloned()).collect::<Vec<_>>()
        });
        let reset = ResetFooter::new(ui, cancel_label, alt_resets).extra(extra_buttons);
        let mut accept = FooterButton::new(ok_label.unwrap_or_else(|| common.ok.clone()), DialogRole::Accept)
            .enabled(ok_enabled)
            .stays_open(accept_stays_open_button);
        if let (false, Some(reason)) = (ok_enabled, &ok_reason) {
            accept = accept.disabled_reason(reason.clone());
        }
        let restore = {
            let bind = Rc::clone(&bind);
            let name = name.clone();
            let mut on_reset = on_reset;
            move |cx: &mut BodyCx<'_>| {
                let Some(opened) = cx.memory::<Opened<V>>().0.clone() else { return };
                let mut bind = bind.borrow_mut();
                let current = match bind.reborrow().read(cx) {
                    Some(Mixed::Same(held)) => held,
                    _ => opened.clone(),
                };
                let target = reset_with.as_ref().map_or(opened.clone(), |target| target(&current, &opened, cx));
                let _ = bind.reborrow().write(cx, &EditEvent::set(Mixed::Same(current), target, name.clone(), None));
                if let Some(restore) = on_reset.as_mut() {
                    restore(cx);
                }
            }
        };
        let mut dialog = Dialog::new(title)
            .closable(closable)
            .close_unchanged(close_unchanged)
            .close_on_enter(enter_confirms && if no_ok { enter_extra } else { ok_enabled })
            .outer_gesture(outer_gesture)
            .button_place(button_place);
        dialog = match close {
            Some(close) => dialog.buttons(close),
            None => reset.install(dialog, accept, restore),
        };
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if collection_body {
            dialog = dialog.collection_body();
        }
        let changed = RefCell::new(None::<V>);
        let (preview_on, comparing) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
        if footer_extra.is_some() || preview.is_some() {
            let (on, held, label) = (Rc::clone(&preview_on), Rc::clone(&comparing), env.words::<FormDialogWords>().preview.clone());
            let (mut extra, mut preview) = (footer_extra, preview);
            dialog = dialog.footer(move |ui, footer| {
                if let Some(owner) = preview.as_mut() {
                    let cx = footer.bar();
                    let before = match owner.as_mut().map(|bind| bind.reborrow().read(cx)) {
                        Some(Some(Mixed::Same(value))) => value,
                        Some(_) => true,
                        None => !cx.memory::<PreviewOff>().0,
                    };
                    let mut value = before;
                    let shown = Checkbox::new(Bind::bound(&mut value)).label(label.clone()).show(ui);
                    if value != before {
                        match owner.as_mut() {
                            Some(bind) => {
                                let _ = bind.reborrow().write(cx, &EditEvent::set(Mixed::Same(before), value, label.as_str().into(), None));
                            }
                            None => cx.memory::<PreviewOff>().0 = !value,
                        }
                    }
                    on.set(value);
                    // The box toggles at the release: while it is held it still shows its value.
                    held.set(value && pressed(&shown.response));
                }
                if let Some(extra) = extra.as_mut() {
                    extra(ui);
                }
            });
        }
        if let Some(shape) = shaped {
            dialog = shape(dialog);
        }
        if hold.is_some() {
            let reason = Rc::clone(&reason_cell);
            dialog = dialog.refusal(move || reason.take());
        }

        let work = RefCell::new(None::<V>);
        let requests = RefCell::new(HostRequests::new());
        let accepts = Cell::new(ok_enabled);
        let body_pick = Cell::new(false);
        let shown = dialog.show(ui, |ui, cx| {
            // A measuring run sees no answers and no opening pass: the real run that follows still has both. The dialog
            // measures its body in a run of its own too (a re-layout under a child dialog), inside a real run of the form.
            let measuring = measuring_run || crate::runtime::measure::active(ui.ctx());
            let first = cx.appeared() && !measuring;
            let mut held = bind.borrow_mut();
            let before = match held.reborrow().read(cx) {
                Some(Mixed::Same(value)) if !(fresh && first) => value,
                _ => V::default(),
            };
            let mut values = before.clone();
            let mut steps = (history && !measuring).then(|| std::mem::take(&mut cx.memory::<History<V>>().0).filter(|_| !first)).flatten();
            if history && !measuring {
                let can = |steps: &Option<EditHistory<V>>| steps.as_ref().map_or((false, false), |steps| (steps.can_undo(), steps.can_redo()));
                while let Some(step) = cx.take_step(can(&steps).0, can(&steps).1) {
                    if let Some(steps) = steps.as_mut() {
                        crate::runtime::edit::cancel_all(ui.ctx(), CancelReason::UndoDuringGesture);
                        steps.step(&mut values, step);
                    }
                }
            }
            let (ok, running, activated) = {
                let mut requests = requests.borrow_mut();
                let mut form = FormCx {
                    cx: &mut *cx,
                    first,
                    answers: if measuring { Vec::new() } else { std::mem::take(&mut answers) },
                    requests: &mut requests,
                    accept: false,
                };
                let ctx = ui.ctx().clone();
                let (ok, running) = crate::runtime::edit::watch_gestures(&ctx, || body(ui, &mut form, &mut values));
                (ok, running, form.accept && !measuring && !no_ok)
            };
            match &opening {
                Some(opening) if !measuring => cx.memory::<Opened<V>>().0 = Some(opening.clone()),
                None if first => cx.memory::<Opened<V>>().0 = Some(values.clone()),
                _ => {}
            }
            if history && !measuring {
                let mut steps = steps.unwrap_or_else(|| EditHistory::new(&values));
                // Use the history's settled value: `before` can be an intermediate drag value
                // because a change becomes a step only after its gesture has ended.
                if !running {
                    steps.record(&EditEvent::set(Mixed::Mixed, values.clone(), name.clone(), None));
                }
                cx.memory::<History<V>>().0 = Some(steps);
            }
            let can_accept = ok && ok_enabled;
            accepts.set(can_accept);
            // The body's activation presses OK: a press that stays open is reported as the button's pick (and filed when
            // held, below); otherwise it closes with the value.
            if activated && can_accept {
                if accept_stays_open_button {
                    body_pick.set(true);
                } else {
                    cx.request_close_as(CloseReason::Activated);
                }
            }
            let edited = cx.memory::<Opened<V>>().0.as_ref().is_some_and(|opened| *opened != values);
            ResetFooter::report(cx, edited, can_accept);
            if values != before || first {
                let _ = held.reborrow().write(cx, &EditEvent::set(Mixed::Same(before.clone()), values.clone(), name.clone(), None));
                *changed.borrow_mut() = Some(values.clone());
            }
            if let Some((reason, refused, blocks)) = &refusal
                && (*blocks || *refused == values)
            {
                reason_cell.set(Some(reason.clone()));
            }
            if finish && !accept_stays_open {
                cx.request_close_as(CloseReason::Activated);
            }
            *work.borrow_mut() = Some(values);
        });

        let DialogOutput { response, closed, picked } = shown.output;
        // Without OK the dialog's buttons start at Close, which keeps index 1, and the extras keep theirs from 2.
        let picked = if no_ok { picked.map(|at| at + 1) } else { picked.or(body_pick.get().then_some(0)) };
        let mut accepted = match closed {
            Some(CloseReason::Enter | CloseReason::Activated) if accepts.get() && hold.is_none() && !no_ok => work.borrow().clone(),
            _ => None,
        };
        if let Some(held) = hold {
            let values = work.into_inner();
            // A blocking refusal ends only when the host withdraws it.
            if let (Some((_, refused)), Some(values)) = (held.refusal(), values.as_ref())
                && refused != values
                && !held.refusal_blocks()
            {
                held.clear_refusal();
            }
            // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
            if let (false, true, Some(0), Some(values)) = (measuring_run, accepts.get(), picked, values.as_ref())
                && !held.is_waiting()
            {
                held.file(values.clone());
            }
            if finish && (accept_stays_open || closed == Some(CloseReason::Activated)) {
                accepted = held.finish();
            }
        }
        FormOutput {
            response,
            closed,
            picked,
            accepted,
            value: changed.take(),
            requests: requests.into_inner(),
            preview_on: preview_on.get(),
            comparing: comparing.get(),
        }
    }
}
