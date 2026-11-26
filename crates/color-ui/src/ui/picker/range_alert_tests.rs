//! A number typed outside a Full picker field's range inserts the closest value and gives the reference image editor's
//! alert, which names the field's range without its unit; in-range input and refused characters give none
//! (photocraft#16741).

use craft_ui::testing::trial;

use super::test_rig::Rig;

const TITLE: &str = "Color Picker (Foreground Color)";

fn alert(low: &str, high: &str) -> Option<(String, String)> {
    Some((TITLE.to_owned(), format!("An integer between {low} and {high} is required. Closest value inserted.")))
}

#[trial]
fn an_out_of_range_number_inserts_the_closest_value_and_gives_the_alert() {
    for (field, typed, kept, low, high) in [
        ("Hue", "400", "0", "0", "360"),
        ("Saturation", "150", "100", "0", "100"),
        ("Brightness", "101", "100", "0", "100"),
        ("Red", "300", "255", "0", "255"),
        ("Green", "256", "255", "0", "255"),
        ("Blue", "999", "255", "0", "255"),
        ("Lightness", "120", "100", "0", "100"),
        ("Green-red", "200", "127", "-128", "127"),
        ("Blue-yellow", "-200", "-128", "-128", "127"),
        ("Cyan", "150", "100", "0", "100"),
        ("Black", "101", "100", "0", "100"),
    ] {
        let mut rig = Rig::open("#336699");
        rig.type_tab(field, typed);
        assert_eq!(rig.text(field), kept, "{field} {typed} inserts the closest value");
        assert_eq!(rig.take_alert(), alert(low, high), "{field} {typed} gives the alert with the field's range");
        assert_eq!(rig.take_alert(), None, "once");
    }
}

/// The colour follows the inserted value: S 150 is S 100 with H and B kept.
#[trial]
fn the_colour_follows_the_closest_value() {
    let mut rig = Rig::open("#336699");
    let (hue, brightness) = (rig.text("Hue"), rig.text("Brightness"));
    rig.type_tab("Saturation", "150");
    assert_eq!((rig.text("Hue"), rig.text("Saturation"), rig.text("Brightness")), (hue, "100".to_owned(), brightness));
    let mut rig = Rig::open("#336699");
    rig.type_tab("Red", "300");
    assert_eq!(rig.color(), "#ff6699", "R 300 is R 255");
}

#[trial]
fn in_range_numbers_and_refused_characters_give_no_alert() {
    for (field, typed, shown) in [("Red", "128", "128"), ("Red", "0", "0"), ("Red", "255", "255"), ("Hue", "359", "359")] {
        let mut rig = Rig::open("#336699");
        rig.type_tab(field, typed);
        assert_eq!(rig.text(field), shown, "{field} {typed}");
        assert_eq!(rig.take_alert(), None, "{field} {typed} gives no alert");
    }
    for typed in ["12.7", "abc", ""] {
        let mut rig = Rig::open("#336699");
        rig.type_tab("Red", typed);
        assert_eq!(rig.take_alert(), None, "{typed:?} is no number out of range: no alert");
    }
}
