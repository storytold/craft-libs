use super::*;
use crate::descriptor::UnicodeString;

fn sample(id: &str, w: u32, h: u32, depth: u16) -> AbrSample {
    let bpp = usize::from(depth / 8);
    let data = (0..w as usize * h as usize * bpp).map(|i| if (i / bpp) % 3 == 0 { 255 } else { (i % 7) as u8 * 30 }).collect();
    AbrSample { id: id.into(), width: w, height: h, depth, data }
}

#[test]
fn embedded_patterns_spend_the_samples_budget() {
    let p = PsdPattern {
        mode: 3,
        width: 5,
        height: 3,
        name: "p".into(),
        id: "id".into(),
        palette: None,
        depth: 8,
        channels: vec![vec![1; 15], vec![2; 15], vec![3; 15]],
        alpha: None,
    };
    let abr = write_v6(2, &[sample("s", 10, 10, 8)], &[p.clone(), p], &[], true).unwrap();
    let parse_within = |left: usize| {
        let mut out = AbrFile::default();
        let mut budget = Budget(left);
        parse_v6_within(&mut Rd::new(&abr[2..]), &mut out, &mut budget).unwrap();
        (out, budget.0)
    };
    // 100 sample bytes plus two 45-byte patterns.
    let (out, left) = parse_within(190);
    assert_eq!((out.samples.len(), out.patterns.len(), left), (1, 2, 0));
    // One byte short: the patterns no longer fit after the sample (they used not to count).
    let (out, left) = parse_within(189);
    assert_eq!((out.samples.len(), out.patterns.len(), left), (1, 0, 14));
    assert!(out.warnings.iter().any(|w| w.contains("embedded patterns unreadable")), "{:?}", out.warnings);
}

#[test]
fn failed_pattern_section_does_not_refund_budget_to_later_sections() {
    let p = PsdPattern {
        mode: 3,
        width: 5,
        height: 3,
        name: "p".into(),
        id: "id".into(),
        palette: None,
        depth: 8,
        channels: vec![vec![1; 15], vec![2; 15], vec![3; 15]],
        alpha: None,
    };
    let valid = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let mut malformed = valid.clone();
    malformed.extend_from_slice(&100u32.to_be_bytes());

    let mut body = 2u16.to_be_bytes().to_vec();
    section(&mut body, b"patt", &malformed);
    section(&mut body, b"patt", &valid);
    let mut out = AbrFile::default();
    let mut budget = Budget(60);
    parse_v6_within(&mut Rd::new(&body), &mut out, &mut budget).unwrap();

    assert!(out.patterns.is_empty(), "a failed block is discarded atomically");
    assert_eq!(budget.0, 0, "the failed block's first 45 bytes and later partial 15-byte charge are retained");
    assert_eq!(out.warnings.iter().filter(|w| w.contains("embedded patterns unreadable")).count(), 2);
}

#[test]
fn v1_v2_round_trip_raw_and_rle() {
    for version in [1u16, 2] {
        for rle in [false, true] {
            let brushes = vec![
                LegacyBrush {
                    name: String::new(),
                    spacing: 25,
                    anti_alias: true,
                    tip: LegacyTip::Computed { diameter: 19, hardness: 80, angle: -30, roundness: 50 },
                },
                LegacyBrush {
                    name: if version == 2 { "Leaf ✓".into() } else { String::new() },
                    spacing: 40,
                    anti_alias: false,
                    tip: LegacyTip::Sampled(sample("", 13, 9, 8)),
                },
                LegacyBrush { name: String::new(), spacing: 10, anti_alias: true, tip: LegacyTip::Sampled(sample("", 4, 6, 16)) },
            ];
            let bytes = write_v12(version, &brushes, rle).unwrap();
            let f = parse(&bytes).unwrap();
            assert_eq!(f.version, version);
            assert_eq!(f.legacy, brushes, "v{version} rle={rle}");
            assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        }
    }
}

#[test]
fn v6_round_trip_both_subversions() {
    let samples = vec![sample("$abc", 20, 11, 8), sample("$def", 5, 5, 16)];
    let preset = Descriptor::new("brushPreset")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Grass")))
        .with("Brsh", Value::Descriptor(Descriptor::new("sampledBrush").with("sampledData", Value::Text(UnicodeString::new_nul("$abc")))));
    for sub in [1u16, 2] {
        for rle in [false, true] {
            let bytes = write_v6(sub, &samples, &[], std::slice::from_ref(&preset), rle).unwrap();
            let f = parse(&bytes).unwrap();
            assert_eq!((f.version, f.subversion), (6, sub));
            assert_eq!(f.samples, samples);
            assert_eq!(f.presets, vec![preset.clone()]);
            assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        }
    }
    assert_eq!(samples[1].to_unit().len(), 25);
}

#[test]
fn hostile_input_errors_without_panicking() {
    assert!(parse(&[]).is_err());
    assert!(parse(&[0, 3, 0, 0]).is_err());
    assert!(parse(&[0, 6, 0, 9]).is_err());
    // Absurd sample bounds.
    let mut s = sample("", 2, 2, 8);
    s.width = MAX_EDGE + 1;
    assert!(write_v12(1, &[LegacyBrush { name: String::new(), spacing: 1, anti_alias: true, tip: LegacyTip::Sampled(s) }], false).is_err());
    let good = write_v6(2, &[sample("$a", 30, 30, 8)], &[], &[Descriptor::new("brushPreset")], true).unwrap();
    for cut in 0..good.len() {
        let _ = parse(&good[..cut]);
    }
    let v2 =
        write_v12(2, &[LegacyBrush { name: "x".into(), spacing: 1, anti_alias: true, tip: LegacyTip::Sampled(sample("", 9, 9, 16)) }], true).unwrap();
    for cut in 0..v2.len() {
        let _ = parse(&v2[..cut]);
    }
    // Flip every byte once: never panics.
    for i in 0..good.len() {
        let mut b = good.clone();
        b[i] ^= 0xa5;
        let _ = parse(&b);
    }
    // A huge declared count with no data is an error, not an allocation.
    assert!(parse(&[0, 2, 0xff, 0xff]).is_err());
}

fn named(n: &str) -> Descriptor {
    Descriptor::new("brushPreset").with("Nm  ", Value::Text(UnicodeString::new_nul(n)))
}

/// Hierarchy tokens: `G(name)` opens, `P` is a preset, `E` closes.
enum Tok<'a> {
    G(&'a str),
    P,
    E,
}

/// A v6 file with `n` presets and a hand-made `phry` token stream.
fn with_phry(n: usize, toks: &[Tok]) -> Vec<u8> {
    let presets: Vec<Descriptor> = (0..n).map(|i| named(&format!("B{i}"))).collect();
    let mut bytes = write_v6(2, &[], &[], &presets, false).unwrap();
    section(&mut bytes, b"phry", &phry_bytes(toks));
    bytes
}

/// A `phry` section body holding `toks`.
fn phry_bytes(toks: &[Tok]) -> Vec<u8> {
    let items = toks
        .iter()
        .map(|t| {
            Value::Descriptor(match t {
                Tok::G(name) => Descriptor::new("Grup")
                    .with("Nm  ", Value::Text(UnicodeString::new_nul(name)))
                    .with("zuid", Value::Text(UnicodeString::new_nul("c9ee8fd5-eb7d-924c-9e95-ddc98e40bb6c"))),
                Tok::P => Descriptor::new("preset"),
                Tok::E => Descriptor::new("groupEnd"),
            })
        })
        .collect();
    VersionedDescriptor::new(Descriptor::new("null").with("hierarchy", Value::List(items))).to_bytes()
}

fn path(p: &[&str]) -> Vec<String> {
    p.iter().map(|s| s.to_string()).collect()
}

#[test]
fn phry_flat_folders_like_photoshop_25() {
    // The layout of Photoshop 25.1's default brushes: four folders, every preset in one.
    use Tok::*;
    let f = parse(&with_phry(5, &[G("General"), P, P, E, G("Dry Media"), P, E, G("Wet Media"), P, E, G("Special Effects"), P, E])).unwrap();
    assert_eq!(f.folders, vec![path(&["General"]), path(&["General"]), path(&["Dry Media"]), path(&["Wet Media"]), path(&["Special Effects"])]);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
}

#[test]
fn phry_nested_folders_and_top_level_presets() {
    use Tok::*;
    let f = parse(&with_phry(5, &[P, G("Animals"), G("Birds"), P, E, P, G("Fish"), P, E, E, P])).unwrap();
    assert_eq!(f.folders, vec![path(&[]), path(&["Animals", "Birds"]), path(&["Animals"]), path(&["Animals", "Fish"]), path(&[])]);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    // No folder at all: no hierarchy.
    let f = parse(&with_phry(2, &[P, P])).unwrap();
    assert!(f.folders.is_empty());
}

#[test]
fn phry_mismatches_become_warnings() {
    use Tok::*;
    // A stray groupEnd is ignored; an unclosed folder still holds its presets.
    let f = parse(&with_phry(2, &[E, G("A"), P, E, E, G("B"), P])).unwrap();
    assert_eq!(f.folders, vec![path(&["A"]), path(&["B"])]);
    assert!(f.warnings.iter().any(|w| w.contains("folder end")), "{:?}", f.warnings);
    // Fewer preset tokens than presets: the rest are at the top level.
    let f = parse(&with_phry(3, &[G("A"), P, E])).unwrap();
    assert_eq!(f.folders, vec![path(&["A"]), path(&[]), path(&[])]);
    assert!(f.warnings.iter().any(|w| w.contains("list 1 brushes but the file holds 3")), "{:?}", f.warnings);
    // More preset tokens than presets: extras ignored.
    let f = parse(&with_phry(1, &[G("A"), P, P, P, E])).unwrap();
    assert_eq!(f.folders, vec![path(&["A"])]);
    assert_eq!(f.presets.len(), 1);
    assert!(!f.warnings.is_empty());
    // A depth bomb: folders deeper than the limit merge into their ancestor at the limit.
    let mut toks: Vec<Tok> = (0..10_000).map(|_| G("deep")).collect();
    toks.push(P);
    toks.extend((0..10_000).map(|_| E));
    toks.extend([G("next"), P, E]);
    let f = parse(&with_phry(2, &toks)).unwrap();
    assert_eq!(f.folders[0].len(), MAX_FOLDER_DEPTH);
    assert_eq!(f.folders[1], path(&["next"]));
    assert!(f.warnings.iter().any(|w| w.contains("deeper")), "{:?}", f.warnings);
    // An unreadable hierarchy: imported flat, with a warning.
    let mut bytes = write_v6(2, &[], &[], &[named("x")], false).unwrap();
    section(&mut bytes, b"phry", &[0, 0, 0, 16, 0xff, 0xff]);
    let f = parse(&bytes).unwrap();
    assert!(f.folders.is_empty() && f.presets.len() == 1);
    assert!(f.warnings.iter().any(|w| w.contains("phry")), "{:?}", f.warnings);
    // Truncations and bit flips of a file with a hierarchy never panic.
    let good = with_phry(3, &[G("A"), P, G("B"), P, E, E, P]);
    for cut in 0..good.len() {
        let _ = parse(&good[..cut]);
    }
    for i in 0..good.len() {
        let mut b = good.clone();
        b[i] ^= 0x5a;
        let _ = parse(&b);
    }
}

#[test]
fn phry_builds_paths_only_for_the_files_presets() {
    use Tok::*;
    // A few-KB section listing 100,000 deep presets for a file with 2: two paths are built.
    let mut toks: Vec<Tok> = (0..MAX_FOLDER_DEPTH).map(|_| G("folder")).collect();
    toks.extend((0..100_000).map(|_| P));
    let mut warnings = Vec::new();
    let folders = read_phry(&phry_bytes(&toks), 2, &mut warnings).unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders.capacity(), 2);
    assert_eq!(folders[1].len(), MAX_FOLDER_DEPTH);
    assert!(warnings.iter().any(|w| w.contains("list 100000 brushes but the file holds 2")), "{warnings:?}");
    // `phry` before `desc` still pairs with the file's presets.
    let mut bytes = write_v6(2, &[], &[], &[named("a"), named("b")], false).unwrap();
    let at = bytes.windows(8).position(|w| w == b"8BIMdesc").unwrap();
    let mut phry = Vec::new();
    section(&mut phry, b"phry", &phry_bytes(&[G("A"), P, E, P]));
    bytes.splice(at..at, phry);
    let f = parse(&bytes).unwrap();
    assert_eq!(f.folders, vec![path(&["A"]), path(&[])]);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
}

#[test]
fn phry_folder_paths_stay_within_their_byte_budget() {
    use Tok::*;
    // Thousands of presets under 32 folders of 255-character names would need ~27 MB of paths.
    let long = "x".repeat(MAX_FOLDER_NAME);
    let mut toks: Vec<Tok> = (0..MAX_FOLDER_DEPTH).map(|_| G(&long)).collect();
    toks.extend((0..3_000).map(|_| P));
    let mut warnings = Vec::new();
    let folders = read_phry(&phry_bytes(&toks), 3_000, &mut warnings).unwrap();
    assert_eq!(folders.len(), 3_000);
    let held: usize = folders.iter().flatten().map(|n| n.len() + size_of::<String>()).sum();
    assert!(held <= MAX_FOLDER_BYTES, "{held}");
    assert_eq!(folders[0].len(), MAX_FOLDER_DEPTH);
    assert!(folders[2_999].is_empty(), "past the budget: top level");
    assert!(warnings.iter().any(|w| w.contains("too large")), "{warnings:?}");
}

#[test]
fn phry_round_trips_through_the_writer() {
    let presets: Vec<Descriptor> = (0..6).map(|i| named(&format!("B{i}"))).collect();
    let folders = vec![path(&[]), path(&["Set", "Ink"]), path(&["Set", "Ink"]), path(&["Set"]), path(&["Other"]), path(&["Set", "Ink"])];
    let bytes = write_v6_folders(2, &[], &[], &presets, &folders, true).unwrap();
    let f = parse(&bytes).unwrap();
    assert_eq!(f.presets, presets);
    assert_eq!(f.folders, folders);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    // No folders: no phry section, same bytes as write_v6.
    assert_eq!(write_v6_folders(2, &[], &[], &presets, &vec![Vec::new(); 6], true).unwrap(), write_v6(2, &[], &[], &presets, true).unwrap());
    // Mismatched lengths are an error.
    assert!(write_v6_folders(2, &[], &[], &presets, &folders[..2], true).is_err());
}

#[test]
fn sample_budget_covers_2_gib_packs_on_64_bit() {
    let want = if cfg!(target_pointer_width = "64") { 2 << 30 } else { 512 << 20 };
    assert_eq!(MAX_TOTAL_BYTES, want);
    // The pack of photocraft#5949 (1 077 579 672 sample bytes) fits.
    #[cfg(target_pointer_width = "64")]
    const {
        assert!(MAX_TOTAL_BYTES > 1_077_579_672)
    };
}
