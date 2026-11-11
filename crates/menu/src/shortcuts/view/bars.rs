//! The bars under the table: the question being asked, the conflict bar, the message after an action, and the
//! counts and problems of the keymap as a whole.

use craft_keymap::keymap::Problem;
use craft_theme::variant::Tone;
use craft_ui::contract::{BodyCx, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::{Notice, NoticeAction, NoticeLayout};
use craft_ui::runtime::access::Politeness;
use craft_ui::sections::Section;
use egui::Ui;

use crate::shortcuts::rows::Filter;
use crate::shortcuts::state::{Answer, DialogState, Env, Prompt, binding_error};
use crate::shortcuts::words::{ShortcutWords, around_chord};

/// The bars for one pass.
pub(super) fn show(ui: &mut Ui, _cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    prompt(ui, env, state);
    conflict_bar(ui, env, state);
    message(ui, env, state);
    info(ui, env, state);
}

/// The question being asked, with its answers: Save, Don't Save and Cancel, or the confirmation and Cancel.
fn prompt(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) {
    let Some(p) = state.prompt.clone() else { return };
    let words = env.text.words();
    let (text, yes, no) = match &p {
        Prompt::ResetAll => (words.restore_every_default_shortcut_in_this.as_str().to_owned(), words.reset_all.clone(), None),
        Prompt::DeleteSet(set) => (words.delete_the_set_set.fill(&[("set", set)]), words.delete.clone(), None),
        Prompt::SaveChanges { set, .. } => (words.save_changes_to_set.fill(&[("set", set)]), words.save.clone(), Some(words.dont_save.clone())),
    };
    let mut notice = Notice::new(Tone::Info, text).id_salt("shortcuts.prompt").layout(NoticeLayout::Inline).announce(Politeness::Polite);
    notice = notice.action(NoticeAction::new(yes).default_action());
    if let Some(no) = no.clone() {
        notice = notice.action(NoticeAction::new(no));
    }
    notice = notice.action(NoticeAction::new(words.cancel.clone()));
    let answer = notice.show(ui).action.map(|at| match (at, no.is_some()) {
        (0, _) => Answer::Yes,
        (1, true) => Answer::No,
        _ => Answer::Cancel,
    });
    if let Some(answer) = answer {
        state.answer(env, answer);
    }
}

fn conflict_bar(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) {
    let Some(conflict) = state.conflict.clone() else { return };
    let words = env.text.words();
    let chord = env.text.trigger(&conflict.binding.trigger, env.keymap.platform());
    let text = around_chord(env.text.rtl(), &conflict.text, &chord);
    let mut notice = Notice::new(Tone::Warn, text)
        .id_salt("shortcuts.conflict")
        .layout(NoticeLayout::Inline)
        .announce(Politeness::Polite)
        .action(NoticeAction::new(words.take_it.clone()))
        .action(NoticeAction::new(words.accept_and_go_to_conflict.clone()));
    let keep = env.options.allow_keep;
    if keep {
        notice = notice.action(NoticeAction::new(words.keep_both.clone()));
    }
    // Photoshop's wording for turning the new shortcut down is "Undo Changes", the notice's last action.
    let undo_at = if keep { 3 } else { 2 };
    notice = notice.action(NoticeAction::new(words.undo_changes.clone()));
    let out = notice.show(ui);
    match out.action {
        Some(0) => state.take_it(env),
        Some(1) => state.take_it_and_go(env),
        Some(2) if keep => state.keep_both(env),
        Some(at) if at == undo_at => state.dismiss_conflict(),
        _ => {}
    }
}

fn message(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) {
    let Some(m) = state.message.clone() else { return };
    let words = env.text.words();
    let text = around_chord(env.text.rtl(), &m.text, m.chord.as_deref().unwrap_or_default());
    let tone = if m.refused { Tone::Warn } else { Tone::Info };
    let mut notice = Notice::new(tone, text).id_salt("shortcuts.message").layout(NoticeLayout::Inline).dismiss().announce(Politeness::Polite);
    let goto = m.goto.clone();
    let undo = m.undo && state.can_undo() && !state.modal();
    if goto.is_some() {
        notice = notice.action(NoticeAction::new(words.go_to_conflict.clone()));
    }
    if undo {
        notice = notice.action(NoticeAction::new(words.undo.clone()));
    }
    let out = notice.show(ui);
    match (out.action, &goto) {
        (Some(0), Some(command)) => state.go_to(command),
        (Some(at), Some(_)) if undo && at == 1 => state.undo(env),
        (Some(0), None) if undo => state.undo(env),
        _ => {}
    }
    if out.dismissed {
        state.message = None;
    }
}

/// Clashes, shadows and problems in the keymap as a whole.
fn info(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) {
    let (clashes, shadows) = {
        let built = state.lines(env);
        (built.clashes, built.shadows)
    };
    let problems = env.keymap.problems();
    if clashes == 0 && shadows == 0 && problems.is_empty() {
        return;
    }
    let words = env.text.words();
    let mut conflicts = false;
    Row::new().show(ui, |ui, _| {
        if clashes > 0 {
            let n = clashes as u64;
            let text = words.count_clashes.text(n, &n.to_string());
            conflicts = super::warn_link(ui, text, "clashes", 0);
        }
        if shadows > 0 {
            let n = shadows as u64;
            Text::new(words.count_other_scope.text(n, &n.to_string())).shade(Shade::Muted).show(ui);
        }
    });
    if conflicts {
        state.set_filter(Filter::Conflicts);
    }
    if !problems.is_empty() {
        let n = problems.len() as u64;
        let title = words.count_unusable.text(n, &n.to_string());
        Section::new(title).id_salt("problems").initially_open(false).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for problem in &problems {
                    Text::new(problem_text(env, words, problem)).show(ui);
                }
            });
        });
    }
}

fn problem_text(env: &Env<'_>, words: &ShortcutWords, problem: &Problem) -> String {
    match problem {
        Problem::UnknownCommand { key } => words.key_no_command_has_this_name.fill(&[("key", key)]),
        Problem::BadBinding { key, text, error } => {
            let error = binding_error(env, error);
            words.key_text_is_not_a_shortcut.fill(&[("key", key), ("text", text), ("error", &error)])
        }
        Problem::Reserved { key, text } => words.key_text_is_reserved_by_the.fill(&[("key", key), ("text", text)]),
        Problem::UnknownScope { key, text } => words.key_the_scope_in_text_is.fill(&[("key", key), ("text", text)]),
        Problem::DuplicateKey { key } => words.key_listed_more_than_once.fill(&[("key", key)]),
        Problem::Dropped { key, .. } => words.key_part_of_the_entry_was.fill(&[("key", key)]),
        Problem::TooManyCommands { ignored } => {
            let n = *ignored as u64;
            words.count_ignored.text(n, &n.to_string())
        }
        _ => String::new(),
    }
}
