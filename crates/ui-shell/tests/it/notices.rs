//! The notices family: the settings notices and the toast stack.

use craft_theme::variant::Tone;
use craft_ui::notices::{SaveResult, SettingsNoticeState, SettingsStatus, ToastStack};
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::toast::{self, Toast};
use craft_ui::shell::AppShell;
use craft_ui::testing::{Fixture, trial};

fn failed() -> SaveResult {
    SaveResult::Failed("disk full".to_owned())
}

/// The first failure speaks in the status line, the third adds the notice, and the next success clears it and says so.
#[trial]
fn repeated_save_failures_escalate_and_a_success_clears_them() {
    let ctx = egui::Context::default();
    let mut state = SettingsNoticeState::new();
    state.ticked(&ctx, &failed());
    assert_eq!(state.take_status(), Some(SettingsStatus::SaveFailed("disk full".to_owned())), "the first failure reaches the status line");
    assert!(!state.has_notice(), "one failure shows no notice");
    state.ticked(&ctx, &failed());
    assert_eq!(state.take_status(), None, "a second failure adds no status");
    state.ticked(&ctx, &failed());
    assert!(state.has_notice(), "the third failure in a row shows the notice");
    state.ticked(&ctx, &SaveResult::Saved);
    assert!(!state.has_notice(), "a success removes the notice");
    let status = state.take_status().expect("a success after failures says so");
    assert_eq!(status.tone(), Tone::Good);
}

/// A toast stack in a shell body shows once per pass although the body also runs for sizing; debug builds panic
/// when its presence stage opens twice in a pass.
#[trial]
fn a_toast_stack_in_a_shell_body_opens_its_layer_once_per_pass() {
    let mut fixture = Fixture::new(|ui| {
        let ctx = ui.ctx().clone();
        if toast::cards(&ctx).is_empty() {
            toast::post(&ctx, Toast::new("poster.png was saved to Pictures."));
        }
        AppShell::embedded(WindowKey::new("toast-stack-shell")).body(|ui, _, _| ToastStack::show(ui)).show(ui, &mut ());
    });
    fixture.settle();
    assert!(!fixture.texts().is_empty(), "the toast card shows");
}

/// Dismissing the focused toast hands the focus to the next card, and the last one back to where it came from.
#[trial]
fn a_dismissed_toast_hands_the_focus_on_and_the_last_returns_it() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;
    use egui::{Key, Modifiers};

    let mut posted = false;
    let mut fixture = Fixture::new(move |ui| {
        let ctx = ui.ctx().clone();
        if !posted {
            toast::post(&ctx, Toast::new("poster.png was saved."));
            toast::post(&ctx, Toast::new("flyer.png was saved."));
            posted = true;
        }
        AppShell::embedded(WindowKey::new("toast-focus-shell"))
            .body(|ui, _, _| {
                Button::new("Before").show(ui);
                ToastStack::show(ui);
            })
            .show(ui, &mut ());
    });
    fixture.settle();
    let focused_name = |fixture: &Fixture| {
        let tree = fixture.tree();
        tree.name_of(craft_ui::testing::access::focused_key(tree)).map(str::to_owned)
    };
    for _ in 0..6 {
        if focused_name(&fixture).as_deref() == Some("Before") {
            break;
        }
        fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    }
    assert_eq!(focused_name(&fixture).as_deref(), Some("Before"), "Tab reaches the body's button");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused_name(&fixture).as_deref(), Some("Dismiss"), "Tab reaches the first card's Dismiss");
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert_eq!(toast::cards(fixture.ctx()).len(), 1, "one card ended");
    assert_eq!(focused_name(&fixture).as_deref(), Some("Dismiss"), "the focus moved to the other card's Dismiss");
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(toast::cards(fixture.ctx()).is_empty(), "both cards ended");
    assert_eq!(focused_name(&fixture).as_deref(), Some("Before"), "the focus is back where it came from");
}
