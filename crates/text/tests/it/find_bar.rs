//! The find bar over a test-held query: asking for the bar again selects its query, so typing replaces it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_text::find::{FindMatches, FindOptions};
use craft_text::ui::find::{FindBar, FindBarState, FindStep};
use craft_ui::runtime::commands::CommandState;
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::frame::take_output;
use craft_ui::shell::AppShell;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, ImeEvent, Key, Modifiers};

struct Bar {
    fixture: Fixture,
    query: Rc<RefCell<String>>,
    state: Rc<RefCell<FindBarState>>,
    /// The next frame runs a second pass.
    discard: Rc<Cell<bool>>,
}

impl Bar {
    fn new() -> Self {
        let query = Rc::new(RefCell::new(String::new()));
        let state = Rc::new(RefCell::new(FindBarState::default()));
        let discard = Rc::new(Cell::new(false));
        let (shown_query, shown_state, shown_discard) = (Rc::clone(&query), Rc::clone(&state), Rc::clone(&discard));
        let mut options = FindOptions::default();
        let mut matches = FindMatches::new(0);
        let fixture = Fixture::new(move |ui| {
            if shown_discard.take() {
                ui.ctx().request_discard("the trial reruns the pass");
            }
            FindBar::new(&mut shown_query.borrow_mut(), &mut options, &mut matches).state(&mut shown_state.borrow_mut()).show(ui);
        });
        Bar { fixture, query, state, discard }
    }

    /// Cmd+F: the app asks for the bar.
    fn ask(&mut self) {
        self.ask_in(false);
    }

    /// Cmd+F, in a frame that runs a second pass when `rerun`.
    fn ask_in(&mut self, rerun: bool) {
        self.discard.set(rerun);
        self.state.borrow_mut().open();
        self.fixture.frame(Vec::new()).settle();
        assert!(!self.state.borrow().is_opening(), "the query field took the focus request");
    }

    fn query(&self) -> String {
        self.query.borrow().clone()
    }
}

/// An unchanged query must not request another search and discard a user's selected match.
#[trial]
fn settling_a_typed_query_does_not_request_the_same_search_again() {
    let query = Rc::new(RefCell::new(String::new()));
    let replacement = Rc::new(RefCell::new(String::new()));
    let searches = Rc::new(Cell::new(0));
    let (shown_query, shown_replacement, shown_searches) = (Rc::clone(&query), Rc::clone(&replacement), Rc::clone(&searches));
    let mut options = FindOptions::default();
    let mut matches = FindMatches::new(0);
    let mut state = FindBarState::default();
    state.open_replace();
    let mut open = true;
    let mut fixture = Fixture::new(move |ui| {
        let before = shown_query.borrow().clone();
        let previous_options = options;
        let out = FindBar::new(&mut shown_query.borrow_mut(), &mut options, &mut matches)
            .state(&mut state)
            .replace(&mut shown_replacement.borrow_mut(), &mut open)
            .show(ui);
        if out.search && *shown_query.borrow() == before && options == previous_options {
            shown_searches.set(shown_searches.get() + 1);
        }
    });
    fixture.frame(Vec::new()).settle();
    fixture.type_text("cat");
    *replacement.borrow_mut() = "dog".into();
    fixture.key(Key::Tab, Modifiers::NONE);
    for _ in 0..8 {
        fixture.frame(Vec::new());
    }
    assert_eq!(&*query.borrow(), "cat");
    assert_eq!(searches.get(), 0, "the unchanged query did not ask for another search");
}

#[trial]
fn asking_for_the_bar_again_selects_its_query_unless_a_composition_runs() {
    let mut bar = Bar::new();
    bar.ask();
    bar.fixture.type_text("cat").settle();
    assert_eq!(bar.query(), "cat");

    bar.ask();
    bar.fixture.type_text("dog").settle();
    assert_eq!(bar.query(), "dog", "with the field holding the keyboard, typing replaces the selected query");

    // The first pass serves the request; the rerun pass asks nothing and keeps the selection it applies.
    bar.ask_in(true);
    bar.fixture.type_text("owl").settle();
    assert_eq!(bar.query(), "owl", "a frame that runs twice still selects the query once");

    let field = bar.fixture.focused();
    bar.fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert_ne!(bar.fixture.focused(), field, "the field lost the keyboard to the next stop");
    bar.ask();
    bar.fixture.type_text("cat").settle();
    assert_eq!(bar.query(), "cat", "the focus-gain caret does not undo the select");

    bar.fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: "ka".to_owned(), active_range_chars: Some(0..2) })]).settle();
    // Selecting the query here would make the commit replace all of it.
    bar.ask();
    bar.fixture.frame(vec![Event::Ime(ImeEvent::Commit("か".to_owned()))]).settle();
    assert_eq!(bar.query(), "catか", "a request during a composition is dropped");

    bar.ask();
    bar.fixture.type_text("dog").settle();
    assert_eq!(bar.query(), "dog", "after the commit the next request selects the query again");
}

// Production's find field is a plain integrated one: no clear button beside the text, and Escape still clears it.
#[trial]
fn the_find_field_has_no_clear_button_and_escape_clears_it() {
    let mut bar = Bar::new();
    bar.ask();
    bar.fixture.type_text("cat");
    bar.fixture.frame(Vec::new()).settle();
    assert_eq!(bar.query(), "cat");
    assert!(bar.fixture.by_label("Clear").is_none(), "no clear button beside the text");
    bar.fixture.key(Key::Escape, Modifiers::NONE);
    bar.fixture.frame(Vec::new()).settle();
    assert_eq!(bar.query(), "", "Escape clears the query");
}

/// Previous and Next are pressed with the pointer, not only by their keys: each press steps the current match.
#[trial]
fn pressing_next_and_previous_steps_the_current_match() {
    let matches = Rc::new(RefCell::new(FindMatches::new(0)));
    matches.borrow_mut().set_page(0, vec![0..3, 10..13, 20..23]);
    let stepped = Rc::new(Cell::new(0));
    let (shown_matches, shown_stepped) = (Rc::clone(&matches), Rc::clone(&stepped));
    let mut query = String::from("cat");
    let mut options = FindOptions::default();
    // In an app's shell, as an app shows the bar: the shell's body is where app commands are delivered. Before each
    // frame the app takes the frame's output, with the app commands no body took (`embed::before_input`): a command
    // the bar leaves for a later pass is the app's by then.
    let mut fixture = Fixture::new(move |ui| {
        if ui.ctx().current_pass_index() == 0 {
            let _ = take_output(ui.ctx());
        }
        AppShell::new(WindowKey::main())
            .body(|ui, _, _| {
                if FindBar::new(&mut query, &mut options, &mut shown_matches.borrow_mut()).show(ui).stepped {
                    shown_stepped.set(shown_stepped.get() + 1);
                }
            })
            .show(ui, &mut ());
    });
    // The app enables its find commands while a document is open.
    fixture.command_state(FindStep::Next, CommandState::new()).command_state(FindStep::Previous, CommandState::new());
    fixture.frame(Vec::new()).settle();
    let start = matches.borrow().current();

    let next = fixture.by_label("Find Next").expect("the Next button");
    fixture.click(&next).idle(2);
    let after_next = matches.borrow().current();
    assert_ne!(after_next, start, "pressing Next moved the current match");
    assert_eq!(stepped.get(), 1, "the bar reported the step");

    let previous = fixture.by_label("Find Previous").expect("the Previous button");
    fixture.click(&previous).idle(2);
    assert_eq!(matches.borrow().current(), start, "pressing Previous went back to the match before");
    assert_eq!(stepped.get(), 2, "the bar reported the step");
}
