//! The focus service and the key stage, driven through registered stops over real egui widgets.

mod keys;
mod press;
mod roving;
mod scopes;

use craft_behavior::Input;
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::testing::Frames;
use craft_ui::testing::focus::{Stop, register_stop, widget};
use egui::{Context, Event, Id, Key, Modifiers, Ui};

fn id(name: &str) -> Id {
    craft_ui::testing::focus::named_id(name)
}

/// A pointer-activated control: an egui widget and its stop.
fn control(ui: &mut Ui, name: &str) {
    stop_with(ui, name, KeyUse::of(Input::Pointer));
}

fn stop_with(ui: &mut Ui, name: &str, keys: KeyUse) {
    widget(ui, id(name));
    register_stop(ui.ctx(), Stop::new(id(name), keys));
}

/// Focuses `name` in the next pass and runs that pass.
fn focus(frames: &mut Frames, name: &str, show: &mut impl FnMut(&mut Ui)) {
    request(&frames.ctx, FocusTarget::Id(id(name)));
    frames.frame(Vec::new(), show);
}

/// A key press and its release, in two frames, as a keyboard sends them.
fn tap(frames: &mut Frames, key: Key, modifiers: Modifiers, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![craft_ui::testing::input::key_press(key, modifiers)], &mut *show);
    frames.frame(vec![craft_ui::testing::focus::key_release(key, modifiers)], show);
}

fn focused(ctx: &Context) -> Option<Id> {
    craft_ui::testing::focus::egui_focused(ctx)
}

fn event_is_key(event: &Event, key: Key) -> bool {
    matches!(event, Event::Key { key: pressed, .. } if *pressed == key)
}
