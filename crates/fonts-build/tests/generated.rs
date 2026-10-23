//! The tables `craft-fonts-build-consumer`'s build.rs generated from the fixture compile and hold
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

#[test]
fn served_table_names_urls_instead_of_bytes() {
    use craft_fonts_build_consumer::CRAFT_FONTS_SERVED;
    let faces: Vec<(String, &str, u64)> = CRAFT_FONTS_SERVED.iter().map(|f| (f.name(), f.url, f.size)).collect();
    assert_eq!(
        faces,
        [
            ("Sans JP Regular".to_owned(), "fonts/sans-jp/SansJP-Regular.c9d08ef1fc69f15e.ttf", 24),
            ("Sans CJK TC Regular".to_owned(), "fonts/sans-cjk/SansCJK.2fecde232d5dcda2.ttc", 28),
            ("Sans CJK HK Regular".to_owned(), "fonts/sans-cjk/SansCJK.2fecde232d5dcda2.ttc", 28),
        ]
    );
    assert_eq!(CRAFT_FONTS_SERVED[2].index, 3);
    assert!(CRAFT_FONTS_SERVED[2].has_script("Hant") && CRAFT_FONTS_SERVED[2].is_cjk());
    assert!(CRAFT_FONTS_SERVED[0].integrity.starts_with("sha256-"));
}
