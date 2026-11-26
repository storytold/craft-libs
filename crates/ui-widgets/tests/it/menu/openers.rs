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

#[trial]
fn a_context_menu_beneath_a_popover_does_not_open_from_its_click() {
    let mut fixture = Fixture::new(|ui| {
        let background = Button::new("Background").show_at(ui, Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 500.0)));
        Menu::context("Background menu", &background).show(ui, |ui, _| {
            MenuItem::new("Background action").show(ui);
        });
        let opener = Button::new("Open popover").show(ui);
        let mut open = true;
        Popover::new("Floating", Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
            let target = Button::new("Floating target").show(ui);
            Menu::context("Floating menu", &target).show(ui, |ui, _| {
                MenuItem::new("Floating action").show(ui);
            });
        });
    });
    fixture.settle();
    let at = fixture.by_label("Floating target").expect("floating target").center();
    right_click(&mut fixture, at);
    assert!(fixture.by_label("Floating action").is_some(), "the floating menu opens");
    assert!(fixture.by_label("Background action").is_none(), "the covered menu stays closed");
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
        fired
            .iter()
            .any(|fired| fired.pasted.as_ref().is_some_and(|content| content.image.is_some()) && fired.paste_at == Some(egui::pos2(120.0, 180.0))),
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

/// Both click and press openers accept a screen reader's activation, not only a pointer press.
#[trial]
fn a_menu_opener_accepts_accessibility_activation() {
    for trigger in [Trigger::Click, Trigger::Press] {
        let mut fixture = Fixture::new(move |ui| {
            let opener = IconButton::new(IconId::new("settings")).label("Options").popup(PopupKind::Menu).show(ui);
            Menu::new("Options menu").opened_by(&opener.response, trigger).show(ui, |ui, _| {
                MenuItem::new("An option").show(ui);
            });
        });
        fixture.settle();
        fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
        let opener = fixture.by_label("Options").expect("the opener");
        fixture.access(craft_ui::testing::focus::AccessActionKind::Click, &opener).settle();
        assert!(fixture.by_label("An option").is_some(), "{trigger:?} opens through accessibility");
        assert!(!craft_ui::runtime::focus::keyboard_in_menu(fixture.ctx()), "an accessibility activation does not inherit an earlier key");
        fixture.key(egui::Key::ArrowDown, egui::Modifiers::NONE).settle();
        assert!(craft_ui::runtime::focus::keyboard_in_menu(fixture.ctx()), "a later key drives the menu");
    }
}

/// A context menu opened near the top edge of the window, taller than the room above its pointer, stays inside the
/// window (it hangs below the pointer, not clipped above it).
#[trial]
fn a_context_menu_near_the_top_stays_inside_the_window() {
    let mut fixture = Fixture::new(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(400.0, 300.0), egui::Sense::click());
        let target = ui.interact(rect, ui.scope_id().with("area"), egui::Sense::click());
        Menu::context("Page menu", &target).show(ui, |ui, _| {
            for name in ["Copy", "Paste", "Select All", "Rotate", "Delete", "Insert", "Extract", "Print", "Properties", "More"] {
                MenuItem::new(name).show(ui);
            }
        });
    });
    fixture.room(egui::vec2(400.0, 300.0));
    fixture.settle();
    right_click(&mut fixture, pos2(150.0, 2.0));
    let menu = menu_rect(&fixture).expect("the menu opened");
    let window = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 300.0));
    assert!(menu.min.y >= window.min.y - 0.5 && menu.max.y <= window.max.y + 0.5, "the menu lies inside the window: {menu:?}");
}

/// Edit ▸ Paste fired at the app reaches it at once with no content by default; an app that reads the clipboard
/// through the board (`CommandInput::paste_reads_clipboard`) gets it with the clipboard's text once the platform
/// answers, as ⌘V's paste does.
#[trial]
fn a_menu_paste_reads_the_clipboard_when_the_app_asks() {
    for reads in [false, true] {
        let mut fixture = Fixture::new(|ui| {
            Button::new("Target").show(ui);
        });
        fixture.commands(commands::CommandInput::new().paste_reads_clipboard(reads)).settle();
        let _ = take_output(fixture.ctx());
        commands::fire(fixture.ctx(), EditCommands::Paste, FireSource::Menu);
        fixture.settle();
        let now = take_output(fixture.ctx()).fired;
        if !reads {
            assert!(now.iter().any(|fired| fired.command.id == EditCommands::Paste.id() && fired.pasted.is_none()), "at once: {now:?}");
            continue;
        }
        assert!(now.is_empty(), "the paste waits for the platform's clipboard: {now:?}");
        fixture.frame(vec![egui::Event::Paste("clipboard text".to_owned())]).settle();
        let later = take_output(fixture.ctx()).fired;
        assert!(
            later.iter().any(|fired| fired.command.id == EditCommands::Paste.id()
                && fired.source == FireSource::Menu
                && fired.pasted.as_ref().and_then(|content| content.text.as_deref()) == Some("clipboard text")),
            "the paste arrives with the clipboard's text: {later:?}"
        );
    }
}

/// Space on a focused icon button that opens a menu opens the menu, as a click does.
#[trial]
fn space_on_a_focused_menu_opener_opens_its_menu() {
    let mut fixture = Fixture::new(|ui| {
        let opener = IconButton::new(IconId::new("menu")).popup(PopupKind::Menu).label("Panel menu").show(ui);
        Menu::new("Panel menu").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("Clear All").show(ui);
        });
    });
    fixture.settle();
    fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
    fixture.key(egui::Key::Space, egui::Modifiers::NONE).settle();
    assert!(fixture.by_label("Clear All").is_some(), "Space on the focused opener opens the menu");
}

/// A menu opened from the keyboard is driven by the keyboard: Space on the focused opener opens it with its first row
/// lit, and Return then runs that row and closes the menu.
#[trial]
fn return_in_a_menu_opened_by_space_runs_its_first_row() {
    let ran = Rc::new(Cell::new(None::<&'static str>));
    let seen = ran.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = IconButton::new(IconId::new("menu")).popup(PopupKind::Menu).label("Panel menu").show(ui);
        Menu::new("Panel menu").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            for row in ["Clear All", "Reset"] {
                if MenuItem::new(row).show(ui).ran {
                    seen.set(Some(row));
                }
            }
        });
    });
    fixture.settle();
    fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
    fixture.key(egui::Key::Space, egui::Modifiers::NONE).settle();
    assert!(fixture.by_label("Clear All").is_some(), "Space on the focused opener opens the menu");
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE).settle();
    assert_eq!(ran.get(), Some("Clear All"), "Return runs the menu's first row");
    assert!(menu_rect(&fixture).is_none(), "running the row closes the menu");
}

/// A menu the pointer opened puts its first row in focus without lighting it; when that row opens a submenu, Return
/// with no row lit only closes the menu and opens no submenu, and with a row lit under the pointer it runs that row.
#[trial]
fn return_in_a_pointer_opened_menu_whose_first_row_opens_a_submenu() {
    let ran = Rc::new(Cell::new(None::<&'static str>));
    let seen = ran.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Main").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            Menu::submenu_row(ui, "Recent", |ui, _| {
                MenuItem::new("Report.txt").show(ui);
            });
            for row in ["Clear All", "Reset"] {
                if MenuItem::new(row).show(ui).ran {
                    seen.set(Some(row));
                }
            }
        });
    });
    fixture.settle();
    let open = |fixture: &mut Fixture| {
        let opener = fixture.by_label("Open").expect("the opener");
        fixture.advance(1.0).click(&opener).settle();
        assert!(fixture.by_label("Clear All").is_some(), "a click on the opener opens the menu");
    };
    open(&mut fixture);
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE).settle();
    assert!(fixture.by_label("Clear All").is_none(), "Return with no row lit closes the menu");
    assert!(fixture.by_label("Report.txt").is_none(), "and opens no submenu");
    assert_eq!(ran.get(), None, "and runs no row");
    open(&mut fixture);
    let reset = fixture.by_label("Reset").expect("the Reset row").center();
    fixture.frame(vec![pointer::moved(reset)]).settle();
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE).settle();
    assert_eq!(ran.get(), Some("Reset"), "Return runs the row the pointer lit");
    assert!(fixture.by_label("Clear All").is_none(), "and the run closes the menu");
}
