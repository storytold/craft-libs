//! A key command closes the open popovers, except the one whose content declares the command's scope.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CommandSet, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::Commands as _;
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// The commands of the popover's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "popoverInside", scope = "trial.popoverInside")]
#[non_exhaustive]
pub enum InsideCommand {
    #[command(label = "Inside", default = "Cmd+1")]
    Inside,
}

/// The commands of the stack around the popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "popoverAround", scope = "trial.popoverAround")]
#[non_exhaustive]
pub enum AroundCommand {
    #[command(label = "Around", default = "Cmd+2")]
    Around,
}

#[derive(Default)]
struct Seen {
    open: Cell<bool>,
    inside: Cell<u32>,
    around: Cell<u32>,
    button: Cell<Option<Id>>,
}

/// A stack with the around scope holding an open popover whose content has the inside scope and a focused button.
fn fixture() -> (Fixture, Rc<Seen>) {
    let seen = Rc::new(Seen { open: Cell::new(true), ..Seen::default() });
    let shown = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().command_scope(<AroundCommand as CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<AroundCommand>().is_some() {
                shown.around.set(shown.around.get() + 1);
            }
            let opener = Button::new("Options").show(ui);
            let mut open = shown.open.get();
            Popover::new("Options", Bind::bound(&mut open)).opened_by(&opener).command_scope(<InsideCommand as CommandSet>::SCOPE).show(
                ui,
                |ui, cx| {
                    while cx.take_command::<InsideCommand>().is_some() {
                        shown.inside.set(shown.inside.get() + 1);
                    }
                    shown.button.set(Some(Button::new("Snap").show(ui).id));
                },
            );
            shown.open.set(open);
        });
    });
    fixture.idle(10);
    request(fixture.ctx(), FocusTarget::Id(seen.button.get().expect("the popover's content ran")));
    fixture.idle(2);
    (fixture, seen)
}

#[trial]
fn a_command_of_the_popovers_own_content_keeps_it_open_and_one_around_it_closes_it() {
    let (mut fixture, seen) = fixture();
    fixture.key(Key::Num1, Modifiers::COMMAND);
    fixture.idle(4);
    assert_eq!(seen.inside.get(), 1, "the content's command runs");
    assert!(seen.open.get(), "and the popover stays open");
    fixture.key(Key::Num2, Modifiers::COMMAND);
    fixture.idle(4);
    assert_eq!(seen.around.get(), 1, "the command around the popover runs");
    assert!(!seen.open.get(), "and closes it");
}
