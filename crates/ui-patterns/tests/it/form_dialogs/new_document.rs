//! New Document's saved presets: each row's Delete button removes its own preset.

use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{DocumentOptions, NewDocumentDialog};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;

fn save_preset(fixture: &mut Fixture, name: &str) {
    let save = fixture.by_label("Save Preset…").expect("the details offer Save Preset…");
    fixture.click(&save).idle(10);
    let field = fixture.by_label("Preset Name").expect("Save Preset… asks for a name");
    fixture.click(&field).idle(4);
    fixture.type_text(name).idle(4);
    let confirm = fixture.by_label("Save").expect("the name row has Save");
    fixture.click(&confirm).idle(10);
}

fn shows(fixture: &Fixture, text: &str) -> bool {
    fixture.texts().iter().any(|painted| painted.text == text)
}

#[trial]
fn a_saved_rows_delete_button_removes_that_preset() {
    let mut options = DocumentOptions::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = NewDocumentDialog::new(Bind::bound(&mut options)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    save_preset(&mut fixture, "Poster");
    save_preset(&mut fixture, "Flyer");
    assert!(shows(&fixture, "Poster") && shows(&fixture, "Flyer"), "the Saved tab lists both presets");
    let delete = fixture.by_label("Delete").expect("the first saved row has a Delete button");
    fixture.click(&delete).idle(10);
    assert!(!shows(&fixture, "Poster"), "the pressed row's preset is gone");
    assert!(shows(&fixture, "Flyer"), "the other preset stays");
}
