use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cmap::Cmap;
use crate::embed::Env;
use crate::{Embed, Error};

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
    Run { result: result.map(|faces| faces.iter().map(|f| format!("{} {}", f.family, f.style)).collect()), cargo, table }
}

#[test]
fn serve_replaces_a_stale_copy_of_the_same_size() {
    let scratch = Scratch::new("serve-stale");
    let served = scratch.0.join("served");
    let source = std::fs::read(fixture().join("fonts/sans-jp/SansJP-Regular.ttf")).unwrap();
    std::fs::create_dir_all(served.join("sans-jp")).unwrap();
    std::fs::write(served.join("sans-jp/SansJP-Regular.ttf"), vec![0; source.len()]).unwrap();
    let r = run(Embed::new().scripts(&["Jpan"]).filter(|f| f.style == "Regular").serve(&served, "fonts/"), Some(&fixture()), Some("1"));
    r.result.unwrap();
    assert_eq!(std::fs::read(served.join("sans-jp/SansJP-Regular.ttf")).unwrap(), source);
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
fn served_faces_carry_the_characters_their_cmap_maps() {
    let font = tiny_font(&[(0x41, 0x5A, None), (0x4E00, 0x4E02, Some(&[5, 0, 7]))], &[(0x4E03, 0x4E03), (0x2_0000, 0x2_0001)]);
    let coverage = crate::coverage::of_face(&font, 0).unwrap();
    assert_eq!(Cmap::read(&font, 0).unwrap().ranges(), [(0x41, 0x5A), (0x4E00, 0x4E00), (0x4E02, 0x4E03), (0x2_0000, 0x2_0001)]);
    let covered: Vec<char> =
        "@AZ[一丁丂七丄\u{FFFF}\u{20000}\u{20001}\u{20002}".chars().filter(|&c| crate::coverage::contains(&coverage, c)).collect();
    assert_eq!(covered, ['A', 'Z', '一', '丂', '七', '\u{20000}', '\u{20001}']);
    assert!(!crate::coverage::contains(&[], 'A') && !crate::coverage::contains(&[0x80], 'A'));
    assert_eq!(crate::coverage::encode(&[(0, 0x10_FFFF)]), [0, 0xFF, 0xFF, 0x43]);

    let scratch = Scratch::new("serve-coverage");
    let dir = checkout(&scratch, &[("Noto Sans", "Latn", "", font), ("Broken", "Latn", "", b"not a font".to_vec())]);
    let r = run(Embed::new().serve(scratch.0.join("served"), "fonts/"), Some(&dir), None);
    assert_eq!(r.result.as_ref().unwrap().len(), 2);
    let table = &r.table;
    // Gaps and lengths 0x41 0x19, 0x4DA5 0, 1 1, 0x1_B1FC 1 as LEB128.
    assert!(table.contains(r#"coverage: b"A\x19\xa5\x9b\x01\x00\x01\x01\xfc\xe3\x06\x01" }"#), "{table}");
    assert!(table.contains(r#"family: "Broken""#) && table.ends_with("coverage: b\"\" },\n]\n"), "{table}");
    assert!(warnings(&r).iter().any(|w| w.contains("no character map in fonts/1.ttf")), "{:?}", warnings(&r));
}

#[test]
fn manifest_paths_stay_inside_the_checkout() {
    let scratch = Scratch::new("outside");
    let dir = scratch.0.join("craft-fonts");
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(dir.join("fonts/a.ttf"), b"a").unwrap();
    std::fs::write(dir.join("fonts/OFL.txt"), b"licence").unwrap();
    let secret = scratch.0.join("secret.txt");
    std::fs::write(&secret, b"host file").unwrap();
    let served = scratch.0.join("served");
    let line = |file: &str, licence: &str| format!("Sans | Regular | {file} | Latn | OFL-1.1 | {licence} | 00 | src\n");
    let outside = |r: &Run| matches!(r.result, Err(Error::OutsideCheckout { .. }));

    // Parent components and absolute paths, for the face and for its licence, embedded or served.
    for (file, licence) in [
        ("../secret.txt", "fonts/OFL.txt"),
        (secret.to_str().unwrap(), "fonts/OFL.txt"),
        ("fonts/../../secret.txt", "fonts/OFL.txt"),
        ("fonts/a.ttf", "../secret.txt"),
        ("fonts/a.ttf", secret.to_str().unwrap()),
    ] {
        std::fs::write(dir.join("fonts/manifest.txt"), line(file, licence)).unwrap();
        let r = run(Embed::new(), Some(&dir), None);
        assert!(r.result.is_ok() && !r.table.contains("include_bytes!"), "{file} {licence}: {:?} {}", r.result, r.table);
        assert!(warnings(&r).iter().any(|w| w.contains("not a file inside the checkout")), "{file} {licence}: {:?}", r.cargo);
        assert!(outside(&run(Embed::new(), Some(&dir), Some("1"))), "{file} {licence}");
        assert!(outside(&run(Embed::new().serve(&served, "fonts/"), Some(&dir), Some("1"))), "{file} {licence}");
    }
    assert!(!served.exists());

    // A symlink in the checkout that leads out of it.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, dir.join("fonts/link.ttf")).unwrap();
        std::os::unix::fs::symlink(&secret, dir.join("fonts/LINK.txt")).unwrap();
        std::fs::write(dir.join("fonts/manifest.txt"), line("fonts/link.ttf", "fonts/OFL.txt")).unwrap();
        assert!(outside(&run(Embed::new(), Some(&dir), Some("1"))));
        std::fs::write(dir.join("fonts/manifest.txt"), line("fonts/a.ttf", "fonts/LINK.txt")).unwrap();
        assert!(outside(&run(Embed::new(), Some(&dir), Some("1"))));
        assert!(outside(&run(Embed::new().serve(&served, "fonts/"), Some(&dir), Some("1"))));
        assert!(!served.join("LINK.txt").exists());

        // An embedded_elsewhere face behind such a symlink is not read for coverage.
        let text = scratch.0.join("text.ftl");
        std::fs::write(&text, "x = a\n").unwrap();
        let manifest =
            format!("{}Mono | Regular | fonts/link.ttf | Latn | OFL-1.1 | fonts/OFL.txt | 00 | src\n", line("fonts/a.ttf", "fonts/OFL.txt"));
        std::fs::write(dir.join("fonts/manifest.txt"), manifest).unwrap();
        let r = run(Embed::documents(&["Sans"]).embedded_elsewhere(&["Mono"]).check_text([&text]), Some(&dir), None);
        assert!(warnings(&r).iter().any(|w| w.contains("\"fonts/link.ttf\" is not a file inside the checkout")), "{:?}", r.cargo);

        // A symlink in the served directory doesn't redirect the copy.
        std::fs::write(dir.join("fonts/manifest.txt"), line("fonts/a.ttf", "fonts/OFL.txt")).unwrap();
        std::fs::create_dir_all(&served).unwrap();
        std::os::unix::fs::symlink(&secret, served.join("OFL.txt")).unwrap();
        let r = run(Embed::new().serve(&served, "fonts/"), Some(&dir), Some("1"));
        assert!(matches!(r.result, Err(Error::Write { .. })), "{:?}", r.result);
        assert_eq!(std::fs::read(&secret).unwrap(), b"host file");
        std::fs::remove_file(served.join("OFL.txt")).unwrap();
    }

    // A plain checkout path still embeds and serves.
    std::fs::write(dir.join("fonts/manifest.txt"), line("fonts/a.ttf", "fonts/OFL.txt")).unwrap();
    assert_eq!(run(Embed::new(), Some(&dir), Some("1")).result.unwrap(), ["Sans Regular"]);
    run(Embed::new().serve(&served, "fonts/"), Some(&dir), Some("1")).result.unwrap();
    assert_eq!(std::fs::read(served.join("OFL.txt")).unwrap(), b"licence");
}

/// A supplier's licence is published as plain text under its own stem, whatever extension it has:
/// an `.html` licence would be a page on the site's origin.
#[test]
fn a_served_licence_is_always_plain_text() {
    let scratch = Scratch::new("licence-name");
    let dir = scratch.0.join("craft-fonts");
    std::fs::create_dir_all(dir.join("fonts/a")).unwrap();
    std::fs::write(dir.join("fonts/a/a.ttf"), b"a").unwrap();
    let served = scratch.0.join("served");
    for (licence, published) in [("LICENSE.html", "LICENSE.txt"), ("COPYING", "COPYING.txt"), ("OFL.txt", "OFL.txt"), ("NOTICE.svg", "NOTICE.txt")] {
        std::fs::write(dir.join("fonts/a").join(licence), b"<script>alert(1)</script>").unwrap();
        let line = format!("Sans | Regular | fonts/a/a.ttf | Latn | OFL-1.1 | fonts/a/{licence} | 00 | src\n");
        std::fs::write(dir.join("fonts/manifest.txt"), line).unwrap();
        run(Embed::new().serve(&served, "fonts/"), Some(&dir), Some("1")).result.unwrap();
        assert_eq!(std::fs::read(served.join("a").join(published)).unwrap(), b"<script>alert(1)</script>", "{licence}");
        if published != licence {
            assert!(!served.join("a").join(licence).exists(), "{licence} is published under its own name");
        }
        std::fs::remove_dir_all(&served).unwrap();
    }
}

/// `cargo::rerun-if-changed` is one line: a path with a line break would print a second instruction.
#[cfg(unix)]
#[test]
fn a_path_with_a_line_break_is_refused_before_cargo_is_told_to_watch_it() {
    let scratch = Scratch::new("line-break");
    let served = scratch.0.join("served");
    let dir = scratch.0.join("craft-fonts");
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(dir.join("fonts/OFL.txt"), b"licence").unwrap();
    // A font whose name has a carriage return (the manifest is read by lines, so a line feed can't be in it).
    std::fs::write(dir.join("fonts/a\rcargo::warning=x.ttf"), b"a").unwrap();
    let line = "Sans | Regular | fonts/a\rcargo::warning=x.ttf | Latn | OFL-1.1 | fonts/OFL.txt | 00 | src\n";
    std::fs::write(dir.join("fonts/manifest.txt"), line).unwrap();
    for embed in [Embed::new(), Embed::new().serve(&served, "fonts/")] {
        let r = run(embed, Some(&dir), Some("1"));
        assert!(matches!(r.result, Err(Error::LineBreakInPath { .. })), "{:?}", r.result);
        assert!(r.cargo.iter().all(|line| !line.contains('\r') && !line.contains('\n')), "{:?}", r.cargo);
        assert!(!r.table.contains("include_bytes!"));
    }
    // The same through a checkout directory, and a catalog, whose names are the caller's.
    let odd = scratch.0.join("odd\rdir");
    std::fs::create_dir_all(odd.join("fonts")).unwrap();
    std::fs::write(odd.join("fonts/manifest.txt"), "# no faces\n").unwrap();
    let r = run(Embed::new(), Some(&odd), None);
    assert!(r.cargo.iter().all(|line| !line.starts_with("cargo::rerun-if-changed=") || !line.contains('\r')), "{:?}", r.cargo);
    let r = run(Embed::new().check_text([scratch.0.join("t\rext.ftl")]), Some(&dir), None);
    assert!(matches!(r.result, Err(Error::LineBreakInPath { .. })), "{:?}", r.result);
}

/// Reads of the manifest, fonts and licences are bounded and take regular files only. The tests use
/// small limits: nothing here allocates the hostile size.
#[test]
fn reads_are_bounded_and_refuse_what_is_not_a_regular_file() {
    use crate::embed::read_bounded;
    let scratch = Scratch::new("bounded");
    let file = scratch.0.join("f");
    std::fs::write(&file, b"0123456789").unwrap();
    assert_eq!(read_bounded(&file, 10).unwrap(), b"0123456789");
    assert!(read_bounded(&file, 9).unwrap_err().to_string().contains("larger than 9 bytes"));
    assert!(read_bounded(&scratch.0, 10).is_err(), "a directory");
    assert!(read_bounded(&scratch.0.join("missing"), 10).is_err());
    #[cfg(unix)]
    {
        // Opening a FIFO with no writer would block for ever.
        let fifo = scratch.0.join("fifo");
        if std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            assert!(read_bounded(&fifo, 10).unwrap_err().to_string().contains("not a regular file"));
        }
    }
}

/// A manifest past 1 MiB is an error (the real one is a few KiB), and a face that is a directory is
/// not a file inside the checkout.
#[test]
fn an_oversized_manifest_and_a_directory_face_are_errors() {
    let scratch = Scratch::new("manifest-cap");
    let dir = scratch.0.join("craft-fonts");
    std::fs::create_dir_all(dir.join("fonts/a.ttf")).unwrap();
    std::fs::write(dir.join("fonts/OFL.txt"), b"licence").unwrap();
    let valid = "Sans | Regular | fonts/a.ttf | Latn | OFL-1.1 | fonts/OFL.txt | 00 | src\n";
    std::fs::write(dir.join("fonts/manifest.txt"), valid).unwrap();
    let r = run(Embed::new(), Some(&dir), Some("1"));
    assert!(matches!(r.result, Err(Error::OutsideCheckout { .. })), "{:?}", r.result);

    let padding = format!("# {}\n", "x".repeat(1 << 20));
    std::fs::write(dir.join("fonts/manifest.txt"), format!("{padding}{valid}")).unwrap();
    let r = run(Embed::new(), Some(&dir), Some("1"));
    assert!(matches!(&r.result, Err(Error::Read { source, .. }) if source.to_string().contains("larger than")), "{:?}", r.result);
    // Not required: a warning and an empty table, as for any unreadable manifest.
    let r = run(Embed::new(), Some(&dir), None);
    assert!(r.result.is_ok() && warnings(&r).iter().any(|w| w.contains("larger than")), "{:?}", r.cargo);
}
