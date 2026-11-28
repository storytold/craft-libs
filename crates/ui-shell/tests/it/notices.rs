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

/// Past the cap a new card always shows. It makes room by dropping the oldest earlier card that would leave by itself;
/// a sticky card and a card with an action stay. The drop is reported: `toast::end` says `Dropped`, and a notice
/// center reports `NoticeEnd::Dropped` for its own notice.
#[trial]
fn the_cap_drops_a_card_that_would_leave_by_itself_and_reports_it() {
    use craft_ui::notices::{NoticeCenter, NoticeEnd, PostedNotice};
    use craft_ui::runtime::toast::{ToastAction, ToastEnd};
    use std::cell::RefCell;
    use std::rc::Rc;

    let window = WindowKey::new("toast-cap-shell");
    let ends = Rc::new(RefCell::new(Vec::new()));
    let (shell, seen) = (window.clone(), ends.clone());
    let mut fixture = Fixture::new(move |ui| {
        seen.borrow_mut().extend(NoticeCenter::new().window(shell.clone()).show(ui).ends);
        AppShell::embedded(shell.clone()).body(|_, _, _| {}).show(ui, &mut ());
    });
    fixture.room(egui::Vec2::new(800.0, 600.0));
    fixture.settle();
    let ctx = fixture.ctx().clone();
    let ids = |ctx: &egui::Context| toast::cards(ctx, &window).iter().map(|card| card.id).collect::<Vec<_>>();
    let sticky = toast::post(&ctx, &window, Toast::new("The disk is full.").timeout(None));
    let action = toast::post(&ctx, &window, Toast::new("Brushes were updated.").action(ToastAction::new("Undo")));
    let timed = toast::post(&ctx, &window, Toast::new("poster.png was saved."));
    let newest = toast::post(&ctx, &window, Toast::new("flyer.png was saved."));
    fixture.settle();
    assert_eq!(ids(&ctx), vec![sticky, action, newest], "the timed card made room; the sticky and the action card stay");
    assert!(fixture.texts().iter().any(|painted| painted.text == "flyer.png was saved."), "the newest card shows");
    assert_eq!(toast::end(&ctx, timed), Some(ToastEnd::Dropped), "the dropped card reports how it ended");

    // The queue holds the sticky card, the action card and `newest`. The notice drops `newest`, the oldest card that
    // leaves by itself; the next toast drops the notice, which is then the oldest such card.
    let center = NoticeCenter::new().window(window.clone());
    let notice = center.post(&ctx, &PostedNotice::new("poster.pdf was exported."), &[]).expect("no key silences the notice");
    let last = toast::post(&ctx, &window, Toast::new("banner.png was saved."));
    fixture.settle();
    assert_eq!(ids(&ctx), vec![sticky, action, last], "each new card stayed and dropped a timed one");
    assert_eq!(toast::end(&ctx, newest), Some(ToastEnd::Dropped), "the notice dropped the card before it");
    assert!(ends.borrow().contains(&NoticeEnd::Dropped(notice)), "the center reports its notice dropped: {:?}", ends.borrow());
}

/// Past the cap a progress card stays: with only sticky cards beside it the oldest sticky card goes, and with only
/// progress cards earlier the new card is queued past the cap. `set_progress` moves the live card's fraction and
/// message, and its Cancel action ends it as an action.
#[trial]
fn the_cap_never_drops_a_progress_card() {
    use craft_ui::runtime::toast::{ToastAction, ToastEnd};

    let window = WindowKey::new("toast-progress-shell");
    let shell = window.clone();
    let mut fixture = Fixture::new(move |ui| {
        AppShell::embedded(shell.clone()).body(|_, _, _| {}).show(ui, &mut ());
    });
    fixture.room(egui::Vec2::new(800.0, 600.0));
    fixture.settle();
    let ctx = fixture.ctx().clone();
    let ids = |ctx: &egui::Context, window: &WindowKey| toast::cards(ctx, window).iter().map(|card| card.id).collect::<Vec<_>>();
    let job = toast::post(&ctx, &window, Toast::new("Page 1 of 4").title("Exporting").progress(0.25).action(ToastAction::new("Cancel")));
    let first = toast::post(&ctx, &window, Toast::new("The disk is full.").timeout(None));
    let second = toast::post(&ctx, &window, Toast::new("The font is missing.").timeout(None));
    let newest = toast::post(&ctx, &window, Toast::new("The profile is missing.").timeout(None));
    fixture.settle();
    assert_eq!(ids(&ctx, &window), vec![job, second, newest], "the oldest sticky card made room; the progress card stays");
    assert_eq!(toast::end(&ctx, first), Some(ToastEnd::Dropped), "the oldest plain card was dropped");
    assert_eq!(toast::end(&ctx, job), None, "the progress card has not ended");

    let jobs = WindowKey::new("toast-progress-only");
    let posted: Vec<_> = (0..=toast::MAX_CARDS).map(|n| toast::post(&ctx, &jobs, Toast::new(format!("Job {n}")).progress(0.0))).collect();
    assert_eq!(ids(&ctx, &jobs), posted, "with only progress cards none is dropped, past the cap");

    toast::set_progress(&ctx, job, 0.5, Some("Page 2 of 4".into()));
    fixture.settle();
    let card = toast::cards(&ctx, &window).into_iter().find(|card| card.id == job).expect("the progress card is queued");
    assert_eq!(card.toast.progress, Some(0.5), "the fraction moved");
    assert_eq!(card.toast.message, Toast::new("Page 2 of 4").message, "the message moved");
    assert!(fixture.texts().iter().any(|painted| painted.text == "Page 2 of 4"), "the card shows the new message");

    let cancel = fixture.texts().iter().find(|painted| painted.text == "Cancel").map(|painted| painted.rect.center()).expect("Cancel shows");
    fixture.click_at(cancel);
    fixture.settle();
    assert!(!ids(&ctx, &window).contains(&job), "Cancel ended the card");
    assert_eq!(toast::end(&ctx, job), Some(ToastEnd::Action(0)), "it ended by its first action");
}

/// A card the pointer rests on for longer than its timeout, with no pass running meanwhile, gets its whole timeout
/// again when the pointer leaves: the pass that sees the pointer go, run twice in its frame, counts none of the hover
/// against it. It expires one full timeout after the leave.
#[trial]
fn a_card_left_after_a_long_still_hover_gets_its_whole_delay() {
    use craft_ui::runtime::toast::ToastEnd;
    use craft_ui::testing::pointer::moved;
    use std::cell::Cell;
    use std::rc::Rc;

    const TIMEOUT: f32 = 2.0;
    const MESSAGE: &str = "poster.png was saved.";
    let window = WindowKey::new("toast-hover-shell");
    let discard = Rc::new(Cell::new(false));
    let (shell, asks) = (window.clone(), discard.clone());
    let mut posted = false;
    let mut fixture = Fixture::new(move |ui| {
        if !posted {
            toast::post(ui.ctx(), &shell, Toast::new(MESSAGE).timeout(Some(TIMEOUT)));
            posted = true;
        }
        AppShell::embedded(shell.clone()).body(|_, _, _| {}).show(ui, &mut ());
        if asks.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the leave frame runs twice");
        }
    });
    fixture.room(egui::Vec2::new(800.0, 600.0));
    fixture.settle();
    let card = toast::cards(fixture.ctx(), &window).first().map(|card| card.id).expect("the card is queued");
    let queued = |fixture: &Fixture| toast::cards(fixture.ctx(), &window).iter().any(|shown| shown.id == card);

    // Half the timeout runs out before the pointer comes.
    fixture.advance(f64::from(TIMEOUT) / 2.0).frame(Vec::new());
    let on_card = fixture.texts().iter().find(|painted| painted.text == MESSAGE).map(|painted| painted.rect.center()).expect("the card shows");
    fixture.frame(vec![moved(on_card)]).frame(Vec::new());

    // The pointer rests on the card five timeouts long; nothing repaints meanwhile.
    fixture.advance(f64::from(TIMEOUT) * 5.0);
    discard.set(true);
    fixture.frame(vec![moved(egui::Pos2::new(5.0, 5.0))]);
    discard.set(false);
    assert_eq!(fixture.passes(), 2, "the leave frame ran twice");
    fixture.frame(Vec::new());
    assert!(queued(&fixture), "the hover counts nothing against the card");

    fixture.advance(f64::from(TIMEOUT) * 0.75).frame(Vec::new());
    assert!(queued(&fixture), "the card has its whole timeout again, not the half left before the hover");
    fixture.advance(f64::from(TIMEOUT) * 0.5).frame(Vec::new());
    assert!(!queued(&fixture), "a full timeout after the leave the card is gone");
    assert_eq!(toast::end(fixture.ctx(), card), Some(ToastEnd::Expired), "it expired");
}

/// A shell can keep transient notices clear of controls at its trailing edge.
#[trial]
fn a_shell_can_center_its_toasts() {
    let window = WindowKey::new("centered-toast-shell");
    let mut fixture = Fixture::new(move |ui| {
        if toast::cards(ui.ctx(), &window).is_empty() {
            toast::post(ui.ctx(), &window, Toast::new("Saved document").timeout(None));
        }
        AppShell::new(window.clone()).toast_alignment(egui::Align2::CENTER_BOTTOM).body(|_, _, _| {}).show(ui, &mut ());
    });
    fixture.room(egui::vec2(900.0, 500.0)).settle();
    let text = fixture.texts().iter().find(|painted| painted.text == "Saved document").expect("the toast is drawn").rect;
    let layers = craft_ui::testing::snapshot::layer_rects(fixture.ctx());
    let card = layers.iter().find(|rect| rect.contains(text.center())).expect("the toast layer contains its text");
    assert!((card.center().x - fixture.shown().center().x).abs() < 1.0, "the notice is centred: {card:?}");
}
