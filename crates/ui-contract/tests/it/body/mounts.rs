//! The mount host: a viewport mount saves its window placement and drops a stale close request.

use craft_host::target::ShellInfo;
use craft_ui::contract::SaveKey;
use craft_ui::contract::{ContainerProps, Mount, Overlay, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::runtime::InstallSpec;
use craft_ui::testing::layout::Block;
use craft_ui::testing::trial;
use craft_ui::testing::windows::NoWindows;
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::windows::Windows;
use craft_window::restore::Store;
use egui::{Pos2, Rect, Ui, Vec2, ViewportEvent, ViewportInfo};
use std::cell::Cell;
use std::sync::{Arc, Mutex};

/// The inner size of the exported window, in points.
const INNER: [f32; 2] = [420.0, 310.0];
/// The body the layers show.
const BODY: Vec2 = Vec2::new(120.0, 60.0);

/// A viewport mount's window placement goes to the placement store under the container's save key, and the next
/// run opens the window with it.
#[trial]
fn a_viewport_mount_saves_its_window_placement_in_the_store() {
    let store = Arc::new(Mutex::new(Store::in_memory()));
    let spec = InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).placement(store.clone());
    let windows = Windows::with(spec);
    windows.child_info(ViewportInfo {
        inner_rect: Some(Rect::from_min_size(Pos2::new(120.0, 90.0), Vec2::from(INNER))),
        outer_rect: Some(Rect::from_min_size(Pos2::new(118.0, 60.0), Vec2::new(424.0, 342.0))),
        native_pixels_per_point: Some(2.0),
        ..ViewportInfo::default()
    });
    for _ in 0..3 {
        windows.frame(vec![], vec![], |ui| {
            let props = ContainerProps::new()
                .mount(Mount::viewport(Overlay::floating()))
                .sizing(Sizing::Content)
                .save(SaveKey::new("craft_ui_tests.export_window"));
            Slab::new(props).label("Export").id_salt("window").show(ui, |ui, _| {
                Block::new(BODY).show(ui);
            });
        });
    }
    let saved = store.lock().unwrap().get("craft_ui_tests.export_window").expect("the window's placement was saved");
    assert_eq!(saved.normal.size, INNER, "the inner size in points");
    assert_eq!(saved.position_px, Some([236, 120]), "the outer position in physical pixels");
}

/// A close request the container did not take in its frame (it was shown in a parent's sizing pass, which runs no
/// update step) is dropped: showing the container in later frames does not close it.
#[trial]
fn a_close_request_not_taken_this_frame_does_not_close_the_container_later() {
    let windows = Windows::new();
    let ran = Cell::new(0);
    let show = |ui: &mut Ui| {
        let props = ContainerProps::new().mount(Mount::viewport(Overlay::floating())).sizing(Sizing::Content);
        Slab::new(props).label("Export").id_salt("window").show(ui, |ui, _| {
            ran.set(ran.get() + 1);
            Block::new(BODY).show(ui);
        });
    };
    let frame = || {
        windows.frame(vec![], vec![], |ui| show(ui));
    };
    for _ in 0..3 {
        frame();
    }
    windows.child_info(ViewportInfo { events: vec![ViewportEvent::Close], ..ViewportInfo::default() });
    windows.frame(vec![], vec![], |ui| {
        let mut sizing = ui.new_child(egui::UiBuilder::new().sizing_pass().invisible());
        show(&mut sizing);
    });
    windows.child_info(ViewportInfo::default());
    for _ in 0..3 {
        frame();
    }
    let before = ran.get();
    for _ in 0..3 {
        frame();
    }
    assert!(ran.get() > before, "the container stayed open: the stale request did not close it");
}
