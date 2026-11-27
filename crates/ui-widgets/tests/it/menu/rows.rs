//! Menu rows: the detail line under the label, the row's own tooltip, the sample column and which shortcut a row shows.

use craft_behavior::MotionMode;
use craft_host::target::Os;
use craft_i18n::Direction;
use craft_ui::buttons::Button;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{CommandSet, Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::list::{CommandHighlight, ThumbAspect};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::FacePaint;
use craft_ui::runtime::commands::CommandState;
use craft_ui::runtime::env::{Scale, with_env};
use craft_ui::testing::{Fixture, Room, pointer, trial};
use egui::Rect;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// The rect each sample was painted in, by the face's key.
type Cells = Arc<Mutex<HashMap<u64, Rect>>>;

/// The texts painted inside `row` other than `label`.
fn other_runs(fixture: &Fixture, row: Rect, label: &str) -> Vec<(String, Rect)> {
    let runs = fixture.texts().iter().filter(|run| row.contains(run.rect.center()) && run.text != label);
    runs.map(|run| (run.text.clone(), run.rect)).collect()
}

/// The painted run `text`.
fn run(fixture: &Fixture, text: &str) -> Option<Rect> {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect)
}

/// Where the label, the sample and the shortcut of a row with all three lie under `direction`, and, for a
/// `sample_only` row: whether its label text is painted, whether its name is in the access tree, and its sample.
struct SampleRows {
    label: Rect,
    sample: Rect,
    shortcut: Rect,
    only_label_painted: bool,
    only_named: bool,
    only_sample: Rect,
}

fn sample_rows(direction: Direction) -> SampleRows {
    let cells = Cells::default();
    let painted = cells.clone();
    let mut fixture = Fixture::new(move |ui| {
        Menu::inline("Swatches").show(ui, |ui, _| {
            let face = |key: u64| {
                let painted = painted.clone();
                FacePaint::new(key, move |_, rect| {
                    painted.lock().unwrap().insert(key, rect);
                })
            };
            MenuItem::new("Shade").sample(ThumbAspect::new(3, 1), face(1)).shortcut("F5").show(ui);
            MenuItem::new("Ocean").sample(ThumbAspect::new(3, 1), face(2)).sample_only().show(ui);
        });
    });
    fixture.direction(direction);
    fixture.settle();
    let cells = cells.lock().unwrap().clone();
    SampleRows {
        label: run(&fixture, "Shade").expect("the label is painted"),
        sample: cells.get(&1).copied().unwrap_or(Rect::NOTHING),
        shortcut: run(&fixture, "F5").expect("the shortcut is painted"),
        only_label_painted: run(&fixture, "Ocean").is_some(),
        only_named: fixture.by_label("Ocean").is_some(),
        only_sample: cells.get(&2).copied().unwrap_or(Rect::NOTHING),
    }
}

#[trial]
fn a_row_sample_sits_between_the_label_and_the_shortcut_and_mirrors() {
    let ltr = sample_rows(Direction::Ltr);
    assert!(ltr.sample.width() > 0.0, "the sample is painted into a slot: {:?}", ltr.sample);
    assert!(ltr.label.max.x <= ltr.sample.min.x + 0.5, "the sample follows the label: {:?} then {:?}", ltr.label, ltr.sample);
    assert!(ltr.sample.max.x <= ltr.shortcut.min.x + 0.5, "and comes before the shortcut: {:?} then {:?}", ltr.sample, ltr.shortcut);

    let rtl = sample_rows(Direction::Rtl);
    assert!(rtl.sample.width() > 0.0, "the sample is painted under right-to-left too");
    assert!(rtl.sample.max.x <= rtl.label.min.x + 0.5, "right to left, the sample lies left of the label: {:?} {:?}", rtl.sample, rtl.label);
    assert!(rtl.shortcut.max.x <= rtl.sample.min.x + 0.5, "and the shortcut left of the sample: {:?} {:?}", rtl.shortcut, rtl.sample);

    for (rows, direction) in [(&ltr, "left to right"), (&rtl, "right to left")] {
        assert!(!rows.only_label_painted, "a sample-only row paints no label text ({direction})");
        assert!(rows.only_named, "and keeps its label as its name in the access tree ({direction})");
        assert!(rows.only_sample.width() > 0.0, "its sample takes the label's place ({direction})");
    }
}

/// The app commands the shortcut trial's rows bind, each with a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "menuRowsTrial")]
#[non_exhaustive]
enum RowCommand {
    #[command(label = "Find", default = "Cmd+K", modal = "always")]
    Find,
    #[command(label = "Replace", default = "Cmd+J", modal = "always")]
    Replace,
    #[command(label = "Jump", default = "Cmd+L", modal = "always")]
    Jump,
}

#[trial]
fn a_caller_shortcut_wins_over_the_bound_commands_chord_and_hide_shortcut_shows_none() {
    let mut fixture = Fixture::new(|ui| {
        Menu::inline("Search").show(ui, |ui, _| {
            MenuItem::command(RowCommand::Find).show(ui);
            MenuItem::command(RowCommand::Replace).shortcut("X").show(ui);
            MenuItem::command(RowCommand::Jump).hide_shortcut().show(ui);
            MenuItem::new("Plain").show(ui);
        });
    });
    for command in [RowCommand::Find, RowCommand::Replace, RowCommand::Jump] {
        fixture.command_state(command, CommandState::new());
    }
    fixture.settle();
    let row = |fixture: &Fixture, label: &str| fixture.by_label(label).expect("each row is in the access tree");
    let listed = |fixture: &Fixture, label: &str| {
        let node = row(fixture, label).node;
        fixture.tree().shortcut_of(node).map(str::to_owned)
    };

    let find = row(&fixture, "Find");
    assert!(!other_runs(&fixture, find.rect, "Find").is_empty(), "a command row shows its command's chord");
    assert!(listed(&fixture, "Find").is_some(), "and lists it as its shortcut");

    let replace = row(&fixture, "Replace");
    let shown: Vec<String> = other_runs(&fixture, replace.rect, "Replace").into_iter().map(|(text, _)| text).collect();
    assert_eq!(shown, ["X"], "the caller's shortcut text wins over the chord");
    assert_eq!(listed(&fixture, "Replace").as_deref(), Some("X"), "and is the row's listed shortcut");

    let jump = row(&fixture, "Jump");
    assert!(other_runs(&fixture, jump.rect, "Jump").is_empty(), "a hidden shortcut paints none");
    assert_eq!(listed(&fixture, "Jump"), None, "and lists none");

    let plain = row(&fixture, "Plain");
    assert!(other_runs(&fixture, plain.rect, "Plain").is_empty(), "a row with no command and no text shows no shortcut");
    assert_eq!(listed(&fixture, "Plain"), None, "and lists none");
}

#[trial]
fn a_detail_shows_as_a_second_line_under_the_label() {
    let mut fixture = Fixture::new(|ui| {
        Menu::inline("Paper").show(ui, |ui, _| {
            MenuItem::new("Letter").show(ui);
            MenuItem::new("A4").detail("210 × 297 mm").show(ui);
        });
    });
    fixture.settle();
    let plain = fixture.by_label("Letter").expect("the plain row").rect;
    let detailed = fixture.by_label("A4").expect("the row with a detail").rect;
    assert!(detailed.height() > plain.height(), "the row with a detail is a line taller");
    let text = |wanted: &str| fixture.texts().iter().find(|run| run.text == wanted).map(|run| run.rect).expect("the run is painted");
    let (label, detail) = (text("A4"), text("210 × 297 mm"));
    assert!(detail.min.y >= label.max.y - 0.5, "the detail lies under the label");
    assert!((detail.min.x - label.min.x).abs() < 1.0, "the detail starts where the label starts");
}

#[trial]
fn a_row_shows_its_tooltip_when_the_pointer_rests_on_it_in_an_open_menu() {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Edit").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("Undo").tooltip("Take back the last change".to_owned()).show(ui);
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    let row = fixture.by_label("Undo").expect("the menu is open").center();
    fixture.frame(vec![pointer::moved(row)]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(fixture.texts().iter().any(|run| run.text == "Take back the last change"), "the row's tooltip shows");
}

#[trial]
fn a_letter_no_row_takes_while_a_menu_is_open_does_not_reach_the_text_field_under_it() {
    let text = std::rc::Rc::new(std::cell::RefCell::new("hello".to_owned()));
    let held = text.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut value = held.borrow_mut();
        let field = craft_ui::text::TextField::new("Name", craft_ui::contract::Bind::bound(&mut *value)).own_context_menu().show(ui);
        Menu::context("Field menu", &field.response).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
            MenuItem::new("Paste").show(ui);
        });
    });
    fixture.settle();
    let at = fixture.by_label("Name").expect("the field").center();
    fixture.click_at(at);
    fixture.settle();
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    assert!(fixture.by_label("Copy").is_some(), "the context menu is open over the field");
    fixture.type_text("q");
    fixture.settle();
    assert_eq!(*text.borrow(), "hello", "the letter does not reach the field");
}

/// A radio row that is not the choice shows an empty ring in its mark column, so the choices read as a group; the
/// chosen one shows the dot.
#[trial]
fn an_unchosen_radio_row_shows_an_empty_ring() {
    let mut fixture = Fixture::new(|ui| {
        Menu::inline("Zoom").show(ui, |ui, _| {
            MenuItem::new("Fit").radio_mark(false).show(ui);
        });
    });
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let ring = shapes.iter().any(|shape| match shape {
        egui::Shape::Circle(circle) => circle.stroke.width > 0.0 && circle.fill == egui::Color32::TRANSPARENT,
        _ => false,
    });
    assert!(ring, "an unchosen radio row paints a ring");
}

/// The fills painted as rects inside `row`, a row's highlight among them.
fn fills_in(shapes: &[egui::Shape], row: egui::Rect) -> Vec<egui::Color32> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.fill.a() > 0 && row.expand(0.5).contains_rect(rect.rect) && rect.rect.height() > row.height() / 2.0 => {
                Some(rect.fill)
            }
            _ => None,
        })
        .collect()
}

/// A menu highlight switches rows in the frame that moves the pointer onto the next row
/// itself: the row it reaches paints the full highlight and the row it left none, with no time passing in between, so the
/// highlight never trails the pointer (photocraft#15477).
#[trial]
fn the_menu_highlight_moves_to_the_next_row_in_the_frame_the_pointer_reaches_it() {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Edit").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("Undo").show(ui);
            MenuItem::new("Redo").show(ui);
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    let (undo, redo) = (fixture.by_label("Undo").expect("the menu is open").rect, fixture.by_label("Redo").expect("the second row").rect);
    fixture.frame(vec![pointer::moved(undo.center())]);
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let [lit] = fills_in(&shapes, undo)[..] else { panic!("the row under the pointer paints one highlight: {:?}", fills_in(&shapes, undo)) };
    assert!(fills_in(&shapes, redo).is_empty(), "the other row paints no highlight");
    let shapes = fixture.frame_shapes_with(vec![pointer::moved(redo.center())]);
    assert_eq!(fills_in(&shapes, redo), [lit], "the row the pointer reached paints the full highlight at once");
    assert!(fills_in(&shapes, undo).is_empty(), "the row the pointer left paints no highlight at once: {:?}", fills_in(&shapes, undo));
}

/// The fill of an open menu's row under the pointer, with `highlight` set at the root or the default.
fn lit_fill(highlight: Option<CommandHighlight>) -> egui::Color32 {
    let mut fixture = Fixture::new(move |ui| {
        with_env(
            ui,
            |overrides| {
                if let Some(highlight) = highlight {
                    overrides.values.set(highlight);
                }
            },
            |ui| {
                let opener = Button::new("Open").show(ui);
                Menu::new("Edit").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
                    MenuItem::new("Undo").show(ui);
                    MenuItem::new("Redo").show(ui);
                });
            },
        );
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    let undo = fixture.by_label("Undo").expect("the menu is open").rect;
    let shapes = fixture.frame_shapes_with(vec![pointer::moved(undo.center())]);
    let [lit] = fills_in(&shapes, undo)[..] else { panic!("the row under the pointer paints one highlight: {:?}", fills_in(&shapes, undo)) };
    lit
}

/// An app that sets `CommandHighlight::Hover` at its root gets a highlight other than the default accent on the row
/// under the pointer, and the menu still shows which row is active.
#[trial]
fn a_root_command_highlight_changes_the_active_rows_fill() {
    assert_eq!(lit_fill(Some(CommandHighlight::Accent)), lit_fill(None), "the accent is the default");
    assert_ne!(lit_fill(Some(CommandHighlight::Hover)), lit_fill(None), "the hover highlight is not the accent");
}

/// A label long enough that a menu of it is wider than a popup's least width, so a sample column shows in the width.
const LONG: &str = "Neue Grotesk Display Condensed Extra Light Italic";

/// An open "Font family" menu of a long-labelled row and a short one, with a wide and a narrow sample when `samples`:
/// the fixture and where the samples were painted.
fn font_family(samples: bool) -> (Fixture, Cells) {
    let cells = Cells::default();
    let painted = cells.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Font family").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            for (key, label, wide) in [(1_u64, LONG, 12_u16), (2, "Mono", 6)] {
                let mut row = MenuItem::new(label);
                if samples {
                    let painted = painted.clone();
                    row = row.sample(
                        ThumbAspect::new(wide, 1),
                        FacePaint::new(key, move |_, rect| {
                            painted.lock().unwrap().insert(key, rect);
                        }),
                    );
                }
                row.show(ui);
            }
        });
    });
    // A window's room, so the long row shows unelided beside its sample.
    fixture.in_room(Room::Window).settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    (fixture, cells)
}

/// A font menu's samples sit in one column after the labels: the narrower sample starts where the wider one does,
/// and the column widens the menu by the widest sample once. A menu without samples reserves no column.
#[trial]
fn a_level_with_samples_shares_one_sample_column_and_a_level_without_has_none() {
    let (sampled, cells) = font_family(true);
    let cells = cells.lock().unwrap().clone();
    let (wide, narrow) = (cells[&1], cells[&2]);
    assert!(wide.width() > narrow.width(), "the aspect sets a sample's width");
    assert!((wide.height() - narrow.height()).abs() < 0.5, "every sample is as high as its row's text line");
    assert!((wide.min.x - narrow.min.x).abs() < 0.5, "the narrower sample starts where the wider one does: one column");
    let row = |fixture: &Fixture, label: &str| fixture.by_label(label).expect("the menu is open").rect;
    assert!(row(&sampled, "Mono").expand(0.5).contains_rect(narrow), "the sample lies inside its row");
    let label = |text: &str| sampled.texts().iter().find(|run| run.text == text).map(|run| run.rect).expect("the label is painted");
    assert!(label(LONG).max.x <= wide.min.x + 0.5, "the long label ends before the column");

    let (plain, none) = font_family(false);
    assert!(none.lock().unwrap().is_empty(), "a level without samples paints none");
    let grown = row(&sampled, "Mono").width() - row(&plain, "Mono").width();
    assert!(grown >= wide.width() && grown < wide.width() + narrow.width(), "the column is the widest sample and its gap, once: {grown}");
}

#[trial]
fn the_open_submenu_parent_keeps_the_accent_when_the_pointer_enters_its_child() {
    for scale in [1.0, 1.25, 1.5] {
        let parent_state = Rc::new(Cell::new((false, false)));
        let observed = Rc::clone(&parent_state);
        let mut fixture = Fixture::new(move |ui| {
            let opener = Button::new("Open").show(ui);
            Menu::new("Filter").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
                MenuItem::new("Liquify").show(ui);
                let blur = Menu::submenu_row(ui, "Blur", |ui, _| {
                    for label in ["Average", "Gaussian Blur", "Motion Blur", "Shape Blur", "Smart Blur"] {
                        MenuItem::new(label).show(ui);
                    }
                });
                observed.set((blur.active, blur.submenu_open));
                Menu::submenu_row(ui, "Distort", |ui, _| {
                    MenuItem::new("Wave").show(ui);
                });
            });
        });
        fixture.os(Os::Windows).room(egui::vec2(660.0, 460.0)).scale(Scale::new(scale)).motion(MotionMode::Off).settle();
        let opener = fixture.by_label("Open").expect("the opener");
        fixture.click(&opener).settle();
        let plain = fixture.by_label("Liquify").expect("the plain row").rect;
        let parent = fixture.by_label("Blur").expect("the submenu row").rect;
        let sibling = fixture.by_label("Distort").expect("the sibling submenu row").rect;

        let shapes = fixture.frame_shapes_with(vec![pointer::moved(plain.center())]);
        let accent = fills_in(&shapes, plain);
        assert_eq!(accent.len(), 1, "the plain row paints one highlight at {scale}×");

        let shapes = fixture.frame_shapes_with(vec![pointer::moved(parent.center())]);
        assert_eq!(fills_in(&shapes, parent), accent, "the submenu row immediately uses the same highlight at {scale}×");
        assert_eq!(parent_state.get(), (true, true), "the parent is active and open while hovered");
        let child = fixture.by_label("Average").expect("the submenu opens on hover").rect;

        fixture.frame(vec![pointer::moved(parent.center())]);
        let crossing = egui::pos2(sibling.max.x - 2.0, sibling.center().y);
        let shapes = fixture.frame_shapes_with(vec![pointer::moved(crossing)]);
        assert_eq!(fills_in(&shapes, sibling), accent, "the crossed sibling highlights immediately at {scale}×");
        assert_ne!(fills_in(&shapes, parent), accent, "while aiming through a sibling, the parent keeps its subdued tint");
        assert!(fixture.by_label("Average").is_some() && fixture.by_label("Wave").is_none(), "aim still holds the original submenu open");

        let shapes = fixture.frame_shapes_with(vec![pointer::moved(child.center())]);
        assert_eq!(fills_in(&shapes, child), accent, "the child row immediately uses the same highlight at {scale}×");
        assert_eq!(parent_state.get(), (false, true), "the parent remains open with the active row now in its submenu");
        assert_eq!(fills_in(&shapes, parent), accent, "the parent keeps its accent while its child is active at {scale}×");
    }
}

/// A menu row stands `size.row.menu` high (VectorCraft's 29-point rows), and an app that sets only the control height
/// keeps its rows a control high.
#[trial]
fn a_menu_row_stands_a_menu_row_high() {
    let row = |overrides: &[(&str, f32)]| {
        let mut sizes = craft_theme::AppSizes::default();
        for (path, value) in overrides {
            sizes.all.insert((*path).to_owned(), craft_theme::SizeValue::Number(*value));
        }
        let mut root = craft_ui::runtime::env::RootEnv::default();
        root.sizes = Arc::new(sizes);
        let mut fixture = Fixture::new(|ui| {
            Menu::inline("File").show(ui, |ui, _| {
                MenuItem::new("Open").show(ui);
                MenuItem::new("Save").show(ui);
            });
        });
        fixture.root(root).settle();
        fixture.by_label("Save").expect("the row is shown").rect.height()
    };
    let (standard, menu, control) = (row(&[]), row(&[("size.row.menu", 44.0)]), row(&[("size.control.md", 44.0)]));
    assert!(menu > standard + 10.0, "a taller menu row raises the row: {standard} then {menu}");
    assert!((control - menu).abs() < 0.5, "a row follows the control height when the app sets no menu row: {control} and {menu}");
}
