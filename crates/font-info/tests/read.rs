//! Reads a font built here with `write-fonts` (no font file in the repository): two axes, one
//! hidden, a named instance, a stylistic set and a character variant with their own names, and a
//! positioning feature.

use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::fvar::InstanceRecord;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, VariationAxisRecord};
use write_fonts::tables::gpos::{Gpos, PositionLookupList};
use write_fonts::tables::gsub::{Gsub, SingleSubst, SubstitutionLookupList};
use write_fonts::tables::layout::{
    CharacterVariantParams, CoverageTable, Feature, FeatureList, FeatureParams, FeatureRecord, LangSys, Lookup, LookupFlag, Script, ScriptList,
    ScriptRecord, StylisticSetParams,
};
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::types::{Fixed, GlyphId, GlyphId16, NameId, Tag};

fn name(id: u16, s: &str) -> NameRecord {
    NameRecord::new(3, 1, 0x409, NameId::new(id), s.to_string().into())
}

fn single(from: u16, delta: i16) -> SingleSubst {
    SingleSubst::format_1([GlyphId16::new(from)].into_iter().collect::<CoverageTable>(), delta)
}

/// Glyphs: 1 `a`, 2 `b`, 3 `1`, 4 `1.ss01`, 5 `a.cv11`.
fn font() -> Vec<u8> {
    let cmap = Cmap::from_mappings([('a', GlyphId::new(1)), ('b', GlyphId::new(2)), ('1', GlyphId::new(3))]).unwrap();
    let mut names = vec![
        name(256, "Weight"),
        name(257, "Parametric Thick Stroke"),
        name(258, "Heavy"),
        name(259, "Open digits"),
        name(260, "Single-story a"),
        name(261, "   "),
    ];
    names.sort_by_key(|r| r.name_id);
    let axes = vec![
        VariationAxisRecord::new(Tag::new(b"wght"), Fixed::from_f64(100.0), Fixed::from_f64(400.0), Fixed::from_f64(900.0), 0, NameId::new(256)),
        VariationAxisRecord::new(Tag::new(b"XOPQ"), Fixed::from_f64(27.0), Fixed::from_f64(96.0), Fixed::from_f64(175.0), 1, NameId::new(257)),
    ];
    let instances = vec![
        InstanceRecord {
            subfamily_name_id: NameId::new(258),
            flags: 0,
            coordinates: vec![Fixed::from_f64(800.0), Fixed::from_f64(96.0)],
            post_script_name_id: None,
        },
        // A blank name is no instance.
        InstanceRecord {
            subfamily_name_id: NameId::new(261),
            flags: 0,
            coordinates: vec![Fixed::from_f64(100.0), Fixed::from_f64(96.0)],
            post_script_name_id: None,
        },
    ];
    let fvar = Fvar::new(AxisInstanceArrays::new(axes, instances));
    let lookups = SubstitutionLookupList::new(vec![
        Lookup::new(LookupFlag::empty(), vec![single(3, 1)]).into(),
        Lookup::new(LookupFlag::empty(), vec![single(1, 4)]).into(),
    ]);
    let ss01 = FeatureParams::StylisticSet(StylisticSetParams::new(NameId::new(259)));
    let cv11 =
        FeatureParams::CharacterVariant(CharacterVariantParams::new(NameId::new(260), NameId::new(0), NameId::new(0), 0, NameId::new(0), vec![]));
    let features = FeatureList::new(vec![
        FeatureRecord::new(Tag::new(b"ss01"), Feature::new(Some(ss01), vec![0])),
        FeatureRecord::new(Tag::new(b"cv11"), Feature::new(Some(cv11), vec![1])),
        // A second record of a tag (another script) is merged, and its out-of-range lookup skipped.
        FeatureRecord::new(Tag::new(b"ss01"), Feature::new(None, vec![99])),
    ]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0, 1, 2])), vec![]))]);
    let gsub = Gsub::new(scripts.clone(), features, lookups);
    let gpos = Gpos::new(
        scripts,
        FeatureList::new(vec![FeatureRecord::new(Tag::new(b"kern"), Feature::new(None, vec![]))]),
        PositionLookupList::new(vec![]),
    );
    let mut b = FontBuilder::new();
    b.add_table(&cmap).unwrap().add_table(&Name::new(names)).unwrap().add_table(&fvar).unwrap();
    b.add_table(&gsub).unwrap().add_table(&gpos).unwrap();
    b.build()
}

#[test]
fn reads_axes_instances_and_named_features() {
    let d = craft_font_info::read(&font(), 0).unwrap();
    let wght = d.axis("wght").unwrap();
    assert_eq!((wght.name.as_deref(), wght.min, wght.default, wght.max, wght.hidden), (Some("Weight"), 100.0, 400.0, 900.0, false));
    let xopq = d.axis("XOPQ").unwrap();
    assert_eq!((xopq.name.as_deref(), xopq.hidden), (Some("Parametric Thick Stroke"), true));
    assert_eq!(d.instances.len(), 1, "{:?}", d.instances);
    assert_eq!(d.instances[0].name, "Heavy");
    assert_eq!(d.instances[0].coords, vec![("wght".to_string(), 800.0), ("XOPQ".to_string(), 96.0)]);

    let tags: Vec<&str> = d.features.iter().map(|f| f.tag.as_str()).collect();
    assert_eq!(tags, ["cv11", "kern", "ss01"], "sorted, each tag once");
    let ss01 = d.feature("ss01").unwrap();
    assert_eq!((ss01.name.as_deref(), ss01.sample.as_str()), (Some("Open digits"), "1"));
    let cv11 = d.feature("cv11").unwrap();
    assert_eq!((cv11.name.as_deref(), cv11.sample.as_str()), (Some("Single-story a"), "a"));
    assert_eq!(d.feature("kern").map(|f| (f.name.clone(), f.sample.clone())), Some((None, String::new())), "positioning has no sample");
    assert!(d.has_feature("kern") && !d.has_feature("liga") && d.feature("zzzz").is_none());
}

#[test]
fn not_a_font_is_an_error() {
    assert!(craft_font_info::read(b"not a font", 0).is_err());
    assert!(craft_font_info::read(&[], 0).is_err());
    assert!(craft_font_info::read(&font(), 5).is_err(), "no face 5");
    assert!(craft_font_info::read(b"not a font", 0).unwrap_err().to_string().starts_with("not a readable font"));
}

/// Fonts are input: every truncation and a sweep of corrupted bytes reads or fails, never panics.
#[test]
fn damaged_fonts_never_panic() {
    let f = font();
    for n in 0..f.len() {
        let _ = craft_font_info::read(&f[..n], 0);
    }
    for i in 0..f.len() {
        for v in [0x00, 0xFF, 0x7F] {
            let mut g = f.clone();
            g[i] = v;
            let _ = craft_font_info::read(&g, 0);
        }
    }
}
