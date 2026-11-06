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
        .with_skipped_path(dir.path().join("skipped.json"))
}

#[test]
fn a_skipped_release_is_not_offered_at_start() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    skipped::skip(&dir.path().join("skipped.json"), PHOTOCRAFT.id, &newer().tag).unwrap();
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
    skipped::skip(&dir.path().join("skipped.json"), PHOTOCRAFT.id, &newer().tag).unwrap();
    let mut u = updater(&dir, newer());
    u.check_now(&ctx);
    settle(&mut u, &ctx);
    assert!(u.prompt_open());
}

#[test]
fn the_next_release_is_offered_after_a_skip() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    skipped::skip(&dir.path().join("skipped.json"), PHOTOCRAFT.id, &newer().tag).unwrap();
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
    assert!(skipped::is_skipped(&dir.path().join("skipped.json"), PHOTOCRAFT.id, &newer().tag));
    let mut again = updater(&dir, newer());
    again.check_at_start(&ctx);
    settle(&mut again, &ctx);
    assert!(!again.prompt_open(), "the next start offers nothing");
}
