//! An app that drives its own frames hosts the runtime through the `embed` hooks alone.

use craft_behavior::Intent;
use craft_ui::contract::{Bind, Role, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::fonts::{I18nScripts, UiFonts};
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::frame::FrameInput;
use craft_ui::runtime::keys;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::shell::embed::{self, CloseDecision};
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Frames, install_spec, trial};
use egui::{Event, Key, Modifiers};

/// The app's own state: its dialog's open state and the Escapes it read at window level.
#[derive(Default)]
struct App {
    open: bool,
    window_escapes: u32,
}

/// One frame as an eframe app with its own `App` runs it: `raw_input_hook`, then `logic` and `ui` in the pass.
fn frame(frames: &mut Frames, app: &mut App, events: Vec<Event>) {
    embed::before_input(&frames.ctx, |_| FrameInput::new(RootEnv::default()));
    frames.frame_without_input(events, |ui| {
        embed::logic(ui.ctx(), || CloseDecision::Close);
        Dialog::new("Settings").opened(Bind::bound(&mut app.open)).buttons([FooterButton::new("Done", DialogRole::Accept)]).show(ui, |_, _| {});
        // Last in the UI: what no component took.
        keys::with_window_intents(ui.ctx(), false, |window| {
            let took = window.take(Intent::Cancel);
            app.window_escapes += u32::from(took);
            took
        });
    });
}

fn idle(frames: &mut Frames, app: &mut App, n: usize) {
    for _ in 0..n {
        frame(frames, app, Vec::new());
    }
}

/// An Escape press and release, then frames until the window rests (a closing dialog's fade has ended).
fn escape(frames: &mut Frames, app: &mut App) {
    frame(frames, app, vec![key_press(Key::Escape, Modifiers::NONE)]);
    frame(frames, app, vec![key_release(Key::Escape, Modifiers::NONE)]);
    for _ in 0..60 {
        frame(frames, app, Vec::new());
        if frames.repaint_delay().is_none() {
            break;
        }
    }
}

/// The app opens a framework dialog and Escape closes it; the next Escape, which no component takes, reaches the
/// app's window-level read, which the dialog's Escape never did.
#[trial]
fn an_app_with_its_own_frame_loop_hosts_a_dialog_and_reads_the_escape_no_component_took() {
    let mut frames = Frames::uninstalled();
    embed::install(&frames.ctx, install_spec(), &UiFonts::new(&I18nScripts)).expect("a fresh context takes the runtime");
    let mut app = App::default();
    idle(&mut frames, &mut app, 2);
    app.open = true;
    idle(&mut frames, &mut app, 3);
    assert!(!frames.tree().with_role(Role::DIALOG).is_empty(), "the dialog shows");
    escape(&mut frames, &mut app);
    assert!(!app.open, "Escape closes the dialog");
    assert_eq!(app.window_escapes, 0, "the dialog took its Escape");
    escape(&mut frames, &mut app);
    assert_eq!(app.window_escapes, 1, "the app reads the Escape no component took");
}
