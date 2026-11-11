//! Named-colour (`ncl2`) profiles, on synthetic profiles built here.

use craft_cms::{Builtin, CmsError, Intent, NamedColors, Pcs, Profile, Transform, math};

/// One entry: root name, three PCS codes, device codes.
type Entry<'a> = (&'a str, [u16; 3], &'a [u16]);

/// A 32-byte NUL-padded field.
fn field(s: &str) -> Vec<u8> {
    let mut f = s.as_bytes().to_vec();
    f.resize(32, 0);
    f
}

/// An `ncl2` tag holding `entries`, each with `coords` device coordinates.
fn ncl2(prefix: &str, suffix: &str, coords: u32, entries: &[Entry]) -> Vec<u8> {
    let mut t = b"ncl2".to_vec();
    t.extend([0; 8]);
    t.extend((entries.len() as u32).to_be_bytes());
    t.extend(coords.to_be_bytes());
    t.extend(field(prefix));
    t.extend(field(suffix));
    for (name, pcs, device) in entries {
        t.extend(field(name));
        pcs.iter().chain(device.iter()).for_each(|c| t.extend(c.to_be_bytes()));
    }
    t
}

/// A named-colour profile of version `major`.4 whose only tag is `tag`.
fn profile(major: u8, pcs: &[u8; 4], tag: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; 144];
    p[8] = major;
    p[9] = 0x40;
    p[12..16].copy_from_slice(b"nmcl");
    p[16..20].copy_from_slice(b"CMYK");
    p[20..24].copy_from_slice(pcs);
    p[36..40].copy_from_slice(b"acsp");
    p[128..132].copy_from_slice(&1u32.to_be_bytes());
    p[132..136].copy_from_slice(b"ncl2");
    p[136..140].copy_from_slice(&144u32.to_be_bytes());
    p[140..144].copy_from_slice(&(tag.len() as u32).to_be_bytes());
    p.extend(tag);
    let size = p.len() as u32;
    p[0..4].copy_from_slice(&size.to_be_bytes());
    p
}

fn named(bytes: &[u8]) -> NamedColors {
    Profile::parse(bytes).unwrap().named_colors().unwrap().unwrap()
}

#[test]
fn lab_uses_the_legacy_encoding_in_any_version() {
    // 0xFF00 is L* 100 in the legacy encoding; the v4 encoding would read it as 99.61.
    let tag = ncl2("", "", 0, &[("White", [0xFF00, 0x8000, 0x8000], &[]), ("Mid", [0x7F80, 0x9000, 0x7000], &[])]);
    for major in [2, 4] {
        let n = named(&profile(major, b"Lab ", &tag));
        assert_eq!(n.pcs, Pcs::Lab);
        assert_eq!(n.find("White").unwrap().pcs, [100.0, 0.0, 0.0], "v{major}");
        let mid = n.find("Mid").unwrap().pcs;
        assert_eq!(mid[0], 50.0);
        assert!((mid[1] - (0x9000 as f64 * 255.0 / 65280.0 - 128.0)).abs() < 1e-12);
    }
}

#[test]
fn names_join_prefix_root_and_suffix_and_device_values_scale_to_one() {
    let tag = ncl2("PRE ", " C", 4, &[("Orange", [0x8000, 0xA000, 0xC000], &[0, 0x8000, 0xFFFF, 0])]);
    let n = named(&profile(4, b"Lab ", &tag));
    assert_eq!(n.len(), 1);
    let c = n.find("PRE Orange C").unwrap();
    assert_eq!(c.device.len(), 4);
    assert_eq!((c.device[0], c.device[2]), (0.0, 1.0));
    assert!((c.device[1] - 32768.0 / 65535.0).abs() < 1e-12);
    assert!(n.find("pre orange c").is_none(), "lookup is case-sensitive");
}

#[test]
fn a_profile_without_ncl2_has_no_named_colours() {
    assert_eq!(Builtin::Srgb.profile().named_colors().unwrap(), None);
    let srgb = Profile::parse(&Builtin::Srgb.profile().to_bytes()).unwrap();
    assert_eq!(srgb.named_colors().unwrap(), None);
}

#[test]
fn hostile_tags_are_errors() {
    let entry: Entry = ("A", [0xFF00, 0x8000, 0x8000], &[]);
    let mut short = ncl2("", "", 0, &[entry, entry]);
    short.truncate(short.len() - 1);
    let mut huge = ncl2("", "", 0, &[entry]);
    huge[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
    let many_coords = ncl2("", "", 16, &[]);
    let mut wrong_type = ncl2("", "", 0, &[entry]);
    wrong_type[..4].copy_from_slice(b"text");
    for tag in [short, huge, many_coords, wrong_type] {
        assert!(NamedColors::parse(&tag, Pcs::Lab).is_err());
        let p = Profile::parse(&profile(4, b"Lab ", &tag)).unwrap();
        assert!(p.named_colors().is_err());
    }
    assert_eq!(NamedColors::parse(b"ncl2", Pcs::Lab), Err(CmsError::Truncated));
}

#[test]
fn a_named_colour_converts_into_a_cmyk_profile() {
    // A muted orange, well inside the coated gamut.
    let lab: [f64; 3] = [60.0, 12.0, 24.0];
    let code = [lab[0] * 65280.0 / 100.0, (lab[1] + 128.0) * 65280.0 / 255.0, (lab[2] + 128.0) * 65280.0 / 255.0].map(|v: f64| v.round() as u16);
    let tag = ncl2("", "", 0, &[("Spot", code, &[]), ("White", [0xFF00, 0x8000, 0x8000], &[])]);
    let n = named(&profile(4, b"Lab ", &tag));
    let cmyk = Builtin::CoatedFogra39.profile();
    let ink = n.to_device("Spot", cmyk).unwrap().unwrap();
    assert_eq!(ink.len(), 4);
    // Back through the profile, the ink lands on the stored colour.
    let back = Transform::new(cmyk, Builtin::LabD50.profile(), Intent::RelativeColorimetric, false).unwrap();
    let mut o = [0.0f32; 3];
    back.eval(&ink, &mut o).unwrap();
    let got = [o[0] as f64 * 100.0, o[1] as f64 * 255.0 - 128.0, o[2] as f64 * 255.0 - 128.0];
    assert!(math::delta_e2000(got, lab) < 1.0, "{got:?}");
    let white = n.to_device("White", cmyk).unwrap().unwrap();
    assert!(white.iter().all(|&v| v.abs() < 1e-3), "{white:?}");
    assert_eq!(n.to_device("Missing", cmyk).unwrap(), None);
    // A named-colour profile has no PCS → device path to convert into.
    let nmcl = Profile::parse(&profile(4, b"Lab ", &tag)).unwrap();
    assert!(matches!(n.to_device("Spot", &nmcl), Err(CmsError::Unsupported(_))));
}

#[test]
fn an_xyz_named_colour_converts_through_lab() {
    // D50 white in u1Fixed15.
    let white = math::D50.map(|v| (v * 32768.0).round() as u16);
    let n = named(&profile(4, b"XYZ ", &ncl2("", "", 0, &[("Paper", white, &[])])));
    assert_eq!(n.pcs, Pcs::Xyz);
    let rgb = n.to_device("Paper", Builtin::Srgb.profile()).unwrap().unwrap();
    assert!(rgb.iter().all(|&v| (v - 1.0).abs() < 2e-3), "{rgb:?}");
}
