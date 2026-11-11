//! The find bar: Acrobat's Cmd+F bar over a document.

use craft_behavior::IconId;
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, BodyCx, Commands as _, Entry, Outcome as _, Props as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft_ui::display::Text;
use craft_ui::layout_components::Row;
use craft_ui::menu::Menu;
use craft_ui::runtime::access::PopupKind;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::text::TextField;
use egui::{Response, Ui};

use super::FindLabels;
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
/// it, and again when it is asked for with the bar open). The bar clears it once the query field holds focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindBarState {
    opened: bool,
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
#[derive(craft_ui::contract::Pattern)]
pub struct FindBar<'a> {
    query: &'a mut String,
    options: &'a mut FindOptions,
    matches: &'a mut FindMatches,
    progress: Option<(usize, usize)>,
    state: Option<&'a mut FindBarState>,
}

impl<'a> FindBar<'a> {
    /// A find bar over the app's `query` and `options`, listing `matches`.
    pub fn new(query: &'a mut String, options: &'a mut FindOptions, matches: &'a mut FindMatches) -> Self {
        FindBar { query, options, matches, progress: None, state: None }
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
        let FindBar { query, options, matches, progress, state } = self;
        let words = Env::of(ui).words::<FindLabels>();
        let mut state = state;
        let opened = state.as_deref().is_some_and(|state| state.opened);
        let mut out = FindBarOutput { search: false, stepped: false, closed: false, response: ui.response() };
        let mut step: Option<bool> = None;
        let row = Row::new().show(ui, |ui, cx: &mut BodyCx<'_>| {
            while let Some((command, _)) = cx.take_command::<FindStep>() {
                step = Some(command == FindStep::Previous);
            }
            let edit = TextField::search(words.find_text.clone(), Bind::given(query.clone())).placeholder(words.find_text.clone()).show(ui);
            if opened {
                focus::request(ui.ctx(), FocusTarget::Id(edit.response.id));
                // Asking again each run until the field holds focus keeps the request across the measuring run.
                if edit.response.has_focus()
                    && let Some(state) = state.as_deref_mut()
                {
                    state.opened = false;
                }
            }
            if let Some(value) = edit.events.value() {
                query.clone_from(value);
                out.search = true;
            }
            if edit.submitted {
                step = Some(edit.submit_mods.shift);
            }
            // The field takes the Esc that clears it; an Esc it passes on closes the bar.
            if cx.take_cancel() {
                out.closed = true;
            }
            let text = count_text(&words, query.trim().is_empty(), matches, progress);
            if !text.is_empty() {
                Text::new(text).show(ui);
            }
            if clicked(&IconButton::command(IconId::new("chevron_up"), FindStep::Previous).show(ui)) {
                step = Some(true);
            }
            if clicked(&IconButton::command(IconId::new("chevron_down"), FindStep::Next).show(ui)) {
                step = Some(false);
            }
            let active = options.match_case || options.whole_word;
            let button = IconButton::new(IconId::new("settings")).label(words.options.clone()).popup(PopupKind::Menu).selected(active).show(ui);
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
        out.response = row.output.response().clone();
        if let Some(back) = step {
            out.stepped = matches.step(back).is_some();
        }
        out
    }
}

/// The bar's count: nothing for an empty query, "Searching… s/p" while pages are being read and nothing is found yet,
/// "No matches", else "c of n" with a "+" while pages are still being read or a page hit its match limit.
fn count_text(words: &FindLabels, empty_query: bool, matches: &FindMatches, progress: Option<(usize, usize)>) -> String {
    if empty_query {
        return String::new();
    }
    let reading = progress.filter(|(searched, pages)| searched < pages);
    if matches.is_empty() {
        return match reading {
            Some((searched, pages)) => words.searching.fill(&[("s", &searched.to_string()), ("p", &pages.to_string())]),
            None => words.no_matches.as_str().to_owned(),
        };
    }
    let current = matches.current().map_or(0, |c| c + 1);
    let more = if reading.is_some() || matches.limit_reached() { "+" } else { "" };
    {
        let (current, total) = (current.to_string(), matches.len().to_string());
        words.count.fill(&[("c", current.as_str()), ("n", total.as_str()), ("more", more)])
    }
}
