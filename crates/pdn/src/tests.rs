use super::*;
use std::io::Write;

enum Field {
    Int(i32),
    Long(i64),
    Byte(u8),
    Bool(bool),
    Ref(i32),
}

struct Writer(Vec<u8>);

impl Writer {
    fn int(&mut self, n: i32) {
        self.0.extend(n.to_le_bytes());
    }
    fn string(&mut self, s: &str) {
        let mut len = s.len();
        while len >= 128 {
            self.0.push((len as u8 & 127) | 128);
            len >>= 7;
        }
        self.0.push(len as u8);
        self.0.extend(s.as_bytes());
    }
    fn object(&mut self, id: i32, class: &str, fields: &[(&str, Field)]) {
        self.0.push(5);
        self.int(id);
        self.string(class);
        self.int(fields.len() as i32);
        for (name, _) in fields {
            self.string(name);
        }
        for (_, f) in fields {
            self.0.push(if matches!(f, Field::Ref(_)) { 2 } else { 0 });
        }
        for (_, f) in fields {
            match f {
                Field::Int(_) => self.0.push(8),
                Field::Long(_) => self.0.push(9),
                Field::Byte(_) => self.0.push(2),
                Field::Bool(_) => self.0.push(1),
                Field::Ref(_) => {}
            }
        }
        self.int(1);
        for (_, f) in fields {
            match f {
                Field::Int(n) => self.int(*n),
                Field::Long(n) => self.0.extend(n.to_le_bytes()),
                Field::Byte(n) => self.0.push(*n),
                Field::Bool(b) => self.0.push(u8::from(*b)),
                Field::Ref(id) => {
                    self.0.push(9);
                    self.int(*id);
                }
            }
        }
    }
}

const GO: &dyn Fn() -> bool = &|| true;

/// Parses and decodes everything, as an application would.
fn read_all(bytes: &[u8]) -> Result<(Vec<Layer>, Vec<Surface>)> {
    let doc = Document::parse(bytes, GO)?;
    let mut surfaces = Vec::new();
    doc.read_surfaces(GO, &mut |i, s| {
        assert_eq!(i, surfaces.len());
        surfaces.push(s);
    })?;
    assert_eq!(surfaces.iter().map(|s| s.data.len()).sum::<usize>(), doc.decoded_bytes());
    Ok((doc.layers, surfaces))
}

/// Independent synthetic PDN writer: padded rows, forward references, a root cycle,
/// spare ArrayList capacity, and reversed gzip/raw chunks. There is no production PDN writer.
fn fixture(modes: &[i32], legacy: bool, compressed: bool) -> Vec<u8> {
    use Field::*;
    let mut w = Writer(b"PDN3\0\0\0\0\x01".to_vec());
    w.0.push(0);
    for n in [1, -1, 1, 0] {
        w.int(n);
    }
    w.0.push(12);
    w.int(1);
    w.string("PaintDotNet.Data");
    w.object(1, "PaintDotNet.Document", &[("width", Int(2)), ("height", Int(2)), ("layers", Ref(2))]);
    w.object(2, "PaintDotNet.LayerList", &[("parent", Ref(1)), ("ArrayList+_size", Int(modes.len() as i32)), ("ArrayList+_items", Ref(3))]);
    w.0.push(16);
    w.int(3);
    w.int(modes.len() as i32 + 2);
    for i in 0..modes.len() {
        w.0.push(9);
        w.int(16 + i as i32 * 10);
    }
    w.0.extend([13, 2]);
    for (i, mode) in modes.iter().enumerate() {
        let id = 16 + i as i32 * 10;
        w.object(
            id,
            "PaintDotNet.BitmapLayer",
            &[
                ("Layer+width", Int(2)),
                ("Layer+height", Int(2)),
                ("surface", Ref(id + 1)),
                ("Layer+properties", Ref(id + 2)),
                ("properties", Ref(id + 3)),
            ],
        );
        w.object(id + 1, "PaintDotNet.Surface", &[("width", Int(2)), ("height", Int(2)), ("stride", Int(12)), ("scan0", Ref(id + 7))]);
        let mut fields =
            vec![("name", Ref(id + 4)), ("opacity", Byte(if i == 0 { 255 } else { 128 })), ("visible", Bool(i != 1)), ("isBackground", Bool(i == 0))];
        if !legacy {
            fields.push(("blendMode", Ref(id + 5)));
        }
        w.object(id + 2, "PaintDotNet.Layer+LayerProperties", &fields);
        w.object(id + 3, "PaintDotNet.BitmapLayer+BitmapLayerProperties", &[("blendOp", Ref(id + 6))]);
        w.0.push(6);
        w.int(id + 4);
        w.string(&format!("Läyer {i}"));
        w.object(id + 5, "PaintDotNet.LayerBlendMode", &[("value__", Int(*mode))]);
        w.object(id + 6, &format!("PaintDotNet.UserBlendOps+{}BlendOp", BlendMode::ALL.get(*mode as usize).map_or("Unknown", |m| m.name())), &[]);
        w.object(id + 7, "PaintDotNet.MemoryBlock", &[("length64", Long(24)), ("hasParent", Bool(false)), ("deferred", Bool(true))]);
    }
    w.0.push(11);
    for _ in modes {
        w.0.push(if compressed { 0 } else { 1 });
        w.0.extend(12u32.to_be_bytes());
        for index in [1u32, 0] {
            let raw = if index == 0 {
                vec![30, 20, 10, 255, 60, 50, 40, 128, 99, 99, 99, 99]
            } else {
                vec![90, 80, 70, 64, 120, 110, 100, 0, 99, 99, 99, 99]
            };
            let data = if compressed {
                let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(&raw).unwrap();
                enc.finish().unwrap()
            } else {
                raw
            };
            w.0.extend(index.to_be_bytes());
            w.0.extend((data.len() as u32).to_be_bytes());
            w.0.extend(data);
        }
    }
    w.0
}

#[test]
fn all_blend_modes_and_layer_properties_are_read() {
    let modes: Vec<_> = (0..14).collect();
    for legacy in [false, true] {
        for compressed in [false, true] {
            let bytes = fixture(&modes, legacy, compressed);
            assert!(is_pdn(&bytes));
            let doc = Document::parse(&bytes, GO).unwrap();
            assert_eq!((doc.width, doc.height, doc.layers.len(), doc.surface_count(), doc.has_user_metadata), (2, 2, 14, 14, false));
            assert_eq!(doc.decoded_bytes(), 14 * 24);
            let (layers, surfaces) = read_all(&bytes).unwrap();
            for (i, layer) in layers.iter().enumerate() {
                assert_eq!(
                    layer,
                    &Layer {
                        name: format!("Läyer {i}"),
                        visible: i != 1,
                        opacity: if i == 0 { 255 } else { 128 },
                        blend: BlendMode::ALL[i],
                        surface: i
                    }
                );
                // Rows are padded to the stride (12 bytes for 2 pixels); the padding is cut off.
                let surface = &surfaces[layer.surface];
                assert_eq!((surface.width, surface.stride, surface.data.len()), (2, 12, 24));
                let rows: Vec<&[u8]> = surface.rows().collect();
                assert_eq!(rows, [&[30, 20, 10, 255, 60, 50, 40, 128][..], &[90, 80, 70, 64, 120, 110, 100, 0]], "B, G, R, A");
            }
        }
    }
}

#[test]
fn truncation_unknown_modes_and_corrupt_chunks_fail_without_panicking() {
    let raw = fixture(&[0], false, false);
    for len in 0..raw.len() {
        assert!(read_all(&raw[..len]).is_err(), "length {len}");
    }
    assert!(read_all(&raw).is_ok());
    let mut trailing = raw.clone();
    trailing.push(0);
    assert_eq!(read_all(&trailing).err(), Some(Error::Invalid("unexpected trailing data".into())));
    for legacy in [false, true] {
        assert!(Document::parse(&fixture(&[999], legacy, false), GO).is_err());
        assert!(Document::parse(&fixture(&[14], legacy, false), GO).is_err());
    }
    let mut corrupt = raw.clone();
    let block = corrupt.len() - 45;
    // Zero chunk size.
    corrupt[block + 1..block + 5].fill(0);
    assert!(read_all(&corrupt).is_err());
    let mut duplicate = raw.clone();
    // Both row chunks claim index 1.
    duplicate[block + 25..block + 29].copy_from_slice(&1u32.to_be_bytes());
    assert!(read_all(&duplicate).is_err());
    let mut version = raw.clone();
    version[block] = 2;
    assert!(read_all(&version).is_err());
    let mut zipped = fixture(&[0], false, true);
    let last = zipped.len() - 1;
    zipped[last] ^= 1;
    assert!(Document::parse(&zipped, GO).is_ok() && read_all(&zipped).is_err(), "gzip checksum");
}

#[test]
fn reading_can_be_cancelled_during_parsing_and_pixel_decode() {
    use std::cell::Cell;
    let bytes = fixture(&[0, 1], false, true);
    for limit in [0, 10] {
        let checks = Cell::new(0);
        let keep_going = || {
            checks.set(checks.get() + 1);
            checks.get() <= limit
        };
        assert_eq!(Document::parse(&bytes, &keep_going).err(), Some(Error::Cancelled), "{limit}");
    }
    let doc = Document::parse(&bytes, GO).unwrap();
    let seen = Cell::new(0);
    let result = doc.read_surfaces(&|| seen.get() == 0, &mut |_, _| seen.set(seen.get() + 1));
    assert_eq!((result, seen.get()), (Err(Error::Cancelled), 1), "cancelled after the first surface");
}

proptest::proptest! {
    #[test]
    fn arbitrary_serialized_data_returns_an_error_without_panicking(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096)) {
        let mut bytes = b"PDN3\0\0\0\0\x01".to_vec();
        bytes.extend(data);
        let _ = read_all(&bytes);
    }

    #[test]
    fn mutated_layered_documents_never_panic(edits in proptest::collection::vec((proptest::prelude::any::<usize>(), proptest::prelude::any::<u8>()), 1..8)) {
        let mut bytes = fixture(&[0, 5, 13], false, true);
        for (offset, value) in edits {
            let index = offset % bytes.len();
            bytes[index] = value;
        }
        let _ = read_all(&bytes);
    }
}

#[test]
fn malformed_headers_are_errors() {
    for bytes in [b"".as_slice(), b"PDN3", b"PDN3\xff\xff\xff", b"PDN4\0\0\0", b"PDN3\0\0\0\0\x02"] {
        assert!(matches!(Document::parse(bytes, GO), Err(Error::Invalid(_))), "{bytes:?}");
    }
    assert!(!is_pdn(b"PDN4\0\0\0") && !is_pdn(b"PD"));
    // The XML header between the signature and the stream is skipped by its 24-bit length.
    let plain = fixture(&[0], false, true);
    let mut with_header = b"PDN3\x05\0\0<pdn>".to_vec();
    with_header.extend_from_slice(&plain[7..]);
    assert_eq!(read_all(&with_header), read_all(&plain));
}

/// A stream of `records` after the header record.
fn stream(records: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer(vec![0]);
    for n in [1, -1, 1, 0] {
        w.int(n);
    }
    records(&mut w);
    w.0
}

#[test]
fn serialized_counts_depth_and_references_are_bounded() {
    let array = |w: &mut Writer, id: i32, len: i32| {
        w.0.push(16);
        w.int(id);
        w.int(len);
    };
    let nested = |depth: i32| {
        stream(|w| {
            (1..=depth).for_each(|id| array(w, id, 1));
            w.0.extend([10, 11]);
        })
    };
    let strings = |total: usize| {
        stream(|w| {
            for id in 1..=(total >> 20) as i32 {
                w.0.push(6);
                w.int(id);
                w.string(&"s".repeat(1 << 20));
            }
            w.0.push(11);
        })
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("an array over the item limit", stream(|w| array(w, 1, nrbf::MAX_ITEMS as i32 + 1))),
        ("nesting past the depth limit", nested(nrbf::MAX_DEPTH as i32 + 2)),
        ("strings past the limit", strings(nrbf::MAX_STRINGS + (1 << 20))),
        (
            "a duplicate object id",
            stream(|w| {
                for _ in 0..2 {
                    w.0.push(6);
                    w.int(1);
                    w.string("duplicate");
                }
                w.0.push(11);
            }),
        ),
        (
            "a null run longer than its array",
            stream(|w| {
                array(w, 1, 1);
                w.0.extend([13, 2, 11]);
            }),
        ),
        (
            "an object of an undeclared class",
            stream(|w| {
                w.0.push(1);
                w.int(1);
                w.int(999);
            }),
        ),
        ("no root object", stream(|w| w.0.push(11))),
        ("an unknown record", stream(|w| w.0.push(99))),
        ("a negative count", stream(|w| array(w, 1, -1))),
    ];
    for (what, bytes) in cases {
        let result = nrbf::parse(&bytes, GO).map(|(g, _)| g.array(&g.root).map(|a| a.len()));
        assert!(matches!(result, Err(Error::Invalid(_)) | Ok(Err(Error::Invalid(_)))), "{what}");
    }
    // An array holding a reference to itself is a legal graph: resolving stops at the array.
    let own = stream(|w| {
        array(w, 1, 1);
        w.0.push(9);
        w.int(1);
        w.0.push(11);
    });
    let (graph, _) = nrbf::parse(&own, GO).unwrap();
    let items = graph.array(&graph.root).unwrap();
    assert_eq!(graph.array(&items[0]).map(<[Value]>::len), Ok(1));
    assert!(nrbf::parse(&nested(nrbf::MAX_DEPTH as i32), GO).is_ok());
    assert!(nrbf::parse(&strings(nrbf::MAX_STRINGS), GO).is_ok());
}

#[test]
fn a_graph_is_read_by_class_field_and_reference() {
    use Field::*;
    let mut bytes = stream(|w| {
        w.object(
            1,
            "Root",
            &[("count", Int(-7)), ("big", Long(1 << 40)), ("flag", Bool(true)), ("byte", Byte(200)), ("child", Ref(2)), ("text", Ref(3))],
        );
        w.object(2, "Child", &[("parent", Ref(1))]);
        w.0.push(6);
        w.int(3);
        w.string("héllo");
        w.0.push(11);
    });
    let len = bytes.len();
    bytes.extend_from_slice(b"pixels follow");
    let (graph, used) = nrbf::parse(&bytes, GO).unwrap();
    assert_eq!(used, len, "what follows the stream is left for the caller");
    let root = &graph.root;
    assert_eq!(graph.class_name(root).unwrap(), "Root");
    assert_eq!(graph.integer(graph.field(root, "count").unwrap()), Ok(-7));
    assert_eq!(graph.integer(graph.field(root, "big").unwrap()), Ok(1 << 40));
    assert_eq!(graph.integer(graph.field(root, "byte").unwrap()), Ok(200));
    assert_eq!(graph.boolean(graph.field(root, "flag").unwrap()), Ok(true));
    assert_eq!(graph.string(graph.field(root, "text").unwrap()), Ok("héllo"));
    let child = graph.field(root, "child").unwrap();
    assert_eq!(graph.class_name(graph.field(child, "parent").unwrap()).unwrap(), "Root", "a cycle through objects is fine");
    assert!(graph.optional_field(root, "missing").unwrap().is_none() && graph.field(root, "missing").is_err());
    assert!(graph.integer(graph.field(root, "flag").unwrap()).is_err() && graph.string(root).is_err() && graph.array(root).is_err());
    assert_eq!(graph.objects_of_class("Child").map(|(id, _)| id).collect::<Vec<_>>(), [2]);
    match graph.resolve(root).unwrap() {
        Value::Object { class, members } => assert_eq!((class.name(), class.member_names().len(), members.len()), ("Root", 6, 6)),
        other => panic!("{other:?}"),
    }
    assert_eq!(nrbf::parse(&bytes, &|| false).err(), Some(Error::Cancelled));
}

#[test]
fn truncated_uncompressed_pixels_are_rejected_before_allocating() {
    // A raw (version 1) block of 256 MiB chunks that declares 1 GiB of pixels but holds a few bytes.
    let mut reader = Reader { bytes: &[1, 0x10, 0, 0, 0, 0, 0, 0, 0], pos: 0 };
    assert_eq!(pixels(&mut reader, 1 << 30, GO).err(), Some(Error::Invalid("uncompressed pixel data is truncated".into())));
}

/// The fixture with `edit` applied to the serialized text: same-length replacements only.
fn edited(from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let mut bytes = fixture(&[0, 1], false, false);
    let at = bytes.windows(from.len()).position(|w| w == from).unwrap_or_else(|| panic!("{from:?} not in the fixture"));
    bytes[at..at + from.len()].copy_from_slice(to);
    bytes
}

#[test]
fn documents_this_reader_does_not_cover_are_refused() {
    let cases: [(&str, &[u8], &[u8]); 6] = [
        ("root object is not a Paint.NET document", b"PaintDotNet.Document", b"PaintDotNet.Documenx"),
        ("only PDN3 bitmap layers are supported", b"PaintDotNet.BitmapLayer\x05", b"PaintDotNet.VectorLayer\x05"),
        ("invalid bitmap surface", b"PaintDotNet.Surface", b"PaintDotNet.Surfacf"),
        ("unsupported non-deferred or parented pixel memory", b"\x18\0\0\0\0\0\0\0\0\x01", b"\x18\0\0\0\0\0\0\0\x01\x01"),
        ("pixel memory length does not match the surface", b"\x18\0\0\0\0\0\0\0\0\x01", b"\x19\0\0\0\0\0\0\0\0\x01"),
        ("surface stride is too short for BGRA32 pixels", b"\x02\0\0\0\x02\0\0\0\x0c\0\0\0", b"\x02\0\0\0\x02\0\0\0\x07\0\0\0"),
    ];
    for (why, from, to) in cases {
        assert_eq!(Document::parse(&edited(from, to), GO).err(), Some(Error::Invalid(why.into())));
    }
    // The canvas: 2 × 2 in the fixture's Document object, right after its class header.
    let canvas = |w: i32, h: i32| {
        let mut bytes = fixture(&[0], false, false);
        let at = bytes.windows(8).position(|w| w == [2, 0, 0, 0, 2, 0, 0, 0]).unwrap();
        bytes[at..at + 4].copy_from_slice(&w.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&h.to_le_bytes());
        Document::parse(&bytes, GO).err()
    };
    let limit = Some(Error::Invalid("canvas is empty or exceeds the 1 GiB pixel limit".into()));
    assert_eq!(canvas(0, 2), limit);
    assert_eq!(canvas(16385, 16384), limit);
    assert_eq!(canvas(-1, 2), Some(Error::Invalid("invalid width".into())));
    assert_eq!(
        canvas(16384, 16384),
        Some(Error::Invalid("layer dimensions do not match the canvas".into())),
        "1 GiB exactly passes the canvas check"
    );
}

#[test]
fn layer_counts_and_names_are_bounded() {
    assert!(Document::parse(&fixture(&[], false, false), GO).is_err(), "no layers");
    // 1025 layers: over the limit. The fixture's ids step by ten, so this stays far below i32.
    let modes = vec![0; MAX_LAYERS + 1];
    assert_eq!(Document::parse(&fixture(&modes, false, false), GO).err(), Some(Error::Invalid("document must have 1..1024 bitmap layers".into())));
    assert_eq!(Document::parse(&fixture(&modes[..MAX_LAYERS], false, false), GO).map(|d| d.layers.len()), Ok(MAX_LAYERS));
}
