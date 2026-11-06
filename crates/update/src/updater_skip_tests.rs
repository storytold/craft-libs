//! A release skipped at the startup prompt isn't offered again at start; a requested check and the
//! next release still are.

use std::sync::Arc;

use super::tests::{newer, settle};
use super::*;
use crate::suite::PHOTOCRAFT;

fn updater(dir: &tempfile::TempDir, release: Release) -> Updater {
    Updater::new(&PHOTOCRAFT, "0.13.0")
        .with_source(Arc::new(move || Ok(Some(release.clone()))))
        .with_pins_path(dir.path().join("versions.json"))
        .with_skipped_path(dir.path().join("photocraft.skipped"))
}

#[test]
fn a_skipped_release_is_not_offered_at_start() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    skipped::skip(&dir.path().join("photocraft.skipped"), &newer().tag).unwrap();
    let mut u = updater(&dir, newer());
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    assert!(!u.prompt_open());
    assert_eq!(u.status(), &Status::Idle);
}

#[test]
fn a_requested_check_still_offers_a_skipped_release() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    skipped::skip(&dir.path().join("photocraft.skipped"), &newer().tag).unwrap();
    let mut u = updater(&dir, newer());
    u.check_now(&ctx);
    settle(&mut u, &ctx);
    assert!(u.prompt_open());
}

#[test]
fn the_next_release_is_offered_after_a_skip() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    skipped::skip(&dir.path().join("photocraft.skipped"), &newer().tag).unwrap();
    let next = Release { tag: "v9.1.0".into(), version: crate::version::parse("9.1.0"), ..Release::default() };
    let mut u = updater(&dir, next);
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    assert!(u.prompt_open());
}

#[test]
fn skipping_from_the_prompt_remembers_the_release() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut u = updater(&dir, newer());
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    assert!(u.prompt_open());
    u.skip_offered();
    assert!(!u.prompt_open());
    assert!(skipped::is_skipped(&dir.path().join("photocraft.skipped"), &newer().tag));
    let mut again = updater(&dir, newer());
    again.check_at_start(&ctx);
    settle(&mut again, &ctx);
    assert!(!again.prompt_open(), "the next start offers nothing");
}

#[test]
fn an_offer_of_a_skipped_release_is_held() {
    let dir = tempfile::tempdir().unwrap();
    skipped::skip(&dir.path().join("photocraft.skipped"), &newer().tag).unwrap();
    let mut u = updater(&dir, newer());
    u.offer(newer());
    assert!(!u.prompt_open());
    let next = Release { tag: "v9.1.0".into(), version: crate::version::parse("9.1.0"), ..Release::default() };
    u.offer(next);
    assert!(u.prompt_open(), "the next release is offered");
}

#[test]
fn a_skip_that_cant_be_saved_keeps_the_prompt_and_says_so() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    // A folder where the file should be: it can't be written.
    std::fs::create_dir_all(dir.path().join("photocraft.skipped")).unwrap();
    let mut u = updater(&dir, newer());
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    if let Some(prompt) = &mut u.prompt {
        prompt.open = false;
    }
    u.skip_offered();
    assert!(u.prompt_open());
    let prompt = u.prompt.as_ref().unwrap();
    assert!(prompt.open, "it shows again");
    assert!(prompt.message().is_some_and(|m| m.starts_with("This version couldn't be skipped")), "{:?}", prompt.message());
}
