//! The export dialogs' own logic: the settings a document can take and what Export As answers.

use craft_behavior::PixelSize;
use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{ExportAsDialog, ExportOptions};
use craft_ui::runtime::requests::PathRequest;
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

fn document() -> PixelSize {
    PixelSize::new(1200, 800)
}

/// The file name Export asks the host to save under for a document called `name`.
fn requested_file(name: &str) -> String {
    let name = name.to_owned();
    let asked = Rc::new(RefCell::new(None::<PathRequest>));
    let request = Rc::clone(&asked);
    let mut options = ExportOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut out = ExportAsDialog::new(name.clone(), document(), Bind::bound(&mut options)).show(ui);
        if let Some(filed) = out.requests.take_path() {
            *request.borrow_mut() = Some(filed);
        }
    });
    fixture.settle();
    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export);
    fixture.settle();
    let filed = asked.borrow().clone().expect("Export files a save request");
    filed.file_name.expect("the request names a file")
}

/// The default file name is the document's, made safe on every platform: reserved characters become underscores, dots and
/// spaces at the ends go, and a name with nothing left is Untitled.
#[trial]
fn the_default_file_name_is_made_safe() {
    assert_eq!(requested_file("Poster"), "Poster.png");
    assert_eq!(requested_file("  ..My: poster/v2?.. "), "My_ poster_v2_.png");
    assert_eq!(requested_file(" . "), "Untitled.png");
    let long = "x".repeat(100);
    assert_eq!(requested_file(&long).len(), 80 + ".png".len(), "at most 80 characters");
}
