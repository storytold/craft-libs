//! The Full picker's `#` field reads short input as the reference image editor does: two digits pad with zeros, three
//! are the shorthand, and seven hold to `ffffff` with the "Closest value inserted" alert (photocraft#16739).

use craft_ui::testing::trial;

use super::test_rig::Rig;

const HEX: &str = "Hex color";
const TITLE: &str = "Color Picker (Foreground Color)";

#[trial]
fn the_hex_field_pads_short_input_and_holds_seven_digits_with_an_alert() {
    for (typed, want) in [("abc", "#aabbcc"), ("12", "#000012"), ("#ff00ff", "#ff00ff"), ("zzz", "#ff8000"), ("336699", "#336699")] {
        let mut rig = Rig::open("#ff8000");
        rig.type_tab(HEX, typed);
        assert_eq!(rig.color(), want, "{typed}");
        assert_eq!(rig.take_alert(), None, "{typed} shows no alert");
    }
    let mut rig = Rig::open("#ff8000");
    rig.type_tab(HEX, "1234567");
    assert_eq!(rig.color(), "#ffffff", "seven digits insert the closest value");
    let message = "A number between 000000 and ffffff is required. Closest value inserted.";
    assert_eq!(rig.take_alert(), Some((TITLE.to_owned(), message.to_owned())), "and the dialog holds the alert, titled like it");
    assert_eq!(rig.take_alert(), None, "once");
}
