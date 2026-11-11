#![cfg(not(target_arch = "wasm32"))]

use craft_ui::testing::trial;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use craft_theme::{ContrastChoice, ModeChoice, SuiteChoice, TextSize, ThemeId, ThemePair};

use super::SuiteClient;
use crate::{
    Op, SelectionChange, StoreOptions, SuiteDir, SuiteWatch,
    testing::{SAMPLE_THEME_ID, SAMPLE_THEME_JSON},
};

fn options() -> StoreOptions {
    StoreOptions::new("test 1.0").lock_timeout(Duration::from_millis(100))
}

fn id(s: &str) -> ThemeId {
    ThemeId::parse(s).unwrap()
}

/// `theme` in both slots.
fn theme(s: &str) -> SelectionChange {
    SelectionChange::themes(ThemePair::same(id(s)))
}

/// A suite choice of `theme` in both slots and `mode`, nothing else.
fn choice(theme: &str, mode: ModeChoice) -> SuiteChoice {
    SuiteChoice { theme_light: Some(id(theme)), theme_dark: Some(id(theme)), mode: Some(mode), ..SuiteChoice::default() }
}

/// The limit only ends a hung test; the built-in themes are parsed before the clock starts (see
/// `watch::tests::wait_for`).
fn settle(client: &mut SuiteClient) {
    let _ = craft_theme::builtins();
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.is_pending() {
        assert!(Instant::now() < deadline, "the change never settled");
        client.sync();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[trial]
fn quick_picks_never_conflict_with_each_other() {
    // Theme, then mode: both land.
    let mut client = SuiteClient::memory();
    client.sync();
    client.change(theme("craft.graphite"));
    client.change(SelectionChange::mode(ModeChoice::Dark));
    assert_eq!(client.shown(), choice("craft.graphite", ModeChoice::Dark));
    settle(&mut client);
    assert_eq!(client.accepted().suite, choice("craft.graphite", ModeChoice::Dark));
    assert!(client.conflict().is_none() && client.failure().is_none());

    // Theme, then theme, then theme: the last one wins, in two writes.
    client.change(theme("craft.studio"));
    client.change(theme("craft.artcraft"));
    client.change(theme("craft.graphite"));
    settle(&mut client);
    assert_eq!(client.accepted().suite, choice("craft.graphite", ModeChoice::Dark));
    assert_eq!(client.accepted().revision, 4);
    assert!(client.conflict().is_none() && client.failure().is_none());

    // The same with a worker thread.
    let tmp = tempfile::tempdir().unwrap();
    let mut threaded = SuiteClient::new(SuiteWatch::open(&SuiteDir::at(tmp.path()), options(), || {}));
    threaded.sync();
    threaded.change(theme("craft.graphite"));
    threaded.change(SelectionChange::mode(ModeChoice::Light));
    settle(&mut threaded);
    assert_eq!(threaded.accepted().suite, choice("craft.graphite", ModeChoice::Light));
    assert!(threaded.conflict().is_none() && threaded.failure().is_none());

    // Picks of different fields while one is on its way merge into one queued change.
    client.change(SelectionChange::theme_light(id("craft.soft")));
    client.change(SelectionChange::contrast(ContrastChoice::High));
    client.change(SelectionChange::text_size(TextSize::Medium));
    let expected = SuiteChoice {
        theme_light: Some(id("craft.soft")),
        contrast: Some(ContrastChoice::High),
        text_size: Some(TextSize::Medium),
        ..choice("craft.graphite", ModeChoice::Dark)
    };
    assert_eq!(client.shown(), expected);
    settle(&mut client);
    assert_eq!((&client.accepted().suite, client.accepted().revision), (&expected, 6));
    assert!(client.conflict().is_none() && client.failure().is_none());
}

#[trial]
fn a_library_theme_on_its_way_never_shows_the_app_default() {
    let mut client = SuiteClient::memory();
    client.sync();
    client.watch().request(Op::Import { bytes: Arc::from(SAMPLE_THEME_JSON.as_bytes()), as_copy: false });
    client.change(theme("craft.graphite"));
    settle(&mut client);
    let sample = id(SAMPLE_THEME_ID);

    // Picked for the dark slot: the previous theme stays there until the record is there, and
    // the light slot is untouched.
    client.change(SelectionChange::theme_dark(sample.clone()));
    assert!(client.theme(&sample).is_none());
    assert!(client.is_loading(&sample));
    assert_eq!((client.shown().theme_light, client.shown().theme_dark), (Some(id("craft.graphite")), Some(id("craft.graphite"))));
    settle(&mut client);
    assert_eq!((client.shown().theme_light, client.shown().theme_dark), (Some(id("craft.graphite")), Some(sample.clone())));
    assert!(client.theme(&sample).is_some());
    assert!(!client.is_loading(&sample));

    // Picked for the app alone: loading until the snapshot has it.
    let mut own = SuiteClient::memory();
    own.sync();
    own.watch().request(Op::Import { bytes: Arc::from(SAMPLE_THEME_JSON.as_bytes()), as_copy: false });
    own.set_app_theme(Some(ThemePair::same(sample.clone())));
    assert!(own.is_loading(&sample));
    own.sync();
    assert!(own.theme(&sample).is_some());
    assert!(!own.is_loading(&sample));
    // A theme that can't load is not "loading" forever.
    own.set_app_theme(Some(ThemePair::same(id("gone"))));
    own.sync();
    assert!(!own.is_loading(&id("gone")));
}
