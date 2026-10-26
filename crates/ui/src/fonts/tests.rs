use super::*;
use egui::{FallbackRequest, FontInsert, FontPriority, InsertFontFamily, TextStyle};
use std::sync::Mutex;

const ARABIC: &[u8] = include_bytes!("../../tests/fonts/NotoSansArabic-test.ttf");
/// Noto Sans with Vietnamese ổ but not ẫ.
const NOTO_SANS_LATIN: &[u8] = include_bytes!("../../tests/fonts/NotoSans-test.ttf");
/// Noto Sans Symbols' ⌃ ⎋ and Noto Sans Symbols 2's ⌥ ⌫ ⏎: the keyboard keys are split between them.
const SYMBOLS: &[u8] = include_bytes!("../../tests/fonts/NotoSansSymbols-test.ttf");
const SYMBOLS_2: &[u8] = include_bytes!("../../tests/fonts/NotoSansSymbols2-test.ttf");
const CJK: &[u8] = include_bytes!("../../tests/fonts/NotoSansCJKsc-test.otf");

const OFFERED: &[&str] = &["en", "ar", "ja", "zh-CN", "zh-TW", "zh-HK", "ko", "th", "ka", "ru"];

const fn face(family: &'static str, style: &'static str, scripts: &'static [&'static str]) -> CraftFace {
    CraftFace { family, style, scripts, bytes: &[], index: 0 }
}

/// craft-fonts' manifest today, plus the regional CJK faces and Georgian it is planned to have.
const CRAFT: &[CraftFace] = &[
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
        .app_face("ui", FontData::from_static(&[]), &[FontFamily::Proportional])
        .app_face("mono", FontData::from_static(&[]), &[FontFamily::Monospace])
        .craft_faces(CRAFT)
        .languages(OFFERED)
        .ui_language(ui_language)
        .system_fonts(SystemFonts::Off)
}

fn head(stack: &Stack, family: &FontFamily, n: usize) -> Vec<String> {
    stack.chain(family).iter().take(n).cloned().collect()
}

#[test]
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
        assert_eq!(head(&stack, &FontFamily::Proportional, want.len()), *want, "{language}");
        // Monospace keeps the app's face first, so code and numbers stay aligned.
        assert_eq!(stack.chain(&FontFamily::Monospace).first().map(String::as_str), Some("mono"), "{language}");
    }
}

#[test]
fn families_without_app_faces_start_with_egui_defaults() {
    let defaults = FontDefinitions::default();
    let first = |family: &FontFamily| defaults.families.get(family).and_then(|c| c.first()).cloned().expect("egui default_fonts in dev-dependencies");
    let (egui_ui, egui_mono) = (first(&FontFamily::Proportional), first(&FontFamily::Monospace));
    let only_ui = |language| fonts_without_faces(language).app_face("ui", FontData::from_static(&[]), &[FontFamily::Proportional]).stack();
    let no_faces = |language| fonts_without_faces(language).stack();

    let stack = only_ui("en");
    assert_eq!(head(&stack, &FontFamily::Proportional, 2), ["ui", "Noto Sans Arabic Regular"]);
    assert_eq!(head(&stack, &FontFamily::Monospace, 1), [egui_mono.as_str()], "egui's mono face, not a craft face");
    let stack = only_ui("ar");
    assert_eq!(head(&stack, &FontFamily::Proportional, 2), ["Noto Sans Arabic Regular", "ui"]);
    assert_eq!(head(&stack, &FontFamily::Monospace, 1), [egui_mono.as_str()]);

    // egui's defaults come right after the UI language's faces and before other scripts.
    for (language, lead) in [("en", None), ("ar", Some("Noto Sans Arabic Regular"))] {
        let stack = no_faces(language);
        let chain = stack.chain(&FontFamily::Proportional);
        let egui_at = chain.iter().position(|n| *n == egui_ui).expect("egui's face");
        assert_eq!(chain.first().map(String::as_str), Some(lead.unwrap_or(&egui_ui)), "{language}");
        let other = if lead.is_some() { "BIZ UDPGothic Regular" } else { "Noto Sans Arabic Regular" };
        assert!(chain.iter().position(|n| n == other) > Some(egui_at), "{language}: {chain:?}");
        assert_eq!(head(&stack, &FontFamily::Monospace, 1), [egui_mono.as_str()], "{language}");
    }
}

fn fonts_without_faces(ui_language: &str) -> UiFonts<'static> {
    UiFonts::new(&I18nScripts).craft_faces(CRAFT).languages(OFFERED).ui_language(ui_language).system_fonts(SystemFonts::Off)
}

#[test]
fn craft_preference_orders_the_best_region_only() {
    // With only Japanese faces, they are the best Han region a Taiwanese UI has.
    let stack = UiFonts::new(&I18nScripts).craft_faces(&CRAFT[..3]).ui_language("zh-TW").system_fonts(SystemFonts::Off).stack();
    let chain = stack.chain(&FontFamily::Proportional);
    let at = |name: &str| chain.iter().position(|n| n == name);
    assert!(at("BIZ UDPGothic Regular") < at("Shippori Mincho Regular"), "{chain:?}");
}

#[test]
fn every_shipped_face_is_in_every_stack() {
    for language in OFFERED {
        let stack = fonts(language).stack();
        let chain = stack.chain(&FontFamily::Proportional);
        for face in CRAFT.iter().filter(|f| f.style == "Regular") {
            assert!(chain.contains(&face.name()), "{language}: {} missing", face.name());
        }
        assert!(!chain.contains(&"BIZ UDPGothic Bold".to_owned()), "{language}: bold face in the regular chain");
    }
}

#[test]
fn app_can_prefer_add_replace_and_reorder_faces() {
    let stack = fonts("ja").prefer("Jpan", "Shippori Mincho").stack();
    assert_eq!(head(&stack, &FontFamily::Proportional, 3), ["ui", "Shippori Mincho Regular", "BIZ UDPGothic Regular"]);

    // An app whose UI face covers Arabic keeps it first.
    let stack = fonts("ar").app_scripts(&["Latn", "Arab"]).stack();
    assert!(stack.script_face.is_none(), "{:?}", stack.script_face);
    assert_eq!(head(&stack, &FontFamily::Proportional, 1), ["ui"]);
    assert!(stack.chain(&FontFamily::Proportional).contains(&"Noto Sans Arabic Regular".to_owned()));

    let mut stack = fonts("en").stack();
    stack.insert(&FontFamily::Proportional, 1, "brand", FontData::from_static(&[]));
    stack.remove("Noto Sans Arabic Regular");
    assert_eq!(head(&stack, &FontFamily::Proportional, 3), ["ui", "brand", "BIZ UDPGothic Regular"]);
    stack.insert(&FontFamily::Proportional, 0, "BIZ UDPGothic Regular", FontData::from_static(&[]));
    assert_eq!(head(&stack, &FontFamily::Proportional, 3), ["BIZ UDPGothic Regular", "ui", "brand"]);
    let definitions = stack.checked_definitions();
    assert!(!definitions.families.values().flatten().any(|n| n == "Noto Sans Arabic Regular"));
}

#[test]
fn named_families_take_their_weight_and_lead_script() {
    let stack = fonts("ja")
        .app_face("ui-semibold", FontData::from_static(&[]), &[FontFamily::Name("semibold".into())])
        .family(Family::named("semibold").weight(600))
        .family(Family::named("arabic").script("Arab"))
        .family(Family::monospace().weight(500))
        .stack();
    assert!(stack.weights().contains(&(FontFamily::Monospace, 500)));
    let semibold = FontFamily::Name("semibold".into());
    assert_eq!(head(&stack, &semibold, 3), ["ui-semibold", "BIZ UDPGothic Bold", "BIZ UDPGothic Regular"]);
    // A family without an app face of its own falls back to the proportional faces.
    let arabic = FontFamily::Name("arabic".into());
    assert_eq!(head(&stack, &arabic, 3), ["Noto Sans Arabic Regular", "ui", "BIZ UDPGothic Regular"]);
}

#[test]
fn monospaced_named_families_keep_their_latin_face_first() {
    // Georgian's own face leads the text families; the monospaced ones keep their app face.
    let (code, console, plain) = (FontFamily::Name("code".into()), FontFamily::Name("console".into()), FontFamily::Name("plain".into()));
    let stack = fonts("ka")
        .app_face("code-face", FontData::from_static(&[]), std::slice::from_ref(&code))
        .family(Family::named("code").monospaced())
        .family(Family::named("console").monospaced())
        .family(Family::named("plain"))
        .stack();
    assert_eq!(head(&stack, &code, 2), ["code-face", "Noto Sans Georgian Regular"]);
    assert_eq!(head(&stack, &console, 2), ["mono", "Noto Sans Georgian Regular"], "the monospace app faces");
    assert_eq!(head(&stack, &plain, 2), ["Noto Sans Georgian Regular", "ui"]);

    let egui_mono = FontDefinitions::default().families[&FontFamily::Monospace][0].clone();
    let stack = fonts_without_faces("ka").family(Family::named("console").monospaced()).stack();
    assert_eq!(head(&stack, &console, 1), [egui_mono], "egui's mono face");
}

#[test]
fn system_fonts_before_craft_moves_other_scripts_after_them() {
    let emoji = Emoji { color: Some(FontData::from_static(&[])), monochrome: Some(FontData::from_static(&[])), ..Emoji::default() };
    let stack = fonts("ja").system_fonts(SystemFonts::BeforeCraft).emoji(emoji).stack();
    let configured = stack.chain(&FontFamily::Proportional);
    assert!(configured.contains(&"BIZ UDPGothic Regular".to_owned()));
    assert!(!configured.contains(&"Noto Sans Arabic Regular".to_owned()));
    let after = stack.after_system.chain(&FontFamily::Proportional);
    let position = |name: &str| after.iter().position(|n| n == name);
    let (arabic, color, mono) = (position("Noto Sans Arabic Regular"), position("emoji-color"), position("emoji-monochrome"));
    assert!(arabic.is_some() && arabic < color && color < mono, "{after:?}");
}

fn test_stack(ui_language: &str) -> Stack {
    test_fonts(ui_language).stack()
}

fn test_fonts(ui_language: &str) -> UiFonts<'static> {
    let defaults = FontDefinitions::default();
    let ubuntu = defaults.font_data.get("Ubuntu-Light").map(|d| (**d).clone()).expect("egui default_fonts in dev-dependencies");
    UiFonts::new(&I18nScripts)
        .app_face("ui", ubuntu, &[FontFamily::Proportional])
        .craft_faces(&[
            CraftFace { family: "Noto Sans Arabic", style: "Regular", scripts: &["Arab"], bytes: ARABIC, index: 0 },
            CraftFace { family: "Noto Sans CJK SC", style: "Regular", scripts: &["Hans", "Latn"], bytes: CJK, index: 0 },
        ])
        .languages(&["en", "ar", "ja", "ka"])
        .ui_language(ui_language)
        .system_fonts(SystemFonts::Off)
}

#[test]
fn craft_faces_come_from_embedded_faces() {
    let embedded = craft_fonts_build::EmbeddedFace { family: "Noto Sans Arabic", style: "Regular", scripts: &["Arab"], index: 2, bytes: ARABIC };
    let face = CraftFace::from(embedded);
    assert_eq!((face.name().as_str(), face.scripts, face.index, face.bytes.len()), ("Noto Sans Arabic Regular", &["Arab"][..], 2, ARABIC.len()));
}

#[test]
fn coverage_names_language_string_and_character() {
    let stack = test_stack("en");
    let samples = [
        // With U+061C ARABIC LETTER MARK and U+034F COMBINING GRAPHEME JOINER, which draw nothing.
        Sample { language: "ar", key: "menu.file", text: "ملف\u{061C} عر\u{034F}بي" },
        Sample { language: "ja", key: "menu.file", text: "日本語のファイル" },
        Sample { language: "ka", key: "menu.file", text: "ფაილი" },
    ];
    let missing = missing_glyphs(&stack, &FontFamily::Proportional, samples, false);
    let chars: Vec<char> = missing.iter().map(|m| m.character).collect();
    assert_eq!(chars, ['ფ', 'ა', 'ი', 'ლ']);
    assert!(missing.iter().all(|m| m.language == "ka" && m.key == "menu.file"));
    assert_eq!(missing[0].to_string(), "ka menu.file: U+10E4 'ფ' in \"ფაილი\"");

    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| assert_covers(&stack, samples))).expect_err("Georgian has no face");
    let message = failure.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("4 characters") && message.contains("ka menu.file: U+10E4"), "{message}");

    assert_covers(&stack, samples.into_iter().take(2));
}

#[test]
fn a_latin_ui_takes_letters_its_face_lacks_from_a_latin_craft_face() {
    const NOTO_SANS: &[u8] = include_bytes!("../../tests/fonts/NotoSans-test.ttf");
    let cjk = CraftFace { family: "Noto Sans CJK SC", style: "Regular", scripts: &["Hans", "Latn"], bytes: CJK, index: 0 };
    let noto_sans = CraftFace { family: "Noto Sans", style: "Regular", scripts: &["Latn", "Cyrl", "Grek", "Deva"], bytes: NOTO_SANS, index: 0 };
    let defaults = FontDefinitions::default();
    let ubuntu = defaults.font_data.get("Ubuntu-Light").expect("egui default_fonts in dev-dependencies");
    assert!(!providers::has_char(ubuntu, 'ổ') && !providers::has_char(ubuntu, 'ạ'));
    let vietnamese = [Sample { language: "vi", key: "menu.file", text: "Tiếng Việt ổ ư ạ" }];
    let stack = |faces: &[CraftFace]| {
        UiFonts::new(&I18nScripts).craft_faces(faces).languages(&["en", "vi", "zh-CN"]).ui_language("vi").system_fonts(SystemFonts::Off).stack()
    };

    let without = stack(&[cjk]);
    assert!(missing_glyphs(&without, &FontFamily::Proportional, vietnamese, false).iter().any(|m| m.character == 'ổ'));
    let with = stack(&[cjk, noto_sans]);
    assert_covers(&with, vietnamese);
    // Noto Sans CJK has Vietnamese letters too, at CJK proportions: Noto Sans comes first.
    let chain = with.chain(&FontFamily::Proportional);
    let at = |name: &str| chain.iter().position(|n| n == name).unwrap_or(usize::MAX);
    assert!(at("Ubuntu-Light") < at("Noto Sans Regular") && at("Noto Sans Regular") < at("Noto Sans CJK SC Regular"), "{chain:?}");
}

#[test]
fn remove_and_replace_reach_the_faces_after_the_system_fonts() {
    let arabic = [Sample { language: "ar", key: "menu.file", text: "ملف" }];
    let stack = || test_fonts("en").system_fonts(SystemFonts::BeforeCraft).stack();
    let draws_arabic = |stack: &Stack| missing_glyphs(stack, &FontFamily::Proportional, arabic, false).is_empty();
    let mut removed = stack();
    assert!(removed.after_system.contains("Noto Sans Arabic Regular"));
    assert!(draws_arabic(&removed));
    removed.remove("Noto Sans Arabic Regular");
    assert!(!draws_arabic(&removed));
    let mut replaced = stack();
    replaced.replace("Noto Sans Arabic Regular", FontData::from_static(CJK));
    assert!(!draws_arabic(&replaced));
}

#[test]
fn flat_definitions_end_each_chain_with_the_faces_after_the_system_fonts() {
    let stack = test_fonts("en").emoji(Emoji { monochrome: Some(FontData::from_static(CJK)), ..Emoji::default() }).stack();
    let after: Vec<String> = stack.after_system.chain(&FontFamily::Proportional).to_vec();
    assert!(after.iter().any(|n| n == "egui-icons") && after.iter().any(|n| n == "emoji-monochrome"), "{after:?}");
    let names: Vec<&str> = stack.after_system.faces().map(|(name, _)| name).collect();
    assert!(after.iter().all(|n| names.contains(&n.as_str())), "{names:?}");
    assert_eq!(stack.after_system.get("emoji-monochrome").map(|d| d.bytes().len()), Some(CJK.len()));

    let flat = stack.flat_definitions();
    let chain = &flat.families[&FontFamily::Proportional];
    assert_eq!(chain[..stack.chain(&FontFamily::Proportional).len()], *stack.chain(&FontFamily::Proportional));
    assert!(chain.ends_with(&after), "{chain:?}");
    assert!(chain.iter().all(|n| flat.font_data.contains_key(n)));
    assert!(flat.families[&FontFamily::Monospace].ends_with(stack.after_system.chain(&FontFamily::Monospace)));
}

#[test]
fn face_list_prefers_a_face_with_the_whole_cluster() {
    let defaults = FontDefinitions::default();
    let mut faces = FaceList::default();
    for name in ["Ubuntu-Light", "Hack"] {
        faces.insert(name, (**defaults.font_data.get(name).expect("egui default_fonts in dev-dependencies")).clone());
    }
    faces.set_chain(FontFamily::Proportional, vec!["Ubuntu-Light".into(), "Hack".into()]);
    let face = |cluster| faces.font_for(&FallbackRequest { cluster, family: &FontFamily::Proportional }).map(|insert| insert.name);
    assert_eq!(face("a").as_deref(), Some("Ubuntu-Light"));
    // Only Hack has U+0301 COMBINING ACUTE ACCENT, and neither has U+0302.
    assert_eq!(face("a\u{0301}").as_deref(), Some("Hack"));
    assert_eq!(face("a\u{0302}").as_deref(), Some("Ubuntu-Light"));
}

#[test]
fn installed_text_faces_sit_on_the_baseline_but_emoji_and_symbols_do_not() {
    fn patched(edits: &[(&[u8], &[u8])]) -> FontData {
        let mut bytes = ARABIC.to_vec();
        for (from, to) in edits {
            let at = bytes.windows(from.len()).position(|w| w == *from).expect("in the test face");
            bytes[at..at + to.len()].copy_from_slice(to);
        }
        FontData::from_owned(bytes)
    }
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    let name = "system-fallback";
    assert!(providers::is_text_face(name, &FontData::from_static(ARABIC)));
    // A face with colour glyphs: the table directory names a COLR table.
    assert!(!providers::is_text_face(name, &patched(&[(b"post", b"COLR")])));
    // By the face's own family name, "Noto Sans Symbol", whatever the provider calls it.
    let (arabic, symbol) = (utf16("Arabic"), utf16("Symbol"));
    assert!(!providers::is_text_face(name, &patched(&[(&arabic, &symbol)])));
    assert!(!providers::is_text_face("system:Segoe UI Emoji:0:0", &FontData::from_static(ARABIC)));
}

#[test]
fn variable_faces_draw_at_regular_weight() {
    // The Arabic test face's default instance is Thin (100), like many variable system fonts.
    let thin = || FontData::from_static(ARABIC);
    let wght = |data: &FontData| data.tweak.coords.as_ref().iter().find(|(t, _)| *t == egui::epaint::text::Tag::new(b"wght")).map(|(_, v)| *v);
    assert_eq!(wght(&thin()), None);

    let provider = move |request: &FallbackRequest<'_>| {
        Some(FontInsert::new("system:Arabic", thin(), vec![InsertFontFamily { family: request.family.clone(), priority: FontPriority::Lowest }]))
    };
    let wrapped = RegularWeight::new(Arc::new(provider)).with_weight(FontFamily::Name("bold".into()), 700);
    let regular = wrapped.font_for(&FallbackRequest { cluster: "ع", family: &FontFamily::Proportional }).expect("found");
    assert_eq!((regular.name.as_str(), wght(&regular.data)), ("system:Arabic", Some(400.0)));
    let bold = wrapped.font_for(&FallbackRequest { cluster: "ع", family: &FontFamily::Name("bold".into()) }).expect("found");
    assert_eq!((bold.name.as_str(), wght(&bold.data)), ("system:Arabic @700", Some(700.0)));

    // Craft faces get the same treatment, and a weight the app set stays.
    let stack = test_stack("ar");
    assert_eq!(stack.definitions.font_data.get("Noto Sans Arabic Regular").map(|d| wght(d)), Some(Some(400.0)));
    let mut set = thin();
    set.tweak.coords.push(*b"wght", 300.0);
    assert!(!set_weight(&mut set, 400));
}

#[test]
fn widgets_grow_by_how_much_taller_the_script_face_rows_are() {
    let mut style = Style::default();
    style.spacing.button_padding.y = 5.0;
    let before = style.spacing.interact_size.y;
    test_stack("en").fit_interact_size(&mut style);
    assert_eq!(style.spacing.interact_size.y, before, "Latin rows keep the theme's height, whatever its padding");

    let body = style.text_styles.get(&TextStyle::Body).cloned().expect("body style");
    let row = |stack: &Stack| stack.fonts(false).with_pixels_per_point(1.0).row_height(&body);
    let arabic = test_stack("ar");
    let extra = row(&arabic) - row(&test_stack("en"));
    assert!(extra > 0.0, "Arabic rows are taller");
    arabic.fit_interact_size(&mut style);
    assert!((style.spacing.interact_size.y - (before + extra)).abs() < 0.01, "{} + {extra}", before);
}

#[test]
fn the_fit_is_the_same_whether_the_theme_comes_before_or_after_install() {
    let theme = |ctx: &egui::Context| {
        ctx.all_styles_mut(|s| {
            s.spacing.interact_size.y = 20.0;
            s.spacing.button_padding.y = 5.0;
        });
    };
    let pass = |ctx: &egui::Context| ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    let height = |ctx: &egui::Context| ctx.style_of(egui::Theme::Dark).spacing.interact_size.y;
    for language in ["en", "ar"] {
        let before = egui::Context::default();
        theme(&before);
        test_stack(language).install(&before);
        let after = egui::Context::default();
        test_stack(language).install(&after);
        theme(&after);
        pass(&after);
        assert_eq!(height(&after), height(&before), "{language}");
        assert_eq!(height(&after) == 20.0, language == "en", "{language}: {}", height(&after));
        // Passes without a new theme leave the fit as it is.
        pass(&after);
        assert_eq!(height(&after), height(&before), "{language}");
    }
}

#[test]
fn cjk_uis_keep_the_rows_and_widgets_of_a_latin_ui() {
    let latin_only = || {
        let ubuntu = FontDefinitions::default().font_data.get("Ubuntu-Light").map(|d| (**d).clone()).expect("egui default_fonts");
        UiFonts::new(&I18nScripts).app_face("ui", ubuntu, &[FontFamily::Proportional]).system_fonts(SystemFonts::Off).stack()
    };
    let interact = |stack: &Stack| {
        let ctx = egui::Context::default();
        stack.install(&ctx);
        ctx.style_of(egui::Theme::Dark).spacing.interact_size.y
    };
    let latin = latin_only();
    for language in ["ja", "ko", "zh-CN", "en"] {
        let stack = test_stack(language);
        assert_eq!(stack.chain(&FontFamily::Proportional).first().map(String::as_str), Some("ui"), "{language}");
        assert_eq!(interact(&stack), interact(&latin), "{language}");
        for (size, ppp) in [(14.0, 1.0), (13.0, 1.5), (40.0, 2.0)] {
            let font = FontId::proportional(size);
            let layout = |stack: &Stack, text: &str| {
                stack.fonts(false).with_pixels_per_point(ppp).layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
            };
            let label = layout(&stack, "Ab漢한中");
            let row = egui::Rect::from_x_y_ranges(label.rect.x_range(), layout(&latin, "Ab").rect.y_range());
            assert_eq!(label.rect.height(), row.height(), "{language} at {size} pt");
            // Rasterised glyphs reach at most a pixel past their outlines.
            assert!(row.expand(1.0 / ppp).contains_rect(label.mesh_bounds), "{language} at {size} pt: {:?} in {row:?}", label.mesh_bounds);
        }
    }
}

#[test]
fn cjk_glyphs_sit_on_the_latin_baseline() {
    let stack = test_stack("ja");
    let face = stack.definitions.font_data.get("Noto Sans CJK SC Regular").expect("CJK face");
    let (above, below) = fit::ink_extent(face, "中");
    let size = 40.0;
    let layout =
        |text: &str| stack.fonts(false).with_pixels_per_point(1.0).layout_no_wrap(text.to_owned(), FontId::proportional(size), egui::Color32::WHITE);
    let baseline = layout("A").rows[0].glyphs[0].pos.y;
    let ink = layout("中").mesh_bounds;
    let scaled = face.tweak.scale * size;
    assert!((ink.max.y - (baseline + below * scaled)).abs() <= 1.0, "bottom {} vs baseline {baseline} + {}", ink.max.y, below * scaled);
    assert!((ink.min.y - (baseline - above * scaled)).abs() <= 1.0, "top {} vs baseline {baseline} - {}", ink.min.y, above * scaled);
}

#[test]
fn cjk_rows_grow_only_when_the_app_asks() {
    let mut style = Style::default();
    let before = style.spacing.interact_size.y;
    test_fonts("ja").cjk_rows(CjkRows::Grow).stack().fit_interact_size(&mut style);
    assert!(style.spacing.interact_size.y > before, "CJK rows are taller than the Latin face's");
}

#[test]
fn installing_another_language_refits_widgets() {
    let ctx = egui::Context::default();
    let before = ctx.style_of(egui::Theme::Dark).spacing.interact_size.y;
    test_stack("ar").install(&ctx);
    let arabic = ctx.style_of(egui::Theme::Dark).spacing.interact_size.y;
    assert!(arabic > before);
    test_stack("en").install(&ctx);
    assert_eq!(ctx.style_of(egui::Theme::Dark).spacing.interact_size.y, before, "back to the style's own height");
    assert_eq!(ctx.style_of(egui::Theme::Light).spacing.interact_size.y, before);
    // A height the app set after installing stays.
    ctx.all_styles_mut(|s| s.spacing.interact_size.y = 40.0);
    test_stack("ar").install(&ctx);
    test_stack("en").install(&ctx);
    assert_eq!(ctx.style_of(egui::Theme::Dark).spacing.interact_size.y, 40.0);
}

#[test]
fn install_keeps_the_context_providers_unless_it_brings_a_system_lookup() {
    let arabic = |_: &FallbackRequest<'_>| {
        Some(FontInsert::new(
            "host:Arabic",
            FontData::from_static(ARABIC),
            vec![InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Lowest }],
        ))
    };
    let draws_arabic = |system: SystemFonts| {
        let ctx = egui::Context::default();
        ctx.add_font_provider(Arc::new(arabic));
        let defaults = FontDefinitions::default();
        let ubuntu = defaults.font_data.get("Ubuntu-Light").map(|d| (**d).clone()).expect("egui default_fonts");
        let stack = UiFonts::new(&I18nScripts).app_face("ui", ubuntu, &[FontFamily::Proportional]).system_fonts(system).stack();
        stack.install(&ctx);
        stack.install(&ctx);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(14.0), 'ع'))
    };
    assert!(!draws_arabic(SystemFonts::Off), "Off asks no system fonts");
    if cfg!(not(all(feature = "system_fonts", not(target_arch = "wasm32")))) {
        assert!(draws_arabic(SystemFonts::AfterCraft), "the host's provider stays");
    }
}

#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
#[test]
fn the_system_provider_is_shared_per_locale() {
    let (a, b) = (system_font_provider("ja"), system_font_provider("ja"));
    assert!(a.zip(b).is_some_and(|(a, b)| Arc::ptr_eq(&a, &b)));
}

#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
#[test]
fn c_and_posix_locales_prefer_no_language() {
    use system_index::locale_language;
    for locale in ["C", "POSIX", "C.UTF-8", "C.utf8", "POSIX@euro", ""] {
        assert_eq!(locale_language(locale), None, "{locale}");
    }
    for (locale, tag) in [("zh_TW.UTF-8", "zh-TW"), ("sr_RS@latin", "sr-RS"), ("zh-Hant", "zh-Hant")] {
        assert_eq!(locale_language(locale).map(|l| l.to_string()).as_deref(), Some(tag), "{locale}");
    }
}

#[cfg(all(feature = "system_fonts", not(target_arch = "wasm32")))]
#[test]
fn every_locale_asks_one_index_of_the_installed_fonts() {
    let request = FallbackRequest { cluster: "ب", family: &FontFamily::Proportional };
    for locale in ["ja", "ar", "zh-CN"] {
        system_font_provider(locale).and_then(|provider| provider.font_for(&request));
    }
    assert_eq!(system_index::BUILDS.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn egui_faces_join_the_families_the_app_names() {
    let semibold = FontFamily::Name("semibold".into());
    let stack = fonts("en")
        .family(Family::named("semibold").weight(600))
        .egui_face("Hack", &[FontFamily::Proportional, semibold.clone()])
        .egui_face("no such face", &[FontFamily::Proportional])
        .stack();
    let proportional = stack.chain(&FontFamily::Proportional);
    assert_eq!(proportional.last().map(String::as_str), Some("Hack"), "{proportional:?}");
    assert!(stack.chain(&semibold).iter().any(|n| n == "Hack"));
    assert!(!stack.chain(&FontFamily::Monospace).iter().any(|n| n == "Hack"), "only the families named");
    assert!(stack.definitions.font_data.contains_key("Hack"));
    assert!(!proportional.iter().any(|n| n == "no such face"));
}

#[test]
fn covers_language_checks_every_script_of_the_language() {
    let stack = test_stack("en");
    for language in ["en", "ar", "ja"] {
        assert!(stack.covers_language(language, false), "{language}");
    }
    assert!(!stack.covers_language("ka", false), "no Georgian face");
    // Not offered: the scripts come from craft-i18n.
    assert!(stack.covers_language("zh-CN", false));
    assert!(!stack.covers_language("th", false));
}

/// Coverage of every character, for faces whose tests are about scripts.
const ALL: &[u8] = &[0, 0xFF, 0xFF, 0x43];

const fn served(family: &'static str, style: &'static str, scripts: &'static [&'static str], url: &'static str, index: u32) -> ServedFace {
    ServedFace { family, style, scripts, index, url, size: 1, integrity: "", coverage: ALL }
}

fn coverage_of(font: &[u8]) -> &'static [u8] {
    Box::leak(craft_fonts_build::coverage::of_face(font, 0).expect("a font").into_boxed_slice())
}

fn coverage_of_chars(chars: &str) -> &'static [u8] {
    let mut ranges: Vec<(u32, u32)> = chars.chars().map(|c| (u32::from(c), u32::from(c))).collect();
    ranges.sort_unstable();
    Box::leak(craft_fonts_build::coverage::encode(&ranges).into_boxed_slice())
}

/// craft-fonts' faces as a web build serves them; the CJK collection's faces share one file.
const SERVED: &[ServedFace] = &[
    served("BIZ UDPGothic", "Regular", &["Jpan", "Latn"], "fonts/biz-ud-pgothic/BIZUDPGothic-Regular.ttf", 0),
    served("BIZ UDPGothic", "Bold", &["Jpan", "Latn"], "fonts/biz-ud-pgothic/BIZUDPGothic-Bold.ttf", 0),
    served("Noto Sans CJK SC", "Regular", &["Hans", "Latn"], "fonts/noto-sans-cjk/NotoSansCJK.ttc", 2),
    served("Noto Sans CJK TC", "Regular", &["Hant", "Latn"], "fonts/noto-sans-cjk/NotoSansCJK.ttc", 3),
    served("Noto Sans Arabic", "Regular", &["Arab"], "fonts/noto-sans-arabic/NotoSansArabic.ttf", 0),
    served("Noto Sans Thai", "Regular", &["Thai"], "fonts/noto-sans-thai/NotoSansThai.ttf", 0),
];

fn web_fonts(ui_language: &str, served: &[ServedFace]) -> UiFonts<'static> {
    let defaults = FontDefinitions::default();
    let egui_face = |name: &str| defaults.font_data.get(name).map(|d| (**d).clone()).expect("egui default_fonts in dev-dependencies");
    UiFonts::new(&I18nScripts)
        .app_face("ui", egui_face("Ubuntu-Light"), &[FontFamily::Proportional])
        .app_face("mono", egui_face("Hack"), &[FontFamily::Monospace])
        .family(Family::named("semibold").weight(600))
        .served_faces(served)
        .languages(OFFERED)
        .ui_language(ui_language)
        .system_fonts(SystemFonts::Off)
}

fn names(faces: Vec<&ServedFace>) -> Vec<String> {
    faces.into_iter().map(ServedFace::name).collect()
}

#[test]
fn served_faces_take_the_places_of_embedded_ones_without_data() {
    let embedded: Vec<CraftFace> = SERVED.iter().map(ServedFace::craft_face).collect();
    for language in ["en", "ar", "ja", "zh-TW", "th"] {
        let web = web_fonts(language, SERVED).stack();
        let desktop = web_fonts(language, &[]).craft_faces(&embedded).stack();
        for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name("semibold".into())] {
            assert_eq!(web.chain(&family), desktop.chain(&family), "{language} {family:?}");
        }
        assert!(SERVED.iter().all(|face| !web.definitions.font_data.contains_key(&face.name())), "{language}");
        assert_eq!(web.served().len(), SERVED.len());
    }
    // Served faces count before they are fetched: they are fetched when text needs them.
    let stack = web_fonts("en", SERVED).stack();
    assert!(stack.covers_language("ar", false) && stack.covers_language("ja", false));
    assert!(!stack.covers_language("km", false));
}

#[test]
fn the_ui_language_downloads_its_own_faces_first() {
    let cases: &[(&str, &[&str])] = &[
        ("en", &[]),
        ("ru", &[]),
        ("ar", &["Noto Sans Arabic Regular"]),
        // The semibold family takes the bold face.
        ("ja", &["BIZ UDPGothic Regular", "BIZ UDPGothic Bold"]),
        ("zh-TW", &["Noto Sans CJK TC Regular"]),
        ("zh-CN", &["Noto Sans CJK SC Regular"]),
        ("th", &["Noto Sans Thai Regular"]),
    ];
    for (language, want) in cases {
        assert_eq!(names(web_fonts(language, SERVED).stack().ui_downloads()), *want, "{language}");
    }
}

#[test]
fn a_missing_character_downloads_the_face_for_its_script() {
    let stack = web_fonts("en", SERVED).stack();
    let regular = FontFamily::Proportional;
    assert_eq!(names(stack.downloads_for(&regular, 'ع')), ["Noto Sans Arabic Regular"]);
    // Han follows the offered languages: Japanese before Chinese.
    assert_eq!(names(stack.downloads_for(&regular, '日'))[..2], ["BIZ UDPGothic Regular", "Noto Sans CJK SC Regular"]);
    assert_eq!(names(stack.downloads_for(&FontFamily::Name("semibold".into()), 'あ'))[0], "BIZ UDPGothic Bold");
    // The CJK faces' Latin doesn't stand in for a Latin face.
    for c in ['a', 'ж', 'λ', '1', 'ổ', '🔍', '⌘'] {
        assert!(stack.downloads_for(&regular, c).is_empty(), "{c}");
    }
    let twemoji = served("Twemoji", "Regular", &[], "fonts/Twemoji.ttf", 0);
    let stack = web_fonts("en", SERVED).emoji(Emoji { served: Some(twemoji), ..Emoji::default() }).stack();
    for c in ['🔍', '⌚', '✅', '⭐'] {
        assert_eq!(names(stack.downloads_for(&regular, c)), ["Twemoji Regular"], "{c}");
    }
    assert!(stack.downloads_for(&regular, '⌘').is_empty(), "a symbol, not an emoji");
    assert!(stack.ui_downloads().is_empty());

    assert_eq!(served::script_of('\u{0600}'), Some("Arab"));
    assert_eq!(served::script_of('\u{05FF}'), Some("Hebr"));
    assert_eq!(served::script_of('한'), Some("Hang"));
    assert_eq!(served::script_of('\u{2F800}'), Some("Hani"));
    assert_eq!(served::script_of('\u{10FFFF}'), None);
}

#[test]
fn a_fetched_file_reaches_every_face_and_family_that_uses_it() {
    let mut stack = web_fonts("en", SERVED).stack();
    let font_id = |family: FontFamily| FontId::new(14.0, family);
    let semibold = FontFamily::Name("semibold".into());
    assert!(!stack.fonts(false).has_glyph(&font_id(FontFamily::Proportional), 'ع'));

    // The Arabic test face is variable, so the semibold family gets its own instance at 600.
    let file: egui::epaint::text::Blob = Arc::new(ARABIC.to_vec());
    stack.provide("fonts/noto-sans-arabic/NotoSansArabic.ttf", &file);
    let mut fonts = stack.fonts(false);
    assert!(fonts.has_glyph(&font_id(FontFamily::Proportional), 'ع'));
    assert!(fonts.has_glyph(&font_id(semibold.clone()), 'ع'));
    assert!(stack.chain(&FontFamily::Proportional).contains(&"Noto Sans Arabic Regular".to_owned()));
    assert!(stack.chain(&semibold).contains(&"Noto Sans Arabic Regular @600".to_owned()));
    assert!(!stack.definitions.font_data.contains_key("Noto Sans Thai Regular"));

    // Both faces of a collection come from one file.
    let file: egui::epaint::text::Blob = Arc::new(CJK.to_vec());
    stack.provide("fonts/noto-sans-cjk/NotoSansCJK.ttc", &file);
    for name in ["Noto Sans CJK SC Regular", "Noto Sans CJK TC Regular"] {
        assert!(stack.definitions.font_data.contains_key(name), "{name}");
    }
}

#[test]
fn fetched_cjk_faces_fit_the_latin_rows_as_embedded_ones_do() {
    const SC: &[ServedFace] = &[served("Noto Sans CJK SC", "Regular", &["Hans", "Latn"], "fonts/sc.otf", 0)];
    let mut web = web_fonts("zh-CN", SC).stack();
    web.provide("fonts/sc.otf", &(Arc::new(CJK.to_vec()) as egui::epaint::text::Blob));
    let embedded = CraftFace { bytes: CJK, ..SC[0].craft_face() };
    let desktop = web_fonts("zh-CN", &[]).craft_faces(&[embedded]).stack();
    let tweak = |stack: &Stack| {
        let tweak = &stack.definitions.font_data.get("Noto Sans CJK SC Regular").expect("CJK face").tweak;
        (tweak.scale, tweak.y_offset_factor)
    };
    assert_ne!(tweak(&desktop).1, 0.0, "the CJK face moves onto the Latin baseline");
    assert_eq!(tweak(&web), tweak(&desktop));
}

/// The CJK interface faces as `Embed::subset_to` embeds them, with Noto Sans CJK SC as the
/// Simplified one.
fn ui_faces(others: &'static [u8]) -> [CraftFace; 5] {
    [
        CraftFace { family: "Noto Sans SC UI", style: "Regular", scripts: &["Hans", "Latn"], bytes: CJK, index: 0 },
        CraftFace { family: "Noto Sans TC UI", style: "Regular", scripts: &["Hant", "Latn"], bytes: others, index: 0 },
        CraftFace { family: "Noto Sans HK UI", style: "Regular", scripts: &["Hant-HK", "Latn"], bytes: others, index: 0 },
        CraftFace { family: "Noto Sans KR UI", style: "Regular", scripts: &["Kore", "Latn"], bytes: others, index: 0 },
        CraftFace { family: "BIZ UDPGothic", style: "Regular", scripts: &["Jpan", "Latn"], bytes: others, index: 0 },
    ]
}

#[test]
fn preferences_name_the_interface_faces_too() {
    let order = |faces: &[CraftFace], prefer: Option<&str>| {
        let mut fonts = UiFonts::new(&I18nScripts).craft_faces(faces).ui_language("ja").system_fonts(SystemFonts::Off);
        if let Some(family) = prefer {
            fonts = fonts.prefer("Jpan", family);
        }
        let stack = fonts.stack();
        stack.chain(&FontFamily::Proportional).iter().filter(|n| faces.iter().any(|f| f.name() == **n)).cloned().collect::<Vec<_>>()
    };
    let mincho = face("Shippori Mincho", "Regular", &["Jpan", "Latn"]);
    let gothic = ui_faces(&[])[4];
    // craft-fonts' preference for BIZ UDPGothic.
    assert_eq!(order(&[mincho, gothic], None), ["BIZ UDPGothic Regular", "Shippori Mincho Regular"]);
    // The app's.
    let mincho_ui = face("Shippori Mincho UI", "Regular", &["Jpan", "Latn"]);
    assert_eq!(order(&[gothic, mincho_ui], Some("Shippori Mincho")), ["Shippori Mincho UI Regular", "BIZ UDPGothic Regular"]);
}

#[test]
fn a_taiwanese_ui_takes_characters_no_interface_face_has_from_the_system_fonts() {
    // Noto Sans stands in for faces cut without 中 and 文; the system fonts have them.
    const NOTO_SANS: &[u8] = include_bytes!("../../tests/fonts/NotoSans-test.ttf");
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let stack = |simplified: &'static [u8]| {
        let mut faces = ui_faces(NOTO_SANS);
        faces[0].bytes = simplified;
        let recorded = Arc::clone(&asked);
        let system = move |request: &FallbackRequest<'_>| {
            recorded.lock().map(|mut asked| asked.push(request.cluster.to_owned())).ok()?;
            Some(FontInsert::new(
                "system:Noto Sans CJK SC",
                FontData::from_static(CJK),
                vec![InsertFontFamily { family: request.family.clone(), priority: FontPriority::Lowest }],
            ))
        };
        UiFonts::new(&I18nScripts)
            .app_face("ui", FontData::from_static(NOTO_SANS), &[FontFamily::Proportional])
            .craft_faces(&faces)
            .languages(&["en", "zh-TW", "zh-CN", "ja", "ko"])
            .ui_language("zh-TW")
            .system_provider(Arc::new(system))
            .stack()
    };
    let sample = [Sample { language: "zh-TW", key: "menu.language", text: "中文" }];
    let missing = |stack: &Stack, with_system| -> Vec<char> {
        missing_glyphs(stack, &FontFamily::Proportional, sample, with_system).iter().map(|m| m.character).collect()
    };

    let without = stack(NOTO_SANS);
    // Other regions' interface faces come before the system fonts, so they draw what they have.
    assert_eq!(head(&without, &FontFamily::Proportional, 4), ["ui", "Noto Sans TC UI Regular", "Noto Sans HK UI Regular", "Noto Sans SC UI Regular"]);
    assert_eq!(missing(&without, false), ['中', '文']);
    assert_eq!(missing(&without, true), Vec::<char>::new());
    // The face the system provider returns for 中 draws 文 too.
    assert_eq!(*asked.lock().unwrap(), ["中"]);

    asked.lock().unwrap().clear();
    let with_simplified = stack(CJK);
    assert_eq!(missing(&with_simplified, true), Vec::<char>::new());
    assert!(asked.lock().unwrap().is_empty(), "the Simplified interface face has them");
}

#[test]
fn a_web_build_downloads_the_interface_face_for_its_language() {
    let faces = [
        served("Noto Sans SC UI", "Regular", &["Hans", "Latn"], "fonts/ui/NotoSansSCUI-Regular.0123456789abcdef.ttf", 0),
        served("Noto Sans TC UI", "Regular", &["Hant", "Latn"], "fonts/ui/NotoSansTCUI-Regular.0123456789abcdef.ttf", 0),
        served("BIZ UDPGothic", "Regular", &["Jpan", "Latn"], "fonts/biz-ud-pgothic/BIZUDPGothic-Regular.0123456789abcdef.ttf", 0),
        served("Noto Sans Arabic", "Regular", &["Arab"], "fonts/noto-sans-arabic/NotoSansArabic.ttf", 0),
    ];
    // The interface faces are Regular only: the semibold family takes that face too.
    for (language, want) in [("zh-TW", "Noto Sans TC UI Regular"), ("zh-CN", "Noto Sans SC UI Regular"), ("ja", "BIZ UDPGothic Regular")] {
        assert_eq!(names(web_fonts(language, &faces).stack().ui_downloads()), [want], "{language}");
    }
}

/// craft-fonts' faces for Latin, Greek and Cyrillic, symbols and emoji, as a web build serves them.
const SERVED_SYMBOLS: &[ServedFace] = &[
    served("Noto Sans", "Regular", &["Latn", "Cyrl", "Grek", "Deva"], "fonts/noto-sans/NotoSans.ttf", 0),
    served("Noto Color Emoji", "Regular", &["Zsye"], "fonts/noto-color-emoji/NotoColorEmoji.ttf", 0),
    served("Noto Sans Symbols", "Regular", &["Zsym"], "fonts/noto-sans-symbols/NotoSansSymbols.ttf", 0),
    served("Noto Sans Symbols 2", "Regular", &["Zsym"], "fonts/noto-sans-symbols-2/NotoSansSymbols2-Regular.ttf", 0),
    served("Noto Sans Math", "Regular", &["Zmth", "Zsym"], "fonts/noto-sans-math/NotoSansMath-Regular.ttf", 0),
];

#[test]
fn letters_symbols_and_emoji_download_their_own_kind_of_face() {
    let stack = web_fonts("en", &[SERVED, SERVED_SYMBOLS].concat()).stack();
    let regular = FontFamily::Proportional;
    let first = |c: char| names(stack.downloads_for(&regular, c)).first().cloned().unwrap_or_default();
    for c in ['ổ', 'ẫ', 'ạ', 'ư', 'λ', 'ῶ', 'ж', 'ӂ', 'ǅ', '₫'] {
        assert_eq!(names(stack.downloads_for(&regular, c)), ["Noto Sans Regular"], "{c}");
    }
    let symbols = ["Noto Sans Symbols Regular", "Noto Sans Symbols 2 Regular", "Noto Sans Math Regular"];
    for c in ['⌘', '⌥', '⇧', '⌃', '⎋', '⏎', '⌫', '⇥', '↩', '⌦', '⇪', '⏏', '☰', '✓', '♠', '🁣', '♔'] {
        assert_eq!(names(stack.downloads_for(&regular, c)), symbols, "{c}");
    }
    for c in ['⊞', '∀', '⟨', '𝔸'] {
        assert_eq!(first(c), "Noto Sans Math Regular", "{c}");
    }
    for c in ['🔍', '😀', '⌚', '☕', '✅', '⭐', '🃏'] {
        assert_eq!(names(stack.downloads_for(&regular, c)), ["Noto Color Emoji Regular"], "{c}");
    }
    // A variation selector chooses between the two.
    assert_eq!(served::script_of_cluster("↩\u{FE0F}"), Some("Zsye"));
    assert_eq!(served::script_of_cluster("⌚\u{FE0E}"), Some("Zsym"));
    assert_eq!(served::script_of_cluster("1\u{FE0F}\u{20E3}"), Some("Zsye"));
    assert_eq!(served::script_of_cluster("↩"), Some("Zsym"));
}

/// Fetches nothing until the test answers, on a clock the test moves.
#[derive(Default)]
struct FakeFetch {
    pending: Mutex<Vec<(&'static str, served::loader::Done)>>,
    requested: Mutex<Vec<&'static str>>,
    now: Mutex<f64>,
    timers: Mutex<Vec<(f64, served::loader::Wake)>>,
}

impl served::loader::Fetcher for FakeFetch {
    fn fetch(&self, url: &'static str, _integrity: &'static str, done: served::loader::Done) {
        self.requested.lock().unwrap().push(url);
        self.pending.lock().unwrap().push((url, done));
    }

    fn now(&self) -> f64 {
        *self.now.lock().unwrap()
    }

    fn after(&self, seconds: f64, wake: served::loader::Wake) {
        let at = self.now() + seconds;
        self.timers.lock().unwrap().push((at, wake));
    }
}

impl FakeFetch {
    /// The URLs requested since the last call.
    fn requested(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.requested.lock().unwrap())
    }

    fn answer(&self, url: &str, result: Result<&[u8], &str>) {
        let mut pending = self.pending.lock().unwrap();
        let i = pending.iter().position(|(u, _)| *u == url).unwrap_or_else(|| panic!("{url} is not being fetched"));
        let (_, done) = pending.remove(i);
        drop(pending);
        done(result.map(<[u8]>::to_vec).map_err(str::to_owned));
    }

    fn wait(&self, seconds: f64) {
        let now = {
            let mut now = self.now.lock().unwrap();
            *now += seconds;
            *now
        };
        let due: Vec<_> = {
            let mut timers = self.timers.lock().unwrap();
            let (due, later) = std::mem::take(&mut *timers).into_iter().partition(|(at, _)| *at <= now);
            *timers = later;
            due
        };
        for (_, wake) in due {
            wake();
        }
    }
}

const NOTO_SANS_URL: &str = "fonts/noto-sans/NotoSans.ttf";
const SYMBOLS_2_URL: &str = "fonts/noto-sans-symbols-2/NotoSansSymbols2-Regular.ttf";
const MATH_URL: &str = "fonts/noto-sans-math/NotoSansMath-Regular.ttf";

/// An English UI on the web whose faces lack Vietnamese ổ and the keyboard keys, with `fetch`
/// fetching its served faces; and whether the context draws a character, as a pass would ask.
fn web_context(fetch: &Arc<FakeFetch>) -> (egui::Context, impl Fn(char) -> bool) {
    let mut faces = SERVED_SYMBOLS.to_vec();
    faces[0].coverage = coverage_of(NOTO_SANS_LATIN);
    faces[2].coverage = coverage_of(SYMBOLS);
    faces[3].coverage = coverage_of(SYMBOLS_2);
    // Arrows and technical symbols, ⌥ and ⇥ among them; never served.
    faces[4].coverage = craft_fonts_build::coverage::encode(&[(0x2190, 0x21FF), (0x2300, 0x23FF)]).leak();
    web_context_with("en", &faces, fetch)
}

/// A web context in `ui_language` with these served faces, fetched by `fetch`; and whether the
/// context draws a character, as a pass would ask.
fn web_context_with(ui_language: &str, faces: &[ServedFace], fetch: &Arc<FakeFetch>) -> (egui::Context, impl Fn(char) -> bool + use<>) {
    let ctx = egui::Context::default();
    let stack = web_fonts(ui_language, faces).stack();
    served::loader::install(&ctx, stack, Arc::clone(fetch) as Arc<dyn served::loader::Fetcher>);
    let draws = {
        let ctx = ctx.clone();
        move |c: char| {
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
            ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(14.0), c))
        }
    };
    (ctx, draws)
}

#[test]
fn a_missing_character_fetches_only_the_face_that_draws_it() {
    let fetch = Arc::new(FakeFetch::default());
    let (_ctx, draws) = web_context(&fetch);
    assert!(fetch.requested().is_empty(), "English needs no served face up front");

    assert!(!draws('ổ'));
    assert_eq!(fetch.requested(), [NOTO_SANS_URL]);
    // While a face is on its way, characters it draws wait for it.
    assert!(!draws('ạ'));
    assert!(fetch.requested().is_empty());
    fetch.answer(NOTO_SANS_URL, Ok(NOTO_SANS_LATIN));
    assert!(draws('ổ') && draws('ạ'));

    // ⌥ is in the second symbol face only: the first is never fetched for it.
    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [SYMBOLS_2_URL]);
    fetch.answer(SYMBOLS_2_URL, Ok(SYMBOLS_2));
    assert!(draws('⌥') && draws('⌫'));
    assert!(fetch.requested().is_empty());

    // A letter no served face has fetches nothing.
    assert!(!draws('ẫ'));
    assert!(fetch.requested().is_empty(), "the emoji face is never a candidate for letters or keys");
}

#[test]
fn a_han_character_fetches_the_one_interface_face_that_has_it() {
    let cut = |family: &'static str, scripts: &'static [&'static str], url: &'static str, chars: &str| ServedFace {
        coverage: coverage_of_chars(chars),
        ..served(family, "Regular", scripts, url, 0)
    };
    let faces = [
        cut("Noto Sans SC UI", &["Hans", "Latn"], "fonts/ui/SC.ttf", "中文日本语ổ"),
        cut("Noto Sans TC UI", &["Hant", "Latn"], "fonts/ui/TC.ttf", "中文日本語體ổ"),
        cut("Noto Sans HK UI", &["Hant-HK", "Latn"], "fonts/ui/HK.ttf", "中文日本語ổ"),
        cut("Noto Sans KR UI", &["Kore", "Latn"], "fonts/ui/KR.ttf", "中文日本語한ổ"),
        cut("BIZ UDPGothic", &["Jpan", "Latn"], "fonts/ui/JP.ttf", "中文日本語のổ"),
        ServedFace { coverage: coverage_of(NOTO_SANS_LATIN), ..SERVED_SYMBOLS[0] },
    ];
    let fetch = Arc::new(FakeFetch::default());
    let (_ctx, draws) = web_context_with("zh-Hans", &faces, &fetch);
    assert_eq!(fetch.requested(), ["fonts/ui/SC.ttf"], "the UI language's face, at once");
    fetch.answer("fonts/ui/SC.ttf", Ok(CJK));

    // 體 is in the Traditional face only.
    assert!(!draws('體'));
    assert_eq!(fetch.requested(), ["fonts/ui/TC.ttf"]);
    fetch.answer("fonts/ui/TC.ttf", Ok(CJK));
    assert!(!draws('體'), "the test file lacks it");
    assert!(fetch.requested().is_empty(), "a fetched face is not fetched again, and no other face has it");
    // A rare character no face has fetches nothing: tofu or the browser draws it.
    assert!(!draws('龘'));
    assert!(fetch.requested().is_empty());
    // Every cut has ổ, but Latin letters come from Noto Sans.
    assert!(!draws('ổ'));
    assert_eq!(fetch.requested(), [NOTO_SANS_URL]);

    // Among faces that have a character, the UI language's region comes first.
    let both =
        [cut("Noto Sans TC UI", &["Hant", "Latn"], "fonts/ui/TC.ttf", "體"), cut("Noto Sans HK UI", &["Hant-HK", "Latn"], "fonts/ui/HK.ttf", "體")];
    for (language, want) in [("zh-Hant", "Noto Sans TC UI Regular"), ("zh-HK", "Noto Sans HK UI Regular"), ("zh-Hans", "Noto Sans TC UI Regular")] {
        assert_eq!(names(web_fonts(language, &both).stack().downloads_for(&FontFamily::Proportional, '體'))[0], want, "{language}");
    }
    let regular = FontFamily::Proportional;
    for (language, want) in [
        ("zh-Hans", "Noto Sans SC UI Regular"),
        ("zh-Hant", "Noto Sans TC UI Regular"),
        ("zh-HK", "Noto Sans HK UI Regular"),
        ("ja", "BIZ UDPGothic Regular"),
        ("ko", "Noto Sans KR UI Regular"),
    ] {
        assert_eq!(names(web_fonts(language, &faces).stack().downloads_for(&regular, '中'))[0], want, "{language}");
    }
}

#[test]
fn a_failed_download_moves_on_and_is_retried_later() {
    let fetch = Arc::new(FakeFetch::default());
    let (_ctx, draws) = web_context(&fetch);

    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [SYMBOLS_2_URL]);
    fetch.answer(SYMBOLS_2_URL, Err("HTTP 503"));
    // The next face that draws it is fetched at once.
    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [MATH_URL]);
    fetch.answer(MATH_URL, Err("HTTP 503"));
    assert!(!draws('⌥'));
    assert!(fetch.requested().is_empty(), "failed faces wait for their retry time");

    // Failures of one script don't stop another's face.
    assert!(!draws('ổ'));
    assert_eq!(fetch.requested(), [NOTO_SANS_URL]);
    fetch.answer(NOTO_SANS_URL, Ok(NOTO_SANS_LATIN));
    assert!(draws('ổ'));

    // At their retry time the failed faces are fetched again, in order.
    fetch.wait(1.0);
    assert!(!draws('⌥'));
    assert!(fetch.requested().is_empty());
    fetch.wait(1.0);
    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [SYMBOLS_2_URL]);
    fetch.answer(SYMBOLS_2_URL, Ok(SYMBOLS_2));
    assert!(draws('⌥'));

    // A face that fails again waits twice as long.
    assert!(!draws('⇥'));
    assert_eq!(fetch.requested(), [MATH_URL]);
    fetch.answer(MATH_URL, Err("HTTP 503"));
    fetch.wait(3.0);
    assert!(!draws('⇥'));
    assert!(fetch.requested().is_empty());
    fetch.wait(1.0);
    assert!(!draws('⇥'));
    assert_eq!(fetch.requested(), [MATH_URL]);
}
