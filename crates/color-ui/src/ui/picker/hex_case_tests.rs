//! The Full picker's `#` field shows lower-case digits, as the reference image editor's Color Picker does; other hex
//! fields keep upper case (photocraft#16740).

use craft_behavior::Codec as _;
use craft_theme::Rgba;
use craft_ui::testing::trial;
use craft_ui::text::HexCodec;

use super::test_rig::Rig;

const HEX: &str = "Hex color";

#[trial]
fn the_pickers_hex_field_shows_lower_case_digits() {
    let mut rig = Rig::open("#ff8000");
    assert_eq!(rig.text(HEX), "ff8000", "the opening colour");
    for (typed, shown) in [("3366CC", "3366cc"), ("AABBCC", "aabbcc"), ("003366", "003366"), ("ABC", "aabbcc")] {
        rig.type_tab(HEX, typed);
        assert_eq!(rig.text(HEX), shown, "{typed} is taken and shown in lower case");
        assert_eq!(rig.color(), format!("#{shown}"));
    }
}

#[trial]
fn other_hex_fields_keep_upper_case() {
    let orange = Rgba::rgb(0xff, 0x80, 0x00);
    assert_eq!(HexCodec::default().format(&orange), "FF8000", "the default codec, the Color panel's and Web Color Sliders'");
    assert_eq!(HexCodec::default().digits(1).format(&Rgba::rgb(0xff, 0x88, 0x00)), "F80");
    assert_eq!(HexCodec::default().lower_case(true).format(&orange), "ff8000");
}
