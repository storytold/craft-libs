use craft_text::ui::character::{CharacterControls, CharacterValues, Kerning, Leading};
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

#[trial]
fn character_fields_are_readable_when_the_panel_is_narrowed() {
    let mut fixture = Fixture::new(|ui| {
        let values =
            CharacterValues { size: Some(144.0), leading: Some(Leading::Auto), kerning: Some(Kerning::Metrics), ..CharacterValues::default() };
        CharacterControls::new("Character", &values, &[], &[]).show(ui);
    });

    for (step, width) in [402.0, 222.0, 402.0].into_iter().enumerate() {
        fixture.room(vec2(width, 500.0)).settle();
        let size = fixture.by_label("Font size").expect("the font-size field is shown").rect;
        let leading = fixture.by_label("Leading").expect("the leading field is shown").rect;
        for value in ["144", "172.8", "Metrics"] {
            let text = fixture
                .texts()
                .iter()
                .find(|run| run.text == value)
                .unwrap_or_else(|| panic!("{value} is painted at {width} pt (resize step {step})"));
            assert!(!text.elided && text.clip.expand(0.5).contains_rect(text.rect), "{value} fits inside its field at {width} pt: {text:?}");
        }
        if width == 222.0 {
            assert!(size.bottom() <= leading.top(), "the two fields stack at {width} pt");
        } else {
            assert!(size.right() <= leading.left(), "the two fields sit side by side at {width} pt");
        }
    }
}
