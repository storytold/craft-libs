//! The in-window menu bar in an app's title bar: every title shows while the bar has room for them, the document title
//! takes the rest of the bar, and only a bar too narrow for every title folds the last into "…". In a `Stack`
//! narrower than its titles the bar folds the same way and draws nothing past the stack's edge. In any room the bar
//! reports the width its titles take side by side.

use std::cell::Cell;
use std::rc::Rc;

use craft_host::target::{Os, WindowChrome};
use craft_menu::{CommandRef, MenuBar, MenuBarUi, Row, Target, from_items};
use craft_ui::contract::{Role, Show as _, ShowContainer as _, Tracked};
use craft_ui::display::Picture;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

const MENUS: [&str; 10] = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];
const PREFERRED: [(&str, char); 10] = [
    ("File", 'F'),
    ("Edit", 'E'),
    ("Image", 'I'),
    ("Layer", 'L'),
    ("Type", 'Y'),
    ("Select", 'S'),
    ("Filter", 'T'),
    ("View", 'V'),
    ("Window", 'W'),
    ("Help", 'H'),
];
const DOCUMENT: &str = "Untitled-1.psd";

fn model() -> Tracked<MenuBar> {
    let rows: Vec<Row> = MENUS.iter().map(|menu| Row::new(&[menu], CommandRef::new(format!("{}.first", menu.to_lowercase())), "First")).collect();
    Tracked::new(from_items(&MENUS, &rows))
}

fn brand() -> egui::ColorImage {
    egui::ColorImage::filled([128, 128], egui::Color32::RED)
}

/// An app window `width` points wide at `pixels_per_point` whose frame the app draws (Windows), with the title bar as
/// PhotoCraft lays it out: a brand picture, the menu bar, then the document title, and the captions at the end.
fn window(width: f32, pixels_per_point: f32) -> Fixture {
    let bar = Rc::new(model());
    let mut fixture = Fixture::new(move |ui| {
        let bar = Rc::clone(&bar);
        AppShell::new(WindowKey::main())
            .title_bar(
                |_| None,
                move |ui, _, _: &mut ()| {
                    let bar = Rc::clone(&bar);
                    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
                    TitleBar::new().frame(Frame::Integrated(Captions { family: CaptionFamily::Windows, layout })).show_titled(
                        ui,
                        Title::new(DOCUMENT),
                        move |ui, _| {
                            Picture::new("", 7, &brand).show(ui);
                            MenuBarUi::new(&bar).target(Target::Windows).mnemonics(&PREFERRED).help_search(true).show(ui);
                        },
                        |_, _| {},
                    );
                },
            )
            .body(|ui, _, _| {
                ui.label("Document");
            })
            .show(ui, &mut ());
    });
    fixture.os(Os::Windows).chrome(WindowChrome::Integrated).pixels_per_point(pixels_per_point).room(vec2(width, 600.0)).settle();
    fixture
}

/// The bar's title nodes by name.
fn titles(fixture: &Fixture) -> Vec<String> {
    let tree = fixture.tree();
    fixture.by_role(Role::MENU_ITEM).iter().filter_map(|title| tree.name_of(title.node).map(str::to_owned)).collect()
}

/// Where `text` was painted, and whether it was cut.
fn painted(fixture: &Fixture, text: &str) -> Option<(egui::Rect, bool)> {
    fixture.texts().iter().find(|run| run.text == text).map(|run| (run.rect, run.elided))
}

#[trial]
fn every_menu_title_and_the_document_title_show_in_a_wide_title_bar() {
    for pixels_per_point in [1.0, 1.25] {
        for width in [960.0, 1280.0, 1520.0] {
            let fixture = window(width, pixels_per_point);
            let at = format!("{width} pt at {pixels_per_point}x");
            assert_eq!(titles(&fixture), MENUS, "{at} shows every title and no \"…\"");
            let (help, _) = painted(&fixture, "Help").unwrap_or_else(|| panic!("{at}: Help is painted"));
            let (title, cut) = painted(&fixture, DOCUMENT).unwrap_or_else(|| panic!("{at}: the document title is painted"));
            assert!(title.left() > help.right(), "{at}: the document title {title:?} lies after Help {help:?}");
            assert!(!cut, "{at}: the document title is whole");
        }
    }
}

#[trial]
fn a_narrow_title_bar_folds_the_last_menus_into_more() {
    for pixels_per_point in [1.0, 1.25] {
        let mut fixture = window(400.0, pixels_per_point);
        let shown = titles(&fixture);
        assert_eq!(shown.last().map(String::as_str), Some("…"), "{pixels_per_point}x: {shown:?}");
        assert!(!shown.iter().any(|title| title == "Help"), "{pixels_per_point}x: Help is not a title: {shown:?}");
        let more = fixture.by_label("…").expect("the \"…\" title");
        fixture.click(&more).settle();
        assert!(fixture.by_label("Help").is_some(), "{pixels_per_point}x: Help is a row of the \"…\" menu");
    }
}

#[trial]
fn a_bar_in_a_stack_narrower_than_its_titles_folds_them_and_draws_nothing_past_its_edge() {
    let bar = Rc::new(model());
    let stack = Rc::new(Cell::new(egui::Rect::NOTHING));
    let seen = Rc::clone(&stack);
    let mut fixture = Fixture::new(move |ui| {
        let bar = Rc::clone(&bar);
        let shown = Stack::new().show(ui, |ui, _| {
            MenuBarUi::new(&bar).target(Target::Windows).show(ui);
        });
        seen.set(shown.output.rect);
    });
    fixture.os(Os::Windows).room(vec2(240.0, 600.0)).settle();
    let shown = titles(&fixture);
    assert_eq!(shown.last().map(String::as_str), Some("…"), "{shown:?}");
    assert!(!shown.iter().any(|title| title == "Help"), "Help is not a title: {shown:?}");
    let edge = stack.get().right();
    for run in fixture.texts().iter().filter(|run| MENUS.contains(&run.text.as_str()) || run.text == "…") {
        assert!(run.rect.right() <= edge + 0.5, "{:?} at {:?} reaches past the stack's edge {edge}", run.text, run.rect);
    }
    let more = fixture.by_label("…").expect("the \"…\" title");
    fixture.click(&more).settle();
    assert!(fixture.by_label("Help").is_some(), "Help is a row of the \"…\" menu");
}

/// The bar reports the width every title takes side by side: in a stack that holds them all it is the titles' width,
/// and in a stack too narrow for them it is the same width while the drawn titles are narrower.
#[trial]
fn the_bar_reports_the_width_every_title_takes_in_a_wide_and_a_narrow_stack() {
    let mut reports = Vec::new();
    for width in [1520.0, 240.0] {
        let bar = Rc::new(model());
        let seen = Rc::new(Cell::new((0.0, 0.0)));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let bar = Rc::clone(&bar);
            let out = Rc::clone(&out);
            Stack::new().show(ui, move |ui, _| {
                let shown = MenuBarUi::new(&bar).target(Target::Windows).show(ui);
                out.set((shown.natural, shown.titles.width()));
            });
        });
        fixture.os(Os::Windows).room(vec2(width, 600.0)).settle();
        reports.push(seen.get());
    }
    let [(wide, wide_titles), (narrow, narrow_titles)] = [reports[0], reports[1]];
    assert!(wide > 0.0, "the bar reports a width");
    assert!((wide - wide_titles).abs() < 0.5, "with room for every title the natural width {wide} is the titles' width {wide_titles}");
    assert!((narrow - wide).abs() < 0.5, "a narrow bar reports the same natural width {narrow} as a wide one {wide}");
    assert!(narrow_titles < narrow, "a narrow bar draws titles {narrow_titles} narrower than its natural width {narrow}");
}
