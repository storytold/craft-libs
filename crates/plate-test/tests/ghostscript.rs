//! Local plate check: separates every scene with Ghostscript and judges its sample points.
//! Run it with `scripts/plate-check.sh`; it never runs in CI.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plate_test::reference::{self, Context};
use craft_plate_test::report::{self, Status};
use craft_plate_test::{gs, scene};

#[test]
#[ignore = "needs Ghostscript; run scripts/plate-check.sh"]
fn every_scene_separates_as_expected() {
    let Some(version) = gs::version() else {
        println!("note: Ghostscript ({:?}) not found; set {} or install gs", gs::program(), gs::GS_ENV);
        return;
    };
    let scenes = scene::load_dir(&scene::scenes_dir()).unwrap();
    let ctx = Context::from_env(&scenes);
    if ctx.font.is_none() {
        println!("note: {} not set; text points are skipped", reference::FONT_DIR_ENV);
    }
    let report = report::run(&ctx, version);
    let path = report::write(&report, &report::output_dir()).unwrap();
    for result in &report.scenes {
        let pending = result.pending.as_deref().map(|t| format!(" (pending {t})")).unwrap_or_default();
        println!("{} {:?}{pending}", result.id, result.status);
    }
    println!("report: {}", path.display());
    let wrong = report.unexpected();
    assert!(wrong.is_empty(), "see {}:\n{}", path.display(), wrong.join("\n"));
    assert!(report.scenes.iter().any(|r| r.status == Status::Pass));
}
