//! Stories of the screen picker: its pattern and the capture window and view it shows.

use super::capture::{CaptureView, CaptureWindow};
use super::{LOUPE_ZOOM, ScreenColorPicker, ScreenPick, Snapshot, StillCapture, capture};
use craft_platform::screen::{Capture, ScreenImage};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui, pos2, vec2};
use std::sync::{Arc, Mutex};

/// A monitor of `size` pixels at `origin`, a gradient that tells its pixels apart.
fn monitor(origin: [i32; 2], size: [u32; 2], tag: u8) -> ScreenImage {
    let rgba = (0..size[1]).flat_map(|y| (0..size[0]).flat_map(move |x| [(x * 4) as u8, (y * 4) as u8, tag, 255])).collect();
    ScreenImage { origin, size, scale: 1.0, rgba: Arc::new(rgba), icc_profile: None, name: format!("monitor {tag}") }
}

/// The story's pick: story values are `Clone + PartialEq + Sync`, which `ScreenPick` is not.
#[derive(Clone)]
struct SharedPick(Arc<Mutex<ScreenPick>>);

impl PartialEq for SharedPick {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Shows the capture windows of a pick over a small monitor while the story's `open` value is on; the pick's end turns
/// it off.
fn captures(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    if !*data.value("open", true) {
        return StoryOutput::default();
    }
    let shared = data.value("pick", SharedPick(Arc::new(Mutex::new(ScreenPick::new())))).0.clone();
    let mut pick = shared.lock().unwrap_or_else(|e| e.into_inner());
    if !pick.busy() {
        pick.start("story");
    }
    let image = monitor([0, 0], [96, 72], 1);
    pick.tick_with(ui.ctx(), 1, &move || Box::new(StillCapture::new(Ok(Capture::Images(vec![image.clone()])))));
    ScreenColorPicker::new(&mut pick, "Story").show(ui);
    *data.value("open", true) = pick.busy();
    StoryOutput::default()
}

#[story(ScreenColorPicker<'static>, covers = [Default], drive = Drive::Never)]
fn screen_color_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    captures(ui, data)
}

#[story(CaptureWindow, covers = [Default], drive = Drive::Never)]
fn capture_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let open = data.value("open", true);
    if !*open {
        return StoryOutput::default();
    }
    let image = Snapshot::new(monitor([0, 0], [96, 72], 1)).ok().expect("a valid monitor");
    let [x, y, w, h] = capture::area(&image).cover();
    let rect = Rect::from_min_size(pos2(x, y), vec2(w, h));
    let hints = [Label::from("Click to pick"), Label::from("Scroll to zoom")];
    let shown = CaptureWindow::new(Label::from("Story"), rect).show(ui, |ui, _| {
        let rect = ui.max_rect();
        CaptureView::new("monitor 1", &image, 1, LOUPE_ZOOM.1, hints.clone()).show_at(ui, rect)
    });
    *open = shown.output.closed.is_none();
    match shown.inner {
        Some(output) => StoryOutput::from(output),
        None => StoryOutput::from(shown.output),
    }
}

#[story(CaptureView<'static>, covers = [Default], drive = Drive::Never)]
fn capture_view(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let image = Snapshot::new(monitor([0, 0], [96, 72], 1)).ok().expect("a valid monitor");
    let hints = [Label::from("Click to pick"), Label::from("Scroll to zoom")];
    StoryOutput::from(CaptureView::new("monitor 1", &image, 1, LOUPE_ZOOM.1, hints).show(ui))
}
