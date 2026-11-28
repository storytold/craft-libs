//! `PreferencesWindow`: the preferences body in an app-modal `Dialog` that opens in its own OS window where windows
//! exist.

#[cfg(feature = "suite")]
pub(crate) mod stories;

#[cfg(doc)]
use super::model::PreferencesModel;
use super::model::{Closed, PreferencesDraft, PreferencesOutput};
use super::page::{PreferencesLayout, PreferencesPage};
use super::words::PreferencesWords;
use crate::buttons::Button;
use crate::contract::{Bind, Props as _, SaveKey, Show as _, ShowContainer as _, clicked};
use crate::dialog::{ButtonPlace, Dialog, DialogOutput};
use crate::layout_components::Row;
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Env, Label};
use crate::runtime::frame::request_repaint;
use crate::runtime::memory::StageAccess;
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use craft_behavior::{EditEvent, Mixed};
use craft_theme::variant::Tone;
use egui::{Ui, Vec2, vec2};
use std::cell::RefCell;
use std::sync::Arc;

/// The size the window opens at until the user resizes it, with the page list ([`PreferencesLayout::Paged`]).
pub const DEFAULT_SIZE: Vec2 = vec2(1040.0, 800.0);
/// The least size the window may be resized to.
pub const MIN_SIZE: Vec2 = vec2(560.0, 400.0);
/// The size a window of cards ([`PreferencesLayout::Cards`]) opens at: one column of cards needs no room for a page
/// list.
const CARDS_SIZE: Vec2 = vec2(640.0, 720.0);
/// The least size a window of cards may be resized to.
const CARDS_MIN_SIZE: Vec2 = vec2(420.0, 320.0);
/// The size a [`PreferencesLayout::Classic`] window opens at: a desktop preferences dialog's.
const CLASSIC_SIZE: Vec2 = vec2(1030.0, 660.0);

/// When preferences changes are applied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreferencesMode {
    /// Keep a draft until OK or Apply.
    #[default]
    Staged,
    /// Apply changes as they happen; OK closes the window.
    Live,
}

/// What the window keeps between passes; the host owns it and binds it with [`PreferencesWindow::new`].
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PreferencesState {
    /// The id of the page the user picked. While a search hides it another page shows, and this one returns when the
    /// search is cleared.
    pub selected: String,
    /// The text in the search field. Empty lists every page.
    pub search: String,
    /// The page the view last scrolled to for `selected`, so a page the host names is brought into view once.
    pub scrolled_to: String,
    /// The message of a failed Apply, shown in the footer until the next edit or Apply.
    pub error: Option<String>,
}

/// The preferences window: a search field over a page list and the shown page, with OK, Cancel and Apply under them.
///
/// The body is a [`PreferencesPage`] over the window's state. The host keeps the model ([`PreferencesModel`] and
/// [`PreferencesDraft`]) and calls [`show`](PreferencesWindow::show) on every pass the window
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
/// - **Layout.** [`layout`](PreferencesWindow::layout) passes a [`PreferencesLayout`] to the page:
///   [`PreferencesLayout::Cards`] shows one scrolling column of cards with the search's match count, and the window
///   opens narrower, as it needs no room for a page list.
/// - **Live mode.** [`PreferencesMode::Live`] applies changes as they occur and shows only OK, which closes without
///   applying again. Closing the window does not cancel changes. A failed apply shows its message until another edit.
/// - **Page steps.** [`page_steps`](PreferencesWindow::page_steps) adds Previous and Next to the footer. They pick the
///   page before and after the shown one among the pages the search lists, and each is disabled at its end of the
///   list. A window of cards, which scrolls from page to page, does not show them.
/// - **Nested dialogs.** While [`PreferencesModel::nested_open`] is true the window takes no input and refuses to
///   close; [`PreferencesModel::nested`] draws the nested dialogs with the body's `Ui`, inside the window's own
///   viewport.
/// - **App-modal.** While the window is open the app's other windows take no input, show the blocked cursor and
///   refuse to close, and a press on one raises the window.
/// - **Placement.** The window opens in its own OS window where there are any, else as a modal dialog layer
///   ([`embedded`](PreferencesWindow::embedded) forces the layer). [`size`](PreferencesWindow::size) sets the size it
///   opens at. [`remember`](PreferencesWindow::remember) keeps its place and size across runs.
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
    /// Selects whether changes are staged or applied while the window is open.
    #[prop]
    mode: PreferencesMode,
    /// Lays the page out as `layout` says ([`PreferencesPage::layout`]); the default is [`PreferencesLayout::Paged`].
    #[prop]
    layout: PreferencesLayout,
    /// The size the window opens at until the user resizes it, in place of the layout's own ([`DEFAULT_SIZE`] for
    /// pages, a narrower one for cards). A size under the layout's least ([`MIN_SIZE`] for pages) opens at the least;
    /// a size that is not finite is ignored.
    #[prop(quantity = "points, the window's opening size")]
    size: Option<Vec2>,
    /// Escape keeps the typing in the focused field ([`Dialog::escape_keeps_typing`]): in [`PreferencesMode::Live`]
    /// the typed value stays applied when Escape closes the window. Off by default: Escape cancels the typing.
    #[prop(flag)]
    escape_keeps_typing: bool,
    /// Adds Previous and Next to the footer, which pick the listed page before and after the shown one. Only
    /// [`PreferencesLayout::Paged`] shows them.
    #[prop(flag)]
    page_steps: bool,
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
    /// The listed pages before and after the shown one, which Previous and Next pick.
    steps: (Option<String>, Option<String>),
    /// The pages the body listed this pass, in order, for Prev and Next.
    pages: Vec<String>,
    applied: bool,
}

impl<H: PreferencesDraft + ?Sized> Session<'_, '_, H> {
    /// Shows the page `step` places from the shown one among the listed pages, wrapping at either end; the first
    /// listed page when none is shown.
    fn step_page(&mut self, cx: &mut impl StageAccess, step: isize) {
        let count = self.pages.len();
        if count == 0 {
            return;
        }
        let at = self.page.as_ref().and_then(|page| self.pages.iter().position(|id| id == page));
        let to = match at {
            Some(at) => (at as isize + step).rem_euclid(count as isize) as usize,
            None => 0,
        };
        if let Some(id) = self.pages.get(to) {
            self.work.selected = id.clone();
            self.page = Some(id.clone());
        }
        self.commit(cx);
    }

    /// Reads the state at the start of the body. The page clears the search and the error of a window that has just
    /// opened.
    fn begin(&mut self, cx: &mut impl StageAccess) {
        self.before = match self.state.reborrow().read(cx) {
            Some(Mixed::Same(held)) => held,
            _ => PreferencesState::default(),
        };
        self.work = self.before.clone();
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

    /// Picks the page `page` (Previous or Next) and writes the state, so the next pass shows it.
    fn step(&mut self, cx: &mut impl StageAccess, page: Option<String>) {
        if let Some(page) = page {
            self.work.selected = page;
            self.commit(cx);
        }
    }
}

/// The pages of `listed` before and after `shown`.
fn neighbors(listed: &[String], shown: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(at) = listed.iter().position(|id| Some(id.as_str()) == shown) else { return (None, None) };
    let before = at.checked_sub(1).and_then(|at| listed.get(at));
    (before.cloned(), listed.get(at + 1).cloned())
}

impl<'a> PreferencesWindow<'a> {
    /// A window over the host's `state`.
    pub fn new(state: Bind<'a, PreferencesState>) -> Self {
        PreferencesWindow {
            state,
            title: None,
            embedded: false,
            remember: None,
            mode: PreferencesMode::Staged,
            layout: PreferencesLayout::Paged,
            size: None,
            escape_keeps_typing: false,
            page_steps: false,
        }
    }

    /// Shows the window for this pass over `host`. The output's `closed` tells the host to stop showing it.
    pub fn show<H: PreferencesDraft + ?Sized>(self, ui: &mut Ui, host: &mut H) -> PreferencesOutput {
        let PreferencesWindow { state, title, embedded, remember, mode, layout, size: asked, escape_keeps_typing, page_steps } = self;
        let page_steps = page_steps && layout == PreferencesLayout::Paged;
        let env = Env::of(ui);
        let words = env.words::<PreferencesWords>();
        let common = env.words::<CommonWords>();

        let nested = host.nested_open();
        let can_ok = host.can_ok() && !nested;
        let differs = host.differs() && !nested;
        let title = title.unwrap_or_else(|| words.title.clone());
        let mut ok = FooterButton::new(common.ok.clone(), DialogRole::Accept).enabled(can_ok);
        if !can_ok {
            ok = ok.disabled_reason(words.cannot_ok.clone());
        }
        let classic = layout == PreferencesLayout::Classic;
        let cancel = FooterButton::new(common.cancel.clone(), DialogRole::Cancel);
        let buttons = if classic {
            // A desktop preferences dialog steps through its pages instead of applying in place.
            let step = |label: &Label| FooterButton::new(label.clone(), DialogRole::Other).enabled(!nested).stays_open(true);
            let mut buttons = vec![ok];
            if mode == PreferencesMode::Staged {
                buttons.push(cancel);
            }
            buttons.extend([step(&words.prev), step(&words.next)]);
            buttons
        } else if mode == PreferencesMode::Live {
            vec![ok]
        } else {
            vec![ok, FooterButton::new(words.apply.clone(), DialogRole::Other).enabled(differs).stays_open(true), cancel]
        };

        let (size, least) = match layout {
            PreferencesLayout::Paged => (DEFAULT_SIZE, MIN_SIZE),
            PreferencesLayout::Cards => (CARDS_SIZE, CARDS_MIN_SIZE),
            PreferencesLayout::Classic => (CLASSIC_SIZE, MIN_SIZE),
        };
        let size = asked.filter(|asked| asked.is_finite()).unwrap_or(size);
        let mut open = true;
        // Filed by a swatch press in this pass.
        let mut pick_color = None;
        let session = RefCell::new(Session {
            host: &mut *host,
            state,
            title: title.as_str().into(),
            before: PreferencesState::default(),
            work: PreferencesState::default(),
            page: None,
            steps: (None, None),
            pages: Vec::new(),
            applied: false,
        });
        let mut dialog = Dialog::new(title.clone())
            .collection_body()
            .resizable(true)
            .overlay(|overlay| overlay.size(size).min_size(least))
            .close_on_enter(can_ok)
            .escape_keeps_typing(escape_keeps_typing)
            .on_pick(|button, cx| {
                if button.role() != DialogRole::Other || nested {
                    return;
                }
                let mut session = session.borrow_mut();
                if !classic {
                    session.apply(cx);
                } else if button.label() == &words.prev {
                    session.step_page(cx, -1);
                } else if button.label() == &words.next {
                    session.step_page(cx, 1);
                }
            })
            .close_guard(|button, cx| mode == PreferencesMode::Live || button.role() != DialogRole::Accept || session.borrow_mut().apply(cx));
        if classic {
            dialog = dialog.button_place(ButtonPlace::Side);
        }
        if !embedded {
            dialog = dialog.viewport();
        }
        if let Some(key) = remember {
            dialog = dialog.remember(key);
        }
        let shown = dialog
            .footer(|ui, cx| {
                if page_steps {
                    let (before, after) = session.borrow().steps.clone();
                    let mut picked = None;
                    Row::new().show(ui, |ui, _| {
                        for (label, key, page) in [(&words.previous, "previous", &before), (&words.next, "next", &after)] {
                            let button = Button::new(label.clone()).enabled(page.is_some() && !nested).id_salt(key).show(ui);
                            if clicked(&button) {
                                picked.clone_from(page);
                            }
                        }
                    });
                    // The footer is measured in a sizing pass before it is shown; only the shown pass steps.
                    if !ui.is_sizing_pass() && picked.is_some() {
                        session.borrow_mut().step(cx.dialog(), picked);
                        request_repaint(ui.ctx());
                    }
                }
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
                let shown = PreferencesPage::new(Bind::bound(&mut session.work)).layout(layout).show(ui, &mut *session.host);
                if let Some(request) = shown.pick_color {
                    pick_color = Some(request);
                }
                session.host.nested(ui);
                if mode == PreferencesMode::Live && session.work.error.is_none() && session.host.differs() {
                    session.apply(cx);
                }
                if page_steps {
                    session.steps = neighbors(&shown.pages, shown.page.as_deref());
                }
                session.page = shown.page;
                session.pages = shown.pages;
                session.commit(cx);
            });

        let Session { host, page, applied, .. } = session.into_inner();
        let DialogOutput { response, closed, .. } = shown.output;
        let mut result = PreferencesOutput::new(response);
        result.page = page;
        result.applied = applied;
        result.pick_color = pick_color;
        match closed.filter(|_| !nested) {
            Some(CloseReason::Activated) => result.closed = Some(Closed::Ok),
            Some(_) => {
                if mode == PreferencesMode::Staged {
                    host.cancel();
                }
                result.closed = Some(Closed::Cancel);
            }
            None => {}
        }
        result
    }
}
