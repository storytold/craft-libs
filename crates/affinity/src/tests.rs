//! Unit tests on synthetic inputs; `tests/it/real_files.rs` covers public Affinity documents.

mod preview {
    use crate::container::header;
    use crate::*;
    use proptest::prelude::*;

    fn fixture() -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(13u32.to_be_bytes());
        png.extend(b"IHDR");
        png.extend(2u32.to_be_bytes());
        png.extend(1u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        png.extend(crc32fast::hash(&png[12..]).to_be_bytes());
        png.extend([0; 4]);
        png.extend(b"IDAT");
        png.extend(crc32fast::hash(b"IDAT").to_be_bytes());
        png.extend([0; 4]);
        png.extend(b"IEND");
        png.extend(crc32fast::hash(b"IEND").to_be_bytes());
        let mut b = vec![0; 72];
        b[..4].copy_from_slice(MAGIC);
        b[4..6].copy_from_slice(&12u16.to_le_bytes());
        b[8..12].copy_from_slice(b"nsrP");
        b[12..16].copy_from_slice(b"#Inf");
        b[24..32].copy_from_slice(&72u64.to_le_bytes());
        b[64..68].copy_from_slice(b"Prot");
        b.extend(b"\xff\xff\xff\xffThmb");
        b.extend(1u32.to_le_bytes());
        b.extend((png.len() as u32 + 13).to_le_bytes());
        b.extend(29u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend((png.len() as u32).to_le_bytes());
        b.push(1);
        b.extend(png);
        b
    }

    #[test]
    fn indexed_preview_and_header() {
        let b = fixture();
        assert_eq!(header(&b).unwrap().version, 12);
        let p = preview(&b).unwrap();
        assert_eq!((p.width, p.height), (2, 1));
        assert_eq!(p.png, &b[101..]);
    }

    #[test]
    fn every_truncation_is_rejected() {
        let b = fixture();
        for end in 0..b.len() {
            assert!(preview(&b[..end]).is_err(), "{end}");
        }
    }

    #[test]
    fn rejects_assets_versions_offsets_lengths_and_bombs() {
        let b = fixture();
        for (offset, value) in [
            (4, 13u64.to_le_bytes().to_vec()),
            (8, b"urBR".to_vec()),
            (24, u64::MAX.to_le_bytes().to_vec()),
            (24, 1u64.to_le_bytes().to_vec()),
            (24, 0u64.to_le_bytes().to_vec()),
            (84, u32::MAX.to_le_bytes().to_vec()),
            (88, 30u32.to_le_bytes().to_vec()),
            (96, u32::MAX.to_le_bytes().to_vec()),
            (117, 5000u32.to_be_bytes().to_vec()),
        ] {
            let mut broken = b.clone();
            broken[offset..offset + value.len()].copy_from_slice(&value);
            assert!(preview(&broken).is_err(), "offset {offset}");
        }
        // Versions 8 to 11 store the same record (checked on public Affinity 1 and 2 files).
        for version in 8..12 {
            let mut legacy = b.clone();
            legacy[4..6].copy_from_slice(&(version as u16).to_le_bytes());
            assert!(preview(&legacy).is_ok());
        }
        // Flags 4 and 8 occur in real files; the low two bits mark other container variants.
        for (flags, ok) in [(4u16, true), (8, true), (1, false), (2, false)] {
            let mut variant = b.clone();
            variant[6..8].copy_from_slice(&flags.to_le_bytes());
            assert_eq!(preview(&variant).is_ok(), ok, "flags {flags}");
        }
        let mut old = b.clone();
        old[4..6].copy_from_slice(&7u16.to_le_bytes());
        assert!(matches!(preview(&old), Err(Error::Unsupported(_))));
    }

    #[test]
    fn an_unindexed_png_or_resource_is_never_used() {
        let mut b = fixture();
        b[24..32].copy_from_slice(&0u64.to_le_bytes());
        assert!(preview(&b).is_err());
        b[24..32].copy_from_slice(&72u64.to_le_bytes());
        b[76..80].copy_from_slice(b"Meta");
        assert!(preview(&b).is_err());
    }

    #[test]
    fn every_chunk_checksum_including_the_end_is_checked() {
        let b = fixture();
        for at in [130, b.len() - 1] {
            let mut damaged = b.clone();
            damaged[at] ^= 1;
            assert_eq!(preview(&damaged).unwrap_err(), Error::Malformed("PNG chunk checksum"));
        }
        let mut ancillary = vec![0; 4];
        ancillary.extend(b"tEXt");
        ancillary.extend(crc32fast::hash(b"tEXt").to_be_bytes());
        let mut with_ancillary = b.clone();
        let at = b.len() - 12;
        with_ancillary.splice(at..at, ancillary);
        let png_length = (with_ancillary.len() - 101) as u32;
        with_ancillary[84..88].copy_from_slice(&(png_length + 13).to_le_bytes());
        with_ancillary[96..100].copy_from_slice(&png_length.to_le_bytes());
        assert!(preview(&with_ancillary).is_ok());
        with_ancillary[at + 8] ^= 1;
        assert_eq!(preview(&with_ancillary).unwrap_err(), Error::Malformed("PNG chunk checksum"));
    }

    #[test]
    fn compressed_metadata_and_animation_are_not_preview_support() {
        for kind in [b"zTXt", b"iTXt", b"iCCP", b"acTL", b"fcTL", b"fdAT"] {
            let mut extra = vec![0; 4];
            extra.extend(kind);
            extra.extend(crc32fast::hash(kind).to_be_bytes());
            let mut b = fixture();
            let at = b.len() - 12;
            b.splice(at..at, extra);
            let length = (b.len() - 101) as u32;
            b[84..88].copy_from_slice(&(length + 13).to_le_bytes());
            b[96..100].copy_from_slice(&length.to_le_bytes());
            assert!(matches!(preview(&b), Err(Error::Unsupported(_))));
        }
    }

    #[test]
    fn critical_chunks_follow_a_single_complete_png_layout() {
        fn with_chunks(kinds: &[&[u8; 4]]) -> Vec<u8> {
            let original = fixture();
            let mut b = original[..134].to_vec(); // File, record and complete first IHDR.
            for kind in kinds {
                b.extend([0; 4]);
                b.extend(*kind);
                b.extend(crc32fast::hash(*kind).to_be_bytes());
            }
            let length = (b.len() - 101) as u32;
            b[84..88].copy_from_slice(&(length + 13).to_le_bytes());
            b[96..100].copy_from_slice(&length.to_le_bytes());
            b
        }
        assert!(preview(&with_chunks(&[b"IDAT", b"IDAT", b"IEND"])).is_ok());
        // PNG readers must not reject an otherwise valid unknown ancillary chunk just
        // because its reserved (third-letter) bit is set; future versions can define it.
        assert!(preview(&with_chunks(&[b"abct", b"IDAT", b"IEND"])).is_ok());
        for kinds in [
            vec![b"IEND"],
            vec![b"IDAT", b"IHDR", b"IEND"],
            vec![b"PLTE", b"PLTE", b"IDAT", b"IEND"],
            vec![b"IDAT", b"PLTE", b"IEND"],
            vec![b"IDAT", b"tEXt", b"IDAT", b"IEND"],
            vec![b"ABCD", b"IDAT", b"IEND"],
            vec![b"IDAT", b"IEND", b"tEXt"],
        ] {
            assert!(preview(&with_chunks(&kinds)).is_err(), "{kinds:?}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn arbitrary_input_never_panics(b in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = header(&b);
            let _ = preview(&b);
        }
        #[test]
        fn mutations_never_panic(edits in prop::collection::vec((0usize..200, any::<u8>()), 0..32)) {
            let mut b = fixture();
            for (i, value) in edits { if let Some(v) = b.get_mut(i) { *v = value; } }
            let _ = preview(&b);
        }
    }
}

mod archive {
    use crate::Error;
    use crate::container::{Archive, Limits};
    use crate::stream::{self, Kind, Tag, Value};
    use crate::synth::{self, F, Method, tag};
    use proptest::prelude::*;

    fn doc() -> Vec<u8> {
        synth::stream(&[
            (tag(b"Desc"), F::Str("Document".into())),
            (tag(b"Visi"), F::Bool(true)),
            (tag(b"Xfrm"), F::F64s(vec![1.0, 0.0, 5.0, 0.0, 1.0, -2.0])),
            (tag(b"Colr"), F::Def(7, vec![tag(b"RGBA")], vec![(tag(b"_col"), F::Struct([0u8; 16].to_vec()))])),
            (tag(b"Same"), F::Ref(7)),
            (tag(b"Null"), F::Null),
            (tag(b"Kids"), F::Shared(vec![F::Def(8, vec![tag(b"Grup"), tag(b"Node")], vec![(tag(b"Desc"), F::Str("g".into()))]), F::Ref(8)])),
            (
                tag(b"Crvs"),
                F::Obj(tag(b"PCvD"), vec![(tag(b"Data"), F::Pos(vec![F::U8(0), F::U32(1), F::Bool(true), F::Records(18, vec![vec![0; 18]; 4])]))]),
            ),
            (tag(b"Enum"), F::Enum(2, 1)),
            (tag(b"Tile"), F::Entry("d/3".into())),
            (tag(b"Stat"), F::U8s(vec![4, 0, 2])),
        ])
    }

    #[test]
    fn every_method_round_trips_with_checksums() {
        let d = doc();
        for method in [Method::Stored, Method::Zlib, Method::Zstd] {
            let file = synth::container(&[("doc.dat", &d, method), ("d/3", b"tile", method)], None);
            let mut a = Archive::open(&file, Limits::default()).unwrap();
            assert_eq!(a.header.version, 12);
            assert_eq!(a.read("doc.dat").unwrap(), d);
            assert_eq!(a.read("d/3").unwrap(), b"tile");
            assert_eq!(a.extracted(), d.len() + 4);
            assert!(a.read("missing").is_err());
        }
    }

    #[test]
    fn the_stream_parses_into_shared_objects_and_links() {
        let s = stream::parse(&doc()).unwrap();
        let root = &s.objects[s.root];
        assert_eq!(root.class, Tag::of(b"Pers"));
        assert_eq!(root.get(Tag::of(b"Desc")), Some(&Value::Str("Document".into())));
        assert_eq!(root.get(Tag::of(b"Xfrm")), Some(&Value::Floats(vec![1.0, 0.0, 5.0, 0.0, 1.0, -2.0])));
        let (Some(Value::Obj(Some(a))), Some(Value::Obj(Some(b)))) = (root.get(Tag::of(b"Colr")), root.get(Tag::of(b"Same"))) else { panic!() };
        assert_eq!(a, b);
        assert_eq!(s.objects[*a].class, Tag::of(b"RGBA"));
        assert_eq!(s.objects[*a].kind, Kind::Shared);
        let Some(Value::Array(kids)) = root.get(Tag::of(b"Kids")) else { panic!() };
        assert_eq!(kids[0], kids[1]);
        let Value::Obj(Some(g)) = kids[0] else { panic!() };
        assert_eq!(s.objects[g].chain, vec![Tag::of(b"Grup"), Tag::of(b"Node")]);
        assert_eq!(root.get(Tag::of(b"Enum")), Some(&Value::Enum { id: 2, version: 1 }));
        assert_eq!(root.get(Tag::of(b"Tile")), Some(&Value::Entry("d/3".into())));
        assert_eq!(root.get(Tag::of(b"Null")), Some(&Value::Obj(None)));
    }

    #[test]
    fn checksums_sizes_and_budgets_are_enforced() {
        let d = doc();
        let file = synth::container(&[("doc.dat", &d, Method::Zstd)], None);
        // Flip a byte inside the compressed payload, past the "#Fil" tag at offset 72.
        let mut broken = file.clone();
        broken[90] ^= 0x55;
        let mut a = Archive::open(&broken, Limits::default()).unwrap();
        assert!(a.read("doc.dat").is_err());
        let mut a = Archive::open(&file, Limits { max_entry: d.len() - 1, max_total: usize::MAX, ..Limits::default() }).unwrap();
        assert!(matches!(a.read("doc.dat"), Err(Error::Limit(_))));
        let mut a = Archive::open(&file, Limits { max_entry: usize::MAX, max_total: d.len() + 1, ..Limits::default() }).unwrap();
        a.read("doc.dat").unwrap();
        assert!(matches!(a.read("doc.dat"), Err(Error::Limit(_))));
    }

    #[test]
    fn an_entry_that_fails_its_checksum_is_charged_once_and_not_decompressed_again() {
        let d = doc();
        let file = synth::container(&[("doc.dat", &d, Method::Zstd)], None);
        let mut broken = file.clone();
        broken[90] ^= 0x55;
        let mut a = Archive::open(&broken, Limits::default()).unwrap();
        let first = a.read("doc.dat").unwrap_err();
        // The work was done, so it counts against the import even though it failed.
        assert_eq!(a.extracted(), d.len());
        assert_eq!(a.read("doc.dat").unwrap_err(), first);
        assert_eq!(a.extracted(), d.len());
        // A budget that only covers one attempt is not exceeded by asking again.
        let mut a = Archive::open(&broken, Limits { max_entry: usize::MAX, max_total: d.len(), ..Limits::default() }).unwrap();
        assert!(a.read("doc.dat").is_err());
        assert_eq!(a.read("doc.dat").unwrap_err(), first);
    }

    #[test]
    fn the_table_index_has_a_budget_for_records_and_names() {
        let d = doc();
        let long = "n".repeat(300);
        let file = synth::container(&[("doc.dat", &d, Method::Stored), (&long, b"tile", Method::Stored)], None);
        Archive::open_within(&file, Limits::default(), 2, 307).unwrap();
        assert_eq!(Archive::open_within(&file, Limits::default(), 1, 1 << 20).unwrap_err(), Error::Limit("too many archive entries"));
        assert_eq!(Archive::open_within(&file, Limits::default(), 2, 306).unwrap_err(), Error::Limit("archive entry names too long"));
    }

    // Regression test for #1710: ruzstd 0.9.0 let a block decompress to more than its frame's
    // window allows (RUSTSEC-2026-0344), so a crafted entry made the decoder produce far more than
    // the format permits for a block. 0.9.1 refuses it.
    #[test]
    fn a_zstd_block_larger_than_its_frames_window_is_refused() {
        const LEN: u32 = 100_000;
        let plain = vec![0x61u8; LEN as usize];
        let file = synth::container(&[("doc.dat", &plain, Method::Zstd)], None);
        // A Zstandard frame with a 1 KiB window (descriptor byte 0, window byte 0) holding one RLE
        // block, the last, that repeats a byte LEN times: block header = last | RLE << 1 | LEN << 3.
        let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x00];
        frame.extend((1 | (1 << 1) | (LEN << 3)).to_le_bytes().get(..3).unwrap());
        frame.push(0x61);
        // Swap it in for the entry's compressed bytes (they follow the "#Fil" tag at offset 72).
        let fat = u64::from_le_bytes(file[16..24].try_into().unwrap()) as usize;
        let old_len = u64::from_le_bytes(file[fat + 59 + 21..fat + 59 + 29].try_into().unwrap()) as usize;
        let mut swapped = file[..76].to_vec();
        swapped.extend(&frame);
        swapped.extend(&file[76 + old_len..]);
        let new_fat = fat - old_len + frame.len();
        swapped[16..24].copy_from_slice(&(new_fat as u64).to_le_bytes());
        swapped[new_fat + 59 + 21..new_fat + 59 + 29].copy_from_slice(&(frame.len() as u64).to_le_bytes());
        let mut a = Archive::open(&swapped, Limits::default()).unwrap();
        assert!(a.read("doc.dat").is_err(), "a block of {LEN} bytes was decompressed from a frame with a 1 KiB window");
    }

    #[test]
    fn a_declared_size_smaller_than_the_data_is_rejected_not_truncated() {
        let big = vec![7u8; 1 << 20];
        let mut file = synth::container(&[("doc.dat", &big, Method::Zlib)], None);
        // Rewrite the declared size in the table to 1 KiB: decoding must stop and fail.
        let fat = u64::from_le_bytes(file[16..24].try_into().unwrap()) as usize;
        let size_at = fat + 59 + 5 + 8;
        file[size_at..size_at + 8].copy_from_slice(&1024u64.to_le_bytes());
        let mut a = Archive::open(&file, Limits::default()).unwrap();
        assert_eq!(a.read("doc.dat").unwrap_err(), Error::Malformed("archive entry size mismatch"));
    }

    #[test]
    fn libraries_and_cyclic_tables_are_rejected() {
        let d = doc();
        let mut file = synth::container(&[("doc.dat", &d, Method::Stored)], None);
        let mut library = file.clone();
        library[8..12].copy_from_slice(b"rAsA");
        assert!(matches!(Archive::open(&library, Limits::default()), Err(Error::Unsupported(_))));
        let fat = u64::from_le_bytes(file[16..24].try_into().unwrap());
        let at = fat as usize + 4;
        file[at..at + 8].copy_from_slice(&fat.to_le_bytes());
        assert_eq!(Archive::open(&file, Limits::default()).unwrap_err(), Error::Malformed("allocation tables form a cycle"));
    }

    #[test]
    fn hostile_streams_fail_cleanly() {
        let deep = (0..2000).fold(Vec::new(), |inner, _| vec![(tag(b"Chld"), F::Obj(tag(b"Grup"), inner))]);
        assert_eq!(stream::parse(&synth::stream(&deep)).unwrap_err(), Error::Limit("objects nested too deeply"));
        // An array claiming four billion elements in a few bytes.
        let huge = synth::stream(&[(tag(b"Big_"), F::Raw(vec![0x81, b'_', b'g', b'i', b'B', 0xff, 0xff, 0xff, 0xff, 0]))]);
        assert_eq!(stream::parse(&huge).unwrap_err(), Error::Malformed("array longer than its data"));
        let forward = synth::stream(&[(tag(b"Link"), F::Ref(99))]);
        assert!(stream::parse(&forward).is_err());
        let twice = synth::stream(&[(tag(b"A___"), F::Def(1, vec![tag(b"RGBA")], vec![])), (tag(b"B___"), F::Def(1, vec![tag(b"RGBA")], vec![]))]);
        assert!(stream::parse(&twice).is_err());
        let unknown = synth::stream(&[(tag(b"X___"), F::Raw(vec![0x0b, 1, 2, 3, 4]))]);
        assert!(matches!(stream::parse(&unknown), Err(Error::Unsupported(_))));
        let d = doc();
        for end in 0..d.len() {
            assert!(stream::parse(&d[..end]).is_err(), "{end}");
        }
    }

    #[test]
    fn a_small_compressed_entry_cannot_make_the_stream_allocate_gigabytes() {
        // Each decoded value takes about 40 bytes, and a bool array packs eight in a byte: 16 MiB
        // of zero bits, a few KiB once compressed, would ask for 2^27 values (about 4 GiB). An
        // array of one-byte values just past the budget fails the same way, before allocating.
        let field = |code: u8, n: u32, data: usize| {
            let mut f = vec![0x80 | code];
            f.extend(tag(b"Big_").0.to_le_bytes());
            f.extend(n.to_le_bytes());
            f.resize(f.len() + data, 0);
            F::Raw(f)
        };
        let bools = 1u32 << 27;
        let bytes = u32::try_from(stream::MAX_VALUES).unwrap() + 1;
        for f in [field(0x29, bools, (bools / 8) as usize), field(0x01, bytes, bytes as usize)] {
            let d = synth::stream(&[(tag(b"Big_"), f)]);
            let file = synth::container(&[("doc.dat", &d, Method::Zstd)], None);
            assert!(file.len() < d.len() / 100, "{} of {}", file.len(), d.len());
            let entry = Archive::open(&file, Limits::default()).unwrap().read("doc.dat").unwrap();
            assert_eq!(stream::parse(&entry).unwrap_err(), Error::Limit("too many values in the document stream"));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn mutated_containers_and_streams_never_panic(edits in prop::collection::vec((0usize..4096, any::<u8>()), 1..24)) {
            let d = doc();
            for method in [Method::Stored, Method::Zlib, Method::Zstd] {
                let mut file = synth::container(&[("doc.dat", &d, method)], None);
                for (i, v) in &edits {
                    let n = file.len();
                    file[i % n] = *v;
                }
                if let Ok(bytes) = Archive::open(&file, Limits::default()).and_then(|mut a| a.read("doc.dat")) {
                    let _ = stream::parse(&bytes);
                }
            }
            let mut s = d.clone();
            for (i, v) in &edits {
                let n = s.len();
                s[i % n] = *v;
            }
            let _ = stream::parse(&s);
        }
    }
}

/// The table-based plane conversion against the per-pixel `f64` conversion it replaced, and the
/// CMYK helper against the same formula in floating point.
mod raster_identity {
    use crate::model::Pixels;
    use crate::raster::interleave;

    /// Deterministic filler (xorshift).
    fn bytes(n: usize, mut x: u64) -> Vec<u8> {
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    fn reference(planes: &[Vec<u8>], channels: usize, bps: usize, n: usize) -> Vec<u8> {
        let sample = |p: &Vec<u8>, i: usize| -> f64 {
            if bps == 2 {
                f64::from(u16::from_le_bytes([p.get(2 * i).copied().unwrap_or(0), p.get(2 * i + 1).copied().unwrap_or(0)])) / 65535.0
            } else {
                f64::from(p.get(i).copied().unwrap_or(0)) / 255.0
            }
        };
        let mut rgba = vec![0u8; n * 4];
        for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v: Vec<f64> = planes.iter().map(|p| sample(p, i)).collect();
            let (r, g, b, a) = if channels == 1 {
                (v[0], v[0], v[0], 1.0)
            } else if channels == 5 {
                let k = 1.0 - v[3];
                ((1.0 - v[0]) * k, (1.0 - v[1]) * k, (1.0 - v[2]) * k, v[4])
            } else {
                (v[0], v[1], v[2], v[3])
            };
            px.copy_from_slice(&[r, g, b, a].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
        rgba
    }

    #[test]
    fn eight_bit_samples_round_trip() {
        let ramp: Vec<u8> = (0..=255).collect();
        let planes = vec![ramp.clone(), ramp.iter().rev().copied().collect(), ramp.clone(), ramp.iter().rev().copied().collect()];
        assert_eq!(interleave(&planes, 4, 1, 256), reference(&planes, 4, 1, 256));
        assert_eq!(interleave(&planes[..1], 1, 1, 256), reference(&planes[..1], 1, 1, 256));
    }

    #[test]
    fn sixteen_bit_samples_are_exhaustive() {
        let all: Vec<u8> = (0..=u16::MAX).flat_map(u16::to_le_bytes).collect();
        let rot = |k: usize| -> Vec<u8> { (0..=u16::MAX).flat_map(|v| v.wrapping_add((k * 4099) as u16).to_le_bytes()).collect() };
        let planes = vec![all, rot(1), rot(2), rot(3)];
        assert_eq!(interleave(&planes, 4, 2, 65536), reference(&planes, 4, 2, 65536));
        assert_eq!(interleave(&planes[..1], 1, 2, 65536), reference(&planes[..1], 1, 2, 65536));
    }

    #[test]
    fn random_planes_match_for_every_format() {
        for (channels, bps) in [(1, 1), (1, 2), (4, 1), (4, 2)] {
            let n = 5000;
            let planes: Vec<Vec<u8>> = (0..channels).map(|c| bytes(n * bps, 1 + c as u64)).collect();
            assert_eq!(interleave(&planes, channels, bps, n), reference(&planes, channels, bps, n), "{channels}x{bps}");
        }
    }

    #[test]
    fn cmyk_to_rgba_is_exhaustive_over_colour_and_black() {
        // Every (ink, black) pair in each of the c, m and y positions; alpha is arbitrary.
        let n = 256 * 256;
        let k: Vec<u8> = (0..n).map(|i| (i >> 8) as u8).collect();
        let c: Vec<u8> = (0..n).map(|i| i as u8).collect();
        let m: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(7).wrapping_add(3)).collect();
        let y: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(13).wrapping_add(250)).collect();
        let planes = vec![c, m, y, k, bytes(n, 9)];
        let cmyka: Vec<u8> = (0..n).flat_map(|i| planes.iter().map(move |p| p[i])).collect();
        assert_eq!(Pixels::cmyka8_to_rgba8(&cmyka), reference(&planes, 5, 1, n));
    }
}
