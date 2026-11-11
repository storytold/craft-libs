//! `FormDialog<V>`: the one typed-value form loop. A dialog that edits one value `V` through a `Bind` keeps the values
//! it opened with, writes the bind as the body edits, settles the footer (OK and Reset) in the pass the body ran and
//! answers with the accepted value. `ParamDialog` is a `FormDialog<DialogFields>`.

use super::dialog::ResetFooter;
use super::host_requests::HostRequests;
use crate::contract::{Bind, BodyCx, Remember as _, ShowContainer as _};
use crate::runtime::requests::HostAnswer;

use crate::dialog::{Dialog, DialogOutput};

use crate::runtime::env::{Env, Label};

use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;

use crate::sections::{DialogRole, FooterButton};

use craft_behavior::{EditEvent, Mixed, Value};

use egui::{Response, Ui};

use std::cell::{Cell, RefCell};

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// The values the dialog opened with, taken after the body's first pass.
#[derive(Clone)]
struct Opened<V>(Option<V>);

impl<V> Default for Opened<V> {
    fn default() -> Self {
        Opened(None)
    }
}

/// What a form body sees: the dialog's body context, whether this is the pass the dialog opened in, the answers the
/// host gave since the last pass and the requests the body files for the host.
pub struct FormCx<'a, 'b> {
    cx: &'a mut BodyCx<'b>,
    first: bool,
    answers: Vec<HostAnswer>,
    requests: &'a mut HostRequests,
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
    /// The index of the footer button picked in this pass, in the order `ResetFooter::buttons` gives them.
    pub picked: Option<usize>,
    /// The value, the pass the dialog closed as OK while it could be accepted.
    pub accepted: Option<V>,
    /// The value after the pass when it differs from the bind's at the start of the pass or the dialog opened in this
    /// pass: a bind that does not write through applies it.
    pub value: Option<V>,
    /// What the body asked of the host in this pass.
    pub requests: HostRequests,
}

type FooterExtraFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

type OnResetFn<'a> = Box<dyn FnMut(&mut BodyCx<'_>) + 'a>;

type ResetWithFn<'a, V> = Box<dyn Fn(&V, &V) -> V + 'a>;

/// A dialog over one value `V`.
///
/// - The bind holds the value. [`own`](FormDialog::own) binds the dialog's memory instead, and the value starts from
///   `V::default()` each time the dialog opens.
/// - Keys: Enter is OK unless the body owns it, Escape cancels. Holding Alt turns Cancel into Reset, which restores
///   the value from when the dialog opened (and runs [`on_reset`](FormDialog::on_reset) for what the body keeps).
/// - The body returns whether the value can be accepted; OK is enabled while it can and `ok_enabled` is true.
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
    ok_enabled: bool,
    ok_reason: Option<Label>,
    alt_resets: bool,
    outer_gesture: bool,
    collection_body: bool,
    closable: bool,
    accept_stays_open: bool,
    footer_extra: Option<FooterExtraFn<'a>>,
    shaped: Option<Box<dyn FnOnce(Dialog<'a>) -> Dialog<'a> + 'a>>,
    answers: Vec<HostAnswer>,
    reset_with: Option<ResetWithFn<'a, V>>,
    on_reset: Option<OnResetFn<'a>>,
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
            ok_enabled: true,
            ok_reason: None,
            alt_resets: true,
            outer_gesture: false,
            collection_body: false,
            closable: true,
            accept_stays_open: false,
            footer_extra: None,
            shaped: None,
            answers: Vec::new(),
            reset_with: None,
            on_reset: None,
        }
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

    /// The second button's label when it is not Cancel.
    pub fn cancel_label(mut self, label: impl Into<Label>) -> Self {
        self.cancel_label = Some(label.into());
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

    /// Whether OK keeps the dialog open when it is picked, until the host answers with a close (an export asks for a
    /// path first, say). The body closes it with `FormCx::request_close_as`; the Reset footer is unchanged.
    pub fn accept_stays_open(mut self, on: bool) -> Self {
        self.accept_stays_open = on;
        self
    }

    /// Controls before the buttons in the footer.
    pub fn footer_extra(mut self, extra: impl FnMut(&mut Ui) + 'a) -> Self {
        self.footer_extra = Some(Box::new(extra));
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

    /// The value Reset sets, given the current value and the one from when the dialog opened; without it Reset sets
    /// the opening value.
    pub fn reset_with(mut self, target: impl Fn(&V, &V) -> V + 'a) -> Self {
        self.reset_with = Some(Box::new(target));
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
            ok_enabled,
            ok_reason,
            alt_resets,
            outer_gesture,
            collection_body,
            closable,
            accept_stays_open,
            footer_extra,
            shaped,
            mut answers,
            reset_with,
            on_reset,
        } = self;
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        let name: Arc<str> = title.as_str().into();
        let bind = Rc::new(RefCell::new(bind));

        let reset = ResetFooter::new(ui, cancel_label, alt_resets);
        let mut accept =
            FooterButton::new(ok_label.unwrap_or_else(|| common.ok.clone()), DialogRole::Accept).enabled(ok_enabled).stays_open(accept_stays_open);
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
                let target = reset_with.as_ref().map_or(opened.clone(), |target| target(&current, &opened));
                let _ = bind.reborrow().write(cx, &EditEvent::set(Mixed::Same(current), target, name.clone(), None));
                if let Some(restore) = on_reset.as_mut() {
                    restore(cx);
                }
            }
        };
        let mut dialog = Dialog::new(title).closable(closable).close_on_enter(ok_enabled).outer_gesture(outer_gesture);
        dialog = reset.install(dialog, accept, restore);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if collection_body {
            dialog = dialog.collection_body();
        }
        if let Some(mut extra) = footer_extra {
            dialog = dialog.footer(move |ui, _| extra(ui));
        }
        if let Some(shape) = shaped {
            dialog = shape(dialog);
        }

        let work = RefCell::new(None::<V>);
        let changed = RefCell::new(None::<V>);
        let requests = RefCell::new(HostRequests::new());
        let accepts = Cell::new(ok_enabled);
        let shown = dialog.show(ui, |ui, cx| {
            // A measuring run sees no answers and no opening pass: the real run that follows still has both.
            let measuring = crate::runtime::measure::active(ui.ctx());
            let first = cx.appeared() && !measuring;
            let mut held = bind.borrow_mut();
            let before = match held.reborrow().read(cx) {
                Some(Mixed::Same(value)) if !(fresh && first) => value,
                _ => V::default(),
            };
            let mut values = before.clone();
            let ok = {
                let mut requests = requests.borrow_mut();
                let mut form = FormCx {
                    cx: &mut *cx,
                    first,
                    answers: if measuring { Vec::new() } else { std::mem::take(&mut answers) },
                    requests: &mut requests,
                };
                body(ui, &mut form, &mut values)
            };
            if first {
                cx.memory::<Opened<V>>().0 = Some(values.clone());
            }
            let can_accept = ok && ok_enabled;
            accepts.set(can_accept);
            let edited = cx.memory::<Opened<V>>().0.as_ref().is_some_and(|opened| *opened != values);
            ResetFooter::report(cx, edited, can_accept);
            if values != before || first {
                let _ = held.reborrow().write(cx, &EditEvent::set(Mixed::Same(before.clone()), values.clone(), name.clone(), None));
                *changed.borrow_mut() = Some(values.clone());
            }
            *work.borrow_mut() = Some(values);
        });

        let DialogOutput { response, closed, picked } = shown.output;
        let accepted = match closed {
            Some(CloseReason::Enter | CloseReason::Activated) if accepts.get() => work.into_inner(),
            _ => None,
        };
        FormOutput { response, closed, picked, accepted, value: changed.into_inner(), requests: requests.into_inner() }
    }
}
