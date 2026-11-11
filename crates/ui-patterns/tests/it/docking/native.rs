//! Native dock windows: when a float gets an OS window, what the dock records when the OS moves it, and the OS close.

use craft_behavior::dock::{Action, Floating, Layout, Permissions, Workspace};
use craft_behavior::placement::Placement;
use craft_host::target::{Backend, Caps, Os, ShellInfo};
use craft_ui::contract::{Bind, BodyCx};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::InstallSpec;
use craft_ui::testing::trial;
use craft_ui::testing::windows::FakeSystem;
use craft_ui_test_kit::windows::Windows;
use egui::{Pos2, Rect, Ui, Vec2, ViewportEvent, ViewportId, ViewportInfo};
use std::sync::Arc;

/// The root window and the palette overlap; the palette is the larger.
const ROOT_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(300.0, 300.0) };
const PALETTE_AT: Rect = Rect { min: Pos2::new(200.0, 150.0), max: Pos2::new(600.0, 450.0) };
const PALETTE_TITLE: &str = "Tools";

/// Palette panels with the given rights.
struct Palettes(Permissions);

impl DockPanels<u32> for Palettes {
    fn spec(&self, _: &u32) -> PanelSpec {
        PanelSpec::new(PALETTE_TITLE)
    }

    fn permissions(&self, _: &u32) -> Permissions {
        self.0
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
            let shown = DockArea::new("Native dock", Bind::bound(workspace), &mut Palettes(Permissions::ALL), config).show(ui);
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

/// A float made in the app window, never shown in an OS window: the dock opens it native over its place there, where
/// the host opens OS windows and reports where they are, and records where the OS put it. On Wayland, which reports
/// no positions, it shows in the app window.
#[trial]
fn a_float_opens_an_os_window_where_the_host_reports_window_positions() {
    let opened = |windows: Windows| {
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
        let (mut child, mut recorded) = (false, false);
        for _ in 0..3 {
            let out = windows.frame(vec![], vec![], |ui| {
                let shown =
                    DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui);
                recorded |= shown.actions.iter().any(|action| matches!(action, Action::SetNative { .. }));
            });
            child |= out.builders.iter().any(|(viewport, _)| *viewport != ViewportId::ROOT);
        }
        (child, recorded)
    };
    assert_eq!(opened(Windows::new()), (true, true), "an OS window opens and its placement is recorded");
    let wayland = ShellInfo::default().backend(Backend::Wayland);
    assert_eq!(opened(Windows::with(InstallSpec::new(wayland, Arc::new(FakeSystem::new())))), (false, false), "Wayland: in the app window");
}

/// A native dock window's own title row is its only title bar, on macOS and elsewhere, with no frame decision
/// installed: the OS title bar and its buttons are hidden.
#[trial]
fn a_native_dock_window_shows_no_os_title_bar() {
    let builder = |os: Os| {
        let windows = Windows::with(InstallSpec::new(ShellInfo::default().os(os).caps(Caps::native(os)), Arc::new(FakeSystem::new())));
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
        let out = windows.frame(vec![], vec![], |ui| {
            DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui);
        });
        out.builders.into_iter().find(|(viewport, _)| *viewport != ViewportId::ROOT).map(|(_, builder)| builder).expect("an OS window opens")
    };
    let mac = builder(Os::Mac);
    assert_eq!((mac.titlebar_shown, mac.titlebar_buttons_shown, mac.fullsize_content_view), (Some(false), Some(false), Some(true)), "macOS");
    for os in [Os::Linux, Os::Windows] {
        assert_eq!(builder(os).decorations, Some(false), "{os:?}");
    }
}

/// A native float's OS window opens at the float's own size, not at what its content measured.
#[trial]
fn a_native_float_opens_at_its_own_size() {
    let windows = Windows::new();
    windows.root_info(info(true, ROOT_AT));
    let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
    let mut sizes = Vec::new();
    for _ in 0..3 {
        let out = windows.frame(vec![], vec![], |ui| {
            DockArea::new("Native dock", Bind::bound(&mut workspace), &mut Palettes(Permissions::ALL), DockConfig::default()).show(ui);
        });
        sizes.extend(out.builders.into_iter().filter(|(viewport, _)| *viewport != ViewportId::ROOT).map(|(_, builder)| builder.inner_size));
    }
    let size = float().rect.size();
    assert!(!sizes.is_empty() && sizes.iter().all(|shown| *shown == Some(size)), "{size:?} asked, {sizes:?} opened");
}

/// The OS close of a native float closes its window when every panel may close, and leaves it open otherwise.
#[trial]
fn the_os_close_of_a_native_float_closes_it_when_its_panels_may_close() {
    let closed = |rights: Permissions| {
        let windows = Windows::new();
        windows.root_info(info(true, ROOT_AT));
        windows.child_info(info(true, PALETTE_AT));
        let mut workspace = Workspace::new(Layout::new(None, vec![float()]));
        let run = |workspace: &mut Workspace<u32>| {
            let mut actions = Vec::new();
            windows.frame(vec![], vec![], |ui| {
                actions.extend(DockArea::new("Native dock", Bind::bound(workspace), &mut Palettes(rights), DockConfig::default()).show(ui).actions);
            });
            actions.iter().any(|action| matches!(action, Action::CloseWindow { .. }))
        };
        for _ in 0..3 {
            run(&mut workspace);
        }
        windows.child_info(ViewportInfo { events: vec![ViewportEvent::Close], ..info(true, PALETTE_AT) });
        let mut closed = run(&mut workspace);
        windows.child_info(info(true, PALETTE_AT));
        for _ in 0..2 {
            closed |= run(&mut workspace);
        }
        closed
    };
    assert!(closed(Permissions::ALL), "a closable palette's window closes");
    assert!(!closed(Permissions::ALL.closable(false)), "a window with a panel that may not close stays");
}

/// A palette float made in the app window.
fn float() -> Floating<u32> {
    Floating::single(vec![1], 0, Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(300.0, 200.0)))
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
