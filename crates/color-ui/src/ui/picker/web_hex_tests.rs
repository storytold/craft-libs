//! With Only Web Colors on, a hex value typed into the Full picker snaps per channel to the nearest multiple of 51
//! instead of being dropped; with it off, hex input is unchanged (photocraft#16744).

use craft_ui::testing::trial;

use super::test_rig::Rig;

const HEX: &str = "Hex color";

#[trial]
fn with_only_web_colors_a_typed_hex_snaps_to_the_nearest_web_colour() {
    for (typed, snapped) in
        [("123456", "003366"), ("003366", "003366"), ("ff8000", "ff9900"), ("abc", "99cccc"), ("12", "000000"), ("1234567", "ffffff")]
    {
        let mut rig = Rig::open("#ff0000");
        rig.web_colors(true);
        rig.type_tab(HEX, typed);
        assert_eq!(rig.color(), format!("#{snapped}"), "{typed}");
        assert_eq!(rig.text(HEX), snapped, "the field shows the snapped colour after {typed}");
    }
    let mut rig = Rig::open("#ff0000");
    rig.web_colors(true);
    rig.type_tab("Red", "100");
    assert_eq!(rig.text("Red"), "102", "a typed channel snaps as before");
}

#[trial]
fn without_only_web_colors_a_typed_hex_is_kept() {
    for typed in ["123456", "ff8000", "003366"] {
        let mut rig = Rig::open("#ff0000");
        rig.type_tab(HEX, typed);
        assert_eq!(rig.color(), format!("#{typed}"));
        assert_eq!(rig.text(HEX), typed);
    }
}
