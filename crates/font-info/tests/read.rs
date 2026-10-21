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
use write_fonts::tables::stat::{AxisRecord, AxisValue, AxisValueTableFlags, Stat};
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
        name(262, "TestVF-Heavy"),
        name(263, "Roman"),
        name(25, "TestVF"),
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
            post_script_name_id: Some(NameId::new(262)),
        },
        // A blank name is no instance.
        InstanceRecord {
            subfamily_name_id: NameId::new(261),
            flags: 0,
            coordinates: vec![Fixed::from_f64(100.0), Fixed::from_f64(96.0)],
            // 0xFFFF: the instance has no PostScript name.
            post_script_name_id: Some(NameId::new(0xFFFF)),
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
    // STAT: wght, and an `ital` fvar lacks with this face's value 0 ("Roman"); a second value record
    // for it doesn't override the first.
    let stat = Stat {
        design_axes: vec![AxisRecord::new(Tag::new(b"wght"), NameId::new(256), 0), AxisRecord::new(Tag::new(b"ital"), NameId::new(263), 1)].into(),
        offset_to_axis_values: Some(vec![
            AxisValue::format_3(1, AxisValueTableFlags::ELIDABLE_AXIS_VALUE_NAME, NameId::new(263), Fixed::from_f64(0.0), Fixed::from_f64(1.0))
                .into(),
            AxisValue::format_1(1, AxisValueTableFlags::empty(), NameId::new(263), Fixed::from_f64(1.0)).into(),
        ])
        .into(),
        elided_fallback_name_id: Some(NameId::new(2)),
    };
    let mut b = FontBuilder::new();
    b.add_table(&cmap).unwrap().add_table(&Name::new(names)).unwrap().add_table(&fvar).unwrap().add_table(&stat).unwrap();
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
    assert_eq!(d.instances[0].postscript_name.as_deref(), Some("TestVF-Heavy"));
    assert_eq!(d.variations_prefix.as_deref(), Some("TestVF"));
    let stat: Vec<(&str, Option<f32>)> = d.stat_axes.iter().map(|a| (a.tag.as_str(), a.value)).collect();
    assert_eq!(stat, [("wght", None), ("ital", Some(0.0))]);

    let tags: Vec<&str> = d.features.iter().map(|f| f.tag.as_str()).collect();
    assert_eq!(tags, ["cv11", "kern", "ss01"], "sorted, each tag once");
    let ss01 = d.feature("ss01").unwrap();
    assert_eq!((ss01.name.as_deref(), ss01.sample.as_str()), (Some("Open digits"), "1"));
    let cv11 = d.feature("cv11").unwrap();
    assert_eq!((cv11.name.as_deref(), cv11.sample.as_str()), (Some("Single-story a"), "a"));
    assert_eq!((ss01.changed, cv11.changed, d.feature("kern").map(|f| f.changed)), (1, 1, Some(0)));
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

#[test]
fn registered_feature_names() {
    assert_eq!(craft_font_info::feature_name("tnum"), Some("Tabular Figures"));
    assert_eq!(craft_font_info::feature_name("salt"), Some("Stylistic Alternates"));
    // Numbered sets and variants are named by the font; unknown tags have no name.
    for tag in ["ss01", "cv11", "zzzz", ""] {
        assert_eq!(craft_font_info::feature_name(tag), None, "{tag}");
    }
}

#[test]
fn feature_names_are_one_per_tag() {
    let mut tags: Vec<&str> = craft_font_info::FEATURE_NAMES.iter().map(|(t, _)| *t).collect();
    let n = tags.len();
    tags.sort_unstable();
    tags.dedup();
    assert_eq!(tags.len(), n, "a tag listed twice");
    assert!(tags.iter().all(|t| t.len() == 4 && t.is_ascii()));
}

/// The names are the registry's, so they follow how it capitalises: every word, small ones
/// included, starts with a capital ("Small Capitals From Capitals", "JIS78 Forms"), and none is
/// padding. Whether a name is the *right* word for its tag can't be checked without a second copy
/// of the registry, so that stays a review question; what a test can hold is that no name drifts
/// into house style, repeats another's, or goes missing.
#[test]
fn feature_names_are_capitalised_as_the_registry_writes_them() {
    for (tag, name) in craft_font_info::FEATURE_NAMES {
        assert!(!name.is_empty() && *name == name.trim() && name.is_ascii(), "{tag}: {name:?}");
        for word in name.split(' ') {
            assert!(word.starts_with(|c: char| c.is_ascii_uppercase()), "{tag}: {name:?} has {word:?}");
        }
    }
    // Seven CJK tags once shared one label, so a UI showed seven identical toggles.
    let mut names: Vec<&str> = craft_font_info::FEATURE_NAMES.iter().map(|(_, n)| *n).collect();
    let n = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), n, "two tags share a label");
}
