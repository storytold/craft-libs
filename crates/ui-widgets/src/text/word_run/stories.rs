//! Stories of `WordRun`.

use super::{RunWord, WordRun, WordState};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

/// A recognised line of a scanned page.
const LINE: &str = "The quick brown fox jumps over the lazy dog";

/// A width the line's words wrap in, with the longest word wider than a line.
const NARROW: f32 = 120.0;

/// The words of `text`, keyed by position, each in the state `state` gives it.
fn words(text: &str, state: impl Fn(usize) -> WordState) -> Vec<RunWord<usize>> {
    text.split_whitespace().enumerate().map(|(at, word)| RunWord { key: at, text: word.to_owned(), state: state(at) }).collect()
}

fn pick_quick(fixture: &mut Fixture) {
    if let Some(word) = fixture.by_label("quick") {
        fixture.click(&word);
    }
}

#[story(WordRun<'static, usize>, covers = [Default], drive = Drive::Custom(pick_quick))]
fn word_run(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let line = words(LINE, |_| WordState::Normal);
    WordRun::new("Recognised text", &line, Bind::bound(data.value("picked", None::<usize>))).show(ui).into()
}

/// A word the reader struck as wrong and one it marked for a check, beside plain words.
#[story(WordRun<'static, usize>, covers = [], drive = Drive::Never)]
fn word_run_states(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let line = words(LINE, |at| match at {
        2 => WordState::Struck,
        5 => WordState::Underlined,
        _ => WordState::Normal,
    });
    WordRun::new("Recognised text", &line, Bind::bound(data.value("picked", None::<usize>))).show(ui).into()
}

#[story(WordRun<'static, usize>, covers = [], drive = Drive::Never)]
fn word_run_selected(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let line = words(LINE, |at| if at == 3 { WordState::Underlined } else { WordState::Normal });
    WordRun::new("Recognised text", &line, Bind::bound(data.value("picked", Some(3usize)))).show(ui).into()
}

/// The words wrap over several lines, and a word wider than a line is cut to it with its whole text as its tooltip.
#[story(WordRun<'static, usize>, covers = [LongText], drive = Drive::Never)]
fn word_run_long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ui.set_max_width(NARROW.min(ui.available_width()));
    let line = words("Notwithstanding the aforementioned considerations, counterrevolutionaries hesitated", |_| WordState::Normal);
    WordRun::new("Recognised text", &line, Bind::bound(data.value("picked", None::<usize>))).show(ui).into()
}

#[story(WordRun<'static, usize>, covers = [Invalid], drive = Drive::Never)]
fn word_run_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let line = words(LINE, |_| WordState::Normal);
    let refuse_dog =
        |picked: &Option<usize>| if *picked == Some(8) { Validity::Refuse(Message::new("Pick a recognised word")) } else { Validity::Ok };
    WordRun::new("Recognised text", &line, Bind::bound(data.value("refused", Some(8usize))).check(refuse_dog)).show(ui).into()
}
