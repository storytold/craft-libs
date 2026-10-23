use super::*;
use egui::{FallbackRequest, FontInsert, FontPriority, InsertFontFamily, TextStyle};

const ARABIC: &[u8] = include_bytes!("../../tests/fonts/NotoSansArabic-test.ttf");
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
    let emoji = Emoji { color: Some(FontData::from_static(&[])), web: None, monochrome: Some(FontData::from_static(&[])) };
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
fn cjk_row_samples_are_in_every_cut_face() {
    use skrifa::{FontRef, MetadataProvider as _};
    let kept = |c: char| craft_fonts_build::KEPT_IN_EVERY_CUT.iter().any(|&(start, end)| (start..=end).contains(&c));
    assert!(fit::ROW_SAMPLES.chars().all(kept), "{}", fit::ROW_SAMPLES);
    // The test face has every sample, so the row tests above measure all of them.
    let charmap = FontRef::new(CJK).expect("CJK test face").charmap();
    assert!(fit::ROW_SAMPLES.chars().all(|c| charmap.map(c).is_some()), "{}", fit::ROW_SAMPLES);
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
