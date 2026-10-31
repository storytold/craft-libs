use serde_json::json;

use crate::*;

const COMMIT: &str = "45e3d5269e4a17d823f6d84470bc084dc5ae46f6";

/// A font with the tables a scan and [`is_font`] need: `name`, `OS/2` and an empty `glyf`.
fn font(family: &str, style: &str, weight: u16) -> Vec<u8> {
    use write_fonts::FontBuilder;
    use write_fonts::tables::name::{Name, NameRecord};
    use write_fonts::tables::os2::Os2;
    use write_fonts::types::{NameId, Tag};
    let ps = format!("{}-{}", family.replace(' ', ""), style.replace(' ', ""));
    let mut records: Vec<NameRecord> = [(1, family.to_string()), (2, style.to_string()), (4, format!("{family} {style}")), (6, ps)]
        .into_iter()
        .map(|(id, text)| NameRecord::new(3, 1, 0x409, NameId::new(id), text.into()))
        .collect();
    records.sort_by_key(|r| r.name_id);
    let mut builder = FontBuilder::new();
    builder.add_table(&Name::new(records)).unwrap();
    builder.add_table(&Os2 { us_weight_class: weight, ul_code_page_range_1: Some(1), ul_code_page_range_2: Some(0), ..Default::default() }).unwrap();
    builder.add_raw(Tag::new(b"glyf"), vec![0u8; 4]);
    builder.build()
}

/// An index and the files it lists, by their URL-encoded google/fonts paths.
struct Fixture {
    files: Vec<(String, Vec<u8>)>,
    index: serde_json::Value,
}

fn file_entry(path: &str, bytes: &[u8], style: &str, weight: u16, axes: serde_json::Value, ps: &[&str]) -> serde_json::Value {
    json!({"path": path, "size": bytes.len(), "sha256": sha256_hex(bytes), "style": style, "weight": weight, "axes": axes, "postscript": ps})
}

/// An index of four families, and the bytes of their files.
fn fixture() -> Fixture {
    let regular = font("Test Sans", "Regular", 400);
    let bold = font("Test Sans", "Bold", 700);
    let italic = font("Test Sans", "Italic", 400);
    let var = font("Test Var", "Regular", 400);
    let one = font("Shared One", "Regular", 400);
    let two = font("Shared Two", "Regular", 400);
    let index = json!({
        "schema": 1, "version": "1.0", "source": "google/fonts", "commit": COMMIT,
        "base_url": format!("https://cdn.jsdelivr.net/gh/google/fonts@{COMMIT}/"),
        "fallback_base_url": format!("https://raw.githubusercontent.com/google/fonts/{COMMIT}/"),
        "a_field_from_a_later_minor_version": true,
        "languages": [{"id": "en_Latn", "name": "English", "autonym": "English", "script": "Latn", "population": 1, "historical": false}],
        "scripts": [{"id": "Latn", "name": "Latin"}],
        "families": [
            {"family": "Shared One", "license": "OFL", "license_path": null, "languages": [0], "scripts": ["Latn"],
             "files": [file_entry("ofl/sharedone/SharedOne-Regular.ttf", &one, "normal", 400, json!([]), &["Shared-Regular"])]},
            {"family": "Shared Two", "license": "OFL", "license_path": null,
             "files": [file_entry("ofl/sharedtwo/SharedTwo-Regular.ttf", &two, "normal", 400, json!([]), &["Shared-Regular"])]},
            {"family": "Test Sans", "designer": "Someone", "categories": ["SANS_SERIF"], "license": "OFL", "license_path": "ofl/testsans/OFL.txt",
             "languages": [0], "scripts": ["Latn"], "files": [
                file_entry("ofl/testsans/TestSans-Bold.ttf", &bold, "normal", 700, json!([]), &["TestSans-Bold"]),
                file_entry("ofl/testsans/TestSans-Italic.ttf", &italic, "italic", 400, json!([]), &["TestSans-Italic"]),
                file_entry("ofl/testsans/TestSans-Regular.ttf", &regular, "normal", 400, json!([]), &["TestSans-Regular"])]},
            {"family": "Test Var", "license": "APACHE2", "license_path": "apache/testvar/LICENSE.txt", "files": [
                file_entry("apache/testvar/TestVar[wght].ttf", &var, "normal", 400, json!([{"tag": "wght", "min": 300.0, "max": 700.0}]),
                    &["TestVar-Light", "TestVar-Regular", "TestVar-SemiBold", "TestVar-Bold"])]},
        ]
    });
    let files = vec![
        ("ofl/sharedone/SharedOne-Regular.ttf".into(), one),
        ("ofl/sharedtwo/SharedTwo-Regular.ttf".into(), two),
        ("ofl/testsans/TestSans-Bold.ttf".into(), bold),
        ("ofl/testsans/TestSans-Italic.ttf".into(), italic),
        ("ofl/testsans/TestSans-Regular.ttf".into(), regular),
        ("ofl/testsans/OFL.txt".into(), b"SIL Open Font License".to_vec()),
        ("apache/testvar/TestVar%5Bwght%5D.ttf".into(), var),
        ("apache/testvar/LICENSE.txt".into(), b"Apache License".to_vec()),
    ];
    Fixture { files, index }
}

fn index_of(v: &serde_json::Value) -> Result<Index, IndexError> {
    Index::from_json(v.to_string().as_bytes())
}

#[test]
fn the_index_is_read_and_checked() {
    let f = fixture();
    let index = index_of(&f.index).unwrap();
    let source = &index.sources()[0];
    assert_eq!((source.version.as_str(), source.commit.as_str(), index.families().len()), ("1.0", COMMIT, 4));
    // A format 1.0 index is Google Fonts, curated, with URLs from its base URLs.
    assert_eq!((source.id.as_str(), source.name.as_str(), source.curated), (GOOGLE_FONTS, "Google Fonts", true));
    let sans = index.family("test sans").unwrap();
    assert_eq!((sans.dir(), sans.license_file_name(), sans.designer.as_str()), ("testsans", Some("OFL.txt"), "Someone"));
    assert_eq!((sans.source.as_str(), sans.is_curated()), (GOOGLE_FONTS, true));
    assert_eq!(sans.languages(&index).map(|l| l.id.as_str()).collect::<Vec<_>>(), ["en_Latn"]);

    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = f.index.clone();
        edit(&mut v);
        index_of(&v)
    };
    assert!(with(&|v| v["version"] = json!("1.7")).is_ok(), "a later minor version only adds fields");
    assert!(matches!(with(&|v| v["version"] = json!("2.0")), Err(IndexError::Version(_))));
    assert!(matches!(with(&|v| v["version"] = json!("1")), Err(IndexError::Version(_))));
    assert!(matches!(
        with(&|v| {
            v.as_object_mut().unwrap().remove("version");
        }),
        Err(IndexError::Version(_))
    ));
    // A version 2 index may change any other field: the version is checked first.
    assert!(matches!(
        with(&|v| {
            v["version"] = json!("2.0");
            v["families"] = json!("other");
        }),
        Err(IndexError::Version(_))
    ));
    let invalid = [
        with(&|v| v["base_url"] = json!("http://cdn.jsdelivr.net/gh/google/fonts/")),
        with(&|v| v["commit"] = json!("main")),
        with(&|v| v["families"][2]["files"][0]["path"] = json!("ofl/../TestSans-Bold.ttf")),
        with(&|v| v["families"][2]["files"][0]["path"] = json!("ofl/testsans/.hidden.ttf")),
        with(&|v| v["families"][2]["files"][0]["path"] = json!("ofl/other/TestSans-Bold.ttf")),
        with(&|v| v["families"][2]["files"][0]["sha256"] = json!("00")),
        with(&|v| v["families"][2]["files"][0]["weight"] = json!(0)),
        with(&|v| v["families"][2]["license_path"] = json!("ofl/elsewhere/OFL.txt")),
        with(&|v| v["families"][2]["license_path"] = json!("ofl/testsans/TestSans-Bold.ttf")),
        with(&|v| v["families"][2]["source"] = json!("notofonts")),
        with(&|v| v["base_url"] = json!("https://cdn.jsdelivr.net/gh/{repo}/")),
        with(&|v| v["families"][2]["languages"] = json!([5])),
        with(&|v| v["families"][3]["family"] = json!("TEST SANS")),
    ];
    for (i, r) in invalid.into_iter().enumerate() {
        assert!(matches!(r, Err(IndexError::Invalid(_))), "case {i}: {r:?}");
    }
    assert!(matches!(Index::from_json(b"{"), Err(IndexError::Json(_))));
}

#[cfg(feature = "bundled-index")]
#[test]
fn the_bundled_indexes_are_the_pinned_ones() {
    assert_eq!(sha256_hex(include_bytes!("../index/sources.json")), PINNED_INDEX.sha256);
    let index = Index::bundled().unwrap();
    let ids: Vec<&str> = index.sources().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [GOOGLE_FONTS, "notofonts"]);
    assert!(index.sources().iter().all(|s| s.curated && s.warnings.is_empty()));
    assert!(index.problems().is_empty());
    assert!(index.families().len() > 2000);
    let face = index.find_face("Lato", "Bold Italic").unwrap();
    assert_eq!((face.file.file_name(), face.family.dir()), ("Lato-BoldItalic.ttf", "lato"));

    // Noto's own families, in folders of their own.
    let cjk = index.family("Noto Sans CJK JP").unwrap();
    assert_eq!((cjk.source.as_str(), cjk.dir(), cjk.license_file_name()), ("notofonts", "notofonts/noto-sans-cjk-jp", Some("LICENSE")));
    assert!(index.file_urls(cjk, &cjk.files[0])[0].starts_with("https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@Sans2.004/Sans/OTF/Japanese/"));
    // A family in both sources: Google Fonts first, Noto's with more styles after it.
    let both: Vec<&str> = index.families_named("Noto Serif Myanmar").map(|f| f.source.as_str()).collect();
    assert_eq!(both, [GOOGLE_FONTS, "notofonts"]);
    let noto = index.family_from("notofonts", "Noto Serif Myanmar").unwrap();
    assert_eq!(
        index.license_urls(noto),
        [
            "https://cdn.jsdelivr.net/gh/notofonts/myanmar@NotoSerifMyanmar-v2.106/OFL.txt",
            "https://raw.githubusercontent.com/notofonts/myanmar/NotoSerifMyanmar-v2.106/OFL.txt"
        ]
    );
    // Languages are merged: every family's positions name a language of the merged list.
    let ja = index.languages().iter().position(|l| l.id == "ja_Jpan").unwrap();
    assert!(cjk.languages.contains(&ja));
    let gujarati_ui = index.family_from("notofonts", "Noto Sans Gujarati UI").unwrap();
    assert!(gujarati_ui.files.iter().any(|f| f.width != 100.0));
    assert!(gujarati_ui.face("Bold").unwrap().has_normal_width());
}

#[test]
fn lookups_match_exactly_and_refuse_near_matches() {
    let index = index_of(&fixture().index).unwrap();
    let name = |family: &str, style: &str| index.find_face(family, style).map(|f| f.file.file_name().to_string());
    assert_eq!(name("Test Sans", "Bold").unwrap(), "TestSans-Bold.ttf");
    assert_eq!(name("test sans", "700").unwrap(), "TestSans-Bold.ttf");
    assert_eq!(name("Test Sans", "Regular").unwrap(), "TestSans-Regular.ttf");
    assert_eq!(name("Test Sans", "").unwrap(), "TestSans-Regular.ttf");
    assert_eq!(name("Test Sans", "Italic").unwrap(), "TestSans-Italic.ttf");
    // Near matches are not offered.
    for style in ["Medium", "Bold Italic", "Light", "Condensed"] {
        assert!(matches!(name("Test Sans", style), Err(Unavailable::NoStyle { .. })), "{style}");
    }
    assert!(matches!(name("Test San", "Bold"), Err(Unavailable::NoFamily(_))));
    assert!(matches!(name("TestSans", "Bold"), Err(Unavailable::NoFamily(_))));
    // A variable font has the weights its axis reaches, and no others.
    assert_eq!(name("Test Var", "SemiBold").unwrap(), "TestVar[wght].ttf");
    assert_eq!(name("Test Var", "Light").unwrap(), "TestVar[wght].ttf");
    assert!(matches!(name("Test Var", "Thin"), Err(Unavailable::NoStyle { .. })));
    assert!(matches!(name("Test Var", "Bold Italic"), Err(Unavailable::NoStyle { .. })));
}

#[test]
fn postscript_lookups_match_exactly() {
    let index = index_of(&fixture().index).unwrap();
    assert_eq!(index.find_postscript("TestSans-Bold").unwrap().family.family, "Test Sans");
    assert_eq!(index.find_postscript("TestVar-SemiBold").unwrap().file.file_name(), "TestVar[wght].ttf");
    assert!(matches!(index.find_postscript("testsans-bold"), Err(Unavailable::NoPostScript(_))));
    assert!(matches!(index.find_postscript("TestSans-Medium"), Err(Unavailable::NoPostScript(_))));
    let Err(Unavailable::Ambiguous { families, .. }) = index.find_postscript("Shared-Regular") else { panic!("not ambiguous") };
    assert_eq!(families, ["Shared One", "Shared Two"]);
    assert!(index.family("Shared Two").unwrap().face_by_postscript("Shared-Regular").is_some());
}

#[test]
fn styles_parse_and_urls_pin_the_commit() {
    let index = index_of(&fixture().index).unwrap();
    assert_eq!(parse_style("Extra Bold Italic"), Some((800, true)));
    assert_eq!(parse_style("semi-bold"), Some((600, false)));
    assert_eq!(parse_style("Condensed Bold"), None);
    let var = index.family("Test Var").unwrap();
    assert_eq!(
        index.file_urls(var, &var.files[0]),
        [
            format!("https://cdn.jsdelivr.net/gh/google/fonts@{COMMIT}/apache/testvar/TestVar%5Bwght%5D.ttf"),
            format!("https://raw.githubusercontent.com/google/fonts/{COMMIT}/apache/testvar/TestVar%5Bwght%5D.ttf"),
        ]
    );
}

#[test]
fn verification_checks_size_hash_and_font() {
    let index = index_of(&fixture().index).unwrap();
    let file = index.find_face("Test Sans", "Bold").unwrap().file;
    let bytes = font("Test Sans", "Bold", 700);
    assert_eq!(verify_font(file, &bytes), Ok(()));
    let mut changed = bytes.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert_eq!(verify_font(file, &changed), Err(VerifyError::Sha256));
    assert!(matches!(verify_font(file, &bytes[1..]), Err(VerifyError::Size { .. })));
    assert!(!is_font(b"not a font at all"));
    assert!(is_font(&bytes));
}

const NOTO_COMMIT: &str = "93fac8abebe6c7fed0c5b7d2456d8fa484fa146c";
const VAR_PATH: &str = "fonts/NotoSerifTest/full/variable-ttf/NotoSerifTest[wdth,wght].ttf";
const CJK_PATH: &str = "Sans/OTF/Japanese/NotoSansCJKtest-Regular.otf";
const CONDENSED_PATH: &str = "fonts/TestSans/full/ttf/TestSans-CondensedBold.ttf";

/// A format 1.1 index of another source, as craft-font-index's Noto index has it: URL templates
/// with `{repo}` and `{ref}`, a family in another repository at a tag, licences with a sha256
/// (one in a third repository), and a family Google Fonts also has, in a width it lacks.
fn noto_fixture() -> Fixture {
    let var = font("Noto Serif Test", "Regular", 400);
    let cjk = font("Noto Sans CJK Test", "Regular", 400);
    let condensed = font("Test Sans", "Condensed Bold", 700);
    let ofl = b"Noto OFL".to_vec();
    let cjk_licence = b"noto-cjk LICENSE".to_vec();
    let lic = |path: &str, bytes: &[u8]| json!({"path": path, "size": bytes.len(), "sha256": sha256_hex(bytes)});
    let mut var_entry = file_entry(
        VAR_PATH,
        &var,
        "normal",
        400,
        json!([{"tag": "wght", "min": 100.0, "max": 900.0}, {"tag": "wdth", "min": 62.5, "max": 100.0}]),
        &["NotoSerifTest-Regular", "NotoSerifTest-Bold"],
    );
    var_entry["a_later_field"] = json!(1);
    let mut condensed_entry = file_entry(CONDENSED_PATH, &condensed, "normal", 700, json!([]), &["TestSans-CondensedBold"]);
    condensed_entry["width"] = json!(75.0);
    let mut serif_licence = lic("OFL.txt", &ofl);
    serif_licence["repo"] = json!("notofonts/serif-test");
    serif_licence["ref"] = json!("NotoSerifTest-v2.106");
    let index = json!({
        "schema": 1, "version": "1.1", "source": "notofonts", "commit": NOTO_COMMIT,
        "source_info": {"id": "notofonts", "name": "Noto", "homepage": "https://notofonts.github.io", "curated": true,
            "file_urls": ["https://cdn.jsdelivr.net/gh/{repo}@{ref}/{path}", "https://raw.githubusercontent.com/{repo}/{ref}/{path}"]},
        "repo": "notofonts/notofonts.github.io", "ref": NOTO_COMMIT,
        "languages": [
            {"id": "en_Latn", "name": "English", "script": "Latn", "population": 1},
            {"id": "ja_Jpan", "name": "Japanese", "autonym": "日本語", "script": "Jpan", "population": 1}],
        "scripts": [{"id": "Jpan", "name": "Japanese"}, {"id": "Latn", "name": "Latin"}],
        "families": [
            {"family": "Noto Sans CJK Test", "source": "notofonts", "license": "OFL", "license_path": null,
             "license_file": lic("LICENSE", &cjk_licence), "repo": "notofonts/noto-cjk", "ref": "Sans2.004",
             "languages": [0, 1], "scripts": ["Jpan", "Latn"],
             "files": [file_entry(CJK_PATH, &cjk, "normal", 400, json!([]), &["NotoSansCJKtest-Regular"])]},
            {"family": "Noto Serif Test", "source": "notofonts", "license": "OFL", "license_path": null,
             "license_file": serif_licence, "languages": [0], "files": [var_entry]},
            {"family": "Test Sans", "source": "notofonts", "license": "OFL", "license_path": null,
             "license_file": lic("OFL.txt", &ofl), "files": [condensed_entry]},
        ]
    });
    let files = vec![
        (format!("notofonts/noto-cjk/Sans2.004/{CJK_PATH}"), cjk),
        ("notofonts/noto-cjk/Sans2.004/LICENSE".into(), cjk_licence),
        (format!("notofonts/notofonts.github.io/{NOTO_COMMIT}/fonts/NotoSerifTest/full/variable-ttf/NotoSerifTest%5Bwdth%2Cwght%5D.ttf"), var),
        ("notofonts/serif-test/NotoSerifTest-v2.106/OFL.txt".into(), ofl.clone()),
        (format!("notofonts/notofonts.github.io/{NOTO_COMMIT}/{CONDENSED_PATH}"), condensed),
        (format!("notofonts/notofonts.github.io/{NOTO_COMMIT}/OFL.txt"), ofl),
    ];
    Fixture { files, index }
}

#[test]
fn a_noto_index_resolves_urls_per_family_and_entry() {
    let index = index_of(&noto_fixture().index).unwrap();
    let source = &index.sources()[0];
    assert_eq!((source.id.as_str(), source.name.as_str(), source.curated, source.commit.as_str()), ("notofonts", "Noto", true, NOTO_COMMIT));
    // The top-level repo and ref, with each path segment percent-encoded.
    let var = index.family("Noto Serif Test").unwrap();
    assert_eq!(
        index.file_urls(var, &var.files[0]),
        [
            format!(
                "https://cdn.jsdelivr.net/gh/notofonts/notofonts.github.io@{NOTO_COMMIT}/fonts/NotoSerifTest/full/variable-ttf/NotoSerifTest%5Bwdth%2Cwght%5D.ttf"
            ),
            format!(
                "https://raw.githubusercontent.com/notofonts/notofonts.github.io/{NOTO_COMMIT}/fonts/NotoSerifTest/full/variable-ttf/NotoSerifTest%5Bwdth%2Cwght%5D.ttf"
            ),
        ]
    );
    // The licence's own repository and ref.
    assert_eq!(index.license_urls(var)[1], "https://raw.githubusercontent.com/notofonts/serif-test/NotoSerifTest-v2.106/OFL.txt");
    // The family's repository and ref, for its files and its licence.
    let cjk = index.family("Noto Sans CJK Test").unwrap();
    assert_eq!(index.file_urls(cjk, &cjk.files[0])[0], format!("https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@Sans2.004/{CJK_PATH}"));
    assert_eq!(index.license_urls(cjk)[0], "https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@Sans2.004/LICENSE");
    assert_eq!((cjk.dir(), cjk.license_file_name(), cjk.license_path.as_deref()), ("notofonts/noto-sans-cjk-test", Some("LICENSE"), None));
    assert_eq!(var.files[0].width, 100.0);

    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = noto_fixture().index;
        edit(&mut v);
        index_of(&v)
    };
    let invalid = [
        with(&|v| v["families"][1]["files"][0]["path"] = json!("../x.ttf")),
        with(&|v| v["families"][1]["files"][0]["path"] = json!("/x.ttf")),
        with(&|v| v["families"][1]["files"][0]["path"] = json!("a//b.ttf")),
        with(&|v| v["families"][1]["files"][0]["path"] = json!("a/./b.ttf")),
        with(&|v| v["families"][1]["files"][0]["path"] = json!("a\\b.ttf")),
        with(&|v| v["families"][1]["ref"] = json!("a/../b")),
        with(&|v| v["families"][1]["repo"] = json!("noslash")),
        with(&|v| v["families"][0]["license_file"]["path"] = json!("Sans/OTF/Japanese/NotoSansCJKtest-Regular.otf")),
        with(&|v| v["families"][0]["license_file"]["sha256"] = json!("00")),
        with(&|v| v["families"][1]["files"][0]["width"] = json!(0.0)),
        with(&|v| {
            v.as_object_mut().unwrap().remove("repo");
        }),
        with(&|v| v["source_info"]["file_urls"] = json!(["http://cdn.jsdelivr.net/gh/{repo}@{ref}/{path}"])),
        with(&|v| v["source_info"]["file_urls"] = json!(["https://cdn.jsdelivr.net/gh/{owner}/{path}"])),
        with(&|v| v["source_info"]["file_urls"] = json!(["https://cdn.jsdelivr.net/gh/{repo}@{ref}/"])),
        with(&|v| v["source_info"]["id"] = json!("other")),
    ];
    for (i, r) in invalid.into_iter().enumerate() {
        assert!(matches!(r, Err(IndexError::Invalid(_))), "case {i}: {r:?}");
    }
    // Only licences that allow redistribution: other families are left out, with a warning.
    let index = with(&|v| v["families"][1]["license"] = json!("Proprietary")).unwrap();
    assert!(index.family("Noto Serif Test").is_none());
    assert_eq!((index.families().len(), index.sources()[0].warnings.len()), (2, 1));
}

#[test]
fn sources_merge_with_the_first_preferred() {
    let index = Index::merge([index_of(&fixture().index).unwrap(), index_of(&noto_fixture().index).unwrap()]);
    assert_eq!(index.sources().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [GOOGLE_FONTS, "notofonts"]);
    let names: Vec<(&str, &str)> = index.families().iter().map(|f| (f.family.as_str(), f.source.as_str())).collect();
    assert_eq!(
        names,
        [
            ("Noto Sans CJK Test", "notofonts"),
            ("Noto Serif Test", "notofonts"),
            ("Shared One", GOOGLE_FONTS),
            ("Shared Two", GOOGLE_FONTS),
            ("Test Sans", GOOGLE_FONTS),
            ("Test Sans", "notofonts"),
            ("Test Var", GOOGLE_FONTS),
        ]
    );
    // The first source's family for a name; a style only another source has comes from it.
    assert_eq!(index.family("test sans").unwrap().source, GOOGLE_FONTS);
    assert_eq!(index.find_face("Test Sans", "Bold").unwrap().family.source, GOOGLE_FONTS);
    let condensed = index.find_face("Test Sans", "Condensed Bold").unwrap();
    assert_eq!((condensed.family.source.as_str(), condensed.family.dir()), ("notofonts", "notofonts/test-sans"));
    assert_eq!(index.find_postscript("TestSans-CondensedBold").unwrap().family.source, "notofonts");
    assert!(matches!(index.find_face("Test Sans", "Light"), Err(Unavailable::NoStyle { .. })));
    // Languages of both, by ID, and every family's positions point into them.
    assert_eq!(index.languages().iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["en_Latn", "ja_Jpan"]);
    let cjk = index.family("Noto Sans CJK Test").unwrap();
    assert_eq!(cjk.languages(&index).map(|l| l.id.as_str()).collect::<Vec<_>>(), ["en_Latn", "ja_Jpan"]);
    assert_eq!(index.family("Shared One").unwrap().languages(&index).map(|l| l.id.as_str()).collect::<Vec<_>>(), ["en_Latn"]);
    // URLs stay per source.
    assert!(index.file_urls(cjk, &cjk.files[0])[0].contains("noto-cjk@Sans2.004"));
    // A second source with the same ID is left out and reported.
    let twice = Index::merge([index_of(&noto_fixture().index).unwrap(), index_of(&noto_fixture().index).unwrap()]);
    assert_eq!((twice.sources().len(), twice.families().len(), twice.problems().len()), (1, 3, 1));
}

#[test]
fn a_user_index_is_never_curated_and_cant_shadow_a_built_in_source() {
    let mut v = noto_fixture().index;
    v["source_info"]["file_urls"] = json!(["https://fonts.example.org/files/{path}"]);
    v["families"][1]["license"] = json!("Proprietary");
    let url = "https://fonts.example.org/index.json";
    let index = Index::from_user_bytes(v.to_string().as_bytes(), url).unwrap();
    let source = &index.sources()[0];
    assert_eq!((source.id.as_str(), source.curated, source.user_url.as_deref()), ("user:fonts.example.org/notofonts", false, Some(url)));
    assert_eq!(source.warnings.len(), 1);
    let cjk = index.family("Noto Sans CJK Test").unwrap();
    assert!(!cjk.is_curated());
    assert_eq!(cjk.dir(), "user-fonts-example-org-notofonts-6b687c53/noto-sans-cjk-test");
    assert_eq!(index.file_urls(cjk, &cjk.files[0]), [format!("https://fonts.example.org/files/{CJK_PATH}")]);
    // Merged after a built-in source of the ID it claims, it is a separate source.
    let merged = Index::merge([index_of(&noto_fixture().index).unwrap(), index]);
    assert_eq!(merged.sources().len(), 2);
    assert!(merged.family("Noto Sans CJK Test").unwrap().is_curated());
    assert!(Index::from_user_bytes(v.to_string().as_bytes(), "http://fonts.example.org/index.json").is_err());
}

#[test]
fn user_sources_whose_ids_slug_alike_get_their_own_folders() {
    let mut v = noto_fixture().index;
    v["source_info"]["file_urls"] = json!(["https://fonts.example.org/files/{path}"]);
    let dir = |url: &str| Index::from_user_bytes(v.to_string().as_bytes(), url).unwrap().family("Noto Sans CJK Test").unwrap().dir().to_string();
    let (a, b) = (dir("https://fonts.example.org/index.json"), dir("https://fonts-example.org/index.json"));
    assert_ne!(a, b);
    assert!(a.split('/').all(|s| s.len() <= 64) && b.split('/').all(|s| s.len() <= 64));
    let long = format!("https://{}.example.org/index.json", "a".repeat(60));
    assert!(dir(&long).split('/').all(|s| s.len() <= 64));
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use craft_font_catalog::{Catalog, FontEnv, Os};

    use super::*;

    /// What the server answers for a path.
    #[derive(Clone)]
    enum Reply {
        Ok(Vec<u8>),
        Status(u16),
        /// Sends headers and a few bytes, then nothing.
        Stall,
    }

    /// A local HTTP server for `/cdn/` (jsDelivr) and `/raw/` (raw GitHub).
    struct Server {
        base: String,
        routes: Arc<Mutex<HashMap<String, Reply>>>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}/", listener.local_addr().unwrap());
            let routes: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
            let log: Arc<Mutex<Vec<String>>> = Arc::default();
            let (r, l) = (routes.clone(), log.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let (r, l) = (r.clone(), l.clone());
                    std::thread::spawn(move || answer(stream, &r, &l));
                }
            });
            Self { base, routes, log }
        }

        fn serve(&self, prefix: &str, path: &str, reply: Reply) {
            self.routes.lock().unwrap().insert(format!("/{prefix}/{path}"), reply);
        }

        /// The fixture's files on both hosts.
        fn serve_fixture(&self, f: &Fixture) {
            for (path, bytes) in &f.files {
                self.serve("cdn", &format!("gh/google/fonts@{COMMIT}/{path}"), Reply::Ok(bytes.clone()));
                self.serve("raw", &format!("google/fonts/{COMMIT}/{path}"), Reply::Ok(bytes.clone()));
            }
        }

        /// A fixture whose files are keyed `<owner>/<name>/<ref>/<path>`, on both hosts.
        fn serve_repos(&self, f: &Fixture) {
            for (key, bytes) in &f.files {
                let mut parts = key.splitn(4, '/');
                let (owner, name, git_ref, path) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
                self.serve("cdn", &format!("gh/{owner}/{name}@{git_ref}/{path}"), Reply::Ok(bytes.clone()));
                self.serve("raw", &format!("{owner}/{name}/{git_ref}/{path}"), Reply::Ok(bytes.clone()));
            }
        }

        fn requests(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        fn client(&self) -> Client {
            let t = Timeouts { connect: Duration::from_secs(2), response: Duration::from_secs(5), overall: Duration::from_secs(10) };
            Client::for_tests(
                t,
                vec![
                    ("https://cdn.jsdelivr.net/".into(), format!("{}cdn/", self.base)),
                    ("https://raw.githubusercontent.com/".into(), format!("{}raw/", self.base)),
                    ("https://fonts.example.org/".into(), format!("{}user/", self.base)),
                ],
            )
        }
    }

    fn answer(mut s: TcpStream, routes: &Mutex<HashMap<String, Reply>>, log: &Mutex<Vec<String>>) {
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match s.read(&mut b) {
                Ok(1) => head.push(b[0]),
                _ => return,
            }
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
        log.lock().unwrap().push(path.clone());
        let reply = routes.lock().unwrap().get(&path).cloned().unwrap_or(Reply::Status(404));
        match reply {
            Reply::Ok(body) => {
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(&body);
            }
            Reply::Status(code) => {
                let _ = write!(s, "HTTP/1.1 {code} Nope\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
            }
            Reply::Stall => {
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc");
                let _ = s.flush();
                std::thread::sleep(Duration::from_secs(20));
            }
        }
    }

    fn temp(name: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("craft-font-store-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    /// No font files, staged downloads or family folders anywhere in the store.
    fn assert_no_family_files(dir: &Path) {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let name = e.file_name().to_string_lossy().into_owned();
                    assert!(["settings.json", ".lock"].contains(&name.as_str()), "left behind: {}", p.display());
                }
            }
        }
        assert!(!dir.join("testsans").exists());
        assert!(entries(&dir.join(".staging")).is_empty(), "{:?}", entries(&dir.join(".staging")));
    }

    fn user_store(server: &Server, dir: &Path) -> FontStore {
        let store = FontStore::new(dir.to_path_buf(), Caller::User).with_client(server.client());
        store.enable_network().unwrap();
        store
    }

    fn none(_: Progress) {}

    #[test]
    fn consent_is_off_by_default_and_only_the_user_turns_it_on() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let dir = temp("consent");
        let user = FontStore::new(dir.clone(), Caller::User).with_client(server.client());
        let agent = FontStore::new(dir.clone(), Caller::Agent).with_client(server.client());
        assert_eq!((user.network(), agent.network()), (Network::Off, Network::Off));

        let face = index.find_face("Test Sans", "Bold").unwrap();
        assert!(matches!(user.install_face(&index, face, &Cancel::new(), &mut none), Err(Error::NetworkOff)));
        assert!(matches!(agent.enable_network(), Err(Error::NotAllowed)));
        assert_eq!(agent.network(), Network::Off);
        assert!(matches!(agent.install_face(&index, face, &Cancel::new(), &mut none), Err(Error::NetworkOff)));
        assert!(server.requests().is_empty(), "no request while the network is off");

        // The user's setting is persisted in the store folder; an agent then may install.
        user.enable_network().unwrap();
        assert_eq!(FontStore::new(dir.clone(), Caller::Agent).network(), Network::On);
        agent.install_face(&index, face, &Cancel::new(), &mut none).unwrap();
        agent.disable_network().unwrap();
        assert_eq!(user.network(), Network::Off);

        // The agent constructor opens the same folder as the user's.
        let env = FontEnv { os: Os::Unix, font_store: Some(dir.clone()), ..Default::default() };
        assert_eq!(FontStore::for_agent(&env).unwrap().caller(), Caller::Agent);
        assert!(matches!(FontStore::for_agent(&env).unwrap().enable_network(), Err(Error::NotAllowed)));
        assert_eq!(FontStore::for_user(&env).unwrap().dir(), dir);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn install_verify_and_remove_a_family() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let dir = temp("install");
        let root = temp("install-root");
        let env = FontEnv { os: Os::Unix, root: root.clone(), font_store: Some(dir.clone()), ..Default::default() };
        let catalog = Arc::new(Catalog::new(env).with_font_service(false));
        catalog.rescan();
        assert!(catalog.by_postscript_name("TestSans-Bold").is_none());
        let store = user_store(&server, &dir).with_catalog(catalog.clone());

        let mut seen = Vec::new();
        let family = index.family("Test Sans").unwrap();
        let installed = store.install_family(&index, family, &Cancel::new(), &mut |p| seen.push(p)).unwrap();
        assert_eq!(entries(&dir.join("testsans")), ["OFL.txt", "TestSans-Bold.ttf", "TestSans-Italic.ttf", "TestSans-Regular.ttf"]);
        assert_eq!(installed.files.len(), 3);
        assert_eq!((installed.dir.as_str(), installed.license_file.as_deref()), ("testsans", Some("OFL.txt")));
        assert!(installed.files.iter().all(|x| x.commit == COMMIT && x.sha256.len() == 64));
        assert_eq!(store.installed(), std::slice::from_ref(&installed));
        let last = seen.last().unwrap();
        assert_eq!((last.done, last.total, last.files), (family.size(), family.size(), 3));
        assert!(seen.windows(2).all(|w| w[0].done <= w[1].done));
        // The catalog was rescanned.
        assert_eq!(catalog.by_postscript_name("TestSans-Bold").unwrap().family, "Test Sans");
        assert!(store.verify().is_empty());
        assert!(store.is_installed(family, index.find_face("Test Sans", "Italic").unwrap().file));

        // Installed files aren't downloaded again.
        let before = server.requests().len();
        store.install_face(&index, index.find_face("Test Sans", "Bold").unwrap(), &Cancel::new(), &mut none).unwrap();
        assert_eq!(server.requests().len(), before);

        // Damaged and missing files are reported.
        std::fs::write(dir.join("testsans/TestSans-Italic.ttf"), b"damaged").unwrap();
        std::fs::remove_file(dir.join("testsans/TestSans-Regular.ttf")).unwrap();
        let problems = store.verify();
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().any(|p| p.file == "TestSans-Italic.ttf" && matches!(p.error, Some(VerifyError::Size { .. }))));
        assert!(problems.iter().any(|p| p.file == "TestSans-Regular.ttf" && p.error.is_none()));

        // A single face of another family, with its licence.
        let face = index.find_face("Test Var", "Bold").unwrap();
        store.install_face(&index, face, &Cancel::new(), &mut none).unwrap();
        assert_eq!(entries(&dir.join("testvar")), ["LICENSE.txt", "TestVar[wght].ttf"]);
        assert_eq!(store.installed().iter().map(|f| f.family.as_str()).collect::<Vec<_>>(), ["Test Sans", "Test Var"]);

        // One style out, then the last one takes its family with it.
        store.remove_files(GOOGLE_FONTS, "Test Sans", &["TestSans-Bold.ttf".into()]).unwrap();
        assert!(!dir.join("testsans/TestSans-Bold.ttf").exists());
        assert_eq!(store.installed_family(GOOGLE_FONTS, "Test Sans").unwrap().files.len(), 2);
        store.remove_files(GOOGLE_FONTS, "Test Var", &["TestVar[wght].ttf".into()]).unwrap();
        assert!(!dir.join("testvar").exists());
        assert!(store.installed_family(GOOGLE_FONTS, "Test Var").is_none());

        store.remove_family(GOOGLE_FONTS, "test sans").unwrap();
        assert!(!dir.join("testsans").exists());
        assert!(store.installed().is_empty());
        assert!(catalog.by_postscript_name("TestSans-Bold").is_none());
        assert!(matches!(store.remove_family(GOOGLE_FONTS, "Test Sans"), Err(Error::NotInstalled(_))));
        assert!(entries(&dir.join(".staging")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removing_files_touches_only_listed_names() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let dir = temp("remove-listed");
        let store = user_store(&server, &dir);
        store.install_family(&index, index.family("Test Sans").unwrap(), &Cancel::new(), &mut none).unwrap();
        std::fs::write(dir.join("outside.ttf"), b"x").unwrap();
        std::fs::write(dir.join("testsans/Unlisted.ttf"), b"x").unwrap();

        store.remove_files(GOOGLE_FONTS, "Test Sans", &["../outside.ttf".into(), "Unlisted.ttf".into()]).unwrap();
        assert!(dir.join("outside.ttf").exists());
        assert!(dir.join("testsans/Unlisted.ttf").exists());
        assert_eq!(store.installed_family(GOOGLE_FONTS, "Test Sans").unwrap().files.len(), 3);

        store.remove_files(GOOGLE_FONTS, "Test Sans", &["../outside.ttf".into(), "TestSans-Bold.ttf".into()]).unwrap();
        assert!(dir.join("outside.ttf").exists());
        assert!(!dir.join("testsans/TestSans-Bold.ttf").exists());
        assert_eq!(store.installed_family(GOOGLE_FONTS, "Test Sans").unwrap().files.len(), 2);

        // The last style takes the licence with it; the unlisted file and its folder stay.
        let rest: Vec<String> = store.installed_family(GOOGLE_FONTS, "Test Sans").unwrap().files.iter().map(|f| f.name.clone()).collect();
        store.remove_files(GOOGLE_FONTS, "Test Sans", &rest).unwrap();
        assert!(store.installed_family(GOOGLE_FONTS, "Test Sans").is_none());
        let left: Vec<_> = std::fs::read_dir(dir.join("testsans")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["Unlisted.ttf"]);

        // A folder left empty goes.
        std::fs::remove_file(dir.join("testsans/Unlisted.ttf")).unwrap();
        store.install_face(&index, index.find_face("Test Sans", "Bold").unwrap(), &Cancel::new(), &mut none).unwrap();
        store.remove_files(GOOGLE_FONTS, "Test Sans", &["TestSans-Bold.ttf".into()]).unwrap();
        assert!(!dir.join("testsans").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn jsdelivr_failures_fall_back_to_raw_github() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let bold = format!("gh/google/fonts@{COMMIT}/ofl/testsans/TestSans-Bold.ttf");
        server.serve("cdn", &bold, Reply::Status(503));
        let index = index_of(&f.index).unwrap();
        let dir = temp("fallback");
        let store = user_store(&server, &dir);
        store.install_face(&index, index.find_face("Test Sans", "Bold").unwrap(), &Cancel::new(), &mut none).unwrap();
        let requests = server.requests();
        assert_eq!(requests.first().map(String::as_str), Some(format!("/cdn/{bold}").as_str()));
        assert_eq!(requests.get(1).map(String::as_str), Some(format!("/raw/google/fonts/{COMMIT}/ofl/testsans/TestSans-Bold.ttf").as_str()));
        assert!(store.verify().is_empty());

        // A corrupted copy on jsDelivr also falls back.
        let italic = format!("gh/google/fonts@{COMMIT}/ofl/testsans/TestSans-Italic.ttf");
        server.serve("cdn", &italic, Reply::Ok(font("Test Sans", "Bold", 700)));
        store.install_face(&index, index.find_face("Test Sans", "Italic").unwrap(), &Cancel::new(), &mut none).unwrap();
        assert!(store.verify().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_downloads_are_rejected_and_leave_nothing() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let dir = temp("corrupt");
        let store = user_store(&server, &dir);

        // Wrong bytes of the right size on both hosts: rejected on the sha256.
        let mut other = font("Test Sans", "Italic", 400);
        *other.last_mut().unwrap() ^= 1;
        for (host, path) in [("cdn", format!("gh/google/fonts@{COMMIT}")), ("raw", format!("google/fonts/{COMMIT}"))] {
            server.serve(host, &format!("{path}/ofl/testsans/TestSans-Italic.ttf"), Reply::Ok(other.clone()));
        }
        let family = index.family("Test Sans").unwrap();
        let r = store.install_family(&index, family, &Cancel::new(), &mut none);
        assert!(matches!(r, Err(Error::Verify { error: VerifyError::Sha256, .. })), "{r:?}");
        assert_no_family_files(&dir);
        assert!(store.installed().is_empty());

        // Bytes that match the index but aren't a font.
        let junk = b"this is not a font file".to_vec();
        let mut v = f.index.clone();
        v["families"][2]["files"][0]["sha256"] = json!(sha256_hex(&junk));
        v["families"][2]["files"][0]["size"] = json!(junk.len());
        server.serve("cdn", &format!("gh/google/fonts@{COMMIT}/ofl/testsans/TestSans-Bold.ttf"), Reply::Ok(junk.clone()));
        server.serve("raw", &format!("google/fonts/{COMMIT}/ofl/testsans/TestSans-Bold.ttf"), Reply::Ok(junk));
        let junk_index = index_of(&v).unwrap();
        let r = store.install_face(&junk_index, junk_index.find_face("Test Sans", "Bold").unwrap(), &Cancel::new(), &mut none);
        assert!(matches!(r, Err(Error::Verify { error: VerifyError::NotAFont, .. })), "{r:?}");
        assert_no_family_files(&dir);

        // A file missing on both hosts, after others downloaded: nothing of the family is written.
        server.serve_fixture(&f);
        for (host, path) in [("cdn", format!("gh/google/fonts@{COMMIT}")), ("raw", format!("google/fonts/{COMMIT}"))] {
            server.serve(host, &format!("{path}/ofl/testsans/TestSans-Regular.ttf"), Reply::Status(404));
        }
        let r = store.install_family(&index, family, &Cancel::new(), &mut none);
        assert!(matches!(&r, Err(Error::Download { reasons, .. }) if reasons.len() == 2), "{r:?}");
        assert_no_family_files(&dir);
        assert!(!dir.join("manifest.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_commit_keeps_the_installed_files() {
        use std::os::unix::fs::PermissionsExt;
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let dir = temp("commit");
        let store = user_store(&server, &dir);
        store.install_face(&index, index.find_face("Test Sans", "Regular").unwrap(), &Cancel::new(), &mut none).unwrap();
        let folder = dir.join("testsans");
        let before = (store.installed(), entries(&folder));

        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let writable = std::fs::File::create(folder.join("probe")).is_ok();
        let r = store.install_family(&index, index.family("Test Sans").unwrap(), &Cancel::new(), &mut none);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if writable {
            return; // root ignores permissions
        }
        assert!(matches!(r, Err(Error::Io { .. })), "{r:?}");
        assert_eq!((store.installed(), entries(&folder)), before);
        assert!(entries(&dir.join(".staging")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancel_stops_a_stalled_install() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        server.serve("cdn", &format!("gh/google/fonts@{COMMIT}/ofl/testsans/TestSans-Bold.ttf"), Reply::Stall);
        let index = index_of(&f.index).unwrap();
        let dir = temp("cancel");
        let store = user_store(&server, &dir);
        let cancel = Cancel::new();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            c.cancel();
        });
        let t = Instant::now();
        let r = store.install_family(&index, index.family("Test Sans").unwrap(), &cancel, &mut none);
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert_no_family_files(&dir);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        let mut e = GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    /// `sources.json` listing the Google Fonts fixture (gzipped) and the Noto fixture (plain
    /// JSON), with the index bytes each path serves.
    fn sources_fixture(noto_sha256: Option<&str>) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
        let google = fixture().index.to_string().into_bytes();
        let google_gz = gzip(&google);
        let noto = noto_fixture().index.to_string().into_bytes();
        let sources = json!({"schema": 1, "version": "1.1", "indexes": [
            {"source": GOOGLE_FONTS, "name": "Google Fonts", "curated": true, "path": "indexes/google-fonts.json",
             "size": google.len(), "sha256": sha256_hex(&google), "gz_path": "indexes/google-fonts.json.gz",
             "gz_size": google_gz.len(), "gz_sha256": sha256_hex(&google_gz), "commit": COMMIT, "families": 4, "files": 6},
            {"source": "notofonts", "name": "Noto", "curated": true, "path": "indexes/noto.json",
             "size": noto.len(), "sha256": noto_sha256.map_or_else(|| sha256_hex(&noto), str::to_string), "commit": NOTO_COMMIT}
        ]});
        let files = vec![("indexes/google-fonts.json.gz".into(), google_gz), ("indexes/noto.json".into(), noto)];
        (sources.to_string().into_bytes(), files)
    }

    fn pin_of(sources: &[u8]) -> IndexPin {
        IndexPin { repo: "owner/index", commit: "abc", sha256: Box::leak(sha256_hex(sources).into_boxed_str()) }
    }

    fn serve_sources(server: &Server, sources: &[u8], files: &[(String, Vec<u8>)]) {
        server.serve("cdn", "gh/owner/index@abc/sources.json", Reply::Status(500));
        server.serve("raw", "owner/index/abc/sources.json", Reply::Ok(sources.to_vec()));
        for (path, bytes) in files {
            server.serve("cdn", &format!("gh/owner/index@abc/{path}"), Reply::Ok(bytes.clone()));
        }
    }

    #[test]
    fn the_indexes_are_fetched_only_with_consent_and_checked() {
        let server = Server::start();
        let (sources, files) = sources_fixture(None);
        let pin = pin_of(&sources);
        serve_sources(&server, &sources, &files);
        let dir = temp("index");
        let store = FontStore::new(dir.clone(), Caller::User).with_client(server.client());
        assert!(store.cached_indexes(&pin).is_none());
        assert!(matches!(store.fetch_indexes(&pin, &Cancel::new()), Err(Error::NetworkOff)));
        assert!(server.requests().is_empty());

        store.enable_network().unwrap();
        let index = store.fetch_indexes(&pin, &Cancel::new()).unwrap();
        assert!(index.problems().is_empty(), "{:?}", index.problems());
        assert_eq!(index.sources().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [GOOGLE_FONTS, "notofonts"]);
        assert_eq!(index.families().len(), 7);
        assert_eq!(entries(&dir.join("indexes")), ["google-fonts.json.gz", "notofonts.json", "sources.json"]);
        let cached = store.cached_indexes(&pin).unwrap();
        assert_eq!((cached.families().len(), cached.sources().len()), (7, 2));

        let wrong = IndexPin { sha256: "0000000000000000000000000000000000000000000000000000000000000000", ..pin };
        assert!(store.cached_indexes(&wrong).is_none());
        assert!(matches!(store.fetch_indexes(&wrong, &Cancel::new()), Err(Error::Verify { error: VerifyError::Sha256, .. })));

        // A cached index that was changed is not used.
        std::fs::write(dir.join("indexes/notofonts.json"), b"{}").unwrap();
        assert!(store.cached_indexes(&pin).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_index_with_the_wrong_sha256_fails_alone() {
        let server = Server::start();
        let (sources, files) = sources_fixture(Some("1111111111111111111111111111111111111111111111111111111111111111"));
        let pin = pin_of(&sources);
        serve_sources(&server, &sources, &files);
        let dir = temp("one-bad-index");
        let store = user_store(&server, &dir);
        let index = store.fetch_indexes(&pin, &Cancel::new()).unwrap();
        assert_eq!(index.sources().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [GOOGLE_FONTS]);
        assert_eq!(index.problems().iter().map(|p| p.source.as_str()).collect::<Vec<_>>(), ["notofonts"]);
        assert!(index.family("Test Var").is_some() && index.family("Noto Sans CJK Test").is_none());
        // Both hosts were tried for it.
        assert_eq!(server.requests().iter().filter(|r| r.ends_with("indexes/noto.json")).count(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_noto_family_installs_into_its_own_folder_after_a_jsdelivr_403() {
        let server = Server::start();
        let f = noto_fixture();
        server.serve_repos(&f);
        // jsDelivr refuses files over 20 MB with 403.
        let cjk = format!("gh/notofonts/noto-cjk@Sans2.004/{CJK_PATH}");
        server.serve("cdn", &cjk, Reply::Status(403));
        let index = index_of(&f.index).unwrap();
        let dir = temp("noto");
        let store = user_store(&server, &dir);
        let family = index.family("Noto Sans CJK Test").unwrap();
        let installed = store.install_family(&index, family, &Cancel::new(), &mut none).unwrap();
        assert_eq!(entries(&dir.join("notofonts/noto-sans-cjk-test")), ["LICENSE", "NotoSansCJKtest-Regular.otf"]);
        assert_eq!((installed.source.as_str(), installed.dir.as_str()), ("notofonts", "notofonts/noto-sans-cjk-test"));
        assert!(installed.files.iter().all(|x| x.commit == NOTO_COMMIT));
        let requests = server.requests();
        let at = requests.iter().position(|r| *r == format!("/cdn/{cjk}")).unwrap();
        assert_eq!(requests[at + 1], format!("/raw/notofonts/noto-cjk/Sans2.004/{CJK_PATH}"));
        assert!(store.verify().is_empty());

        // The licence is checked against its sha256: a changed one fails the install.
        let serif = index.family("Noto Serif Test").unwrap();
        for (host, path) in
            [("cdn", "gh/notofonts/serif-test@NotoSerifTest-v2.106/OFL.txt"), ("raw", "notofonts/serif-test/NotoSerifTest-v2.106/OFL.txt")]
        {
            server.serve(host, path, Reply::Ok(b"Noto OFX".to_vec()));
        }
        let r = store.install_family(&index, serif, &Cancel::new(), &mut none);
        assert!(matches!(r, Err(Error::Verify { error: VerifyError::Sha256, .. })), "{r:?}");
        assert!(!dir.join("notofonts/noto-serif-test").exists());
        server.serve_repos(&f);
        store.install_family(&index, serif, &Cancel::new(), &mut none).unwrap();
        assert_eq!(entries(&dir.join("notofonts/noto-serif-test")), ["NotoSerifTest[wdth,wght].ttf", "OFL.txt"]);

        // A family of the same name from Google Fonts is installed and removed separately.
        let google = index_of(&fixture().index).unwrap();
        let both = Index::merge([google, index]);
        server.serve_fixture(&fixture());
        store.install_family(&both, both.family_from(GOOGLE_FONTS, "Test Sans").unwrap(), &Cancel::new(), &mut none).unwrap();
        store.install_family(&both, both.family_from("notofonts", "Test Sans").unwrap(), &Cancel::new(), &mut none).unwrap();
        assert_eq!(entries(&dir.join("notofonts/test-sans")), ["OFL.txt", "TestSans-CondensedBold.ttf"]);
        assert_eq!(store.installed().iter().filter(|f| f.family == "Test Sans").count(), 2);
        store.remove_family("notofonts", "Test Sans").unwrap();
        assert!(!dir.join("notofonts/test-sans").exists());
        assert!(store.installed_family(GOOGLE_FONTS, "Test Sans").is_some() && dir.join("testsans").is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn user_sources_are_added_by_the_user_and_never_curated() {
        let server = Server::start();
        let (sources, files) = sources_fixture(None);
        let pin = pin_of(&sources);
        serve_sources(&server, &sources, &files);
        let mut v = noto_fixture().index;
        v["source_info"]["file_urls"] = json!(["https://fonts.example.org/files/{path}"]);
        let url = "https://fonts.example.org/index.json.gz";
        server.serve("user", "index.json.gz", Reply::Ok(gzip(v.to_string().as_bytes())));
        let f = noto_fixture();
        let cjk_bytes = f.files.iter().find(|(k, _)| k.ends_with(CJK_PATH)).unwrap().1.clone();
        server.serve("user", &format!("files/{CJK_PATH}"), Reply::Ok(cjk_bytes));
        server.serve("user", "files/LICENSE", Reply::Ok(b"noto-cjk LICENSE".to_vec()));

        let dir = temp("user-source");
        let store = user_store(&server, &dir);
        let agent = FontStore::new(dir.clone(), Caller::Agent).with_client(server.client());
        assert!(matches!(agent.add_user_source(url), Err(Error::NotAllowed)));
        assert!(matches!(store.add_user_source("http://fonts.example.org/index.json"), Err(Error::BadUrl(_))));
        store.add_user_source(url).unwrap();
        store.add_user_source(url).unwrap();
        assert_eq!(store.user_sources().len(), 1);
        assert_eq!(store.network(), Network::On, "the setting is kept");

        let index = store.fetch_indexes(&pin, &Cancel::new()).unwrap();
        assert!(index.problems().is_empty(), "{:?}", index.problems());
        let user = index.sources().last().unwrap();
        assert_eq!((user.id.as_str(), user.curated, user.changed), ("user:fonts.example.org/notofonts", false, false));
        let theirs: Vec<&Family> = index.families_named("Noto Sans CJK Test").collect();
        assert_eq!(theirs.iter().map(|f| f.is_curated()).collect::<Vec<_>>(), [true, false]);
        assert!(store.user_sources()[0].last_sha256.is_some());

        // Installs from it are verified like any other.
        let family = theirs[1];
        let installed = store.install_family(&index, family, &Cancel::new(), &mut none).unwrap();
        assert_eq!(installed.source, "user:fonts.example.org/notofonts");
        assert!(dir.join("user-fonts-example-org-notofonts-6b687c53/noto-sans-cjk-test/LICENSE").is_file());

        // A changed file shows as changed, from the network and from the cache.
        v["families"][1]["designer"] = json!("Someone else");
        server.serve("user", "index.json.gz", Reply::Ok(gzip(v.to_string().as_bytes())));
        assert!(store.fetch_indexes(&pin, &Cancel::new()).unwrap().sources().last().unwrap().changed);
        assert!(store.cached_indexes(&pin).unwrap().sources().last().unwrap().changed);

        // Removing the source leaves its families installed.
        store.remove_user_source(url).unwrap();
        assert!(store.user_sources().is_empty());
        assert_eq!(store.cached_indexes(&pin).unwrap().sources().len(), 2);
        assert!(store.installed_family("user:fonts.example.org/notofonts", "Noto Sans CJK Test").is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Fetches the pinned indexes from jsDelivr and GitHub and installs a Noto family whose
    /// licence is in another repository. `cargo test -p craft-font-store -- --ignored`.
    #[test]
    #[ignore = "uses the network"]
    fn the_pinned_indexes_and_a_noto_family_download() {
        let dir = temp("network");
        let store = FontStore::new(dir.clone(), Caller::User);
        store.enable_network().unwrap();
        let index = store.fetch_indexes(&PINNED_INDEX, &Cancel::new()).unwrap();
        assert!(index.problems().is_empty(), "{:?}", index.problems());
        assert_eq!(index.sources().len(), 2);
        let family = index.family_from("notofonts", "Noto Serif Myanmar").unwrap();
        store.install_family(&index, family, &Cancel::new(), &mut none).unwrap();
        assert!(dir.join("notofonts/noto-serif-myanmar/OFL.txt").is_file());
        assert!(store.verify().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
