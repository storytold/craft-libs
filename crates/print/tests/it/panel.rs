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
