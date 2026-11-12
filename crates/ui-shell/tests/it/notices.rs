//! The notices family: the settings notices and each window's toasts.

use craft_theme::variant::Tone;
use craft_ui::notices::{SaveResult, SettingsNoticeState, SettingsStatus};
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

/// A shell shows its window's toasts once per pass although its body also runs for sizing; debug builds panic when
/// the toast layer's presence stage opens twice in a pass.
#[trial]
fn a_shell_opens_its_toast_layer_once_per_pass() {
    let window = WindowKey::new("toast-stack-shell");
    let mut fixture = Fixture::new(move |ui| {
        let ctx = ui.ctx().clone();
        if toast::cards(&ctx, &window).is_empty() {
            toast::post(&ctx, &window, Toast::new("poster.png was saved to Pictures."));
        }
        AppShell::embedded(window.clone()).body(|_, _, _| {}).show(ui, &mut ());
    });
    fixture.settle();
    assert!(!fixture.texts().is_empty(), "the toast card shows");
}

/// A toast posted to an embedded shell's window shows once, inside that shell's body, and not in the shell beside it.
#[trial]
fn a_toast_shows_in_the_body_of_its_own_window() {
    let mut fixture = Fixture::new(|ui| {
        let ctx = ui.ctx().clone();
        let right = WindowKey::new("right-shell");
        if toast::cards(&ctx, &right).is_empty() {
            toast::post(&ctx, &right, Toast::new("poster.png was saved.").timeout(None));
        }
        ui.columns(2, |columns| {
            AppShell::embedded(WindowKey::new("left-shell")).body(|_, _, _| {}).show(&mut columns[0], &mut ());
            AppShell::embedded(right).body(|_, _, _| {}).show(&mut columns[1], &mut ());
        });
    });
    fixture.room(egui::Vec2::new(800.0, 400.0));
    fixture.settle();
    let cards: Vec<egui::Rect> =
        fixture.texts().iter().filter(|painted| painted.text == "poster.png was saved.").map(|painted| painted.rect).collect();
    assert_eq!(cards.len(), 1, "one card, in one window");
    assert!(cards[0].left() > fixture.shown().center().x, "the card sits in the right-hand shell");
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
        let window = WindowKey::new("toast-focus-shell");
        if !posted {
            toast::post(&ctx, &window, Toast::new("poster.png was saved."));
            toast::post(&ctx, &window, Toast::new("flyer.png was saved."));
            posted = true;
        }
        AppShell::embedded(window)
            .body(|ui, _, _| {
                Button::new("Before").show(ui);
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
    let window = WindowKey::new("toast-focus-shell");
    assert_eq!(toast::cards(fixture.ctx(), &window).len(), 1, "one card ended");
    assert_eq!(focused_name(&fixture).as_deref(), Some("Dismiss"), "the focus moved to the other card's Dismiss");
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(toast::cards(fixture.ctx(), &window).is_empty(), "both cards ended");
    assert_eq!(focused_name(&fixture).as_deref(), Some("Before"), "the focus is back where it came from");
}
