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
