use std::collections::BTreeSet;

use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::{FontRef, TableProvider};
use skrifa::{MetadataProvider, Tag};

use write_fonts::FontBuilder;
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::name::Name;

use super::codepoints::{self, canonical, parse};
use super::{Error, Instance, Licence, Naming, Quality, closure, subset};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn naming(licence: Licence, family: &str) -> Naming {
    Naming { licence, family_name: family.to_owned(), sha256: [0x42; 32] }
}

/// Subsets `text` (with its closure) and returns the decoded sfnt.
fn subset_text(font: &[u8], text: &str, instance: &Instance, naming: &Naming) -> Vec<u8> {
    let cps: Vec<u32> = codepoints::of_text(text).into_iter().collect();
    let woff2 = subset(font, &closure(&cps), instance, naming, Quality::OnDemand).unwrap();
    assert_eq!(&woff2[..4], b"wOF2");
    let ttf = wuff::decompress_woff2(&woff2).unwrap();
    FontRef::new(&ttf).expect("subset parses with skrifa");
    ttf
}

fn names(ttf: &[u8], id: u16) -> Vec<String> {
    let font = FontRef::new(ttf).unwrap();
    let name = font.name().unwrap();
    name.name_record().iter().filter(|r| r.name_id().to_u16() == id).map(|r| r.string(name.string_data()).unwrap().to_string()).collect()
}

/// `font` with the name records of the given IDs set to new strings.
fn with_names(font: &[u8], changes: &[(u16, &str)]) -> Vec<u8> {
    let font = FontRef::new(font).unwrap();
    let mut name: Name = font.name().unwrap().to_owned_table();
    for rec in &mut name.name_record {
        if let Some((_, s)) = changes.iter().find(|(id, _)| *id == rec.name_id.to_u16()) {
            rec.string = (*s).to_owned().into();
        }
    }
    let mut builder = FontBuilder::new();
    builder.add_table(&name).unwrap();
    builder.copy_missing_tables(font).build()
}

/// Lowercase letters and digits only.
fn folded(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

#[test]
fn number_is_the_first_eight_hash_bytes_mod_a_billion() {
    assert_eq!(naming(Licence::Apache, "X").number(), 313_060_418);
}

#[test]
fn shared_vectors_round_trip() {
    let file: serde_json::Value = serde_json::from_str(include_str!("../tests/data/preview-vectors.json")).unwrap();
    for v in file["vectors"].as_array().unwrap() {
        let (text, cps, clo) = (v["text"].as_str().unwrap(), v["codepoints"].as_str().unwrap(), v["closure"].as_str().unwrap());
        assert_eq!(canonical(text), cps, "{text:?}");
        let parsed = parse(cps).unwrap();
        assert_eq!(codepoints::format(&closure(&parsed)), clo, "{text:?}");
    }
}

#[test]
fn ofl_subset_drops_reserved_name_and_keeps_licence_fields() {
    let font = fixture("abeezee/ABeeZee-Regular.ttf");
    let naming = naming(Licence::Ofl { reserved_names: vec!["ABeeZee".into()] }, "ABeeZee");
    let ttf = subset_text(&font, "Hello", &Instance::List, &naming);
    let n = naming.number();
    assert_eq!(names(&ttf, 1), [format!("FPS {n}")]);
    assert_eq!(names(&ttf, 6), [format!("FPS-{n}")]);
    for id in [1, 3, 4, 6, 16, 18, 21] {
        for s in names(&ttf, id) {
            assert!(!s.to_lowercase().contains("abeezee"), "ID {id}: {s}");
        }
    }
    for id in [0, 13, 14] {
        assert!(!names(&ttf, id).is_empty(), "ID {id} kept");
        assert_eq!(names(&ttf, id), names(&font, id), "ID {id} unchanged");
    }
}

#[test]
fn apache_subset_says_it_was_changed() {
    let font = fixture("abeezee/ABeeZee-Regular.ttf");
    let ttf = subset_text(&font, "Hi", &Instance::List, &naming(Licence::Apache, "ABeeZee"));
    assert_eq!(names(&ttf, 10), ["Subset of ABeeZee, changed by font-preview-service"]);
}

#[test]
fn ufl_subset_is_a_named_derivative() {
    let font = fixture("ubuntu/Ubuntu-Regular.ttf");
    let naming = naming(Licence::Ufl, "Ubuntu");
    let ttf = subset_text(&font, "Hello", &Instance::List, &naming);
    let n = naming.number();
    assert_eq!(names(&ttf, 1), [format!("Ubuntu derivative FPS{n}")]);
    let ps = &names(&ttf, 6)[0];
    assert_eq!(*ps, format!("{}-derivativeFPS{n}", names(&font, 6)[0]));
    assert!(ps.len() <= 63 && ps.chars().all(|c| c.is_ascii_graphic()));
}

#[test]
fn thai_sara_am_shapes_without_notdef() {
    let font = fixture("sarabun/Sarabun-Regular.ttf");
    let text = "น้ำ";
    let notdefs = |ttf: &[u8]| {
        let face = rustybuzz::Face::from_slice(ttf, 0).unwrap();
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        rustybuzz::shape(&face, &[], buffer).glyph_infos().iter().filter(|g| g.glyph_id == 0).count()
    };
    let ttf = subset_text(&font, text, &Instance::List, &naming(Licence::Ofl { reserved_names: vec![] }, "Sarabun"));
    assert_eq!(notdefs(&ttf), 0);

    // Without the closure, HarfBuzz's SARA AM decomposition hits a missing glyph.
    let bare: BTreeSet<u32> = codepoints::of_text(text);
    let plain = super::subset_sfnt(&font, &bare, &Instance::List).unwrap();
    assert!(notdefs(&plain) > 0);
}

#[test]
fn var_keeps_fvar_and_list_pins_regular() {
    let font = fixture("librefranklin/LibreFranklin[wght].ttf");
    let naming = naming(Licence::Ofl { reserved_names: vec!["Libre Franklin".into()] }, "Libre Franklin");
    let fvar = Tag::new(b"fvar");

    let var = subset_text(&font, "nnn", &Instance::Var, &naming);
    assert!(FontRef::new(&var).unwrap().table_data(fvar).is_some());

    let list = subset_text(&font, "nnn", &Instance::List, &naming);
    let list_font = FontRef::new(&list).unwrap();
    assert!(list_font.table_data(fvar).is_none());
    let full = FontRef::new(&font).unwrap();
    let advance = |f: &FontRef, coords: &[(&str, f32)]| {
        let location = f.axes().location(coords.iter().copied());
        let gid = f.charmap().map('n').unwrap();
        f.glyph_metrics(Size::unscaled(), &location).advance_width(gid).unwrap()
    };
    let pinned = advance(&list_font, &[]);
    assert_eq!(pinned, advance(&full, &[("wght", 400.0)]));
    assert_ne!(pinned, advance(&full, &[]), "the default is Thin, not Regular");

    let bold = subset_text(&font, "nnn", &Instance::Named(vec![(Tag::new(b"wght"), 700.0)]), &naming);
    assert_eq!(advance(&FontRef::new(&bold).unwrap(), &[]), advance(&full, &[("wght", 700.0)]));
}

#[test]
fn family_name_leaves_every_rewritten_record_without_reserved_names() {
    // Libre Franklin has no Reserved Font Name; its family name still goes.
    let font = with_names(
        &fixture("librefranklin/LibreFranklin[wght].ttf"),
        &[(5, "Version 3.000; LibreFranklin"), (17, "Libre Franklin Thin"), (263, "LibreFranklinBold")],
    );
    let licences = [Licence::Ofl { reserved_names: vec![] }, Licence::Apache];
    for licence in licences {
        let naming = naming(licence, "Libre Franklin");
        for instance in [Instance::List, Instance::Var] {
            let ttf = subset_text(&font, "Franklin", &instance, &naming);
            let font = FontRef::new(&ttf).unwrap();
            let name = font.name().unwrap();
            for rec in name.name_record() {
                let id = rec.name_id().to_u16();
                let s = rec.string(name.string_data()).unwrap().to_string();
                if id != 0 && !(7..=14).contains(&id) {
                    assert!(!folded(&s).contains("librefranklin"), "{instance:?} ID {id}: {s}");
                }
            }
            if instance == Instance::Var {
                assert_eq!(names(&ttf, 263), ["Bold"]);
                assert_eq!(names(&ttf, 17), ["Thin"]);
            }
        }
    }
}

#[test]
fn var_subset_renames_instance_postscript_names() {
    let font = fixture("orbitron/Orbitron[wght].ttf");
    let naming = naming(Licence::Ofl { reserved_names: vec!["Orbitron".into()] }, "Orbitron");
    let var = subset_text(&font, "Orbit", &Instance::Var, &naming);
    let var_font = FontRef::new(&var).unwrap();
    let ids: Vec<u16> = var_font.named_instances().iter().filter_map(|i| i.postscript_name_id()).map(|id| id.to_u16()).collect();
    assert!(!ids.is_empty());
    for id in ids {
        let [s] = names(&var, id).try_into().unwrap();
        assert!(s.starts_with(&format!("FPS-{}-", naming.number())), "{s}");
        assert!(!s.to_lowercase().contains("orbitron"), "{s}");
    }
}

#[derive(Default)]
struct CountPen(usize);

impl OutlinePen for CountPen {
    fn move_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {}
}

#[test]
fn cff_names_are_rewritten_and_outlines_survive() {
    let font = fixture("notosanscjk/NotoSansCJKsc-test.otf");
    let full = FontRef::new(&font).unwrap();
    let text: String = full.charmap().mappings().filter_map(|(cp, _)| char::from_u32(cp)).filter(|c| !c.is_ascii()).take(4).collect();
    let naming = naming(Licence::Ofl { reserved_names: vec![] }, "Noto Sans CJK SC");
    let otf = subset_text(&font, &text, &Instance::List, &naming);
    let subset_font = FontRef::new(&otf).unwrap();
    let cff = subset_font.table_data(Tag::new(b"CFF ")).unwrap();
    let cff = cff.as_bytes();
    let contains = |needle: &[u8]| cff.windows(needle.len()).any(|w| w == needle);
    assert!(contains(format!("FPS-{}", naming.number()).as_bytes()));
    assert!(!contains(b"NotoSansCJK"));
    assert!(!contains(b"Noto Sans CJK"));
    for c in text.chars() {
        let gid = subset_font.charmap().map(c).unwrap();
        let mut pen = CountPen::default();
        let glyph = subset_font.outline_glyphs().get(gid).unwrap();
        glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).unwrap();
        assert!(pen.0 > 0, "{c} has an outline");
    }
}

#[test]
fn cff_string_still_naming_the_font_fails_the_face() {
    // "Adobe" is this CID font's ROS Registry, a string renaming leaves alone.
    let font = fixture("notosanscjk/NotoSansCJKsc-test.otf");
    let text: String =
        FontRef::new(&font).unwrap().charmap().mappings().filter_map(|(cp, _)| char::from_u32(cp)).find(|c| !c.is_ascii()).into_iter().collect();
    let cps: Vec<u32> = codepoints::of_text(&text).into_iter().collect();
    let result = subset(&font, &closure(&cps), &Instance::List, &naming(Licence::Apache, "Adobe"), Quality::OnDemand);
    assert!(matches!(result, Err(Error::CffOldName(_))), "{result:?}");
}

#[test]
fn legacy_kern_survives_the_subset() {
    let font = fixture("abel/Abel-Regular.ttf");
    let full = FontRef::new(&font).unwrap();
    assert!(full.table_data(Tag::new(b"kern")).is_some());
    assert!(full.table_data(Tag::new(b"GPOS")).is_none());
    let text: String = (0x20u8..0x7f).map(char::from).collect();
    let ttf = subset_text(&font, &text, &Instance::List, &naming(Licence::Ofl { reserved_names: vec![] }, "Abel Preview"));
    assert!(FontRef::new(&ttf).unwrap().table_data(Tag::new(b"kern")).is_some());
    let advance = |bytes: &[u8]| {
        let face = rustybuzz::Face::from_slice(bytes, 0).unwrap();
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(&text);
        let out = rustybuzz::shape(&face, &[], buffer);
        out.glyph_positions().iter().map(|p| p.x_advance).sum::<i32>()
    };
    assert_eq!(advance(&ttf), advance(&font));
}

#[test]
fn legacy_kern_predicate() {
    let keeps = |path: &str| super::keeps_legacy_kern(&FontRef::new(&fixture(path)).unwrap());
    assert!(keeps("abel/Abel-Regular.ttf"));
    assert!(!keeps("abeezee/ABeeZee-Regular.ttf"));
}

/// The GSUB and GPOS feature tags of `font`.
fn feature_tags(font: &FontRef) -> BTreeSet<Tag> {
    let mut tags = BTreeSet::new();
    if let Ok(gsub) = font.gsub() {
        tags.extend(gsub.feature_list().unwrap().feature_records().iter().map(|r| r.feature_tag()));
    }
    if let Ok(gpos) = font.gpos() {
        tags.extend(gpos.feature_list().unwrap().feature_records().iter().map(|r| r.feature_tag()));
    }
    tags
}

#[test]
fn cut_face_keeps_every_feature_and_cuts_cff() {
    let chars: BTreeSet<u32> = "Hello, Welt! agy 0123".chars().map(u32::from).collect();
    for path in ["sarabun/Sarabun-Regular.ttf", "notosanscjk/NotoSansCJKsc-test.otf"] {
        let source = fixture(path);
        let cut = super::cut_face(&source, 0, &chars).unwrap();
        assert_eq!(cut, super::cut_face(&source, 0, &chars).unwrap(), "{path}: same input, same bytes");
        let (before, after) = (FontRef::new(&source).unwrap(), FontRef::new(&cut).unwrap());
        assert!(after.maxp().unwrap().num_glyphs() < before.maxp().unwrap().num_glyphs(), "{path}");
        let mapped: BTreeSet<u32> = after.charmap().mappings().map(|(cp, _)| cp).collect();
        let wanted: BTreeSet<u32> = chars.iter().copied().filter(|&cp| before.charmap().map(cp).is_some()).collect();
        assert_eq!(mapped, wanted, "{path}");
        // HarfBuzz keeps only fonttools' default features unless told otherwise; `salt` and `ss01` are not
        // among them. A feature none of the kept glyphs reach is dropped.
        let kept = feature_tags(&after);
        for tag in [b"salt", b"ss01"].map(Tag::new).into_iter().filter(|t| feature_tags(&before).contains(t)) {
            assert!(kept.contains(&tag), "{path}: {tag} dropped: {kept:?}");
        }
        assert_eq!(after.table_data(Tag::new(b"CFF ")).is_some(), before.table_data(Tag::new(b"CFF ")).is_some(), "{path}");
    }
}

#[test]
fn cut_face_of_a_missing_index_fails() {
    let chars = BTreeSet::from([u32::from('a')]);
    assert!(super::cut_face(&fixture("sarabun/Sarabun-Regular.ttf"), 1, &chars).is_err());
}
