//! Command scopes of a layout panel and of a focused leaf, and the defaults of a command that differ by platform.

use craft_ui::buttons::Button;
use craft_ui::contract::{CommandSet, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::commands::{CommandScope, Commands as _};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// The commands of the panel scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "panel", scope = "trial.panel")]
#[non_exhaustive]
pub enum PanelCommand {
    #[command(label = "Toggle", default = "Cmd+Alt+P")]
    Toggle,
}

const PANEL: CommandScope = <PanelCommand as CommandSet>::SCOPE;

fn toggle(fixture: &mut Fixture) {
    fixture.key(Key::P, Modifiers::COMMAND | Modifiers::ALT);
    fixture.settle();
}

/// Two stacks with the same command scope: the chord reaches the one that holds focus.
#[trial]
fn a_stack_scope_command_reaches_only_the_focused_stack() {
    let counts = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
    let ids = [Rc::new(Cell::new(None)), Rc::new(Cell::new(None))];
    let (seen, stops) = (counts.clone(), ids.clone());
    let mut fixture = Fixture::new(move |ui| {
        for (at, name) in ["First", "Second"].into_iter().enumerate() {
            let taken = &seen[at];
            Stack::new().command_scope(PANEL).id_salt(name).show(ui, |ui, cx| {
                while cx.take_command::<PanelCommand>().is_some() {
                    taken.set(taken.get() + 1);
                }
                stops[at].set(Some(Button::new(name).show(ui).id));
            });
        }
    });
    fixture.settle();
    for (focused, expected) in [(1, (0, 1)), (0, (1, 1))] {
        request(fixture.ctx(), FocusTarget::Id(ids[focused].get().expect("the button ran")));
        fixture.settle();
        toggle(&mut fixture);
        assert_eq!((counts[0].get(), counts[1].get()), expected, "only the stack holding focus ({focused}) takes the command");
    }
}
