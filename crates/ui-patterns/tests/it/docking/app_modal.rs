//! While the app's own egui code shows a modal (`FrameInput::app_modal`), a panel floated into a window of its own takes
//! no press and no key, as under a framework app-modal layer.

use craft_behavior::dock::{Floating, Layout, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui_test_kit::windows::Windows;
use egui::{Event, Key, Modifiers, Pos2, Rect, Ui, ViewportId, ViewportInfo};
use std::sync::Arc;

const MAIN_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(900.0, 700.0) };
const FLOAT_AT: Rect = Rect { min: Pos2::new(1000.0, 150.0), max: Pos2::new(1300.0, 350.0) };
/// A point of the float's body, in its window's coordinates.
const IN_BODY: Pos2 = Pos2::new(150.0, 120.0);

#[derive(PlainData)]
struct Specs;

impl PanelSpecs<u32> for Specs {
    fn spec(&self, _: &u32) -> PanelSpec {
        PanelSpec::new("Palette").host_body()
    }
}

/// The float's body: records whether a press or a key reached the egui input of a window of its own.
#[derive(Default)]
struct Probe {
    reached: bool,
}

impl DockPanels<u32> for Probe {
    fn body(&mut self, _: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if ui.ctx().viewport_id() == ViewportId::ROOT {
            return;
        }
        let pressed = |event: &Event| matches!(event, Event::PointerButton { pressed: true, .. } | Event::Key { pressed: true, .. });
        self.reached |= ui.ctx().input(|input| input.events.iter().any(pressed));
    }
}

fn info(rect: Rect) -> ViewportInfo {
    ViewportInfo {
        focused: Some(true),
        inner_rect: Some(rect),
        outer_rect: Some(rect),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    }
}

#[trial]
fn a_native_float_takes_no_press_or_key_while_the_apps_own_modal_shows() {
    for modal in [None, Some(ViewportId::ROOT)] {
        let windows = Windows::new();
        windows.root_info(info(MAIN_AT));
        windows.child_info(info(FLOAT_AT));
        let float = Floating::single(100, vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), FLOAT_AT.size()));
        let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![float])));
        let (specs, mut probe) = (Tracked::new(Specs), Probe::default());
        let mut frame = |child: Vec<Event>, probe: &mut Probe| {
            windows.frame(vec![], child, |ui| {
                let config = DockConfig::default().native_windows(true);
                let _ = DockArea::new("Dock", Bind::bound(&mut workspace), &specs, probe, config).show(ui);
            });
        };
        for _ in 0..3 {
            frame(vec![moved(IN_BODY)], &mut probe);
        }
        windows.app_modal(modal);
        frame(vec![], &mut probe);
        probe.reached = false;
        frame(vec![press(IN_BODY)], &mut probe);
        frame(vec![release(IN_BODY)], &mut probe);
        frame(vec![key_press(Key::A, Modifiers::NONE)], &mut probe);
        assert_eq!(probe.reached, modal.is_none(), "app modal in {modal:?}: a press or key reached the floating window");
    }
}
