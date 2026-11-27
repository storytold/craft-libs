//! The find bar: Acrobat's Cmd+F bar over a document.

use craft_behavior::IconId;
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, BodyCx, Commands as _, Entry, Outcome as _, Props as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::Menu;
use craft_ui::runtime::access::PopupKind;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::text::TextField;
use egui::{Response, Ui};

use super::{FindLabels, FindRequest};
use crate::find::{FindMatches, FindOptions};

/// Step through the matches. The bar reads them while it is shown; the chords are the keymap's to rebind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "find")]
#[non_exhaustive]
pub enum FindStep {
    /// Go to the next match.
    #[command(label = "Find Next", default = "Cmd+G", default = "F3")]
    Next,
    /// Go to the previous match.
    #[command(label = "Find Previous", default = "Cmd+Shift+G", default = "Shift+F3")]
    Previous,
}

/// What the bar did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct FindBarOutput {
    /// The query or an option changed: search again ([`FindMatches::new`] from the current page).
    pub search: bool,
    /// Enter, Shift+Enter, the arrows or the [`FindStep`] chords: step through the matches (the bar already called
    /// [`FindMatches::step`]); the app shows the current match.
    pub stepped: bool,
    /// Esc in the field or the close button.
    pub closed: bool,
    /// Replace or Replace All in the replace row; the app runs it.
    pub request: Option<FindRequest>,
    /// The bar's row.
    pub response: Response,
}

impl craft_ui::contract::Outcome for FindBarOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Whether the bar has just been asked for, kept by the app beside its query ([`open`](Self::open) when Cmd+F opens
/// it, and again when it is asked for with the bar open). The bar clears it once the asked-for field holds focus, and
/// that field selects its text, so typing replaces it, whether it still held the keyboard or had lost it. A request
/// made while the field is composing text is dropped, so the composition's commit does not replace the query. In a
/// frame that runs twice, the pass where the field holds focus asks for the select and clears the flag; the select
/// applies at the start of the next pass, which asks nothing more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindBarState {
    opened: bool,
    /// The replace row was asked for with the bar: it opens, and its field takes the focus while the query has text.
    replace: bool,
}

impl FindBarState {
    /// The bar is being opened, or asked for again while open.
    pub fn open(&mut self) {
        self.opened = true;
    }

    /// Whether the bar was asked for and its query field has not taken focus yet.
    pub fn is_opening(&self) -> bool {
        self.opened
    }

    /// Whether the bar was asked for and not yet shown, clearing it, for a caller that shows no [`FindBar`].
    #[must_use]
    pub fn taken(&mut self) -> bool {
        self.replace = false;
        std::mem::take(&mut self.opened)
    }

    /// The bar is being opened with its replace row, and focus goes to the Replace field when the query is not empty.
    pub fn open_replace(&mut self) {
        self.opened = true;
        self.replace = true;
    }

    /// The asked-for field holds focus: the request is done.
    fn served(&mut self) {
        self.opened = false;
        self.replace = false;
    }
}

/// A checked item of the bar's options menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarOption {
    WholeWords,
    MatchCase,
}

/// The find bar, built per pass while open, over the app's query, options and matches.
///
/// - The field is a search `TextField` (placeholder "Find text"); Enter steps to the next match and keeps the
///   keyboard, Shift+Enter the previous; Esc clears the field and a second Esc closes the bar.
/// - The count reads "Searching… {s}/{p}" while pages are still being read, "No matches", or "{c} of {n}" (with "+"
///   in `{more}` when a page hit its match limit).
/// - Previous and Next are `IconButton`s bound to the [`FindStep`] commands, so their tooltips show the chords; the
///   options button opens a `Menu` of the options (check rows) and shows pressed while an option is on; changing an
///   option searches again.
/// - With [`replace`](Self::replace), a toggle before the options button shows a second row: the Replace with field
///   (Enter presses Replace), Replace (while a match is current) and Replace All (while the query has text). Either
///   reports a [`FindRequest`] in [`FindBarOutput::request`].
#[derive(craft_ui::contract::Pattern)]
pub struct FindBar<'a> {
    query: &'a mut String,
    options: &'a mut FindOptions,
    matches: &'a mut FindMatches,
    progress: Option<(usize, usize)>,
    state: Option<&'a mut FindBarState>,
    replace: Option<(&'a mut String, &'a mut bool)>,
}

impl<'a> FindBar<'a> {
    /// A find bar over the app's `query` and `options`, listing `matches`.
    pub fn new(query: &'a mut String, options: &'a mut FindOptions, matches: &'a mut FindMatches) -> Self {
        FindBar { query, options, matches, progress: None, state: None, replace: None }
    }

    /// A replace row under the find field over the app's `with` text, shown while `open`.
    #[must_use]
    pub fn replace(mut self, with: &'a mut String, open: &'a mut bool) -> Self {
        self.replace = Some((with, open));
        self
    }

    /// The app's [`FindBarState`].
    #[must_use]
    pub fn state(mut self, state: &'a mut FindBarState) -> Self {
        self.state = Some(state);
        self
    }

    /// Pages searched so far and the page count, while the search runs.
    #[must_use]
    pub fn progress(mut self, searched: usize, pages: usize) -> Self {
        self.progress = Some((searched, pages));
        self
    }

    /// Paints the bar and returns what the user asked for.
    pub fn show(self, ui: &mut Ui) -> FindBarOutput {
        let FindBar { query, options, matches, progress, mut state, replace } = self;
        let (mut with, mut open) = match replace {
            Some((with, open)) => (Some(with), Some(open)),
            None => (None, None),
        };
        let env = Env::of(ui);
        let (words, units) = (env.words::<FindLabels>(), env.units());
        let opened = state.as_deref().is_some_and(|state| state.opened);
        let replace_asked = opened && open.is_some() && state.as_deref().is_some_and(|state| state.replace);
        if replace_asked && let Some(open) = open.as_deref_mut() {
            *open = true;
        }
        // The focus request goes to the Replace field when there is a query to replace, else to the query field.
        let to_with = replace_asked && !query.is_empty();
        let to_query = opened && !to_with;
        let mut out = FindBarOutput { search: false, stepped: false, closed: false, request: None, response: ui.response() };
        let mut step: Option<bool> = None;
        let bar = Stack::new().show(ui, |ui, cx: &mut BodyCx<'_>| {
            while let Some((command, _)) = cx.take_command::<FindStep>() {
                step = Some(command == FindStep::Previous);
            }
            Row::new().show(ui, |ui, _| {
                let edit = TextField::search(words.find_text.clone(), Bind::given(query.clone()))
                    .clear_button(false)
                    .placeholder(words.find_text.clone())
                    .select_all(to_query)
                    .show(ui);
                if to_query {
                    focus::request(ui.ctx(), FocusTarget::Id(edit.response.id));
                    // Asking again each run until the field holds focus keeps the request across the measuring run.
                    // The field's focus sits on its text part, so the egui response of the field never reports it.
                    if focus::is_focused(ui.ctx(), edit.response.id)
                        && let Some(state) = state.as_deref_mut()
                    {
                        state.served();
                    }
                }
                if let Some(value) = edit.events.value()
                    && value != &*query
                {
                    query.clone_from(value);
                    out.search = true;
                }
                if edit.submitted {
                    step = Some(edit.submit_mods.shift);
                }
                let text = count_text(&words, &units, query.trim().is_empty(), matches, progress);
                if !text.is_empty() {
                    Text::new(text).show(ui);
                }
                if clicked(&IconButton::command(IconId::new("chevron_up"), FindStep::Previous).show(ui)) {
                    step = Some(true);
                }
                if clicked(&IconButton::command(IconId::new("chevron_down"), FindStep::Next).show(ui)) {
                    step = Some(false);
                }
                if let Some(open) = open.as_deref_mut()
                    && clicked(&IconButton::new(IconId::new("swap_horizontal")).label(words.show_replace.clone()).selected(*open).show(ui))
                {
                    *open = !*open;
                }
                let active = options.match_case || options.whole_word;
                let button =
                    IconButton::new(IconId::new("adjustments")).label(words.options.clone()).popup(PopupKind::Menu).selected(active).show(ui);
                let entries = [
                    Entry::new(BarOption::WholeWords, words.whole_words_only.clone()).checked(options.whole_word),
                    Entry::new(BarOption::MatchCase, words.case_sensitive.clone()).checked(options.match_case),
                ];
                match Menu::new(words.options.clone()).opened_by(&button, Trigger::Press).pick(ui, &entries).picked {
                    Some(BarOption::WholeWords) => {
                        options.whole_word = !options.whole_word;
                        out.search = true;
                    }
                    Some(BarOption::MatchCase) => {
                        options.match_case = !options.match_case;
                        out.search = true;
                    }
                    None => {}
                }
                if clicked(&IconButton::new(IconId::new("close")).label(words.close.clone()).show(ui)) {
                    out.closed = true;
                }
            });
            if let (Some(with), Some(true)) = (with.as_deref_mut(), open.as_deref().copied()) {
                Row::new().show(ui, |ui, _| {
                    let edit = TextField::new(words.replace_with.clone(), Bind::given(with.clone())).submits().select_all(to_with).show(ui);
                    if to_with {
                        focus::request(ui.ctx(), FocusTarget::Id(edit.response.id));
                        if focus::is_focused(ui.ctx(), edit.response.id)
                            && let Some(state) = state.as_deref_mut()
                        {
                            state.served();
                        }
                    }
                    if let Some(value) = edit.events.value() {
                        with.clone_from(value);
                    }
                    let current = matches.current().is_some();
                    let pressed = clicked(&Button::new(words.replace.clone()).enabled(current).show(ui));
                    if pressed || (edit.submitted && current) {
                        out.request = Some(FindRequest::replace(query.clone(), with.clone(), *options));
                    }
                    if clicked(&Button::new(words.replace_all.clone()).enabled(!query.is_empty()).show(ui)) {
                        out.request = Some(FindRequest::replace_all(query.clone(), with.clone(), *options));
                    }
                });
            }
            // The query field takes the Esc that clears it; an Esc a field passes on closes the bar.
            if cx.take_cancel() {
                out.closed = true;
            }
        });
        out.response = bar.output.response().clone();
        if let Some(back) = step {
            out.stepped = matches.step(back).is_some();
        }
        out
    }
}

/// The bar's count: nothing for an empty query, "Searching… s/p" while pages are being read and nothing is found yet,
/// "No matches", else "c of n" with a "+" while pages are still being read or a page hit its match limit. The numbers
/// are in the user's digits and groups.
fn count_text(words: &FindLabels, units: &Units, empty_query: bool, matches: &FindMatches, progress: Option<(usize, usize)>) -> String {
    if empty_query {
        return String::new();
    }
    let number = |n: usize| units.number(n as f64, 0..=0);
    let reading = progress.filter(|(searched, pages)| searched < pages);
    if matches.is_empty() {
        return match reading {
            Some((searched, pages)) => words.searching.fill(&[("s", &number(searched)), ("p", &number(pages))]),
            None => words.no_matches.as_str().to_owned(),
        };
    }
    let current = matches.current().map_or(0, |c| c + 1);
    let more = if reading.is_some() || matches.limit_reached() { "+" } else { "" };
    words.count.fill(&[("c", &number(current)), ("n", &number(matches.len())), ("more", more)])
}
