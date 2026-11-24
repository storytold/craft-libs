use craft_text::ui::character::{CharacterControls, CharacterRows, CharacterValues, Kerning, Leading};
use craft_theme::TextSize;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, vec2};

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

#[trial]
fn the_properties_rows_leave_out_baseline_shift_and_the_style_toggles() {
    for (rows, shown) in [(CharacterRows::default(), true), (CharacterRows::PROPERTIES, false)] {
        let mut fixture = Fixture::new(move |ui| {
            let values = CharacterValues { size: Some(12.0), ..CharacterValues::default() };
            CharacterControls::new("Character", &values, &[], &[]).rows(rows).show(ui);
        });
        fixture.room(vec2(402.0, 800.0)).settle();
        assert!(fixture.by_label("Font size").is_some(), "the size field is shown with {rows:?}");
        for label in ["Baseline shift", "All Caps"] {
            assert_eq!(fixture.by_label(label).is_some(), shown, "{label} shown with {rows:?}");
        }
    }
}

/// Edges closer than this are the same edge.
const EDGE: f32 = 0.5;

#[trial]
fn character_fields_line_up_in_their_columns_at_every_width_and_text_size() {
    let families = vec!["Inter".to_owned()];
    let styles = vec!["Regular".to_owned()];
    let mut fixture = Fixture::new(move |ui| {
        let values = CharacterValues {
            family: Some("Inter".to_owned()),
            style: Some("Regular".to_owned()),
            size: Some(12.0),
            leading: Some(Leading::Auto),
            kerning: Some(Kerning::Metrics),
            ..CharacterValues::default()
        };
        CharacterControls::new("Character", &values, &families, &styles).show(ui);
    });
    let left = ["Font size", "Kerning", "Vertical scale", "Baseline shift"];
    let right = ["Leading", "Tracking", "Horizontal scale"];
    let across = ["Font family", "Style"];

    for text_size in [TextSize::Small, TextSize::Large] {
        fixture.text_size(text_size);
        for width in [222.0, 300.0, 340.0, 402.0, 520.0] {
            fixture.room(vec2(width, 900.0)).settle();
            let at = format!("at {width} pt, {text_size:?} text");
            let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown {at}")).rect;
            let same = |a: f32, b: f32| (a - b).abs() <= EDGE;
            let size = rect("Font size");
            let leading = rect("Leading");
            let lined_up = |labels: &[&str], column: Rect| {
                for label in labels {
                    let field = rect(label);
                    assert!(
                        same(field.left(), column.left()) && same(field.right(), column.right()),
                        "{label} {field:?} spans its column {column:?} {at}"
                    );
                }
            };
            let full = Rect::from_x_y_ranges(size.left()..=leading.right(), size.y_range());
            if size.bottom() <= leading.top() {
                // One column: every field spans it.
                lined_up(&left, size);
                lined_up(&right, size);
                lined_up(&across, size);
                assert!(same(rect("Text color").right(), size.right()), "the colour well ends at the column's end {at}");
            } else {
                assert!(size.right() <= leading.left(), "the columns do not overlap {at}");
                lined_up(&left, size);
                lined_up(&right, leading);
                lined_up(&across, full);
                assert!(same(rect("Text color").right(), leading.right()), "the colour well ends at the right column's end {at}");
            }
            assert!(full.right() <= size.left() + width + EDGE, "no field runs past the panel {at}");
            for value in ["12", "Metrics"] {
                let text = fixture.texts().iter().find(|run| run.text == value).unwrap_or_else(|| panic!("{value} is painted {at}")).clone();
                assert!(!text.elided && text.clip.expand(EDGE).contains_rect(text.rect), "{value} fits inside its field {at}: {text:?}");
            }
        }
    }
}
