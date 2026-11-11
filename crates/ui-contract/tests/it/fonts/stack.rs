//! The font stack: chain order by UI language and scripts, app edits, families, CJK row fit and what an install does
//! to the context's providers.

use std::sync::Arc;

use craft_fonts_build::EmbeddedFace;
use craft_ui::fonts::{FaceList, FontRef, I18nScripts, RegularWeight, Stack, SystemFonts, UiFonts, set_weight};
use craft_ui::testing::trial;
use egui::{FallbackRequest, FontData, FontFamily, FontInsert, FontPriority, FontProvider, InsertFontFamily};

const ARABIC: &[u8] = include_bytes!("../../fonts/NotoSansArabic-test.ttf");
const CJK: &[u8] = include_bytes!("../../fonts/NotoSansCJKsc-test.otf");

const OFFERED: &[&str] = &["en", "ar", "ja", "zh-CN", "zh-TW", "zh-HK", "ko", "th", "ka", "ru"];

const fn face(family: &'static str, style: &'static str, scripts: &'static [&'static str]) -> EmbeddedFace {
    EmbeddedFace { family, style, scripts, bytes: &[], index: 0 }
}

/// craft-fonts' manifest today, plus the regional CJK faces and Georgian it is planned to have.
const CRAFT: &[EmbeddedFace] = &[
    face("Shippori Mincho", "Regular", &["Jpan", "Latn"]),
    face("BIZ UDPGothic", "Regular", &["Jpan", "Latn"]),
    face("BIZ UDPGothic", "Bold", &["Jpan", "Latn"]),
    face("Noto Sans CJK SC", "Regular", &["Hans", "Latn"]),
    face("Noto Sans CJK TC", "Regular", &["Hant", "Latn"]),
    face("Noto Sans CJK HK", "Regular", &["Hant-HK", "Latn"]),
    face("Noto Sans CJK KR", "Regular", &["Kore", "Latn"]),
    face("Noto Sans Arabic", "Regular", &["Arab"]),
    face("Noto Sans Thai", "Regular", &["Thai"]),
    face("Noto Sans Georgian", "Regular", &["Geor"]),
];

fn fonts(ui_language: &str) -> UiFonts<'static> {
    UiFonts::new(&I18nScripts)
        .app_face("ui", FontData::from_static(&[]), &[FontRef::proportional()])
        .app_face("mono", FontData::from_static(&[]), &[FontRef::monospace()])
        .craft_faces(CRAFT)
        .languages(OFFERED)
        .ui_language(ui_language)
        .system_fonts(SystemFonts::Off)
}

fn head(stack: &Stack, family: &FontRef, n: usize) -> Vec<String> {
    stack.chain(family).iter().take(n).cloned().collect()
}

fn egui_face(name: &str) -> FontData {
    (**craft_ui::fonts::deflated_default_definitions().font_data.get(name).expect("an egui default face")).clone()
}

#[trial]
fn ui_language_face_leads_when_the_app_face_lacks_its_script() {
    // CJK UIs keep the Latin face first: Han, kana and Hangul fit its rows.
    let cases: &[(&str, &[&str])] = &[
        ("ar", &["Noto Sans Arabic Regular", "ui"]),
        ("ja", &["ui", "BIZ UDPGothic Regular", "Shippori Mincho Regular", "Noto Sans CJK SC Regular"]),
        ("zh-TW", &["ui", "Noto Sans CJK TC Regular", "Noto Sans CJK HK Regular", "Noto Sans CJK SC Regular"]),
        ("zh-HK", &["ui", "Noto Sans CJK HK Regular", "Noto Sans CJK TC Regular", "Noto Sans CJK SC Regular"]),
        ("zh-CN", &["ui", "Noto Sans CJK SC Regular", "Noto Sans CJK TC Regular"]),
        // craft-fonts' preference for BIZ UDPGothic orders Japanese faces only where they are
        // the best region: for Korean they keep manifest order.
        ("ko", &["ui", "Noto Sans CJK KR Regular", "Shippori Mincho Regular", "BIZ UDPGothic Regular"]),
        ("th", &["Noto Sans Thai Regular", "ui"]),
        ("ka", &["Noto Sans Georgian Regular", "ui"]),
        ("ru", &["ui", "Noto Sans Arabic Regular", "BIZ UDPGothic Regular"]),
        ("en", &["ui", "Noto Sans Arabic Regular", "BIZ UDPGothic Regular"]),
    ];
    for (language, want) in cases {
        let stack = fonts(language).stack();
        assert_eq!(head(&stack, &FontRef::proportional(), want.len()), *want, "{language}");
        // Monospace keeps the app's face first, so code and numbers stay aligned.
        assert_eq!(stack.chain(&FontRef::monospace()).first().map(String::as_str), Some("mono"), "{language}");
    }
}

fn test_fonts(ui_language: &str) -> UiFonts<'static> {
    UiFonts::new(&I18nScripts)
        .app_face("ui", egui_face("Ubuntu-Light"), &[FontRef::proportional()])
        .craft_faces(&[
            EmbeddedFace { family: "Noto Sans Arabic", style: "Regular", scripts: &["Arab"], bytes: ARABIC, index: 0 },
            EmbeddedFace { family: "Noto Sans CJK SC", style: "Regular", scripts: &["Hans", "Latn"], bytes: CJK, index: 0 },
        ])
        .languages(&["en", "ar", "ja", "ka"])
        .ui_language(ui_language)
        .system_fonts(SystemFonts::Off)
}

fn test_stack(ui_language: &str) -> Stack {
    test_fonts(ui_language).stack()
}

#[trial]
fn face_list_prefers_a_face_with_the_whole_cluster() {
    let mut faces = FaceList::default();
    for name in ["Ubuntu-Light", "Hack"] {
        faces.insert(name, egui_face(name));
    }
    faces.set_chain(FontRef::proportional(), vec!["Ubuntu-Light".into(), "Hack".into()]);
    let face = |cluster| faces.font_for(&FallbackRequest { cluster, family: &FontFamily::Proportional }).map(|insert| insert.name);
    assert_eq!(face("a").as_deref(), Some("Ubuntu-Light"));
    // Only Hack has U+0301 COMBINING ACUTE ACCENT, and neither has U+0302.
    assert_eq!(face("a\u{0301}").as_deref(), Some("Hack"));
    assert_eq!(face("a\u{0302}").as_deref(), Some("Ubuntu-Light"));
}

#[trial]
fn variable_faces_draw_at_regular_weight() {
    // The Arabic test face's default instance is Thin (100), like many variable system fonts.
    let thin = || FontData::from_static(ARABIC);
    let wght = |data: &FontData| data.tweak.coords.as_ref().iter().find(|(t, _)| *t == egui::epaint::text::Tag::new(b"wght")).map(|(_, v)| *v);
    assert_eq!(wght(&thin()), None);

    let provider = move |request: &FallbackRequest<'_>| {
        Some(FontInsert::new("system:Arabic", thin(), vec![InsertFontFamily { family: request.family.clone(), priority: FontPriority::Lowest }]))
    };
    let wrapped = RegularWeight::new(Arc::new(provider)).with_weight(FontRef::named("bold"), 700);
    let regular = wrapped.font_for(&FallbackRequest { cluster: "ع", family: &FontFamily::Proportional }).expect("found");
    assert_eq!((regular.name.as_str(), wght(&regular.data)), ("system:Arabic", Some(400.0)));
    let bold = wrapped.font_for(&FallbackRequest { cluster: "ع", family: &FontFamily::Name("bold".into()) }).expect("found");
    assert_eq!((bold.name.as_str(), wght(&bold.data)), ("system:Arabic @700", Some(700.0)));

    // Craft faces get the same treatment, and a weight the app set stays.
    let flat = test_stack("ar").flat_definitions();
    assert_eq!(flat.font_data.get("Noto Sans Arabic Regular").map(|d| wght(d)), Some(Some(400.0)));
    let mut set = thin();
    set.tweak.coords.push(*b"wght", 300.0);
    assert!(!set_weight(&mut set, 400));
}
