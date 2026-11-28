//! The word run's keys and picks.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{RunWord, WordRun, WordState};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const WORDS: [&str; 9] = ["The", "quick", "brown", "fox", "jumps", "over", "the", "lazy", "dog"];

/// A run of the words in a room they wrap in, bound to `picked`; `picks` counts the passes that report a pick, and
/// while `discard` is on each frame runs twice.
struct Run {
    fixture: Fixture,
    picked: Rc<RefCell<Option<usize>>>,
    picks: Rc<Cell<usize>>,
    discard: Rc<Cell<bool>>,
}

fn run(start: Option<usize>) -> Run {
    let picked = Rc::new(RefCell::new(start));
    let picks = Rc::new(Cell::new(0));
    let discard = Rc::new(Cell::new(false));
    let (bound, counted, asks) = (picked.clone(), picks.clone(), discard.clone());
    let mut fixture = Fixture::new(move |ui| {
        let words: Vec<RunWord<usize>> =
            WORDS.iter().enumerate().map(|(at, text)| RunWord { key: at, text: (*text).to_owned(), state: WordState::Normal }).collect();
        let mut value = *bound.borrow();
        let out = WordRun::new("Recognised text", &words, Bind::bound(&mut value)).show(ui);
        *bound.borrow_mut() = value;
        if out.picked.is_some() {
            counted.set(counted.get() + 1);
        }
        if asks.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs the frame twice");
        }
    });
    fixture.room(Vec2::new(140.0, 400.0));
    fixture.settle();
    Run { fixture, picked, picks, discard }
}

/// The word that holds focus, by its position in the run.
fn lead(fixture: &Fixture) -> Option<u64> {
    fixture.focused_stop().and_then(|(_, part)| part).and_then(|part| part.index)
}

/// Arrows move focus over the words in reading order across the run's lines, Home and End jump to the ends, and
/// Space, Enter and a click pick the focused word into the binding, one pick per press. A press in a frame that runs
/// twice picks once: the second pass gets none of the frame's input. Tab enters a run on its picked word, else on its
/// first.
#[trial]
fn keys_move_the_lead_and_pick_one_word() {
    let Run { mut fixture, picked, picks, discard } = run(None);
    let (first, last) = (fixture.by_label("The").expect("the first word"), fixture.by_label("dog").expect("the last word"));
    assert!(last.rect.min.y > first.rect.max.y - 0.5, "the words wrap onto more than one line");

    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(lead(&fixture), Some(0), "with nothing picked, Tab enters the run on its first word");
    for to in 1..WORDS.len() as u64 {
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        assert_eq!(lead(&fixture), Some(to), "each arrow moves focus to the next word, across a line break too");
    }
    assert_eq!((*picked.borrow(), picks.get()), (None, 0), "moving focus picks nothing");

    fixture.key(Key::Home, Modifiers::NONE);
    assert_eq!(lead(&fixture), Some(0), "Home goes to the first word");
    fixture.key(Key::ArrowRight, Modifiers::NONE).key(Key::ArrowRight, Modifiers::NONE);
    fixture.key(Key::Space, Modifiers::NONE);
    assert_eq!(*picked.borrow(), lead(&fixture).map(|at| at as usize), "Space picks the focused word");
    assert_eq!(picks.get(), 1, "one press is one pick");

    fixture.key(Key::End, Modifiers::NONE);
    assert_eq!(lead(&fixture), Some(WORDS.len() as u64 - 1), "End goes to the last word");
    fixture.key(Key::Enter, Modifiers::NONE);
    assert_eq!(*picked.borrow(), lead(&fixture).map(|at| at as usize), "Enter picks the focused word");
    assert_eq!(picks.get(), 2);

    let lazy = fixture.by_label("lazy").expect("a word on a later line");
    fixture.click(&lazy);
    assert_eq!(*picked.borrow(), Some(7), "a click picks the word under it");
    assert_eq!(picks.get(), 3);

    fixture.key(Key::ArrowLeft, Modifiers::NONE);
    let before = picks.get();
    discard.set(true);
    fixture.frame(vec![key_press(Key::Space, Modifiers::NONE)]);
    discard.set(false);
    assert_eq!(fixture.passes(), 2, "the trial's frame ran twice");
    fixture.frame(vec![key_release(Key::Space, Modifiers::NONE)]);
    assert_eq!(picks.get(), before + 1, "a press in a frame of two passes picks once");
    assert_eq!(*picked.borrow(), lead(&fixture).map(|at| at as usize), "and picks the focused word");

    let Run { fixture: mut entered, .. } = run(Some(5));
    entered.key(Key::Tab, Modifiers::NONE);
    assert_eq!(lead(&entered), Some(5), "Tab enters a run that has a picked word on that word");
}
