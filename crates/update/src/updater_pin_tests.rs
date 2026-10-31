//! The startup prompt and requested checks follow the launcher's shared version choice.

use std::sync::Arc;

use super::tests::{newer, settle};
use super::*;
use crate::suite::PHOTOCRAFT;

/// A version-choice file holding `track` for PhotoCraft.
fn pins_with(track: crate::pins::VersionTrack) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut pins = Pins::default();
    pins.set(PHOTOCRAFT.id, &track);
    pins.save(&dir.path().join("versions.json")).unwrap();
    dir
}

#[test]
fn a_pinned_app_is_not_prompted_and_the_network_is_not_asked() {
    use crate::pins::VersionTrack;
    let ctx = egui::Context::default();
    for track in [VersionTrack::Pinned("v0.13.0".into()), VersionTrack::Nightly] {
        let dir = pins_with(track.clone());
        let mut u = Updater::new(&PHOTOCRAFT, "0.13.0")
            .with_source(Arc::new(|| std::panic::panic_any("the network must not be asked")))
            .with_pins_path(dir.path().join("versions.json"));
        u.check_at_start(&ctx);
        settle(&mut u, &ctx);
        assert!(!u.prompt_open(), "{track:?}");
        assert_eq!(u.status(), &Status::Idle);
        u.check_now(&ctx);
        settle(&mut u, &ctx);
        assert_eq!(u.status(), &Status::UpToDate, "{track:?}");
        assert_eq!(u.managed(), Some(&Choice::Managed(track.clone())));
        u.offer(newer());
        assert!(!u.prompt_open(), "an offer is held too: {track:?}");
    }
}

#[test]
fn unpinning_in_the_launcher_brings_updates_back_on_the_next_check() {
    let ctx = egui::Context::default();
    let dir = pins_with(crate::pins::VersionTrack::Pinned("v0.13.0".into()));
    let path = dir.path().join("versions.json");
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(Arc::new(|| Ok(Some(newer())))).with_pins_path(&path);
    u.check_now(&ctx);
    settle(&mut u, &ctx);
    assert!(!u.prompt_open());
    Pins::default().save(&path).unwrap();
    u.check_now(&ctx);
    settle(&mut u, &ctx);
    assert!(u.prompt_open());
    assert_eq!(u.managed(), None);
}

#[test]
fn an_unreadable_choice_from_a_newer_launcher_also_holds_updates() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("versions.json");
    std::fs::write(&path, r#"{"apps":{"photocraft":{"Beta":1}}}"#).unwrap();
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(Arc::new(|| Ok(Some(newer())))).with_pins_path(&path);
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    assert!(!u.prompt_open());
    assert_eq!(u.managed(), Some(&Choice::Unknown));
}

#[test]
fn another_apps_pin_does_not_hold_this_one() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("versions.json");
    let mut pins = Pins::default();
    pins.set("pdfcraft", &crate::pins::VersionTrack::Pinned("v0.2.0".into()));
    pins.save(&path).unwrap();
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(Arc::new(|| Ok(Some(newer())))).with_pins_path(&path);
    u.check_at_start(&ctx);
    settle(&mut u, &ctx);
    assert!(u.prompt_open());
}

#[test]
fn a_prompt_opened_before_the_pin_cannot_install_the_stable_release() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("versions.json");
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(Arc::new(|| Ok(Some(newer())))).with_pins_path(&path);
    u.offer(newer());
    assert!(u.prompt_open());
    let mut pins = Pins::default();
    pins.set(PHOTOCRAFT.id, &crate::pins::VersionTrack::Pinned("v0.13.0".into()));
    pins.save(&path).unwrap();
    u.request_install(&ctx, true);
    assert!(u.install.is_none(), "nothing installs once the launcher pinned the app");
    assert!(!u.prompt_open());
}

#[test]
fn a_check_after_the_pin_closes_a_prompt_opened_before_it() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("versions.json");
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(Arc::new(|| Ok(Some(newer())))).with_pins_path(&path);
    u.offer(newer());
    let mut pins = Pins::default();
    pins.set(PHOTOCRAFT.id, &crate::pins::VersionTrack::Nightly);
    pins.save(&path).unwrap();
    u.check_now(&ctx);
    settle(&mut u, &ctx);
    assert!(!u.prompt_open());
    assert_eq!(u.status(), &Status::UpToDate);
}

#[test]
fn a_running_install_keeps_its_prompt_when_the_app_is_pinned() {
    let dir = pins_with(crate::pins::VersionTrack::Pinned("v0.13.0".into()));
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_pins_path(dir.path().join("versions.json"));
    u.prompt = Some(StartupPrompt::new(&PHOTOCRAFT, newer(), "0.13.0"));
    let (_tx, rx) = channel();
    u.install = Some((rx, newer()));
    assert!(u.read_choice());
    assert!(u.prompt_open(), "tick reports the running install through the prompt");
}

#[test]
fn a_pin_made_while_the_check_runs_still_holds_its_answer() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("versions.json");
    let during = path.clone();
    // The check itself pins the app before answering, as the launcher could mid-check.
    let source: CheckSource = Arc::new(move || {
        let mut pins = Pins::default();
        pins.set(PHOTOCRAFT.id, &crate::pins::VersionTrack::Pinned("v0.13.0".into()));
        pins.save(&during)?;
        Ok(Some(newer()))
    });
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(source).with_pins_path(&path);
    u.check_now(&ctx);
    settle(&mut u, &ctx);
    assert!(!u.prompt_open());
    assert_eq!(u.status(), &Status::UpToDate);
}

#[test]
fn every_answer_after_a_pin_made_mid_check_reports_the_launchers_choice() {
    let ctx = egui::Context::default();
    let answers: [Checked; 2] = [Ok(None), Err("offline".into())];
    for answer in answers {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("versions.json");
        let during = path.clone();
        let source: CheckSource = Arc::new(move || {
            let mut pins = Pins::default();
            pins.set(PHOTOCRAFT.id, &crate::pins::VersionTrack::Nightly);
            pins.save(&during)?;
            answer.clone()
        });
        let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(source).with_pins_path(&path);
        u.check_now(&ctx);
        settle(&mut u, &ctx);
        assert_eq!(u.status(), &Status::UpToDate);
        assert_eq!(u.managed(), Some(&Choice::Managed(crate::pins::VersionTrack::Nightly)));
    }
}
