//! The table `craft-fonts-build-consumer`'s build.rs generated from the fixture compiles and holds
//! the selected faces' bytes.

use craft_fonts_build_consumer::CRAFT_FONTS;

#[test]
fn generated_table_embeds_the_selected_faces() {
    let names: Vec<String> = CRAFT_FONTS.iter().map(|f| f.name()).collect();
    assert_eq!(names, ["Sans JP Regular", "Sans CJK TC Regular", "Sans CJK HK Regular"]);
    assert_eq!(CRAFT_FONTS[0].bytes, include_bytes!("fixture/craft-fonts/fonts/sans-jp/SansJP-Regular.ttf"));
    assert_eq!(CRAFT_FONTS[0].scripts, ["Jpan", "Latn"]);
    let indices: Vec<u32> = CRAFT_FONTS.iter().map(|f| f.index).collect();
    assert_eq!(indices, [0, 2, 3]);
    assert_eq!(CRAFT_FONTS[2].bytes, include_bytes!("fixture/craft-fonts/fonts/sans-cjk/SansCJK.ttc"));
    assert!(CRAFT_FONTS[2].has_script("Hant"));
}
