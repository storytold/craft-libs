//! Local plate check of the flattener: every plate-test scene written, flattened with
//! [`flatten_page`] at 300 dpi, separated with Ghostscript and judged against the scene, with no
//! transparency left in the flattened page. Run it with `scripts/plate-check.sh --flatten`; it
//! never runs in CI.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plate_test::reference::{self, Context};
use craft_plate_test::report::{self, Status};
use craft_plate_test::{gs, scene};
use craft_prepress::flatten::{FlattenPageOptions, flatten_page};

#[test]
#[ignore = "needs Ghostscript; run scripts/plate-check.sh --flatten"]
fn every_scene_separates_as_expected_flattened() {
    let Some(version) = gs::version() else {
        println!("note: Ghostscript ({:?}) not found; set {} or install gs", gs::program(), gs::GS_ENV);
        return;
    };
    let scenes = scene::load_dir(&scene::scenes_dir()).unwrap();
    let ctx = Context::from_env(&scenes);
    if ctx.font.is_none() {
        println!("note: {} not set; text points are skipped", reference::FONT_DIR_ENV);
    }
    let opts = FlattenPageOptions::default();
    let flatten = |pdf: &[u8]| flatten_page(pdf, 0, &opts).map(|(flat, _)| flat).map_err(|e| e.to_string());
    let report = report::run_flattened(&ctx, version, &flatten);
    let path = report::write(&report, &report::output_dir().join("flattened")).unwrap();
    for r in &report.scenes {
        let pending = r.pending.as_deref().map(|t| format!(" (pending {t})")).unwrap_or_default();
        let flat = r.flattened.as_ref().map(|f| format!(", flattened {:?} in {:.0} ms", f.status, f.millis)).unwrap_or_default();
        println!("{} {:?}{flat}{pending}", r.id, r.status);
    }
    println!("report: {}", path.display());
    let wrong = report.unexpected();
    assert!(wrong.is_empty(), "see {}:\n{}", path.display(), wrong.join("\n"));
    assert!(report.scenes.iter().any(|r| r.flattened.as_ref().is_some_and(|f| f.status == Status::Pass)));
}
