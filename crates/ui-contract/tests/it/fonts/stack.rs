//! The font stack: chain order by UI language and scripts, app edits, families, CJK row fit and what an install does
//! to the context's providers.

use std::sync::Arc;

use craft_fonts_build::EmbeddedFace;
use craft_ui::fonts::{FaceList, FontRef, I18nScripts, RegularWeight, Stack, SystemFonts, UiFonts, set_weight};
use craft_ui::testing::trial;
use egui::{FallbackRequest, FontData, FontFamily, FontInsert, FontPriority, FontProvider, InsertFontFamily};

const ARABIC: &[u8] = include_bytes!("../../fonts/NotoSansArabic-test.ttf");
const CJK: &[u8] = include_bytes!("../../fonts/NotoSansCJKsc-test.otf");
const NOTO_SANS: &[u8] = include_bytes!("../../fonts/NotoSans-test.ttf");

const OFFERED: &[&str] = &["en", "ar", "ja", "zh-CN", "zh-TW", "zh-HK", "ko", "th", "ka", "ru"];

const fn face(family: &'static str, style: &'static str, scripts: &'static [&'static str]) -> EmbeddedFace {
    EmbeddedFace { family, style, scripts, bytes: &[], index: 0, role: None }
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
            EmbeddedFace { family: "Noto Sans Arabic", style: "Regular", scripts: &["Arab"], bytes: ARABIC, index: 0, role: None },
            EmbeddedFace { family: "Noto Sans CJK SC", style: "Regular", scripts: &["Hans", "Latn"], bytes: CJK, index: 0, role: None },
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

/// craft-fonts' monospace face leads the Monospace family when the build embeds it, and only that family; without it
/// egui's default monospace face leads.
#[trial]
fn a_craft_face_with_the_mono_role_leads_the_monospace_family() {
    const MONO: EmbeddedFace = EmbeddedFace {
        family: "JetBrains Mono",
        style: "Regular",
        scripts: &["Latn"],
        bytes: &[],
        index: 0,
        role: Some(craft_fonts_build::MONO_ROLE),
    };
    let build = |faces: &[EmbeddedFace]| {
        UiFonts::new(&I18nScripts).craft_faces(faces).languages(&["en"]).ui_language("en").system_fonts(SystemFonts::Off).stack()
    };
    let defaults = craft_ui::fonts::deflated_default_definitions();
    let default_head = defaults.families.get(&FontFamily::Monospace).and_then(|chain| chain.first()).cloned();
    assert!(default_head.is_some(), "egui has a default monospace face");

    let with: Vec<EmbeddedFace> = CRAFT.iter().copied().chain([MONO]).collect();
    let stack = build(&with);
    let mono = head(&stack, &FontRef::monospace(), 2);
    assert_eq!(mono.first().map(String::as_str), Some("JetBrains Mono Regular"), "the mono face leads Monospace");
    assert_eq!(mono.get(1), default_head.as_ref(), "the default monospace face follows as the fallback");
    assert_ne!(
        stack.chain(&FontRef::proportional()).first().map(String::as_str),
        Some("JetBrains Mono Regular"),
        "the proportional family keeps its own head"
    );

    let without = build(CRAFT);
    assert_eq!(without.chain(&FontRef::monospace()).first(), default_head.as_ref(), "without the mono face the default monospace face leads");
}

/// craft-fonts' signature face leads the signature family and draws in no other; without it the signature family is
/// the proportional chain, so `FontRef::signature()` always names a family egui knows.
#[trial]
fn a_craft_face_with_the_signature_role_leads_only_the_signature_family() {
    const SIGNATURE: EmbeddedFace = EmbeddedFace {
        family: "Dancing Script",
        style: "Regular",
        scripts: &["Latn"],
        bytes: &[],
        index: 0,
        role: Some(craft_fonts_build::SIGNATURE_ROLE),
    };
    const MONO: EmbeddedFace = EmbeddedFace {
        family: "JetBrains Mono",
        style: "Regular",
        scripts: &["Latn"],
        bytes: &[],
        index: 0,
        role: Some(craft_fonts_build::MONO_ROLE),
    };
    let build = |faces: &[EmbeddedFace]| {
        UiFonts::new(&I18nScripts).craft_faces(faces).languages(&["en", "vi"]).ui_language("vi").system_fonts(SystemFonts::Off).stack()
    };
    let name = "Dancing Script Regular";

    let with: Vec<EmbeddedFace> = CRAFT.iter().copied().chain([SIGNATURE, MONO]).collect();
    let stack = build(&with);
    let signature = stack.chain(&FontRef::signature());
    assert_eq!(signature.first().map(String::as_str), Some(name), "the signature face leads the signature family");
    assert!(!signature.iter().any(|face| face == "JetBrains Mono Regular"), "the mono face doesn't lead the signature family");
    let proportional = stack.chain(&FontRef::proportional());
    assert!(signature.len() > 1 && signature[1..].iter().all(|face| proportional.contains(face)), "the proportional chain follows");
    for family in [FontRef::proportional(), FontRef::monospace()] {
        // A Vietnamese UI asks Latin faces for the letters its base faces lack: the signature face isn't one of them.
        assert!(!stack.chain(&family).iter().any(|face| face == name), "{family:?} has no signature face");
    }
    assert_eq!(stack.chain(&FontRef::monospace()).first().map(String::as_str), Some("JetBrains Mono Regular"), "the mono lead stays");

    let without = build(CRAFT);
    assert!(!without.chain(&FontRef::signature()).is_empty(), "the signature family is registered without its face");
    assert_eq!(without.chain(&FontRef::signature()), without.chain(&FontRef::proportional()), "without its face it is the proportional chain");
}

/// The x-height of `data` as drawn, in ems of the font size: its x-height at the instance its variation coordinates
/// set, times its scale.
fn drawn_x_height(data: &FontData) -> f32 {
    use skrifa::MetadataProvider as _;
    let font = skrifa::FontRef::from_index(data.bytes(), data.index).expect("a font");
    let location = font.axes().location(data.tweak.coords.as_ref());
    let metrics = font.metrics(skrifa::instance::Size::unscaled(), &location);
    data.tweak.scale * metrics.x_height.expect("an x-height") / f32::from(metrics.units_per_em)
}

/// craft-fonts' tab-title face leads the tab-title family, scaled to the semibold family's x-height and followed by
/// that family's chain, and draws in no other family. The app's own semibold faces follow it under names of their own.
/// Without the face, or with the OS's interface face, the stack has no tab-title family.
#[trial]
fn a_craft_face_with_the_tab_role_leads_the_tab_title_family() {
    const TAB: EmbeddedFace = EmbeddedFace {
        family: "Noto Sans",
        style: "Condensed SemiBold",
        scripts: &["Latn"],
        bytes: NOTO_SANS,
        index: 0,
        role: Some(craft_fonts_build::TAB_ROLE),
    };
    let build = |faces: &[EmbeddedFace], interface| {
        UiFonts::new(&I18nScripts)
            .craft_faces(faces)
            .languages(&["en", "vi"])
            .ui_language("vi")
            .system_fonts(SystemFonts::Off)
            .interface_font(interface)
            .stack()
    };
    let name = "Noto Sans Condensed SemiBold";
    let tab = FontRef::named(craft_ui::fonts::TAB_TITLE);
    let semibold = FontRef::named(craft_ui::fonts::SEMIBOLD);

    let with: Vec<EmbeddedFace> = CRAFT.iter().copied().chain([TAB]).collect();
    let stack = build(&with, craft_theme::InterfaceFont::Suite);
    let chain = stack.chain(&tab);
    assert_eq!(chain.first().map(String::as_str), Some(name), "the tab-title face leads the tab-title family");
    assert_eq!(&chain[1..], stack.chain(&semibold), "the semibold chain follows");
    for family in [FontRef::proportional(), FontRef::monospace(), FontRef::signature(), semibold.clone()] {
        // A Vietnamese UI asks Latin faces for the letters its base faces lack: the tab-title face isn't one of them.
        assert!(!stack.chain(&family).iter().any(|face| face == name), "{family:?} has no tab-title face");
    }
    let definitions = stack.flat_definitions();
    let lead = definitions.font_data.get(name).expect("the tab-title face's data");
    let reference = stack.chain(&semibold).first().and_then(|first| definitions.font_data.get(first)).expect("the semibold family's first face");
    assert!(lead.tweak.scale != 1.0, "the stand-in face is scaled: {}", lead.tweak.scale);
    assert!((drawn_x_height(lead) - drawn_x_height(reference)).abs() < 1e-4, "an x is as tall in a tab title as in semibold text");
    assert!(stack.weights().contains(&(tab.clone(), 600)), "{:?}", stack.weights());

    // The app's semibold face follows under its own name, and the face it names stays in the semibold family.
    let app = UiFonts::new(&I18nScripts)
        .app_face("ui-semibold", FontData::from_static(ARABIC), std::slice::from_ref(&semibold))
        .craft_faces(&[TAB])
        .languages(&["en"])
        .ui_language("en")
        .system_fonts(SystemFonts::Off)
        .stack();
    // The variable test face draws at 600 under the name `ui-semibold @600`.
    let own = app.chain(&semibold).first().cloned().unwrap_or_default();
    assert!(own.starts_with("ui-semibold"), "{:?}", app.chain(&semibold));
    assert_eq!(app.chain(&tab).get(1), Some(&format!("{own} (tab title)")), "{:?}", app.chain(&tab));
    let app_definitions = app.flat_definitions();
    assert!(app_definitions.font_data.contains_key(&format!("{own} (tab title)")), "the copy has its data");
    // The variable face's x-height grows with its weight: the tab-title face matches it at 600.
    let lead = app_definitions.font_data.get(name).expect("the tab-title face's data");
    let reference = app_definitions.font_data.get(&own).expect("the app's semibold face's data");
    assert!(!reference.tweak.coords.as_ref().is_empty(), "the app's semibold face is set to a weight");
    assert!((drawn_x_height(lead) - drawn_x_height(reference)).abs() < 1e-4, "an x is as tall in a tab title as in the app's semibold text");

    assert!(build(CRAFT, craft_theme::InterfaceFont::Suite).chain(&tab).is_empty(), "without the face there is no tab-title family");
    assert!(build(&with, craft_theme::InterfaceFont::System).chain(&tab).is_empty(), "the OS's interface face keeps tab titles");
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
