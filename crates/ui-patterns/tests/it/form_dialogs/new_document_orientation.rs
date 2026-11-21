//! The orientation segments are page glyphs with accessible names and still turn the document.

use craft_ui::contract::Bind;
use craft_ui::patterns::form_dialogs::{DocumentOptions, NewDocumentDialog};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use std::cell::RefCell;
use std::rc::Rc;

#[trial]
fn orientation_uses_icons_and_swaps_the_document_dimensions() {
    let options = Rc::new(RefCell::new(DocumentOptions::default()));
    let bound = Rc::clone(&options);
    let mut fixture = Fixture::new(move |ui| {
        let _ = NewDocumentDialog::new(Bind::bound(&mut *bound.borrow_mut())).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);

    let portrait = fixture.by_label("Portrait").expect("an accessible Portrait choice");
    assert!(fixture.by_label("Landscape").is_some(), "an accessible Landscape choice");
    assert!(
        fixture.texts().iter().all(|text| text.text != "Portrait" && text.text != "Landscape"),
        "the orientation controls show page icons without painting their accessible names"
    );
    let shapes = fixture.frame_clipped_shapes();
    for name in ["Portrait", "Landscape"] {
        let choice = fixture.by_label(name).expect("an orientation choice").rect;
        let painted = shapes.iter().any(|(clip, shape)| {
            let bounds = shape.visual_bounding_rect();
            choice.contains_rect(bounds) && bounds.area() < choice.area() && clip.contains_rect(bounds)
        });
        assert!(painted, "the {name} glyph is painted where it can be seen");
    }
    assert!(fixture.by_label("Height Unit").is_none(), "Width's unit select serves Height too");

    let original = options.borrow().size;
    assert!(original.width > original.height);
    fixture.click_at(portrait.center()).idle(10);
    let rotated = options.borrow().size;
    assert_eq!((rotated.width, rotated.height), (original.height, original.width));

    let landscape = fixture.by_label("Landscape").expect("the Landscape choice after turning the page");
    fixture.click_at(landscape.center()).idle(10);
    assert_eq!(options.borrow().size, original);
}
