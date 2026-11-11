//! A container in an OS window: its placement reports follow the window's facts, and a modal's strips over the title
//! bar stand for the platform's hit test.

use craft_host::target::{Caps, ShellInfo};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::windows::{WindowHit, WindowHost};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::shell::{Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
use craft_ui::testing::layout::Block;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui::testing::windows::NoWindows;
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind};
use egui::{Pos2, Rect, Vec2, ViewportCommand, ViewportId, vec2};
use std::cell::Cell;
use std::sync::Arc;

/// With the platform answering the hit test (a sink installed), the strips a modal shows over the title bar join the
/// window's exclusive list as drag areas, as the bar's band does: the bar beside the dialog still moves the window,
/// through the platform.
#[trial]
fn a_modals_strip_stands_for_the_hit_sink() {
    let shell = ShellInfo::default();
    let shell = shell.clone().caps(shell.caps.union(Caps::OS_RESIZES_UNDECORATED));
    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let windows = Windows::with(InstallSpec::new(shell, Arc::new(NoWindows)).frame(Decision { frame, overridden: None, desktop }));
    let host = WindowHost::of(&windows.ctx);
    let _sink = host.install_hit_sink(ViewportId::ROOT);
    let (bar, dialog) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
    let at = Pos2::new(300.0, 4.0);
    let frame = |events: Vec<egui::Event>| {
        windows
            .frame(events, vec![], |ui| {
                bar.set(TitleBar::new().show_titled(ui, Title::new("Landscape.psd"), |_, _| {}, |_, _| {}).response.rect);
                let mut open = true;
                let shown = Dialog::new("Grid options")
                    .first_place(at)
                    .opened(Bind::bound(&mut open))
                    .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
                    .show(ui, |ui, _| {
                        Block::new(Vec2::new(120.0, 40.0)).show(ui);
                    });
                dialog.set(shown.output.response.rect);
            })
            .commands
    };
    // The strips read the last complete pass: the first publishes the bar's drag areas, the second shows the strips.
    frame(vec![]);
    frame(vec![]);
    assert!(dialog.get().intersects(bar.get()) && dialog.get().left() > bar.get().left(), "the dialog lies over the bar");
    let beside = Pos2::new((bar.get().left() + dialog.get().left()) / 2.0, bar.get().center().y);
    let regions = host.caption_regions(ViewportId::ROOT).expect("the bar registered its parts");
    assert_eq!(regions.hit(beside), Some(WindowHit::Move), "the strip beside the dialog is a drag area for the sink: {regions:?}");
    let mut sent = Vec::new();
    for events in [vec![moved(beside)], vec![press(beside)], vec![moved(beside + vec2(20.0, 0.0))], vec![release(beside + vec2(20.0, 0.0))]] {
        sent.extend(frame(events));
    }
    assert!(sent.iter().all(|(_, command)| *command != ViewportCommand::StartDrag), "the platform moves the window, not the app: {sent:?}");
}
