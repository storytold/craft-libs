//! `PreferencesWindow`: the preferences body in a modeless `Dialog` that opens in its own OS window where windows
//! exist.

#[cfg(feature = "testing")]
mod stories;

use super::body::{self, BodyOut};
use super::model::{Closed, PreferencesModel, PreferencesOutput};
use super::words::PreferencesWords;
use crate::contract::{Bind, Props as _, Remember, SaveKey, Show as _, ShowContainer as _};
use crate::dialog::{Dialog, DialogOutput};
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::StageAccess;
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use craft_behavior::{EditEvent, Mixed};
use craft_i18n::matcher::Matcher;
use craft_theme::variant::Tone;
use egui::Ui;
use std::cell::RefCell;
use std::sync::Arc;

/// What the window keeps between passes; the host owns it and binds it with [`PreferencesWindow::new`].
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PreferencesState {
    /// The id of the page the user picked. While a search hides it another page shows, and this one returns when the
    /// search is cleared.
    pub selected: String,
    /// The text in the search field. Empty lists every page.
    pub search: String,
    /// The message of a failed Apply, shown in the footer until the next edit or Apply.
    pub error: Option<String>,
}

/// The preferences window: a search field over a page list and the shown page, with OK, Cancel and Apply under them.
///
/// The host keeps the model ([`PreferencesModel`]) and calls [`show`](PreferencesWindow::show) on every pass the window
/// should exist; it stops calling to close the window, and does so when the output says `closed`.
///
/// - **Pages and search.** The host builds the pages each pass; the window lists the pages that match the search (the
///   empty ones dimmed) and shows the picked one, or the first listed when the search hides it. Typing in the search
///   field filters; Up, Down and Enter in it go to the page list.
/// - **Rows.** Each row is the library's control for its `RowKind`; an edit goes to [`PreferencesModel::set`] and
///   nothing is saved before OK or Apply. Reset Section resets the shown page's rows in the draft.
/// - **Buttons.** OK applies and closes, Apply applies and stays (enabled while the draft differs), Cancel drops the
///   draft. A failed apply keeps the window open and shows its message in the footer. Enter activates OK and Escape
///   cancels, once the search text, a field's own gesture and a popup have had them.
/// - **Nested dialogs.** While [`PreferencesModel::nested_open`] is true the window takes no input and refuses to
///   close; [`PreferencesModel::nested`] draws the nested dialogs inside the window's own viewport.
/// - **Placement.** The window opens in its own OS window where there are any, else as a modeless dialog layer
///   ([`embedded`](PreferencesWindow::embedded) forces the layer). [`remember`](PreferencesWindow::remember) keeps
///   its place and size across runs.
#[must_use = "show the window with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct PreferencesWindow<'a> {
    state: Bind<'a, PreferencesState>,
    /// The window's title; the translated "Preferences" without it.
    #[prop(into)]
    title: Option<Label>,
    /// Draws the window as a dialog layer even where OS windows exist.
    #[prop(flag)]
    embedded: bool,
    /// Keeps the window's place and size across runs under `key`.
    #[prop]
    remember: Option<SaveKey>,
}

/// What the body, the footer and the buttons' guard share during one pass: the host, the window's state as the pass
/// changes it, and what the buttons did. The state is read and written through its binding by the regions that have
/// the dialog's body context.
struct Session<'h, 'b, H: ?Sized> {
    host: &'h mut H,
    state: Bind<'b, PreferencesState>,
    title: Arc<str>,
    before: PreferencesState,
    work: PreferencesState,
    page: Option<String>,
    applied: bool,
}

impl<H: PreferencesModel + ?Sized> Session<'_, '_, H> {
    /// Reads the state at the start of the body. A window that has just opened starts with no search and no error.
    fn begin(&mut self, cx: &mut (impl StageAccess + Remember)) {
        self.before = match self.state.reborrow().read(cx) {
            Some(Mixed::Same(held)) => held,
            _ => PreferencesState::default(),
        };
        self.work = self.before.clone();
        if cx.appeared() {
            self.work.search.clear();
            self.work.error = None;
        }
    }

    /// Writes the state the pass changed through the binding.
    fn commit(&mut self, cx: &mut impl StageAccess) {
        if self.work != self.before {
            let event = EditEvent::set(Mixed::Same(self.before.clone()), self.work.clone(), self.title.clone(), None);
            let _ = self.state.reborrow().write(cx, &event);
            self.before = self.work.clone();
        }
    }

    /// Applies the draft and records the outcome: the applied flag, or the message the footer shows.
    fn apply(&mut self, cx: &mut impl StageAccess) -> bool {
        let result = self.host.apply();
        let ok = result.is_ok();
        match result {
            Ok(()) => {
                self.applied = true;
                self.work.error = None;
            }
            Err(message) => self.work.error = Some(message),
        }
        self.commit(cx);
        ok
    }
}

impl<'a> PreferencesWindow<'a> {
    /// A window over the host's `state`.
    pub fn new(state: Bind<'a, PreferencesState>) -> Self {
        PreferencesWindow { state, title: None, embedded: false, remember: None }
    }

    /// Shows the window for this pass over `host`. The output's `closed` tells the host to stop showing it.
    pub fn show<H: PreferencesModel + ?Sized>(self, ui: &mut Ui, host: &mut H) -> PreferencesOutput {
        let PreferencesWindow { state, title, embedded, remember } = self;
        let env = Env::of(ui);
        let words = env.words::<PreferencesWords>();
        let common = env.words::<CommonWords>();
        let units = env.units();

        let nested = host.nested_open();
        let can_ok = host.can_ok() && !nested;
        let differs = host.differs() && !nested;
        let title = title.unwrap_or_else(|| words.title.clone());
        let mut ok = FooterButton::new(common.ok.clone(), DialogRole::Accept).enabled(can_ok);
        if !can_ok {
            ok = ok.disabled_reason(words.cannot_ok.clone());
        }
        let apply = FooterButton::new(words.apply.clone(), DialogRole::Other).enabled(differs).stays_open(true);
        let buttons = [ok, apply, FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];

        let mut open = true;
        let session = RefCell::new(Session {
            host: &mut *host,
            state,
            title: title.as_str().into(),
            before: PreferencesState::default(),
            work: PreferencesState::default(),
            page: None,
            applied: false,
        });
        let mut dialog = Dialog::new(title.clone())
            .collection_body()
            .modeless()
            .resizable(true)
            .close_on_enter(can_ok)
            .on_pick(|button, cx| {
                if button.role() == DialogRole::Other && !nested {
                    session.borrow_mut().apply(cx);
                }
            })
            .close_guard(|button, cx| button.role() != DialogRole::Accept || session.borrow_mut().apply(cx));
        if !embedded {
            dialog = dialog.viewport();
        }
        if let Some(key) = remember {
            dialog = dialog.remember(key);
        }
        let shown = dialog
            .footer(|ui, _| {
                let message = session.borrow().work.error.clone();
                if let Some(message) = message {
                    Notice::new(Tone::Bad, Label::from(message)).layout(NoticeLayout::Inline).show(ui);
                }
            })
            .opened(Bind::bound(&mut open))
            .enabled(!nested)
            .buttons(buttons)
            .show(ui, |ui, cx| {
                let mut guard = session.borrow_mut();
                let session = &mut *guard;
                session.begin(cx);
                let matcher = Matcher::new(&env.folder(), session.work.search.trim());
                let snap = body::snapshot(&*session.host, &matcher, &session.work.selected);
                let mut out = BodyOut::default();
                body::show_body(ui, &mut *session.host, &snap, &mut session.work, &words, &units, &mut out);
                let edits = out.edits();
                if !edits.is_empty() || out.reset {
                    session.work.error = None;
                }
                for (key, value) in edits {
                    session.host.set(&key, value);
                }
                if out.reset
                    && let Some(page) = &snap.shown
                {
                    session.host.reset_page(&page.id);
                }
                session.host.nested(ui.ctx());
                session.page = snap.shown.map(|page| page.id);
                session.commit(cx);
            });

        let Session { host, page, applied, .. } = session.into_inner();
        let DialogOutput { response, closed, .. } = shown.output;
        let mut result = PreferencesOutput::new(response);
        result.page = page;
        result.applied = applied;
        match closed.filter(|_| !nested) {
            Some(CloseReason::Activated) => result.closed = Some(Closed::Ok),
            Some(_) => {
                host.cancel();
                result.closed = Some(Closed::Cancel);
            }
            None => {}
        }
        result
    }
}
