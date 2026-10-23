use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cmap::Cmap;
use crate::embed::Env;
use crate::{Embed, EmbeddedFace, Error, parse_manifest};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/craft-fonts")
}

/// A fresh scratch directory under the system temp dir.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("craft-fonts-build-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    result: Result<Vec<String>, Error>,
    cargo: Vec<String>,
    table: String,
    /// `OUT_DIR`, which holds the table and the cut faces.
    #[cfg_attr(not(feature = "subset"), expect(dead_code))]
    out: Scratch,
}

/// Run `embed` with `CRAFT_FONTS_DIR` and `CRAFT_FONTS_REQUIRED` as given; returns the embedded
/// face names, the cargo lines and the generated table.
fn run(embed: Embed, dir: Option<&Path>, required: Option<&str>) -> Run {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let scratch = Scratch::new(&format!("run-{}", RUNS.fetch_add(1, Ordering::Relaxed)));
    let env = Env { dir: dir.map(OsString::from), required: required.map(OsString::from), out_dir: Some(scratch.0.clone().into()) };
    let mut cargo = Vec::new();
    let result = embed.run(&env, Path::new("craft_fonts.rs"), &mut cargo);
    let table = std::fs::read_to_string(scratch.0.join("craft_fonts.rs")).unwrap();
    Run { result: result.map(|faces| faces.iter().map(|f| format!("{} {}", f.family, f.style)).collect()), cargo, table, out: scratch }
}

#[test]
fn parses_the_manifest_with_collection_indices() {
    let text = std::fs::read_to_string(fixture().join("fonts/manifest.txt")).unwrap();
    let entries = parse_manifest(&text).unwrap();
    assert_eq!(entries.len(), 6);
    assert_eq!((entries[0].family.as_str(), entries[0].style.as_str()), ("Sans JP", "Regular"));
    assert_eq!(entries[0].scripts, ["Jpan", "Latn"]);
    assert_eq!((entries[0].file.as_str(), entries[0].index), ("fonts/sans-jp/SansJP-Regular.ttf", 0));
    assert_eq!((entries[4].file.as_str(), entries[4].index), ("fonts/sans-cjk/SansCJK.ttc", 3));
    assert_eq!(entries[4].scripts, ["Hant-HK", "Latn"]);
    assert_eq!(entries[4].licence, "OFL-1.1");

    let short = "# comment\n\nA | Regular | a.ttf | Latn\n";
    let err = parse_manifest(short).unwrap_err();
    assert_eq!(err.line, 3);
    assert!(err.to_string().contains("expected 8 fields"), "{err}");
    let bad_index = "A | Regular | a.ttc#x | Latn | OFL-1.1 | OFL.txt | 00 | src\n";
    assert!(parse_manifest(bad_index).unwrap_err().to_string().contains("face index"));
    let ui = "A UI | Regular | a.ttf | Hant | OFL-1.1 | OFL.txt | 00 | src | ui | future field\nA | Regular | a.ttf | Hant | OFL-1.1 | OFL.txt | 00 | src |\n";
    let entries = parse_manifest(ui).unwrap();
    assert_eq!(entries[0].role.as_deref(), Some("ui"));
    assert!(entries[0].is_ui() && entries[0].is_cjk());
    assert_eq!(entries[1].role, None);
    assert!(!entries[1].is_ui());
    assert!(entries.iter().all(|e| e.role.is_none() || e.is_ui()));
}

#[test]
fn unset_dir_writes_an_empty_table() {
    let r = run(Embed::new(), None, None);
    assert_eq!(r.result.unwrap(), Vec::<String>::new());
    assert_eq!(r.table.trim(), "&[]");
    assert!(r.cargo.contains(&"cargo::rerun-if-env-changed=CRAFT_FONTS_DIR".to_owned()));
    assert!(r.cargo.contains(&"cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED".to_owned()));
    assert!(!r.cargo.iter().any(|l| l.starts_with("cargo::warning")));
    // Only `1` means required.
    for value in ["0", "true", "yes", " 1"] {
        assert!(run(Embed::new(), None, Some(value)).result.is_ok(), "{value:?}");
    }
    assert!(run(Embed::new(), Some(Path::new("")), Some("")).result.is_ok());
}

#[test]
fn required_fails_without_a_usable_checkout() {
    for dir in [None, Some(Path::new(""))] {
        let r = run(Embed::new(), dir, Some("1"));
        assert!(matches!(r.result, Err(Error::NotSet)), "{dir:?}");
        assert_eq!(r.table.trim(), "&[]");
    }
    let err = run(Embed::new(), None, Some("1")).result.unwrap_err().to_string();
    assert!(err.contains("CRAFT_FONTS_DIR") && err.contains("CRAFT_FONTS_REQUIRED"), "{err}");

    let missing = Path::new("/nonexistent/craft-fonts");
    assert!(matches!(run(Embed::new(), Some(missing), Some("1")).result, Err(Error::Read { .. })));
    let r = run(Embed::new(), Some(missing), None);
    assert!(r.result.unwrap().is_empty());
    // Watching a missing manifest would rerun the build script on every build.
    assert!(!r.cargo.iter().any(|l| l.starts_with("cargo::rerun-if-changed")), "{:?}", r.cargo);
    assert!(r.cargo.iter().any(|l| l.starts_with("cargo::warning=building without craft-fonts")), "{:?}", r.cargo);

    let scratch = Scratch::new("missing-file");
    std::fs::create_dir_all(scratch.0.join("fonts")).unwrap();
    std::fs::write(scratch.0.join("fonts/manifest.txt"), "A | Regular | fonts/a.ttf | Latn | OFL-1.1 | OFL.txt | 00 | src\n").unwrap();
    assert!(matches!(run(Embed::new(), Some(&scratch.0), Some("1")).result, Err(Error::Read { .. })));
    std::fs::write(scratch.0.join("fonts/manifest.txt"), "# no faces\n").unwrap();
    assert!(matches!(run(Embed::new(), Some(&scratch.0), Some("1")).result, Err(Error::NoFaces { .. })));
}

#[cfg(unix)]
#[test]
fn rejects_a_checkout_under_a_non_utf8_path() {
    use std::os::unix::ffi::OsStrExt;
    let scratch = Scratch::new("non-utf8");
    let dir = scratch.0.join(std::ffi::OsStr::from_bytes(b"craft-fonts-\xff"));
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::copy(fixture().join("fonts/sans-arabic/SansArabic.ttf"), dir.join("fonts/a.ttf")).unwrap();
    std::fs::write(dir.join("fonts/manifest.txt"), "A | Regular | fonts/a.ttf | Arab | OFL-1.1 | OFL.txt | 00 | src\n").unwrap();
    let r = run(Embed::new(), Some(&dir), Some("1"));
    assert!(matches!(r.result, Err(Error::NonUtf8Path { .. })), "{:?}", r.result);
    assert_eq!(r.table.trim(), "&[]");
}

#[test]
fn embeds_the_selected_scripts() {
    let r = run(Embed::new(), Some(&fixture()), Some("1"));
    assert_eq!(r.result.unwrap().len(), 6);

    let r = run(Embed::new().scripts(&["Jpan", "Hant"]), Some(&fixture()), Some("1"));
    assert_eq!(r.result.unwrap(), ["Sans JP Regular", "Sans JP Bold", "Sans CJK TC Regular", "Sans CJK HK Regular"]);
    let manifest = fixture().join("fonts/manifest.txt");
    assert!(r.cargo.contains(&format!("cargo::rerun-if-changed={}", manifest.display())));
    let jp = fixture().join("fonts/sans-jp/SansJP-Regular.ttf").canonicalize().unwrap();
    assert!(r.cargo.contains(&format!("cargo::rerun-if-changed={}", jp.display())));
    assert!(r.table.contains(&format!("include_bytes!({:?})", jp.display().to_string())), "{}", r.table);
    assert!(r.table.contains("family: \"Sans CJK HK\", style: \"Regular\", scripts: &[\"Hant-HK\", \"Latn\"], index: 3"), "{}", r.table);

    let r = run(Embed::new().scripts(&["Hant-HK"]).filter(|f| f.index != 0), Some(&fixture()), None);
    assert_eq!(r.result.unwrap(), ["Sans CJK HK Regular"]);

    let r = run(Embed::new().scripts(&["Arab", "Kore", "Geor"]), Some(&fixture()), Some("1"));
    let Err(Error::MissingScripts { scripts, .. }) = r.result else { panic!("{:?}", r.result) };
    assert_eq!(scripts, ["Kore", "Geor"]);
    // Not required: whatever matched is embedded.
    let r = run(Embed::new().scripts(&["Arab", "Kore"]), Some(&fixture()), None);
    assert_eq!(r.result.unwrap(), ["Sans Arabic Regular"]);
}

#[test]
fn embedded_face_helpers() {
    let face = |style: &'static str| EmbeddedFace { family: "Sans", style, scripts: &["Hant-HK", "Latn"], index: 0, bytes: b"" };
    assert_eq!(face("Regular").name(), "Sans Regular");
    assert!(face("Regular").has_script("Hant") && face("Regular").has_script("Hant-HK") && !face("Regular").has_script("Han"));
    let weights: Vec<u16> = ["Regular", "Bold", "SemiBold", "Extra Light", "Black Italic", "Italic"].map(|s| face(s).weight()).into();
    assert_eq!(weights, [400, 700, 600, 200, 900, 400]);
    assert!(face("Regular").is_cjk());
    assert!(!EmbeddedFace { scripts: &["Arab"], ..face("Regular") }.is_cjk());
}

#[test]
fn documents_embeds_only_the_named_families() {
    let r = run(Embed::documents(&["Sans JP", "Sans Thai"]), Some(&fixture()), Some("1"));
    assert_eq!(r.result.unwrap(), ["Sans JP Regular", "Sans JP Bold", "Sans Thai Regular"]);
    let r = run(Embed::documents(&["Sans JP", "Sans Thai"]).filter(|f| f.style == "Bold"), Some(&fixture()), Some("1"));
    assert_eq!(r.result.unwrap(), ["Sans JP Bold"]);
    let r = run(Embed::documents(&[]), Some(&fixture()), None);
    assert_eq!(r.result.unwrap(), Vec::<String>::new());

    let r = run(Embed::documents(&["Sans JP", "Sans Jp", "Serif"]), Some(&fixture()), Some("1"));
    let Err(Error::UnknownFamilies { families, .. }) = &r.result else { panic!("{:?}", r.result) };
    assert_eq!(families, &["Sans Jp", "Serif"]);
    let message = r.result.unwrap_err().to_string();
    assert!(
        message.contains("has no family Sans Jp, Serif") && message.ends_with("Sans JP, Sans Arabic, Sans CJK TC, Sans CJK HK, Sans Thai"),
        "{message}"
    );
    assert_eq!(r.table.trim(), "&[]");
    // Not required: a warning and an empty table, as for any checkout that can't be used.
    let r = run(Embed::documents(&["Serif"]), Some(&fixture()), None);
    assert!(r.result.as_ref().unwrap().is_empty());
    assert!(warnings(&r).iter().any(|w| w.contains("building without craft-fonts") && w.contains("no family Serif")), "{:?}", r.cargo);
}

/// A font with only a `cmap` table: a Windows format 4 subtable with `bmp` segments `(first,
/// last, glyphs)`, mapped to glyphs 1.. when `glyphs` is `None` and through the glyph array
/// otherwise, and a format 12 subtable with `groups` `(first, last)` when there are any.
fn tiny_font(bmp: &[(u16, u16, Option<&[u16]>)], groups: &[(u32, u32)]) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());

    let mut segments = bmp.to_vec();
    segments.push((0xFFFF, 0xFFFF, None));
    let n = u16::try_from(segments.len()).unwrap();
    let (mut ends, mut starts, mut deltas, mut offsets, mut glyphs) = (vec![], vec![], vec![], vec![], vec![]);
    for (i, &(first, last, ids)) in segments.iter().enumerate() {
        be16(&mut ends, last);
        be16(&mut starts, first);
        match ids {
            None => {
                // The last segment maps 0xFFFF to glyph 0, as fonts do.
                be16(&mut deltas, 1u16.wrapping_sub(if first == 0xFFFF { 0 } else { first }));
                be16(&mut offsets, 0);
            }
            Some(ids) => {
                be16(&mut deltas, 0);
                be16(&mut offsets, 2 * (n - u16::try_from(i).unwrap()) + u16::try_from(glyphs.len()).unwrap());
                ids.iter().for_each(|&g| be16(&mut glyphs, g));
            }
        }
    }
    let mut format4 = vec![];
    for x in [4, 0, 0, 2 * n, 0, 0, 0] {
        be16(&mut format4, x);
    }
    format4.extend(ends.iter().chain(&[0, 0]).chain(&starts).chain(&deltas).chain(&offsets).chain(&glyphs));
    let len = u16::try_from(format4.len()).unwrap();
    format4[2..4].copy_from_slice(&len.to_be_bytes());

    let mut format12 = vec![];
    be16(&mut format12, 12);
    be16(&mut format12, 0);
    be32(&mut format12, 16 + 12 * u32::try_from(groups.len()).unwrap());
    be32(&mut format12, 0);
    be32(&mut format12, u32::try_from(groups.len()).unwrap());
    for &(first, last) in groups {
        [first, last, 1].into_iter().for_each(|x| be32(&mut format12, x));
    }

    let subtables: Vec<(u16, Vec<u8>)> = [(1, format4), (10, format12)].into_iter().filter(|(enc, _)| *enc == 1 || !groups.is_empty()).collect();
    let mut cmap = vec![];
    be16(&mut cmap, 0);
    be16(&mut cmap, u16::try_from(subtables.len()).unwrap());
    let mut at = 4 + 8 * u32::try_from(subtables.len()).unwrap();
    for (encoding, table) in &subtables {
        [3, *encoding].into_iter().for_each(|x| be16(&mut cmap, x));
        be32(&mut cmap, at);
        at += u32::try_from(table.len()).unwrap();
    }
    subtables.iter().for_each(|(_, table)| cmap.extend(table));

    let mut font = vec![];
    be32(&mut font, 0x0001_0000);
    for x in [1, 0, 0, 0] {
        be16(&mut font, x);
    }
    font.extend_from_slice(b"cmap");
    for x in [0, 28, u32::try_from(cmap.len()).unwrap()] {
        be32(&mut font, x);
    }
    font.extend(cmap);
    font
}

/// A `.ttc` of fonts made by [`tiny_font`].
fn tiny_collection(fonts: &[Vec<u8>]) -> Vec<u8> {
    let header = 12 + 4 * fonts.len();
    let mut ttc = b"ttcf\0\x01\0\0".to_vec();
    ttc.extend_from_slice(&u32::try_from(fonts.len()).unwrap().to_be_bytes());
    let mut body = vec![];
    for font in fonts {
        let start = u32::try_from(header + body.len()).unwrap();
        ttc.extend_from_slice(&start.to_be_bytes());
        let mut font = font.clone();
        // The table offsets of a collection's faces count from the start of the file.
        let offset = u32::from_be_bytes(font[20..24].try_into().unwrap()) + start;
        font[20..24].copy_from_slice(&offset.to_be_bytes());
        body.extend(font);
    }
    ttc.extend(body);
    ttc
}

#[test]
fn cmap_reads_formats_4_and_12_and_collections() {
    let font = tiny_font(&[(0x41, 0x5A, None), (0x4E00, 0x4E02, Some(&[5, 0, 7]))], &[(0x2_0000, 0x2_0001)]);
    let cmap = Cmap::read(&font, 0).unwrap();
    let covered: Vec<char> = "AZa一丁丂七\u{FFFF}\u{20000}\u{20001}\u{20002}".chars().filter(|&c| cmap.covers(c)).collect();
    assert_eq!(covered, ['A', 'Z', '一', '丂', '\u{20000}', '\u{20001}']);

    let bmp_in_format12 = tiny_font(&[], &[(0x4E00, 0x4E0F)]);
    assert!(Cmap::read(&bmp_in_format12, 0).unwrap().covers('丁'));

    let ttc = tiny_collection(&[tiny_font(&[(0x41, 0x41, None)], &[]), tiny_font(&[(0x4E00, 0x4E00, None)], &[])]);
    let second = Cmap::read(&ttc, 1).unwrap();
    assert!(second.covers('一') && !second.covers('A'));
    assert!(Cmap::read(&ttc, 0).unwrap().covers('A'));
    assert!(Cmap::read(&ttc, 2).is_none());
    assert!(Cmap::read(b"fixture: Sans JP Regular", 0).is_none());
    assert!(Cmap::read(&font[..40], 0).is_none());
}

#[test]
fn cmap_reads_each_subtable_within_its_declared_length() {
    // 'A' takes its glyph from an empty glyph array, so the read lands on the format 12 subtable
    // that follows the format 4 one.
    let font = tiny_font(&[(0x41, 0x41, Some(&[]))], &[(0x2_0000, 0x2_0000)]);
    let cmap = Cmap::read(&font, 0).unwrap();
    assert!(!cmap.covers('A') && cmap.covers('\u{20000}'));

    // A format 4 length past the end of the table is clamped rather than rejected. The format 4
    // subtable starts after the 28-byte font header, the 4-byte cmap header and two 8-byte encoding
    // records.
    let mut font = tiny_font(&[(0x41, 0x5A, None)], &[(0x2_0000, 0x2_0000)]);
    let format4 = 28 + 4 + 2 * 8;
    let format12 = format4 + usize::from(u16::from_be_bytes(font[format4 + 2..format4 + 4].try_into().unwrap()));
    font[format4 + 2..format4 + 4].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(Cmap::read(&font, 0).unwrap().covers('A'));

    // A format 12 group past the declared length doesn't count.
    font[format12 + 4..format12 + 8].copy_from_slice(&16u32.to_be_bytes());
    let cmap = Cmap::read(&font, 0).unwrap();
    assert!(cmap.covers('A') && !cmap.covers('\u{20000}'));
}

/// A craft-fonts checkout in `scratch` with these `(family, scripts, role, font)` faces, all
/// Regular.
fn checkout(scratch: &Scratch, faces: &[(&str, &str, &str, Vec<u8>)]) -> PathBuf {
    let dir = scratch.0.join("craft-fonts");
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    let mut manifest = String::new();
    for (i, (family, scripts, role, font)) in faces.iter().enumerate() {
        std::fs::write(dir.join(format!("fonts/{i}.ttf")), font).unwrap();
        manifest += &format!("{family} | Regular | fonts/{i}.ttf | {scripts} | OFL-1.1 | fonts/OFL.txt | 00 | src | {role}\n");
    }
    std::fs::write(dir.join("fonts/manifest.txt"), manifest).unwrap();
    dir
}

fn warnings(r: &Run) -> Vec<String> {
    r.cargo.iter().filter(|l| l.starts_with("cargo::warning")).cloned().collect()
}

#[test]
fn for_ui_embeds_cjk_faces_only_as_cut_interface_faces() {
    let scratch = Scratch::new("for-ui");
    let latin = || tiny_font(&[(0x20, 0x7E, None)], &[]);
    let dir = checkout(
        &scratch,
        &[
            ("Noto Sans TC", "Hant,Latn", "", latin()),
            ("Noto Sans TC UI", "Hant,Latn", "ui", latin()),
            ("Noto Sans HK UI", "Hant-HK,Latn", "ui", latin()),
            ("Noto Sans Arabic", "Arab", "", latin()),
        ],
    );
    // Without catalogs to cut them to, the interface faces are left out too.
    let r = run(Embed::new().for_ui(), Some(&dir), None);
    assert_eq!(r.result.as_ref().unwrap(), &["Noto Sans Arabic Regular"]);
    let w = warnings(&r);
    assert_eq!(w.len(), 1, "{w:#?}");
    assert!(w[0].contains("not embedding the interface faces Noto Sans TC UI Regular, Noto Sans HK UI Regular"), "{}", w[0]);
    assert!(w[0].contains("Embed::subset_to"), "{}", w[0]);
    // A manifest without interface faces loses every CJK face, which is worth a warning.
    let r = run(Embed::new().for_ui(), Some(&fixture()), None);
    assert!(r.cargo.contains(&format!("cargo::rerun-if-changed={}", fixture().join("fonts/manifest.txt").display())), "{:?}", r.cargo);
    let w = warnings(&r);
    assert_eq!(w.len(), 1, "{w:#?}");
    assert!(w[0].contains("not embedding any CJK face for the interface") && w[0].contains("role `ui`"), "{}", w[0]);
    let r = run(Embed::new().for_ui().scripts(&["Hant", "Arab"]), Some(&dir), Some("1"));
    let Err(Error::MissingScripts { scripts, .. }) = r.result else { panic!("{:?}", r.result) };
    assert_eq!(scripts, ["Hant"]);
    assert_eq!(r.table.trim(), "&[]");
    // Without `for_ui` the interface faces' sources are left out, unless a document table names them.
    let r = run(Embed::new().scripts(&["Hant"]), Some(&dir), Some("1"));
    assert_eq!(r.result.as_ref().unwrap(), &["Noto Sans TC Regular"]);
    assert_eq!(warnings(&r), Vec::<String>::new());
    let r = run(Embed::documents(&["Noto Sans TC UI", "Noto Sans TC"]), Some(&dir), Some("1"));
    assert_eq!(r.result.as_ref().unwrap(), &["Noto Sans TC Regular", "Noto Sans TC UI Regular"]);
    assert!(r.table.contains("fonts/1.ttf"), "{}", r.table);
}

#[test]
fn check_text_warns_about_characters_no_face_covers() {
    let scratch = Scratch::new("check-text");
    let dir = checkout(
        &scratch,
        &[
            ("Noto Sans TC", "Hant,Latn", "", tiny_font(&[(0x20, 0x7E, None), (0x4E00, 0x4E03, None)], &[])),
            ("Noto Sans Arabic", "Arab", "", tiny_font(&[(0x600, 0x6FF, None)], &[])),
            ("Noto Sans SC", "Hans,Latn", "", tiny_font(&[(0x20, 0x7E, None), (0x3000, 0x9FFF, None)], &[])),
        ],
    );
    let catalogs = scratch.0.join("i18n");
    std::fs::create_dir_all(&catalogs).unwrap();
    std::fs::write(catalogs.join("zh-TW.ftl"), "\u{FEFF}open = 一丁万。\nclose = 丁万\u{FE0F} \u{1F600}\n").unwrap();
    std::fs::write(catalogs.join("ar.ftl"), "open = فتح\nok = 一\n").unwrap();
    let long: String = (0x4E10..0x4E20).filter_map(char::from_u32).collect();
    std::fs::write(catalogs.join("long.ftl"), format!("x = {long}\n")).unwrap();
    let paths = ["zh-TW.ftl", "ar.ftl", "long.ftl", "missing.ftl"].map(|f| catalogs.join(f));

    let r = run(Embed::new().scripts(&["Hant", "Arab"]).check_text(paths.clone()), Some(&dir), None);
    assert_eq!(r.result.as_ref().unwrap().len(), 2);
    for path in &paths {
        assert!(r.cargo.contains(&format!("cargo::rerun-if-changed={}", path.display())), "{:?}", r.cargo);
    }
    let w = warnings(&r);
    assert_eq!(w.len(), 3, "{w:#?}");
    assert!(w[0].contains("zh-TW.ftl has 2 characters no embedded face covers (万。); system fonts"), "{}", w[0]);
    assert!(w[1].contains("long.ftl has 16 characters") && w[1].contains("(丐丑丒专且丕世丗 …)"), "{}", w[1]);
    assert!(w[2].contains("cannot check") && w[2].contains("missing.ftl"), "{}", w[2]);

    // The full face covers everything; without a checkout there is nothing to check.
    let r = run(Embed::new().check_text(paths[..3].to_vec()), Some(&dir), None);
    assert_eq!(warnings(&r), Vec::<String>::new());
    let r = run(Embed::new().check_text(paths.clone()), None, None);
    assert_eq!(warnings(&r), Vec::<String>::new());
    assert!(r.cargo.contains(&format!("cargo::rerun-if-changed={}", paths[0].display())));

    // Faces another table embeds count as covering their characters.
    let r = run(Embed::new().scripts(&["Arab"]).embedded_elsewhere(&["Noto Sans SC"]).check_text(paths[..3].to_vec()), Some(&dir), None);
    assert_eq!(warnings(&r), Vec::<String>::new());
    let r = run(
        Embed::new().scripts(&["Arab"]).embedded_elsewhere(&["Noto Sans TC", "Noto Sans Math"]).check_text(paths[..2].to_vec()),
        Some(&dir),
        None,
    );
    let w = warnings(&r);
    assert_eq!(w.len(), 2, "{w:#?}");
    assert!(w[0].contains("embedded_elsewhere names Noto Sans Math, which") && w[0].ends_with("manifest.txt lacks"), "{}", w[0]);
    assert!(w[1].contains("zh-TW.ftl has 2 characters no embedded face covers (万。)"), "{}", w[1]);

    // The fixture's "fonts" are text, so they cover nothing.
    let r = run(Embed::new().scripts(&["Arab"]).check_text([&paths[1]]), Some(&fixture()), None);
    let w = warnings(&r);
    assert!(w[0].contains("cannot read the character map of Sans Arabic Regular"), "{w:#?}");
    assert!(w[1].contains("ar.ftl has 1 character no embedded face covers (一)"), "{w:#?}");
}

#[cfg(feature = "subset")]
mod subset {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use super::{Run, Scratch, checkout, run, tiny_font, warnings};
    use crate::cmap::Cmap;
    use crate::{Embed, Error};

    /// `fixture/subset/make-fonts.py` writes these.
    fn tiny(file: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/subset").join(file)).unwrap()
    }

    /// The BMP characters `font` maps.
    fn mapped(font: &[u8]) -> Vec<u32> {
        let cmap = Cmap::read(font, 0).unwrap();
        ('\0'..='\u{FFFF}').filter(|&c| cmap.covers(c)).map(u32::from).collect()
    }

    fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
        let u16_at = |at: usize| usize::from(u16::from_be_bytes([font[at], font[at + 1]]));
        let u32_at = |at: usize| usize::try_from(u32::from_be_bytes(font[at..at + 4].try_into().unwrap())).unwrap();
        let record = (0..u16_at(4)).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == tag)?;
        Some(&font[u32_at(record + 8)..][..u32_at(record + 12)])
    }

    /// The feature tags of a GSUB or GPOS table.
    fn features(table: &[u8]) -> BTreeSet<String> {
        let list = usize::from(u16::from_be_bytes([table[6], table[7]]));
        let count = usize::from(u16::from_be_bytes([table[list], table[list + 1]]));
        (0..count).map(|i| String::from_utf8_lossy(&table[list + 2 + 6 * i..][..4]).into_owned()).collect()
    }

    fn glyph_count(font: &[u8]) -> u16 {
        let maxp = table(font, b"maxp").unwrap();
        u16::from_be_bytes([maxp[4], maxp[5]])
    }

    fn cut_file(r: &Run, family: &str) -> PathBuf {
        r.out.0.join("craft-fonts-ui").join(format!("{}-Regular.ttf", family.replace(' ', "-")))
    }

    #[test]
    fn subset_to_cuts_interface_faces_to_the_catalogs() {
        let scratch = Scratch::new("subset");
        let latin = || tiny_font(&[(0x20, 0x7E, None)], &[]);
        let dir = checkout(
            &scratch,
            &[
                ("Noto Sans TC", "Hant,Latn", "", latin()),
                ("Noto Sans TC UI", "Hant,Latn", "ui", tiny("Tiny-glyf.ttf")),
                ("Noto Sans HK UI", "Hant-HK,Latn", "ui", tiny("Tiny-glyf.ttf")),
                ("Noto Sans Arabic", "Arab", "", latin()),
            ],
        );
        let tw = scratch.0.join("zh-TW.tsv");
        std::fs::write(&tw, "Open\t一万\n").unwrap();
        let ko = scratch.0.join("ko.tsv");
        std::fs::write(&ko, "Open\t丁\u{FE0F} \u{1F600}\n").unwrap();

        let r = run(Embed::new().subset_to([&tw]).scripts(&["Hant"]), Some(&dir), Some("1"));
        assert_eq!(r.result.as_ref().unwrap(), &["Noto Sans TC UI Regular", "Noto Sans HK UI Regular"]);
        let tc = cut_file(&r, "Noto Sans TC UI");
        assert!(r.table.contains(&format!("index: 0, bytes: include_bytes!({:?})", tc.display().to_string())), "{}", r.table);
        assert!(r.cargo.contains(&format!("cargo::rerun-if-changed={}", tw.display())), "{:?}", r.cargo);
        let cut = std::fs::read(&tc).unwrap();
        // The catalog's 一, plus the punctuation, kana and full-width forms the source has; not
        // its Latin, nor the 丁 and 七 no catalog uses.
        assert_eq!(mapped(&cut), [0x3001, 0x300C, 0x3042, 0x4E00, 0xFF01]);
        assert!(table(&cut, b"glyf").is_some());
        assert_eq!(features(table(&cut, b"GSUB").unwrap()), BTreeSet::from(["vert".to_owned()]));
        // .notdef, five characters and the vertical form of U+300C.
        assert_eq!(glyph_count(&cut), 7);
        // The source lacks 万, so system fonts draw it.
        let w = warnings(&r);
        assert_eq!(w.len(), 1, "{w:#?}");
        assert!(w[0].contains("zh-TW.tsv has 1 character no embedded face covers (万)"), "{}", w[0]);

        // The same bytes on every run. Another catalog adds its characters, but no emoji or
        // variation selector, and a `check_text` file is only checked.
        let again = run(Embed::new().for_ui().subset_to([&tw]), Some(&dir), None);
        assert_eq!(std::fs::read(cut_file(&again, "Noto Sans TC UI")).unwrap(), cut);
        let r = run(Embed::new().subset_to([&tw, &ko]).check_text([scratch.0.join("zh-TW.tsv")]), Some(&dir), None);
        assert_eq!(mapped(&std::fs::read(cut_file(&r, "Noto Sans TC UI")).unwrap()), [0x3001, 0x300C, 0x3042, 0x4E00, 0x4E01, 0xFF01]);
        assert_eq!(warnings(&r).len(), 1, "{:#?}", warnings(&r));

        // No catalogs (an app whose CRAFT_I18N_DIR is unset) leaves the faces out, not cut to kana.
        let r = run(Embed::new().subset_to(Vec::<PathBuf>::new()), Some(&dir), None);
        assert_eq!(r.result.as_ref().unwrap(), &["Noto Sans Arabic Regular"]);
        let w = warnings(&r);
        assert_eq!(w.len(), 1, "{w:#?}");
        assert!(w[0].contains("Noto Sans TC UI Regular") && w[0].contains("given no translation catalogs"), "{}", w[0]);
    }

    #[test]
    fn subset_to_rejects_cff_interface_faces() {
        let scratch = Scratch::new("subset-cff");
        let dir = checkout(&scratch, &[("Noto Sans SC UI", "Hans", "ui", tiny("Tiny-cff.otf"))]);
        let catalog = scratch.0.join("zh-CN.tsv");
        std::fs::write(&catalog, "Open\t一\n").unwrap();
        for required in [None, Some("1")] {
            let r = run(Embed::new().subset_to([&catalog]), Some(&dir), required);
            let Err(e @ Error::NotTrueType { .. }) = r.result else { panic!("{:?}", r.result) };
            assert!(e.to_string().contains("0.ttf has CFF outlines"), "{e}");
            assert_eq!(r.table.trim(), "&[]");
        }
    }

    /// With `CRAFT_FONTS_DIR` set to a craft-fonts checkout, cut its BIZ UDPGothic Regular.
    #[test]
    fn cuts_biz_udpgothic() {
        let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").filter(|dir| !dir.is_empty()) else { return };
        let Ok(source) = std::fs::read(Path::new(&dir).join("fonts/biz-ud-pgothic/BIZUDPGothic-Regular.ttf")) else { return };
        let chars: BTreeSet<char> = "ファイルを開く「新規」、保存。編集表示".chars().collect();
        let cut = crate::subset::subset(&source, 0, &chars).unwrap();
        let (source_cmap, cmap) = (Cmap::read(&source, 0).unwrap(), Cmap::read(&cut, 0).unwrap());
        assert!(chars.iter().all(|&c| cmap.covers(c) == source_cmap.covers(c)));
        assert!(!cmap.covers('A') && !cmap.covers('漢'));
        assert!(features(table(&cut, b"GSUB").unwrap()).contains("vert"));
        assert!(cut.len() < source.len() / 20, "{} of {} bytes", cut.len(), source.len());
        assert_eq!(crate::subset::subset(&source, 0, &chars).unwrap(), cut);
    }
}
