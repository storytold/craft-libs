use std::io::{Cursor, Write};

use super::tiles::{Space, TileFile, lzf_decompress};
use super::*;

const T: usize = 4; // small tiles keep the fixtures short

/// LZF with literal runs only (valid liblzf output).
fn lzf_literal(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for run in data.chunks(32) {
        out.push((run.len() - 1) as u8);
        out.extend_from_slice(run);
    }
    out
}

/// A tiled pixel file from (x, y, interleaved Krita pixels) tiles; odd tiles LZF, even raw.
fn tile_file(pixel_size: usize, tiles: &[(i32, i32, Vec<u8>)]) -> Vec<u8> {
    let mut out = format!("VERSION 2\nTILEWIDTH {T}\nTILEHEIGHT {T}\nPIXELSIZE {pixel_size}\nDATA {}\n", tiles.len()).into_bytes();
    for (i, (x, y, px)) in tiles.iter().enumerate() {
        let n = T * T;
        let mut planar = vec![0; n * pixel_size];
        for p in 0..n {
            for k in 0..pixel_size {
                planar[k * n + p] = px[p * pixel_size + k];
            }
        }
        // Krita splits only LZF tiles into byte planes; raw tiles stay interleaved.
        let body: Vec<u8> = if i % 2 == 1 { [vec![1], lzf_literal(&planar)].concat() } else { [&[0][..], px].concat() };
        out.extend(format!("{x},{y},LZF,{}\n", body.len()).into_bytes());
        out.extend(body);
    }
    out
}

fn zip_of(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (i, (name, data)) in files.iter().enumerate() {
        z.start_file(*name, if deflate && i > 0 { deflated } else { stored }).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn archive(maindoc: &str, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut all: Vec<(&str, &[u8])> = vec![("mimetype", MIMETYPE), ("maindoc.xml", maindoc.as_bytes())];
    all.extend(files.iter().map(|(n, d)| (*n, d.as_slice())));
    zip_of(&all, false)
}

fn maindoc(cs: &str, layers: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE DOC PUBLIC '-//KDE//DTD krita 2.0//EN' 'http://www.calligra.org/DTD/krita-2.0.dtd'>\n\
         <DOC xmlns=\"http://www.calligra.org/DTD/krita\" syntaxVersion=\"2.0\"><IMAGE name=\"img\" width=\"12\" height=\"8\" colorspacename=\"{cs}\" x-res=\"300\" y-res=\"300\"><layers>{layers}</layers></IMAGE></DOC>"
    )
}

/// One tile of BGRA8 pixels: pixel p = (b, g, r, a) from `f(p)`.
fn bgra(f: impl Fn(usize) -> [u8; 4]) -> Vec<u8> {
    (0..T * T).flat_map(f).collect()
}

/// Every tile of a pixel file as (x, y, samples in channel order).
fn decode(bytes: &[u8], space: Space) -> Vec<(i32, i32, Vec<u8>)> {
    let file = TileFile::parse(bytes).unwrap();
    assert_eq!(file.pixel_size, space.pixel_size());
    let (mut raw, mut out) = (Vec::new(), Vec::new());
    file.tiles()
        .iter()
        .map(|t| {
            file.decode(t, &mut raw).unwrap();
            assert_eq!(raw.len(), file.tile_bytes());
            space.samples(&raw, &mut out);
            (t.x, t.y, out.clone())
        })
        .collect()
}

#[test]
fn reads_layers_groups_masks_and_properties() {
    let doc = maindoc(
        "RGBA",
        r#"<layer nodetype="grouplayer" name="Through" passthrough="1" compositeop="multiply" visible="1" opacity="255" filename="layer1" x="2" y="1" collapsed="1">
             <layers><layer nodetype="paintlayer" name="Inner" colorspacename="RGBA" filename="layer2" compositeop="screen" opacity="128" x="1" y="0" locked="1"/></layers>
           </layer>
           <layer nodetype="paintlayer" name="Masked" colorspacename="RGBA" filename="layer3" compositeop="diff" visible="0" x="0" y="0" channelflags="1101" inheritalpha="1" keyframes="layer3.keyframes.xml">
             <masks><mask nodetype="transparencymask" name="m" filename="mask4" x="-1" y="3"/><mask nodetype="selectionmask" filename="sel" x="junk"/></masks>
           </layer>
           <layer nodetype="shapelayer" name="Vector" filename="layer5" opacity="junk"/>
           <layer nodetype="paintlayer" name="Lab layer" colorspacename="LABA" filename="layer6"/>"#,
    );
    let red = bgra(|p| [0, 0, 255, (p * 16) as u8]);
    let files = [
        ("img/layers/layer2", tile_file(4, &[(0, 0, red.clone()), (4, 0, bgra(|_| [10, 20, 30, 255]))])),
        ("img/layers/layer2.defaultpixel", vec![0; 4]),
        ("img/layers/layer3", tile_file(4, &[(-4, 4, bgra(|_| [1, 2, 3, 200]))])),
        ("img/layers/mask4.pixelselection", tile_file(1, &[(-4, 4, (0..T * T).map(|p| (p * 10) as u8).collect())])),
        ("img/layers/mask4.pixelselection.defaultpixel", vec![0]),
        ("img/annotations/icc", b"not really a profile".to_vec()),
    ];
    let bytes = archive(&doc, &files);
    let mut a = Archive::open(&bytes).unwrap();
    let image = a.image().clone();
    assert_eq!((image.name.as_str(), image.width, image.height, image.colorspace_name.as_str()), ("img", 12, 8, "RGBA"));
    assert_eq!((image.x_res, image.y_res), (300.0, 300.0));
    assert!(!image.assistants && !image.reference_images);
    assert_eq!(a.icc_profile().unwrap().as_deref(), Some(&b"not really a profile"[..]));
    assert_eq!(a.merged_image().unwrap(), None);
    // Topmost first, every node kind kept as the file names it.
    let kinds: Vec<(&str, &str)> = image.layers.iter().map(|l| (l.node_type.as_str(), l.name.as_deref().unwrap_or(""))).collect();
    assert_eq!(kinds, [("grouplayer", "Through"), ("paintlayer", "Masked"), ("shapelayer", "Vector"), ("paintlayer", "Lab layer")]);
    let through = &image.layers[0];
    assert!(through.passthrough && through.collapsed && through.visible);
    assert_eq!((through.composite_op.as_deref(), through.x, through.y), (Some("multiply"), 2, 1));
    let inner = &through.layers[0];
    assert_eq!((inner.composite_op.as_deref(), inner.locked, inner.opacity, inner.x, inner.y), (Some("screen"), true, 128.0, 1, 0));
    assert_eq!(inner.colorspace_name.as_deref().and_then(Space::from_name), Some(Space::Rgba8));
    let tiles = decode(&a.layer_file(inner.filename.as_deref().unwrap()).unwrap().unwrap(), Space::Rgba8);
    assert_eq!((tiles[0].0, tiles[0].1, tiles[1].0, tiles[1].1), (0, 0, 4, 0));
    // Krita's B, G, R, A becomes R, G, B, A; the raw tile and the LZF tile agree on the layout.
    assert_eq!(tiles[0].2[..8], [255, 0, 0, 0, 255, 0, 0, 16]);
    assert_eq!(tiles[1].2[..4], [30, 20, 10, 255]);
    assert_eq!(a.default_pixel("layer2", Space::Rgba8).unwrap(), Some([0.0; 4]));
    assert_eq!(a.default_pixel("layer3", Space::Rgba8).unwrap(), None);
    let masked = &image.layers[1];
    assert!(!masked.visible && masked.inherit_alpha);
    assert_eq!((masked.channel_flags.as_deref(), masked.keyframes.as_deref()), (Some("1101"), Some("layer3.keyframes.xml")));
    let kinds: Vec<&str> = masked.masks.iter().map(|m| m.node_type.as_str()).collect();
    assert_eq!(kinds, ["transparencymask", "selectionmask"]);
    let mask = &masked.masks[0];
    assert_eq!((mask.x, mask.y, mask.visible), (-1, 3, true));
    let tiles = decode(&a.layer_file(&format!("{}.pixelselection", mask.filename.as_deref().unwrap())).unwrap().unwrap(), Space::Alpha8);
    assert_eq!((tiles[0].0, tiles[0].1, &tiles[0].2[..2]), (-4, 4, &[0, 10][..]));
    assert_eq!(a.layer_file("layer5").unwrap(), None, "a node may name a file the archive lacks");
    // A node whose pixels are not read does not fail the document over a number.
    assert_eq!((image.layers[2].opacity, masked.masks[1].x), (255.0, 0));
    assert_eq!(image.layers[3].colorspace_name.as_deref().and_then(Space::from_name), None);
}

#[test]
fn sixteen_bit_and_gray_spaces_convert_exactly() {
    let px16: Vec<u8> =
        (0..T * T).flat_map(|p| [1000u16 + p as u16, 2000, 3000, 65535].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()).collect();
    let words = |b: &[u8]| -> Vec<u16> { b.as_chunks::<2>().0.iter().map(|b| u16::from_ne_bytes(*b)).collect() };
    let tiles = decode(&tile_file(8, &[(0, 0, px16.clone())]), Space::Rgba16);
    // Krita's B, G, R, A becomes R, G, B, A.
    assert_eq!(words(&tiles[0].2[8..16]), [3000, 2000, 1001, 65535]);
    assert_eq!(Space::Rgba16.rgba(&px16[8..16]), [3000.0 / 65535.0, 2000.0 / 65535.0, 1001.0 / 65535.0, 1.0]);
    let gray: Vec<u8> = (0..T * T).flat_map(|p| [p as u8 * 10, 255]).collect();
    assert_eq!(decode(&tile_file(2, &[(0, 0, gray.clone())]), Space::Gray8)[0].2, gray);
    assert_eq!(Space::Gray8.rgba(&gray[4..6]), [20.0 / 255.0, 20.0 / 255.0, 20.0 / 255.0, 1.0]);
    let g16: Vec<u8> = (0..T * T).flat_map(|p| [(p as u16 * 4000).to_le_bytes(), 40000u16.to_le_bytes()].concat()).collect();
    assert_eq!(words(&decode(&tile_file(4, &[(0, 0, g16)]), Space::Gray16)[0].2[12..16]), [12000, 40000]);
    assert_eq!(Space::Alpha8.rgba(&[51]), [0.2, 0.2, 0.2, 1.0]);
    for (space, size, channels, sample) in
        [(Space::Rgba8, 4, 4, 1), (Space::Rgba16, 8, 4, 2), (Space::Gray8, 2, 2, 1), (Space::Gray16, 4, 2, 2), (Space::Alpha8, 1, 1, 1)]
    {
        assert_eq!((space.pixel_size(), space.channels(), space.bytes_per_sample()), (size, channels, sample));
    }
}

#[test]
fn a_default_pixel_is_read_in_its_space() {
    let doc = maindoc("RGBA", r#"<layer nodetype="paintlayer" name="L" colorspacename="RGBA" filename="l"/>"#);
    let bytes = archive(&doc, &[("img/layers/l", tile_file(4, &[])), ("img/layers/l.defaultpixel", vec![255, 0, 0, 255])]);
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(a.default_pixel("l", Space::Rgba8).unwrap(), Some([0.0, 0.0, 1.0, 1.0]));
    assert!(a.default_pixel("l", Space::Rgba16).is_err(), "four bytes are not an RGBA16 pixel");
    assert!(TileFile::parse(&a.layer_file("l").unwrap().unwrap()).unwrap().tiles().is_empty());
}

#[test]
fn a_default_pixel_is_not_inflated_past_one_pixel() {
    let doc = maindoc("RGBA", "");
    let big = vec![0u8; 1 << 20];
    let bytes = zip_of(&[("mimetype", MIMETYPE), ("maindoc.xml", doc.as_bytes()), ("img/layers/l.defaultpixel", &big)], true);
    let e = Archive::open(&bytes).unwrap().default_pixel("l", Space::Rgba8).unwrap_err();
    assert_eq!(e, Error::Invalid("img/layers/l.defaultpixel: larger than 4 bytes".into()));
}

#[test]
fn raw_tiles_hold_interleaved_pixels() {
    // Krita writes a tile uncompressed (flag 0) when LZF can't shrink it, and then the pixels
    // stay interleaved: only LZF tiles are split into byte planes.
    let mut body = vec![0u8];
    body.extend((0..T * T).flat_map(|_| [10u8, 20, 30, 255]));
    let mut file = format!("VERSION 2\nTILEWIDTH {T}\nTILEHEIGHT {T}\nPIXELSIZE 4\nDATA 1\n0,0,LZF,{}\n", body.len()).into_bytes();
    file.extend(body);
    let tiles = decode(&file, Space::Rgba8);
    assert!(tiles[0].2.as_chunks::<4>().0.iter().all(|p| *p == [30, 20, 10, 255]));
}

#[test]
fn deflated_entries_and_the_merged_image_are_read() {
    let doc = maindoc("CMYKA", "<assistants/>").replace("</IMAGE>", "<ReferenceImages/></IMAGE>");
    let bytes = zip_of(&[("mimetype", MIMETYPE), ("maindoc.xml", doc.as_bytes()), ("mergedimage.png", &[7; 5000])], true);
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(Space::from_name(&a.image().colorspace_name), None);
    assert!(a.image().reference_images);
    assert_eq!(a.merged_image().unwrap(), Some(vec![7; 5000]));
    assert!(a.entry("mergedimage.png", 4999).is_err(), "an entry past the caller's limit is refused");
    assert_eq!(a.icc_profile().unwrap(), None);
}

/// maindoc.xml nested 20 000 deep, directly or through an internal-subset entity, is an error.
/// The small stack makes a regression abort instead of passing.
#[test]
fn deeply_nested_maindoc_is_an_error_not_a_stack_overflow() {
    let (open, close) = ("<layers>".repeat(20_000), "</layers>".repeat(20_000));
    let direct = maindoc("RGBA", &format!("{open}{close}"));
    let entity = format!("<!DOCTYPE DOC [<!ENTITY e \"{open}{close}\">]><DOC><IMAGE width=\"4\" height=\"4\">&e;</IMAGE></DOC>");
    // A quoted `?>` in the XML declaration must not hide the DOCTYPE behind it.
    let declared = format!("<?xml version=\"?>\"?>{entity}");
    for (xml, why) in [(direct, "deeper"), (entity, "document type"), (declared, "")] {
        let bytes = archive(&xml, &[]);
        let e = std::thread::Builder::new().stack_size(512 * 1024).spawn(move || Archive::open(&bytes).err()).unwrap().join().unwrap();
        assert!(matches!(&e, Some(Error::Invalid(m)) if m.contains(why)), "{why}: {e:?}");
    }
    // Just inside the nesting limit: groups 250 deep are read and dropped without recursion trouble.
    let deep = maindoc("RGBA", &format!("{}{}", r#"<layer nodetype="grouplayer"><layers>"#.repeat(250), "</layers></layer>".repeat(250)));
    let bytes = archive(&deep, &[]);
    let depth = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let a = Archive::open(&bytes).unwrap();
            std::iter::successors(a.image().layers.first(), |n| n.layers.first()).count()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(depth, 250);
    let deeper = maindoc("RGBA", &format!("{}{}", r#"<layer nodetype="grouplayer"><layers>"#.repeat(260), "</layers></layer>".repeat(260)));
    assert!(Archive::open(&archive(&deeper, &[])).is_err(), "past MAX_XML_DEPTH");
}

#[test]
fn lzf_decodes_literals_and_overlapping_references() {
    // "abc" then a 6-byte back-reference 3 back: abcabcabc.
    assert_eq!(lzf_decompress(&[2, b'a', b'b', b'c', 0x80, 2], 9).unwrap(), b"abcabcabc");
    // A long back-reference (len 7 + extra) over a single byte: run-length.
    assert_eq!(lzf_decompress(&[0, b'x', 0xe0, 3, 0], 13).unwrap(), vec![b'x'; 13]);
    for bad in [&[0x20, 5][..], &[5, 1, 2], &[0xe0], &[2, 1, 2, 3, 0x20]] {
        assert!(lzf_decompress(bad, 64).is_err(), "{bad:?}");
    }
    assert!(lzf_decompress(&[2, 1, 2, 3], 2).is_err(), "overrun");
    assert!(lzf_decompress(&[2, 1, 2, 3], 4).is_err(), "short");
}

#[test]
fn malformed_archives_fail_cleanly() {
    let layer = |attrs: &str| format!(r#"<layer nodetype="paintlayer" name="L" colorspacename="RGBA" {attrs}/>"#);
    let many = layer("").repeat(MAX_NODES + 1);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not a zip", b"nope".to_vec()),
        ("no maindoc", zip_of(&[("mimetype", MIMETYPE)], false)),
        ("no mimetype", zip_of(&[("maindoc.xml", maindoc("RGBA", "").as_bytes())], false)),
        ("other mimetype", zip_of(&[("mimetype", b"image/openraster"), ("maindoc.xml", maindoc("RGBA", "").as_bytes())], false)),
        ("bad xml", archive("<DOC><IMAGE", &[])),
        ("not utf-8", archive("<DOC>\u{0}</DOC>", &[]).iter().map(|b| if *b == 0 { 0xff } else { *b }).collect()),
        ("no image", archive("<DOC/>", &[])),
        ("zero size", archive(&maindoc("RGBA", "").replace("width=\"12\"", "width=\"0\""), &[])),
        ("fractional size", archive(&maindoc("RGBA", "").replace("width=\"12\"", "width=\"1.5\""), &[])),
        ("too wide", archive(&maindoc("RGBA", "").replace("width=\"12\"", "width=\"300001\""), &[])),
        ("huge", archive(&maindoc("RGBA", "").replace("width=\"12\"", "width=\"300000\"").replace("height=\"8\"", "height=\"300000\""), &[])),
        ("bad opacity", archive(&maindoc("RGBA", &layer(r#"opacity="x""#)), &[])),
        ("bad offset", archive(&maindoc("RGBA", &layer(r#"x="1300000""#)), &[])),
        ("bad resolution", archive(&maindoc("RGBA", "").replace("x-res=\"300\"", "x-res=\"inf\""), &[])),
        ("too many layers", archive(&maindoc("RGBA", &many), &[])),
    ];
    for (what, bytes) in cases {
        assert!(matches!(Archive::open(&bytes), Err(Error::Invalid(_))), "{what}");
    }
    assert!(Archive::open(&archive(&maindoc("RGBA", &layer("").repeat(MAX_NODES)), &[])).is_ok());
    assert!(Archive::open(&archive(&maindoc("RGBA", &layer(r#"x="1200000""#)), &[])).is_ok(), "four canvas sides away is allowed");
}

#[test]
fn malformed_tile_files_fail_cleanly() {
    let head = format!("VERSION 2\nTILEWIDTH {T}\nTILEHEIGHT {T}\nPIXELSIZE 4\n");
    let parse: Vec<(&str, Vec<u8>)> = vec![
        ("no header", b"garbage".to_vec()),
        ("big tiles", b"VERSION 2\nTILEWIDTH 1025\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA 0\n".to_vec()),
        ("zero tiles", b"VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 0\nPIXELSIZE 4\nDATA 0\n".to_vec()),
        ("big pixels", b"VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 65\nDATA 0\n".to_vec()),
        ("long line", format!("VERSION 2{}\n", " ".repeat(300)).into_bytes()),
        ("too many tiles", format!("{head}DATA 999999\n").into_bytes()),
        ("bad tile line", format!("{head}DATA 1\n0,0,ZIP,4\n").into_bytes()),
        ("extra field", format!("{head}DATA 1\n0,0,LZF,1,9\n\x01").into_bytes()),
        ("truncated tile", format!("{head}DATA 1\n0,0,LZF,400\n\0").into_bytes()),
        ("far tile", format!("{head}DATA 1\n2000000000,0,LZF,1\n\x01").into_bytes()),
        ("missing tile", format!("{head}DATA 2\n0,0,LZF,1\n\x01").into_bytes()),
    ];
    for (what, bytes) in parse {
        assert!(matches!(TileFile::parse(&bytes), Err(Error::Invalid(_))), "{what}");
    }
    assert!(matches!(TileFile::parse(b"VERSION 1\n"), Err(Error::Unsupported(_))));
    assert!(TileFile::parse(b"VERSION 2\nTILEWIDTH 1024\nTILEHEIGHT 1024\nPIXELSIZE 64\nDATA 0\n").is_ok());
    assert!(TileFile::parse(format!("{head}DATA 1\n1200000,-1200000,LZF,1\n\x01").as_bytes()).is_ok());
    let decode: Vec<(&str, Vec<u8>)> = vec![
        ("raw size", format!("{head}DATA 1\n0,0,LZF,3\n\0ab").into_bytes()),
        ("flag", format!("{head}DATA 1\n0,0,LZF,1\n\x07").into_bytes()),
        ("empty tile", format!("{head}DATA 1\n0,0,LZF,0\n").into_bytes()),
        ("short lzf", format!("{head}DATA 1\n0,0,LZF,3\n\x01\x00a").into_bytes()),
    ];
    for (what, bytes) in decode {
        let file = TileFile::parse(&bytes).unwrap();
        assert!(file.decode(&file.tiles()[0], &mut Vec::new()).is_err(), "{what}");
    }
}

#[test]
fn truncated_and_corrupted_files_never_panic() {
    let doc = maindoc(
        "RGBA",
        r#"<layer nodetype="paintlayer" name="L" colorspacename="RGBA" filename="l"><masks><mask nodetype="transparencymask" filename="m"/></masks></layer>"#,
    );
    let bytes = archive(
        &doc,
        &[
            ("img/layers/l", tile_file(4, &[(0, 0, bgra(|p| [p as u8; 4])), (4, 4, bgra(|_| [9; 4]))])),
            ("img/layers/m.pixelselection", tile_file(1, &[(0, 0, vec![7; T * T])])),
            ("mergedimage.png", vec![1, 2, 3]),
        ],
    );
    // Everything an application would read from the archive.
    let read_all = |bytes: &[u8]| -> Result<usize> {
        let mut a = Archive::open(bytes)?;
        let (mut raw, mut seen) = (Vec::new(), 0);
        for name in ["l", "m.pixelselection"] {
            if let Some(data) = a.layer_file(name)? {
                let file = TileFile::parse(&data)?;
                for t in file.tiles() {
                    file.decode(t, &mut raw)?;
                    seen += 1;
                }
            }
            a.default_pixel(name, Space::Rgba8)?;
        }
        a.icc_profile()?;
        a.merged_image()?;
        Ok(seen)
    };
    assert_eq!(read_all(&bytes), Ok(3));
    for cut in (0..bytes.len()).step_by((bytes.len() / 300).max(1)) {
        let _ = read_all(bytes.get(..cut).unwrap_or_default());
    }
    let mut state = 0x1234_5678_u32;
    for _ in 0..400 {
        let mut b = bytes.clone();
        for _ in 0..3 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let i = state as usize % b.len();
            b[i] ^= (state >> 24) as u8 | 1;
        }
        let _ = read_all(&b);
    }
}

#[test]
fn the_mimetype_is_read_from_a_leading_stored_entry_only() {
    let bytes = archive("<DOC/>", &[]);
    assert_eq!(zip_mimetype(&bytes), Some(MIMETYPE));
    assert!(is_kra(&bytes));
    assert!(is_kra(&zip_of(&[("mimetype", b"application/x-krita\n")], false)), "a trailing newline is tolerated");
    assert!(!is_kra(&zip_of(&[("maindoc.xml", b"<DOC/>"), ("mimetype", MIMETYPE)], false)), "not the first entry");
    assert!(!is_kra(&zip_of(&[("mimetype", b"image/openraster")], false)));
    assert!(!is_kra(&zip_of(&[("mimetype", &[b'a'; 257])], false)), "longer than a mimetype");
    assert!(!is_kra(b"\x89PNG\r\n\x1a\n") && !is_kra(&bytes[..20]));
    assert_eq!(Space::from_name("RGBA"), Some(Space::Rgba8));
    assert_eq!(Space::from_name("RGBAF32"), None);
}

/// A ZIP64 end record can declare millions of entries in a small file: refused on the raw bytes
/// before the archive is indexed.
#[test]
fn an_archive_declaring_too_many_entries_is_refused_before_it_is_indexed() {
    let bomb = craft_zip::zip64_entry_count_sample(MAX_ENTRIES + 1);
    assert_eq!(
        Archive::open(&bomb).err(),
        Some(Error::Invalid(format!("the archive declares {} entries, more than {MAX_ENTRIES}", MAX_ENTRIES + 1)))
    );
}

/// Two entries of one name: `zip` keeps one, and which one the writer meant is not known.
#[test]
fn two_entries_of_one_name_are_refused() {
    let bytes = archive(&maindoc("RGBA", ""), &[("dupA", vec![1]), ("dupB", vec![2])]);
    assert!(Archive::open(&bytes).is_ok());
    let mut renamed = bytes.clone();
    let mut i = 0;
    while let Some(at) = renamed[i..].windows(4).position(|w| w == b"dupB") {
        renamed[i + at + 3] = b'A';
        i += at + 4;
    }
    assert_eq!(Archive::open(&renamed).err(), Some(Error::Invalid("two entries of the archive share a name".into())));
}
