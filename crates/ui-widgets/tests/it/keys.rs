//! Which key roles take Delete: a command scope gets Delete from a focused button, while a deletable list and a text
//! field keep it, and a collection that leaves its arrows to its owner lets the owner's arrow command run.

use craft_ui::contract::{Bind, CommandSet, Commands as _, Show as _, ShowContainer as _};
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::CommandScope;
use craft_ui::runtime::focus::Initial;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The commands of the scope: Delete and a bare Down, both kept from the window's shortcuts while focus is inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "keysTrial", scope = "trial.keys")]
#[non_exhaustive]
enum KeysCommand {
    #[command(label = "Remove", default = "Delete", modal = "always")]
    Remove,
    #[command(label = "Step", default = "Down", modal = "always")]
    Step,
}

const SCOPE: CommandScope = <KeysCommand as CommandSet>::SCOPE;

/// How often each command of the scope ran.
#[derive(Default)]
struct Ran {
    remove: Cell<u32>,
    step: Cell<u32>,
}

/// Counts the scope's commands that `$cx` has this pass.
macro_rules! count {
    ($cx:expr, $ran:expr) => {
        while let Some((command, _)) = $cx.take_command::<KeysCommand>() {
            let counter = if command == KeysCommand::Remove { &$ran.remove } else { &$ran.step };
            counter.set(counter.get() + 1);
        }
    };
}

/// Backspace and Delete in a text field edit its text, and the scope's Delete command does not run.
#[trial]
fn backspace_and_delete_in_a_text_field_edit_the_text() {
    let ran = Rc::new(Ran::default());
    let text = Rc::new(RefCell::new(String::new()));
    let (seen, shown) = (ran.clone(), text.clone());
    let mut fixture = Fixture::new(move |ui| {
        Popover::new("Card", Bind::given(true)).initial_focus(Initial::First).command_scope(SCOPE).show(ui, |ui, cx| {
            count!(cx, seen);
            TextField::new("Name", Bind::bound(&mut *shown.borrow_mut())).show(ui);
        });
    });
    fixture.settle();
    fixture.type_text("abc");
    fixture.idle(2);
    fixture.key(Key::Backspace, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(*text.borrow(), "ab");
    fixture.key(Key::Home, Modifiers::NONE);
    fixture.key(Key::Delete, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(*text.borrow(), "b");
    assert_eq!(ran.remove.get(), 0);
}
