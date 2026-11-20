//! A menu with several openers: a context menu over a popover.

use craft_behavior::IconId;
use craft_ui::buttons::{Button, IconButton, PopupMark};
use craft_ui::contract::{Bind, PopupKind, Props as _, Role, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::{self, CommandSet as _, EditCommands, FireSource};
use craft_ui::runtime::env::Direction;
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Rect, Shape, pos2};
use std::cell::Cell;
use std::rc::Rc;

fn menu_rect(fixture: &Fixture) -> Option<egui::Rect> {
    fixture.by_role(Role::MENU).first().map(|menu| menu.rect)
}

/// A context menu opened over the popover it was opened from takes the click on its row: the row runs and the menu
/// closes, and the popover stays open below it.
#[trial]
fn a_menu_opened_from_a_popover_takes_a_click_on_its_row() {
    let ran = Rc::new(Cell::new(false));
    let seen = ran.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Brush").show(ui);
        let mut open = true;
        Popover::new("Brush options", Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
            let target = Button::new("Target").show(ui);
            Button::new("Reset").show(ui);
            Button::new("Swap").show(ui);
            Menu::context("Target menu", &target).show(ui, |ui, _| {
                if MenuItem::new("Delete").show(ui).ran {
                    seen.set(true);
                }
            });
        });
    });
    fixture.settle();
    let at = fixture.by_label("Target").expect("the target has a node").center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    let delete = fixture.by_label("Delete").expect("a secondary click on the target opens the menu");
    let popover = fixture.by_role(Role::DIALOG).first().map(|popover| popover.rect).expect("the popover is open");
    let over = popover.intersect(delete.rect);
    assert!(over.is_positive(), "the row lies over the popover");
    let at = over.center();
    assert!(popover.contains(at) && delete.rect.contains(at), "the click point is inside both the popover and the row");
    fixture.click_at(at);
    fixture.settle();
    assert!(ran.get(), "the click ran the row");
    assert!(menu_rect(&fixture).is_none(), "the menu closed");
    assert!(fixture.by_label("Reset").is_some(), "the popover stays open");
}

/// Right-clicks at `at`: move, secondary press, secondary release.
fn right_click(fixture: &mut Fixture, at: egui::Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
}

/// A secondary click on a press menu's button opens its menu, as a primary press does.
#[trial]
fn a_secondary_click_on_a_press_menu_button_opens_its_menu() {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Layers").show(ui);
        Menu::new("Layers menu").opened_by(&opener, Trigger::Press).show(ui, |ui, _| {
            MenuItem::new("New layer").show(ui);
        });
    });
    fixture.settle();
    let at = fixture.by_label("Layers").expect("the button").center();
    right_click(&mut fixture, at);
    assert!(fixture.by_label("New layer").is_some(), "the secondary click opened the menu");
    let menu = menu_rect(&fixture).expect("the menu is open");
    let button = fixture.by_label("Layers").expect("the button").rect;
    assert!(menu.min.y >= button.max.y - 1.0 || menu.max.y <= button.min.y + 1.0, "the menu hangs from the button, not the pointer");
}

/// A right-click elsewhere on the widget while its context menu is open moves the menu to the new point.
#[trial]
fn a_new_right_click_moves_an_open_context_menu_to_the_pointer() {
    let mut fixture = Fixture::new(|ui| {
        let target = Button::new("A wide target to right-click at either end").show(ui);
        Menu::context("Target menu", &target).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
            MenuItem::new("Paste").show(ui);
        });
    });
    fixture.settle();
    let target = fixture.by_label("A wide target to right-click at either end").expect("the target").rect;
    let (first, second) = (target.left_center() + egui::vec2(8.0, 0.0), target.right_center() - egui::vec2(8.0, 0.0));
    right_click(&mut fixture, first);
    let before = menu_rect(&fixture).expect("the first right-click opens the menu");
    assert!((before.min.x - first.x).abs() < 8.0, "the menu opens at the pointer");
    right_click(&mut fixture, second);
    let after = menu_rect(&fixture).expect("the menu is open after the second right-click");
    assert!(after.min.x > before.min.x + 40.0, "the menu moved to the second point: {before:?} then {after:?}");
}

/// A menu request retains its point until an image arrives from the clipboard in a later frame.
#[trial]
fn context_menu_paste_delivers_image_at_the_requested_point() {
    let mut fixture = Fixture::new(|ui| {
        let target = Button::new("Target").show(ui);
        let mut paste = false;
        Menu::context("Target menu", &target).show(ui, |ui, _| {
            if MenuItem::new("Paste").show(ui).ran {
                paste = true;
            }
        });
        if paste {
            commands::request_paste_at(ui.ctx(), egui::pos2(120.0, 180.0));
        }
    });
    fixture.settle();
    let at = fixture.by_label("Target").expect("target").center();
    right_click(&mut fixture, at);
    let paste = fixture.by_label("Paste").expect("paste menu item");
    fixture.click(&paste);
    fixture.frame(vec![egui::Event::PasteImage(std::sync::Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::RED)))]).settle();
    let fired = take_output(fixture.ctx()).fired;
    assert!(
        fired.iter().any(|fired| fired.pasted.as_ref().is_some_and(|content| content.image.is_some()) && fired.paste_at == Some(egui::pos2(120.0, 180.0))),
        "{fired:?}"
    );
}

/// With a context menu open, a paste event (the platform's answer to ⌘V) behind it reaches nothing: no Paste runs
/// and the menu stays. Edit ▸ Paste from a native menu, which no component of the window fires, closes the menu, as a
/// key's command does, and runs.
#[trial]
fn a_paste_behind_a_context_menu_runs_nothing_and_a_native_edit_paste_closes_it() {
    let mut fixture = Fixture::new(|ui| {
        let target = Button::new("Target").show(ui);
        Menu::context("Target menu", &target).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
        });
    });
    fixture.settle();
    let at = fixture.by_label("Target").expect("the target").center();
    right_click(&mut fixture, at);
    assert!(menu_rect(&fixture).is_some(), "the context menu is open");
    let fired = |fixture: &Fixture| take_output(fixture.ctx()).fired.into_iter().map(|fired| fired.command.id).collect::<Vec<_>>();
    fired(&fixture);
    fixture.frame(vec![egui::Event::Paste("clipboard text".to_owned())]).settle();
    assert!(fired(&fixture).is_empty(), "the paste behind the menu ran nothing");
    assert!(menu_rect(&fixture).is_some(), "the menu stays open");
    commands::fire(fixture.ctx(), EditCommands::Paste, FireSource::Menu);
    fixture.settle();
    assert!(menu_rect(&fixture).is_none(), "Edit ▸ Paste closed the menu");
    assert_eq!(fired(&fixture), [EditCommands::Paste.id()], "and ran");
}

/// An icon button that opens a menu hangs the dropdown arrow under the end edge of its glyph, inside the button and
/// clear of the glyph, in either reading direction; a tool group's button shows the corner triangle instead.
#[trial]
fn a_menu_button_hangs_its_arrow_under_the_end_of_its_glyph() {
    const BUTTON: Rect = Rect { min: pos2(20.0, 20.0), max: pos2(44.0, 44.0) };
    for direction in [Direction::Ltr, Direction::Rtl] {
        let at_end = |x: f32| if direction == Direction::Rtl { x < BUTTON.center().x } else { x > BUTTON.center().x };
        for mark in [PopupMark::Menu, PopupMark::ToolGroup] {
            let mut fixture = Fixture::new(move |ui| {
                IconButton::new(IconId::new("plus")).label("Add").popup(PopupKind::Menu).popup_mark(mark).show_at(ui, BUTTON);
            });
            fixture.direction(direction).settle();
            let shapes = fixture.frame_shapes();
            let meshes = |textured: bool| {
                shapes
                    .iter()
                    .filter_map(move |shape| match shape {
                        Shape::Mesh(mesh) if (mesh.texture_id != egui::TextureId::default()) == textured => Some(mesh.calc_bounds()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let glyph = *meshes(true).first().expect("the glyph");
            let arrows: Vec<_> = shapes
                .iter()
                .filter_map(|shape| match shape {
                    Shape::Path(path) if !path.closed => Some((Rect::from_points(&path.points), path.visual_bounding_rect())),
                    _ => None,
                })
                .collect();
            match mark {
                PopupMark::Menu => {
                    let [(points, painted)] = arrows[..] else { panic!("{direction:?}: one arrow, not {arrows:?}") };
                    assert!(BUTTON.contains_rect(painted), "{direction:?}: the arrow {painted:?} leaves the button");
                    assert!(points.top() >= glyph.bottom() - 0.5, "{direction:?}: the arrow {points:?} covers the glyph {glyph:?}");
                    assert!(at_end(points.center().x), "{direction:?}: the arrow {points:?} is not at the end edge");
                    assert!(meshes(false).is_empty(), "{direction:?}: a menu button shows no corner triangle");
                }
                _ => {
                    assert!(arrows.is_empty(), "{direction:?}: a tool group shows no arrow");
                    let triangle = *meshes(false).first().expect("the corner triangle");
                    assert!(BUTTON.contains_rect(triangle) && at_end(triangle.center().x), "{direction:?}: the triangle {triangle:?}");
                }
            }
        }
    }
}
