//! A menu bar in a title bar's leading region takes the room its region declares.

use craft_ui::contract::Show as _;
use craft_ui::menu::{MenuBar, MenuTitle};
use craft_ui::runtime::InstallSpec;
use craft_ui::shell::{Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
use craft_ui::testing::windows::NoWindows;
use craft_ui::testing::{Fixture, trial};
use craft_host::target::ShellInfo;
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind};
use egui::{Rect, Ui, vec2};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

const NAMES: [&str; 10] = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];

/// The rect the menu bar got in a title bar `width` wide, with `declared` as the width its leading region declares.
fn bar_rect(width: f32, declared: Option<f32>) -> Rect {
    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    // The fixture's frame is installed while it lives.
    let _windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).frame(Decision { frame, overridden: None, desktop }));
    let seen = Rc::new(Cell::new(Rect::NOTHING));
    let show = {
        let seen = Rc::clone(&seen);
        move |ui: &mut Ui| {
            let slot = Rc::clone(&seen);
            TitleBar::new().leading_width(declared).show_titled(
                ui,
                Title::new("ComfyUI_00743.png"),
                move |ui: &mut Ui, _| {
                    let titles: Vec<MenuTitle> = NAMES.iter().map(|name| MenuTitle::new(*name, |_ui: &mut Ui| {})).collect();
                    slot.set(MenuBar::new("menu", titles).own_width(declared).show(ui).response.rect);
                },
                |_, _| {},
            );
        }
    };
    let mut fixture = Fixture::new(show);
    fixture.room(vec2(width, 800.0));
    fixture.settle();
    seen.get()
}

/// A menu bar in a title bar's leading region takes the room it declares without an empty region before it:
/// without a declaration the bar fills the line and its last titles cannot fit (#the window and help menus
/// fell into the overflow menu).
#[trial]
fn a_declared_leading_width_is_the_menu_bars_room() {
    let undeclared = bar_rect(1600.0, None);
    let narrow = bar_rect(1600.0, Some(380.0));
    assert!(undeclared.width() > 1500.0, "no declaration: the line less the padding, {}", undeclared.width());
    // The bar starts at the leading edge without an empty region before it, taking its declared width instead of
    // filling the rest of the line.
    let offset = (narrow.min.x - undeclared.min.x).abs();
    assert!(offset < 1.0, "the bar starts at the leading edge without an empty region before it, {offset}");
    assert!(narrow.width() <= 380.0 + 1.0, "the bar keeps its declared width, {}", narrow.width());
}
