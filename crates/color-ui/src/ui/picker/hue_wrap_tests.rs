//! Hue 360 reads 0 in the Full picker, as in the reference image editor's Color Picker: typed, clamped from above,
//! or at the hue slider's top end; other hues are unchanged (photocraft#16742).

use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::pos2;

use super::test_rig::Rig;

#[trial]
fn hue_360_reads_0_and_stays_red() {
    for (typed, shown, colour) in
        [("360", "0", "#ff0000"), ("400", "0", "#ff0000"), ("0", "0", "#ff0000"), ("180", "180", "#00ffff"), ("359", "359", "#ff0004")]
    {
        let mut rig = Rig::open("#00ff00");
        rig.type_tab("Hue", typed);
        assert_eq!(rig.text("Hue"), shown, "H {typed}");
        assert_eq!(rig.color(), colour, "H {typed}");
        assert_eq!((rig.text("Saturation"), rig.text("Brightness")), ("100".to_owned(), "100".to_owned()), "H {typed} keeps S and B");
    }
}

/// The hue slider runs from 360 at its top to 0 at its bottom; a press at its top end reads 0, red.
#[trial]
fn the_hue_sliders_top_end_reads_0() {
    let mut rig = Rig::open("#00ff00");
    let tree = rig.fixture.tree();
    let strip = rig.fixture.by_role(Role::SLIDER).into_iter().find(|t| tree.name_of(t.node) == Some("Hue")).expect("the hue slider beside the field");
    rig.fixture.click_at(pos2(strip.rect.center().x, strip.rect.top()));
    rig.fixture.settle();
    assert_eq!(rig.text("Hue"), "0", "the top end is 0");
    assert_eq!(rig.color(), "#ff0000");
}
