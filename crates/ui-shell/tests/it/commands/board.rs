use super::kit::{AppCommand, Dock, DockCommand, PaneCommand, Tap};
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::runtime::commands::{
    CommandGate, CommandInput, CommandRef, CommandState, Commands as _, EditCommands, FireSource, Fired, fire, resolve,
};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Key, Modifiers, Pos2, Ui};
use std::cell::Cell;

fn ids(fired: &[Fired]) -> Vec<&str> {
    fired.iter().map(|f| f.command.id.as_str()).collect()
}

/// Hovers, presses and releases at `at` over three frames and settles for one; the first pass of the release
/// frame asks for a discard pass when `discard` is set.
fn click_at(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui), discard: bool) {
    frames.frame(vec![moved(at)], &mut *show);
    frames.frame(vec![press(at)], &mut *show);
    let mut first = discard;
    frames.frame(vec![release(at)], |ui| {
        show(ui);
        if std::mem::take(&mut first) {
            ui.ctx().request_discard("command trial");
        }
    });
    frames.frame(vec![], &mut *show);
}

fn tap_center(frames: &mut Frames, command: AppCommand) -> Pos2 {
    let mut center = Pos2::ZERO;
    frames.idle(2, |ui| center = Tap::new().id_salt("go").command(command).show(ui).rect.center());
    center
}

/// Prototype: `discarded_pass_delivers_a_click_and_an_edit_once` (the click).
#[trial]
fn a_command_fired_in_a_discarded_pass_is_delivered_once() {
    let mut frames = Frames::new();
    let at = tap_center(&mut frames, AppCommand::Go);
    let mut show = |ui: &mut Ui| {
        Tap::new().id_salt("go").command(AppCommand::Go).show(ui);
    };
    click_at(&mut frames, at, &mut show, true);
    assert_eq!(ids(&take_output(&frames.ctx).fired), ["trial.go"]);
    frames.idle(2, &mut show);
    assert!(take_output(&frames.ctx).fired.is_empty(), "nothing fires again in later frames");
}

fn focus_on(frames: &mut Frames, id: egui::Id, show: &mut impl FnMut(&mut Ui)) {
    request(&frames.ctx, FocusTarget::Id(id));
    frames.frame(vec![], &mut *show);
}

/// 9.3 and 11.3: with focus in a pane inside a dock, the dock's shortcut beats an application shortcut on the same
/// chord; with focus outside, the application's does.
#[trial]
fn an_enclosing_scopes_shortcut_beats_the_applications() {
    let tap = Cell::new(None);
    let closed = Cell::new(0);
    let outside = Cell::new(None);
    let mut show = |ui: &mut Ui| {
        Dock::new().id_salt("dock").show(ui, |ui, cx| {
            while cx.take_command::<DockCommand>().is_some() {
                closed.set(closed.get() + 1);
            }
            super::kit::Pane::new().id_salt("pane").show(ui, |ui, _| tap.set(Some(Tap::new().id_salt("in").show(ui).id)));
        });
        outside.set(Some(Tap::new().id_salt("out").show(ui).id));
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    focus_on(&mut frames, tap.get().unwrap(), &mut show);
    frames.frame(vec![key_press(Key::J, Modifiers::COMMAND | Modifiers::ALT)], &mut show);
    assert_eq!((closed.get(), take_output(&frames.ctx).fired.len()), (1, 0), "the dock takes it");
    focus_on(&mut frames, outside.get().unwrap(), &mut show);
    frames.frame(vec![key_press(Key::J, Modifiers::COMMAND | Modifiers::ALT)], &mut show);
    assert_eq!((closed.get(), ids(&take_output(&frames.ctx).fired)), (1, vec!["trial.jump"]), "outside the dock the application's command fires");
}

/// 1.8 and 7.2: the gate is app code, so it runs with no runtime lock held and may use the board.
#[trial]
fn the_gate_may_resolve_commands() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let inside = std::sync::atomic::AtomicBool::new(false);
    frames.commands(CommandInput::new().gate(CommandGate::new(move |_: &CommandRef| {
        // The nested `resolve` runs the gate again, which stops here.
        inside.swap(true, std::sync::atomic::Ordering::SeqCst) || {
            let enabled = resolve(&ctx, AppCommand::Go).enabled;
            inside.store(false, std::sync::atomic::Ordering::SeqCst);
            enabled
        }
    })));
    let mut show = |ui: &mut Ui| {
        Tap::new().id_salt("go").command(AppCommand::Save).show(ui);
    };
    frames.idle(2, &mut show);
    frames.frame(vec![key_press(Key::K, Modifiers::COMMAND)], &mut show);
    assert_eq!(ids(&take_output(&frames.ctx).fired), ["trial.save"]);
}

/// 11.3: a scoped entry waits for its instance and is dropped when the instance stops being live.
#[trial]
fn an_untaken_scoped_entry_is_dropped_with_its_instance() {
    let panes = Cell::new(true);
    let taking = Cell::new(false);
    let taken = Cell::new(0);
    let mut show = |ui: &mut Ui| {
        if panes.get() {
            super::kit::Pane::new().id_salt("a").show(ui, |_, cx| {
                if taking.get() {
                    taken.set(taken.get() + usize::from(cx.take_command::<PaneCommand>().is_some()));
                }
            });
        }
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    assert!(fire(&frames.ctx, PaneCommand::ZoomIn, FireSource::Palette));
    panes.set(false);
    frames.idle(3, &mut show);
    panes.set(true);
    taking.set(true);
    frames.idle(3, &mut show);
    assert_eq!(taken.get(), 0, "the entry died with its instance");
}

/// 11.4: a modal layer blocks the application's `ModalRule::Declared` edit commands, except for an instance inside it
/// that declares them: a dialog that publishes the undo state takes it, and the command resolves enabled there.
#[trial]
fn undo_in_a_dialog_reaches_the_dialog_and_not_the_application() {
    use craft_ui::contract::Bind;
    use craft_ui::dialog::Dialog;
    use craft_ui::runtime::focus::Initial;
    use craft_ui::sections::{DialogRole, FooterButton};

    let undone = Cell::new(0);
    let mut show = |ui: &mut Ui| {
        let mut open = true;
        Dialog::new("Options")
            .opened(Bind::bound(&mut open))
            .initial_focus(Initial::First)
            .buttons([FooterButton::new("OK", DialogRole::Accept)])
            .show(ui, |_, cx| {
                cx.publish(EditCommands::Undo, CommandState::new());
                while let Some((command, _)) = cx.take_command::<EditCommands>() {
                    if command == EditCommands::Undo {
                        undone.set(undone.get() + 1);
                    }
                }
            });
    };
    let mut frames = Frames::new();
    frames.idle(4, &mut show);
    assert!(resolve(&frames.ctx, EditCommands::Undo).enabled, "the declared undo is enabled in the dialog");
    assert!(!resolve(&frames.ctx, EditCommands::Redo).enabled, "the redo the dialog does not declare is not");
    frames.frame(vec![key_press(Key::Z, Modifiers::COMMAND)], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(undone.get(), 1, "the dialog took the undo");
    assert!(take_output(&frames.ctx).fired.is_empty(), "the application's undo stays blocked");
}
