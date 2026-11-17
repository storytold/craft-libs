use std::ffi::OsString;
use std::path::{Path, PathBuf};

use egui::TexturesDelta;
use egui_kittest::{Harness, HarnessBuilder, TestRenderer};
use image::{Rgba, RgbaImage};

use super::{DEFAULT_DIR_VAR, MAX_NAME_LEN, SnapshotError, dir_from, save, snapshot_dir, with_snapshot_renderer};

/// Draws a fixed 3 × 2 image, fails, or panics, without a GPU.
enum Fake {
    Image,
    Fails,
    Panics,
}

impl TestRenderer for Fake {
    fn handle_delta(&mut self, delta: &mut TexturesDelta) {
        delta.clear();
    }

    fn render(&mut self, _ctx: &egui::Context, _output: &egui::FullOutput) -> Result<RgbaImage, String> {
        match self {
            Self::Image => Ok(RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]))),
            Self::Fails => Err("no adapter".to_owned()),
            Self::Panics => panic!("Failed to create render state"),
        }
    }
}

/// A folder of this test's own under the system temp folder, removed first.
fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-ui-snapshot-{}-{test}", std::process::id()));
    _ = std::fs::remove_dir_all(&dir);
    dir
}

fn harness(builder: HarnessBuilder<()>) -> Harness<'static> {
    builder.with_size([60.0, 30.0]).build_ui(|ui| _ = ui.label("Export"))
}

fn assert_png(path: &Path, width: u32, height: u32) {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{}", path.display());
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
    assert_eq!((image.width(), image.height()), (width, height));
}

#[test]
fn an_unset_or_empty_variable_is_no_folder() {
    assert_eq!(dir_from(None), None);
    assert_eq!(dir_from(Some(OsString::new())), None);
    assert_eq!(dir_from(Some("shots".into())), Some(PathBuf::from("shots")));
    assert_eq!(snapshot_dir("CRAFT_UI_SNAPSHOT_DIR_TEST_UNSET_8F3A"), None);
    assert_eq!(DEFAULT_DIR_VAR, "CRAFT_UI_SNAPSHOT_DIR");
}

#[test]
fn an_unusable_variable_name_is_no_folder() {
    for var in ["", "A=B", "A\0B", "="] {
        assert_eq!(snapshot_dir(var), None, "{var:?}");
    }
}

#[test]
fn the_caller_names_the_variable() {
    // Cargo sets this one for every test run.
    assert_eq!(snapshot_dir("CARGO_MANIFEST_DIR"), Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))));
}

#[test]
fn no_folder_leaves_the_renderer_alone() {
    let builder = with_snapshot_renderer(Harness::builder().renderer(Fake::Image), None).unwrap();
    let mut harness = harness(builder);
    // The fake renderer is still the one drawing: no wgpu renderer replaced it.
    assert_eq!(harness.render().unwrap().dimensions(), (3, 2));
}

#[test]
fn save_writes_one_png_named_after_the_screen() {
    let dir = temp_dir("save").join("nested");
    let mut harness = harness(Harness::builder().renderer(Fake::Image));
    let path = save(&mut harness, &dir, "export-light@2x").unwrap();
    assert_eq!(path, dir.join("export-light@2x.png"));
    assert_png(&path, 3, 2);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

#[test]
fn save_runs_no_frames() {
    let dir = temp_dir("frames");
    let mut harness = Harness::builder().renderer(Fake::Image).build_ui_state(|_, frames: &mut u32| *frames += 1, 0);
    let before = *harness.state();
    save(&mut harness, &dir, "frames").unwrap();
    assert_eq!(*harness.state(), before);
    _ = std::fs::remove_dir_all(dir);
}

#[test]
fn save_refuses_names_that_leave_the_folder() {
    let dir = temp_dir("names");
    let mut harness = harness(Harness::builder().renderer(Fake::Image));
    let long = "a".repeat(MAX_NAME_LEN + 1);
    for name in [
        "", ".", "..", "../up", "a/b", "a\\b", "/abs", "C:x", ".hidden", "con", "NUL", "com1.x", "lpt9", "COM0", "lpt0.png", "a b", "é", "a\0", &long,
    ] {
        assert!(matches!(save(&mut harness, &dir, name), Err(SnapshotError::InvalidName(_))), "{name:?}");
    }
    assert!(!dir.exists());
    assert!(save(&mut harness, &dir, &"a".repeat(MAX_NAME_LEN)).is_ok());
    _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_render_failure_is_an_error_and_writes_nothing() {
    let dir = temp_dir("render");
    for fake in [Fake::Fails, Fake::Panics] {
        let mut harness = harness(Harness::builder().renderer(fake));
        assert!(matches!(save(&mut harness, &dir, "screen"), Err(SnapshotError::Render(_))));
    }
    assert!(!dir.exists());
}

#[test]
fn a_folder_that_cannot_be_made_is_an_error() {
    let dir = temp_dir("blocked");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("file");
    std::fs::write(&file, b"").unwrap();
    let mut harness = harness(Harness::builder().renderer(Fake::Image));
    let error = save(&mut harness, &file.join("sub"), "screen").unwrap_err();
    assert!(matches!(error, SnapshotError::CreateDir { .. }), "{error}");
    _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_folder_sets_up_wgpu_and_saves_the_frame() {
    let dir = temp_dir("wgpu");
    let builder = match with_snapshot_renderer(Harness::builder(), Some(&dir)) {
        Ok(builder) => builder,
        Err(SnapshotError::Render(message)) => {
            eprintln!("skipped: this machine can't render with wgpu ({message})");
            return;
        }
        Err(error) => panic!("{error}"),
    };
    let mut harness = harness(builder);
    harness.run();
    let path = save(&mut harness, &dir, "label").unwrap();
    assert_png(&path, 60, 30);
    _ = std::fs::remove_dir_all(dir);
}
