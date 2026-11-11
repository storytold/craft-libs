//! Native dock windows: what the dock records when the OS moves its window.

use craft_behavior::dock::{Action, Floating, Layout, Permissions, Workspace};
use craft_behavior::placement::Placement;
use craft_ui::contract::{Bind, BodyCx};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::trial;
use craft_ui_test_kit::windows::Windows;
use egui::{Pos2, Rect, Ui, Vec2, ViewportInfo};

/// The root window and the palette overlap; the palette is the larger.
const ROOT_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(300.0, 300.0) };
const PALETTE_AT: Rect = Rect { min: Pos2::new(200.0, 150.0), max: Pos2::new(600.0, 450.0) };
const PALETTE_TITLE: &str = "Tools";

struct Palettes;

impl DockPanels<u32> for Palettes {
    fn spec(&self, _: &u32) -> PanelSpec {
        PanelSpec::new(PALETTE_TITLE)
    }

    fn permissions(&self, _: &u32) -> Permissions {
        Permissions::ALL
    }

    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// The dock records a move the OS made to its native window, and nothing while the OS reports what the model holds.
#[trial]
fn the_dock_records_where_the_os_put_its_native_window() {
    let windows = Windows::new();
    windows.root_info(info(true, ROOT_AT));
    let mut window = Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(300.0, 200.0)));
    let mut saved = Placement::new(PALETTE_AT.size().into(), 1.0);
    saved.position_px = Some([200, 150]);
    window.native = Some(saved);
    let mut workspace = Workspace::new(Layout::new(None, vec![window]));
    let run = |workspace: &mut Workspace<u32>| {
        let mut actions = Vec::new();
        windows.frame(vec![], vec![], |ui| {
            let config = DockConfig::default().native_windows(true);
            let shown = DockArea::new("Native dock", Bind::bound(workspace), &mut Palettes, config).show(ui);
            actions.extend(shown.actions);
        });
        actions
    };
    windows.child_info(info(true, PALETTE_AT));
    for _ in 0..3 {
        run(&mut workspace);
    }
    let rest = run(&mut workspace);
    assert!(rest.iter().all(|action| !matches!(action, Action::SetNative { .. })), "at rest: {rest:?}");
    let moved = PALETTE_AT.translate(Vec2::new(-120.0, 40.0));
    windows.child_info(info(true, moved));
    let after = run(&mut workspace);
    let recorded = |action: &Action<u32>| matches!(action, Action::SetNative { placement: Some(p), .. } if p.position_px == Some([80, 190]));
    assert!(after.iter().any(recorded), "the OS move is recorded: {after:?}");
}

fn info(focused: bool, rect: Rect) -> ViewportInfo {
    ViewportInfo {
        focused: Some(focused),
        inner_rect: Some(rect),
        outer_rect: Some(rect),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    }
}
