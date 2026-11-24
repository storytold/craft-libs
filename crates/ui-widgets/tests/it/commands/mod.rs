//! The command board: registry, state, the gate, firing and scoped delivery.

pub(crate) mod panel_scope;

use craft_behavior::IconId;
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{CommandSet, Props as _, Show as _};
use craft_ui::runtime::commands::CommandState;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Ui};
use std::cell::Cell;
use std::rc::Rc;

/// App-scope commands whose state carries a hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "hint")]
#[non_exhaustive]
pub enum HintCommand {
    #[command(label = "Save", default = "Cmd+Shift+F9")]
    Save,
    #[command(label = "Open", default = "Cmd+Shift+F10")]
    Open,
}

/// The text the pointer's rest on what `show` shows (it returns its rect) adds after the tooltip delay, its runs
/// joined by line, with `command` given `state`.
fn tooltip_of(command: HintCommand, state: CommandState, show: impl Fn(&mut Ui) -> Rect + 'static) -> String {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| seen.set(show(ui)));
    fixture.command_state(command, state);
    fixture.settle();
    let before: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    fixture.frame(vec![moved(at.get().center())]);
    fixture.advance(DELAY);
    fixture.settle();
    let added: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).filter(|text| !before.contains(text)).collect();
    added.join("\n")
}

/// A bound command's hint is the tooltip's second line, under the label and its chord, enabled or not, also when the
/// label shows whole; with `hint_alone` (L-72, opt-in) and no other tooltip text, the hint is the tooltip, with the
/// chord after it.
#[trial]
fn a_bound_commands_hint_is_the_tooltips_second_line() {
    const NEEDS: &str = "Needs an open document";
    for enabled in [true, false] {
        let text = tooltip_of(HintCommand::Save, CommandState::new().enabled(enabled).hint(NEEDS), |ui| {
            IconButton::new(IconId::new("play")).label("Save").command(HintCommand::Save).show(ui).rect
        });
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "enabled {enabled}: {text:?}");
        assert!(lines[0].starts_with("Save (") && lines[0].ends_with(')'), "enabled {enabled}: the label and its chord first, {text:?}");
        assert_eq!(lines[1], NEEDS, "enabled {enabled}: the hint after the label");
    }
    const OPENS: &str = "Opens a document from disk";
    let text = tooltip_of(HintCommand::Open, CommandState::new().hint(OPENS), |ui| Button::new("Open").command(HintCommand::Open).show(ui).rect);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "by default the label and chord, then the hint: {text:?}");
    assert!(lines[0].starts_with("Open (") && lines[0].ends_with(')') && lines[1] == OPENS, "the label and chord first: {text:?}");
    let text = tooltip_of(HintCommand::Open, CommandState::new().hint(OPENS).hint_alone(true), |ui| {
        Button::new("Open").command(HintCommand::Open).show(ui).rect
    });
    assert!(text.starts_with(&format!("{OPENS} (")) && text.ends_with(')') && !text.contains('\n'), "the hint alone, with the chord: {text:?}");
}
