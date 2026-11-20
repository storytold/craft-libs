//! Stories of `Keytips` over a small recorded region: two tabs and the controls of one, shown with F10 and the keys
//! typed after it.

use std::sync::Arc;

use craft_theme::ControlSize;
use craft_ui::buttons::Button;
use craft_ui::contract::{CommandSet, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::inspect::{self, Recorded};
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Id, Key, Modifiers, Ui};

use super::{KeytipControl, Keytips};

/// The id of the tab button named `name`.
fn tab_id(name: &str) -> Id {
    Id::NULL.with(("story.keytips.tab", name))
}

/// The commands of the story's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "keytipsStory")]
#[non_exhaustive]
enum StoryCommand {
    #[command(label = "Paste", modal = "always")]
    Paste,
    #[command(label = "Bold", modal = "always")]
    Bold,
    #[command(label = "Italic", modal = "always")]
    Italic,
    #[command(label = "Format Painter", modal = "always")]
    FormatPainter,
    #[command(label = "Find", modal = "always")]
    Find,
}

/// The controls in the order they show, with the label their badge letters come from.
const CONTROLS: [(StoryCommand, &str); 5] = [
    (StoryCommand::Paste, "Paste"),
    (StoryCommand::Bold, "Bold"),
    (StoryCommand::Italic, "Italic"),
    (StoryCommand::FormatPainter, "Format Painter"),
    (StoryCommand::Find, "Find"),
];

/// The tab row and the control row, recorded as one inspected region; its record, in a real run.
fn region(ui: &mut Ui, size: ControlSize) -> Option<Arc<Recorded>> {
    let (_, recorded) = inspect::record(ui, Id::NULL.with("story.keytips"), |ui| {
        Stack::new().show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Button::new("Home").id(tab_id("Home")).show(ui);
                Button::new("Insert").id(tab_id("Insert")).show(ui);
            });
            Row::new().show(ui, |ui, _| {
                for (command, _) in CONTROLS {
                    Button::command(command).size(size).show(ui);
                }
            });
        })
    });
    recorded
}

/// The region with its keytips: the controls of size `size`, each with the fixed badge `letters` gives it.
fn shown(ui: &mut Ui, size: ControlSize, letters: [Option<&'static str>; 5]) -> StoryOutput {
    let places: Vec<Arc<Recorded>> = region(ui, size).into_iter().collect();
    let tabs = [(Label::from("Home"), "H", tab_id("Home")), (Label::from("Insert"), "N", tab_id("Insert"))];
    let controls: Vec<KeytipControl<'static>> =
        CONTROLS.iter().zip(letters).map(|((command, label), letters)| KeytipControl::new(CommandRef::from(*command), label, letters)).collect();
    let output = Keytips::new(true).tabs(&tabs).controls(&controls).places(&places).show_over(ui);
    output.response.into()
}

/// F10 shows the tab badges.
fn open(fixture: &mut Fixture) {
    fixture.settle();
    fixture.key(Key::F10, Modifiers::NONE).settle();
}

/// The Home tab's badge shows its controls' badges.
fn open_home(fixture: &mut Fixture) {
    open(fixture);
    fixture.key(Key::H, Modifiers::NONE).settle();
}

/// The first letter of the two-letter badges leaves only theirs.
fn open_prefix(fixture: &mut Fixture) {
    open_home(fixture);
    fixture.key(Key::F, Modifiers::NONE).settle();
}

#[story(Keytips<'static>, covers = [Default], drive = Drive::Custom(open))]
fn keytips_tabs(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, ControlSize::Md, [None; 5])
}

#[story(Keytips<'static>, covers = [], drive = Drive::Custom(open_home))]
fn keytips_commands(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, ControlSize::Md, [None; 5])
}

#[story(Keytips<'static>, covers = [], drive = Drive::Custom(open_prefix))]
fn keytips_two_letter_prefix(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, ControlSize::Md, [None, None, None, Some("FP"), Some("FD")])
}

#[story(Keytips<'static>, covers = [], drive = Drive::Custom(open_home))]
fn keytips_compact(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    shown(ui, ControlSize::Sm, [None; 5])
}
