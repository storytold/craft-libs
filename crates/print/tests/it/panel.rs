//! The print panel: a printer whose own dialog decides the job.

use std::cell::RefCell;
use std::rc::Rc;

use craft_print::ui::{Destination, PrintPanel, Printer, Settings};
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;

/// The note of the printer whose own dialog decides the job.
const NOTE: &str = "The browser's print dialog sets copies, collate, two-sided and colour.";

/// The job controls the printer's own dialog decides.
const JOB_CONTROLS: [&str; 4] = ["Copies", "Collate", "Grayscale", "Two-sided"];

/// Whether each of the job controls is disabled.
fn disabled(fixture: &Fixture) -> [bool; 4] {
    JOB_CONTROLS.map(|label| {
        let control = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} control shows"));
        fixture.tree().is_disabled_key(control.node)
    })
}

/// Picking a printer with its own dialog disables Copies, Collate, Grayscale and Two-sided and shows its note; picking
/// one without enables them again.
#[trial]
fn a_printer_with_its_own_dialog_disables_the_job_controls() {
    let settings = Rc::new(RefCell::new(Settings::default()));
    settings.borrow_mut().destination = Destination::Printer("Office".into());
    let shown = Rc::clone(&settings);
    let printers = vec![Printer::new("Office"), Printer::new("Browser").own_dialog(NOTE)];
    let mut fixture = Fixture::new(move |ui| {
        let _ = PrintPanel::new(Bind::bound(&mut *shown.borrow_mut()), &printers).show(ui);
    });
    fixture.room(Vec2::new(900.0, 700.0));
    fixture.settle();
    assert_eq!(disabled(&fixture), [false; 4], "a printer without its own dialog leaves the job controls enabled");
    assert!(fixture.by_label(NOTE).is_none(), "no note without the printer's own dialog");

    let pick = |fixture: &mut Fixture, from: &str, to: &str| {
        let list = fixture.by_role(Role::COMBO_BOX).into_iter().find(|list| fixture.tree().value_of(list.node) == Some(from));
        let list = list.unwrap_or_else(|| panic!("the printer list shows {from}"));
        fixture.click(&list).idle(3);
        let entry = fixture.by_label(to).unwrap_or_else(|| panic!("the list offers {to}"));
        fixture.click(&entry).idle(3);
    };
    pick(&mut fixture, "Office", "Browser");
    assert_eq!(settings.borrow().destination, Destination::Printer("Browser".into()), "the pick chose the browser");
    assert_eq!(disabled(&fixture), [true; 4], "the printer's own dialog decides the job controls");
    assert!(fixture.by_label(NOTE).is_some(), "its note shows under them");

    pick(&mut fixture, "Browser", "Office");
    assert_eq!(disabled(&fixture), [false; 4], "a printer without its own dialog enables them again");
}

/// The long last queue of the trial fleet.
const LAST: &str = "München Satellite 119 - Engineering floor 4 (network queue) - colour duplex device, Building North";

/// A searchable panel over a fleet of 120 queues, Save as PDF after them, the first queue the default and chosen; the
/// settings it edits.
fn fleet(room: Vec2) -> (Fixture, Rc<RefCell<Settings>>) {
    use craft_print::ui::Destinations;
    let settings = Rc::new(RefCell::new(Settings::default()));
    let first = "Office 000 - Engineering floor 4 (network queue)";
    settings.borrow_mut().destination = Destination::Printer(first.into());
    let shown = Rc::clone(&settings);
    let mut printers: Vec<Printer> =
        (0..119).map(|i| Printer::new(format!("Office {i:03} - Engineering floor 4 (network queue)")).is_default(i == 0)).collect();
    printers.push(Printer::new(LAST));
    let mut fixture = Fixture::new(move |ui| {
        let _ = PrintPanel::new(Bind::bound(&mut *shown.borrow_mut()), &printers)
            .destinations(Destinations::PrintersAndFile)
            .printer_search(true)
            .show(ui);
    });
    fixture.room(room);
    fixture.settle();
    (fixture, settings)
}

/// Opens the printer list, which shows `value`.
fn open_list(fixture: &mut Fixture, value: &str) {
    let list =
        fixture.by_role(Role::COMBO_BOX).into_iter().find(|list| fixture.tree().value_of(list.node).is_some_and(|shown| shown.starts_with(value)));
    let list = list.unwrap_or_else(|| panic!("the printer list shows {value}"));
    fixture.click(&list).idle(3);
}

/// Searching folds the names and changes no choice; a pick chooses and closes the list, and the list opens again on
/// the whole fleet (upstream PdfCraft printer_picker_filters_a_large_fleet_without_changing_the_selection).
#[trial]
fn printer_picker_filters_a_large_fleet_without_changing_the_selection() {
    let (mut fixture, settings) = fleet(Vec2::new(1400.0, 900.0));
    let initial = settings.borrow().destination.clone();
    open_list(&mut fixture, "Office 000");
    let search = fixture.by_label("Search printers").expect("the search field heads the list");
    assert!(search.rect.width() > 350.0, "room for long device names: {:?}", search.rect);
    fixture.type_text("  MÜNCHEN satellite  ").idle(3);
    assert_eq!(settings.borrow().destination, initial, "typing is not a printer choice");
    assert!(fixture.by_label("Office 001 - Engineering floor 4 (network queue)").is_none(), "the search hides the other queues");
    let last = fixture.by_label(LAST).expect("the search finds the queue");
    fixture.click(&last).idle(3);
    assert_eq!(settings.borrow().destination, Destination::Printer(LAST.into()), "the pick chooses the queue");
    assert!(fixture.by_label("Search printers").is_none(), "choosing closes the list");

    open_list(&mut fixture, "München");
    let search = fixture.by_label("Search printers").expect("the list opens again");
    assert!(fixture.tree().value_of(search.node).is_none_or(str::is_empty), "the search starts empty: the whole fleet shows again");
    assert!(fixture.by_label("No printers match.").is_none());
}

/// Save as PDF stays a destination when no queue matches the search, and choosing it does not print
/// (upstream printer_picker_keeps_save_as_pdf_available_when_the_filter_matches_nothing).
#[trial]
fn printer_picker_keeps_save_as_pdf_available_when_the_filter_matches_nothing() {
    let (mut fixture, settings) = fleet(Vec2::new(1400.0, 900.0));
    let initial = settings.borrow().destination.clone();
    open_list(&mut fixture, "Office 000");
    let search = fixture.by_label("Search printers").expect("the search field heads the list");
    fixture.click(&search).idle(2);
    assert!(fixture.by_label("Search printers").is_some(), "a click in the search keeps the list open");
    fixture.type_text("no-such-printer").idle(3);
    assert!(fixture.by_label("No printers match.").is_some(), "the list says no queue matches");
    assert_eq!(settings.borrow().destination, initial);
    let pdf = fixture.by_label("Save as PDF").expect("Save as PDF stays in the list");
    fixture.click(&pdf).idle(3);
    assert_eq!(settings.borrow().destination, Destination::File, "the pick chooses Save as PDF");
    assert!(fixture.by_label("Search printers").is_none(), "choosing closes the list");
}

/// In a smaller window the list keeps long names within the screen, the search and Save as PDF on it, and reopens
/// with the chosen queue in sight (upstream printer_picker_scrolls_long_names_without_losing_the_search_or_pdf_choice).
#[trial]
fn printer_picker_scrolls_long_names_without_losing_the_search_or_pdf_choice() {
    let size = Vec2::new(1024.0, 768.0);
    let (mut fixture, settings) = fleet(size);
    open_list(&mut fixture, "Office 000");
    fixture.type_text("münchen").idle(3);
    let last = fixture.by_label(LAST).expect("the search finds the queue");
    assert!(last.rect.left() >= 0.0 && last.rect.right() <= size.x, "the long name stays within the screen: {:?}", last.rect);
    fixture.click(&last).idle(3);
    assert_eq!(settings.borrow().destination, Destination::Printer(LAST.into()));

    open_list(&mut fixture, "München");
    let search = fixture.by_label("Search printers").expect("the search field heads the list").rect;
    let row = fixture.by_label(LAST).expect("the chosen queue shows").rect;
    assert!(search.top() >= 0.0 && row.top() >= search.bottom() && row.bottom() <= size.y, "the chosen queue {row:?} is in sight on reopening");
    fixture.key(egui::Key::End, egui::Modifiers::NONE).idle(3);
    let pdf = fixture.by_label("Save as PDF").expect("Save as PDF ends the list").rect;
    assert!(pdf.bottom() <= size.y, "Save as PDF is on screen: {pdf:?}");
    let pdf = fixture.by_label("Save as PDF").expect("Save as PDF ends the list");
    fixture.click(&pdf).idle(3);
    assert_eq!(settings.borrow().destination, Destination::File, "the PDF row is clickable with the whole fleet shown");
}
