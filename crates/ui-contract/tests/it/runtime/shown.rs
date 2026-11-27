//! Where commands were shown (`commands::holds`, `commands::shown`).

use craft_ui::runtime::commands::{self, CommandRef};
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, trial};

/// A container that holds a hundred commands, as a menu title holds its menu's, registers once per pass: a replay of
/// the menu bar then makes one registration per title, not one per command (photocraft#15493). Each command is still
/// found, at the container's rect, as held.
#[trial]
fn a_container_registers_its_commands_once() {
    let held: Vec<CommandRef> = (0..100).map(|i| CommandRef::new(format!("test.held{i}"))).collect();
    let shown = held.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rect = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(40.0, 20.0));
        commands::holds(ui.ctx(), ui.layer_id(), rect, &shown, Label::from("Image"));
    });
    fixture.settle();
    assert_eq!(commands::shown_registrations(fixture.ctx()), 1, "one registration for the container");
    for command in [&held[0], &held[57], &held[99]] {
        let found = commands::shown(fixture.ctx(), command).expect("a held command is shown");
        assert!(found.holder, "{command} is shown as held");
        assert_eq!(found.rect.size(), egui::vec2(40.0, 20.0), "{command} is shown at the container's rect");
    }
    assert!(commands::shown(fixture.ctx(), &CommandRef::new("test.other")).is_none(), "a command nothing holds is not shown");
}
