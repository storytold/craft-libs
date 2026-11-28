//! Submenus: a row opens its submenu as soon as the pointer is on it, in one pass that runs nothing outside the menu,
//! and an open submenu stays open while the pointer heads for it across other rows.

use craft_ui::buttons::Button;
use craft_ui::contract::{MemoProps, MemoView, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, pointer, trial};
use std::time::Duration;

/// A menu with two submenu rows, Recent and Templates, the first with a submenu taller than the rows below it.
fn open_menu() -> Fixture {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Main").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("New").show(ui);
            Menu::submenu_row(ui, "Recent", |ui, _| {
                for name in ["Report.txt", "Notes.txt", "Plan.txt", "Budget.txt", "Draft.txt"] {
                    MenuItem::new(name).show(ui);
                }
            });
            Menu::submenu_row(ui, "Templates", |ui, _| {
                MenuItem::new("Blank").show(ui);
            });
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener);
    fixture.settle();
    fixture
}

#[trial]
fn a_submenu_opens_as_soon_as_the_pointer_is_on_its_row() {
    let mut fixture = open_menu();
    let recent = fixture.by_label("Recent").expect("the menu is open").center();
    fixture.frame(vec![pointer::moved(recent)]).idle(1);
    assert!(fixture.by_label("Report.txt").is_some(), "the submenu opens without the pointer resting on its row");
}

#[trial]
fn an_open_submenu_stays_open_while_the_pointer_heads_for_it_across_another_row() {
    let mut fixture = open_menu();
    let recent = fixture.by_label("Recent").expect("the menu is open").center();
    fixture.frame(vec![pointer::moved(recent)]).idle(1);
    let first = fixture.by_label("Report.txt").expect("the submenu is open").rect;
    let submenu = first.union(fixture.by_label("Draft.txt").expect("the submenu's last row").rect);
    let templates = fixture.by_label("Templates").expect("the second submenu row").rect;
    // A point on Templates, near the submenu's edge and level with its rows: the pointer heads for the submenu.
    let crossing = egui::pos2(templates.max.x - 2.0, templates.center().y);
    assert!(crossing.y < submenu.max.y, "the submenu reaches below the crossing point");
    fixture.frame(vec![pointer::moved(crossing)]);
    // The rest asks one wake at its end, not a pass in every frame: the frames that follow at once (row hover) end
    // before it.
    let mut frames = 0;
    while fixture.repaint_delay() == Some(Duration::ZERO) && frames < 60 {
        fixture.frame(vec![]);
        frames += 1;
    }
    assert!(fixture.by_label("Report.txt").is_some(), "the submenu the pointer heads for stays open");
    assert!(fixture.by_label("Blank").is_none(), "the crossed row does not open its submenu while it waits ({frames} frames)");
    assert!(fixture.repaint_delay().is_some_and(|wait| wait > Duration::ZERO), "the rest waits for a wake");
    fixture.advance(0.4).idle(2);
    assert!(fixture.by_label("Blank").is_some(), "the crossed row takes over once the pointer rests on it");
}

/// The filled rects a frame painted over most of `row`: a lit row's highlight.
fn fills_in(shapes: &[egui::Shape], row: egui::Rect) -> usize {
    shapes
        .iter()
        .filter(|shape| match shape {
            egui::Shape::Rect(rect) => rect.fill.a() > 0 && row.expand(0.5).contains_rect(rect.rect) && rect.rect.height() > row.height() / 2.0,
            _ => false,
        })
        .count()
}

/// A view beside the menu: a status button.
#[derive(MemoProps)]
struct Status {}

fn status(_: &mut Status, ui: &mut egui::Ui) {
    Button::new("Status").show(ui);
}

/// The runs of the status view in the last frame.
fn status_runs(fixture: &Fixture) -> usize {
    last_frame_memo_runs(fixture.ctx()).iter().filter(|(name, _, run)| name.ends_with("Status") && *run == MemoRun::Ran).count()
}

/// Opening a submenu changes only the menu: the frame that moves the pointer onto its row runs one pass that lights the
/// row and shows the submenu, and a memoized view beside the menu runs in none of the frames until the menu rests.
#[trial]
fn opening_a_submenu_runs_one_pass_and_nothing_outside_the_menu() {
    let mut fixture = Fixture::new(|ui| {
        MemoView::new("status", Status {}, status).show(ui);
        let opener = Button::new("Open").show(ui);
        Menu::new("Main").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("New").show(ui);
            Menu::submenu_row(ui, "Recent", |ui, _| {
                for name in ["Report.txt", "Notes.txt", "Plan.txt"] {
                    MenuItem::new(name).show(ui);
                }
            });
            MenuItem::new("Quit").show(ui);
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle().idle(2);
    let recent = fixture.by_label("Recent").expect("the menu is open").rect;
    assert_eq!(fills_in(&fixture.frame_shapes(), recent), 0, "Recent is not lit before the pointer reaches it");

    let shapes = fixture.frame_shapes_with(vec![pointer::moved(recent.center())]);
    let opening = fixture.passes();
    let mut runs = status_runs(&fixture);
    assert!(fills_in(&shapes, recent) > 0, "Recent is lit in the frame the pointer reaches it");
    assert!(fixture.by_label("Report.txt").is_some(), "the submenu shows in that frame");
    let mut follow_ups = Vec::new();
    while fixture.repaint_requested() && follow_ups.len() < 30 {
        fixture.frame(vec![]);
        follow_ups.push(fixture.passes());
        runs += status_runs(&fixture);
    }
    let counts = format!("passes {opening}, follow-up frames' passes {follow_ups:?}, status view runs {runs}");
    assert_eq!(opening, 1, "the frame that opens the submenu runs one pass: {counts}");
    assert!(follow_ups.iter().all(|&passes| passes == 1), "every follow-up frame runs one pass: {counts}");
    assert!(!fixture.repaint_requested(), "the menu rests: {counts}");
    assert_eq!(runs, 0, "the memoized view beside the menu runs while the submenu opens: {counts}");
}
