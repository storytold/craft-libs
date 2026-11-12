//! Documents written and read back, in both formats; damaged files.

use craft_timeline::time::{FrameRate, Tick};

use crate::model::{CItem, CKind, CMarker, Document, Gain};
use crate::testdoc::{IDENTITY, pcm_ramp, picture_and_sound, sound_only};
use crate::{Error, Report, aaf, omf};

fn aaf_bytes(doc: &Document, small_sectors: bool) -> Vec<u8> {
    let mut report = Report::default();
    let bytes = aaf::write(doc, &aaf::WriteOptions { identity: IDENTITY, small_sectors }, &mut report).unwrap();
    assert!(report.is_empty(), "{report}");
    bytes
}

fn omf_bytes(doc: &Document) -> Vec<u8> {
    let mut report = Report::default();
    let bytes = omf::write(doc, &IDENTITY, &mut report).unwrap();
    assert!(!report.has_warnings(), "{report}");
    bytes
}

fn read_clean(bytes: &[u8], is_aaf: bool) -> Document {
    let mut report = Report::default();
    let doc = if is_aaf { aaf::read(bytes, &mut report) } else { omf::read(bytes, &mut report) }.unwrap();
    assert!(!report.has_warnings(), "{report}");
    doc
}

/// What a track holds, with each clip's source by what it is rather than by index. Gain is stored
/// as a rational: four decimals survive.
fn shape(doc: &Document) -> Vec<(CKind, String, u32, Vec<String>)> {
    let comp = &doc.compositions[0];
    comp.tracks
        .iter()
        .map(|t| {
            let items = t
                .items
                .iter()
                .map(|i| match i {
                    CItem::Filler(l) => format!("gap {}", l.0),
                    CItem::Transition(x) => format!("{} {} cut {}", x.effect, x.len.0, x.cut.0),
                    CItem::Clip(c) => {
                        let s = &doc.sources[c.source];
                        let gain = match &c.gain {
                            None => "unity".to_string(),
                            Some(Gain::Constant(g)) => format!("{g:.4}"),
                            Some(Gain::Varying { linear, points }) => {
                                format!("{linear} {:?}", points.iter().map(|(t, g)| (t.0, format!("{g:.4}"))).collect::<Vec<_>>())
                            }
                        };
                        format!("clip {} from {} of {:?} {:?} gain {gain}", c.len.0, c.start.0, s.path, s.kind)
                    }
                })
                .collect();
            (t.kind, t.name.clone(), t.number, items)
        })
        .collect()
}

#[test]
fn aaf_round_trips_the_edit_at_every_rate() {
    for (rate, drop) in [(FrameRate::FPS_25, false), (FrameRate::FPS_29_97, true), (FrameRate::FPS_23_976, false), (FrameRate::FPS_59_94, true)] {
        let mut doc = picture_and_sound(rate);
        let comp = &mut doc.compositions[0];
        (comp.start_tc, comp.drop) = (rate.timecode_base() * 3600, drop);
        comp.markers = vec![
            CMarker { start: rate.tick_of(10), duration: Tick::ZERO, name: "Slate".into(), comment: "take 2".into(), color: Some("Green".into()) },
            CMarker { start: rate.tick_of(40), duration: rate.tick_of(5), name: "Range".into(), comment: String::new(), color: None },
        ];
        // keyframed gain on the first sound clip, held gain steps on nothing else
        if let Some(CItem::Clip(c)) = comp.tracks[1].items.first_mut() {
            c.gain = Some(Gain::Varying { linear: true, points: vec![(Tick::ZERO, 1.0), (rate.tick_of(25), 0.5)] });
        }
        doc.sources[0].markers =
            vec![CMarker { start: rate.tick_of(30), duration: Tick::ZERO, name: "In media".into(), comment: String::new(), color: None }];
        for small in [false, true] {
            let back = read_clean(&aaf_bytes(&doc, small), true);
            let (want, got) = (&doc.compositions[0], &back.compositions[0]);
            assert_eq!(shape(&back), shape(&doc), "{rate} small {small}");
            assert_eq!((&got.name, got.rate, got.start_tc, got.drop), (&want.name, want.rate, want.start_tc, want.drop), "{rate}");
            assert_eq!((got.width, got.height, got.sample_rate), (1920, 1080, 48_000), "{rate}");
            let markers = |m: &[CMarker]| m.iter().map(|m| (m.start, m.duration, m.name.clone(), m.comment.clone())).collect::<Vec<_>>();
            assert_eq!(markers(&got.markers), markers(&want.markers), "{rate}");
            assert_eq!(back.nested, []);
            let picture = back.sources.iter().find(|s| s.kind == CKind::Picture).unwrap();
            assert_eq!((picture.width, picture.height, picture.frame_rate, picture.length), (1280, 720, rate, doc.sources[0].length), "{rate}");
            assert_eq!((picture.start_tc, picture.tc_rate), (Some(86_400), rate), "{rate}");
            assert_eq!(markers(&picture.markers), markers(&doc.sources[0].markers), "{rate}");
            let sound = back.sources.iter().find(|s| s.kind == CKind::Sound).unwrap();
            assert_eq!((sound.channels, sound.file_channels, sound.sample_rate, sound.bits), (2, 2, 48_000, 24), "{rate}");
            assert_eq!(sound.key, picture.key, "one media item");
            // what was read, written again, reads as the same edit (mob ids, and so source keys, are new)
            let again = read_clean(&aaf_bytes(&back, small), true);
            assert_eq!((shape(&again), &again.compositions), (shape(&back), &back.compositions), "{rate}");
        }
    }
}

#[test]
fn embedded_audio_comes_back_bit_for_bit() {
    let pcm = pcm_ramp(480, 2, 3);
    let mut doc = sound_only(Some(pcm.clone()));
    // trimmed essence: its first sample is media time 0.2 s
    doc.sources[0].offset = Tick::from_units(9_600, 48_000);
    for is_aaf in [true, false] {
        let bytes = if is_aaf { aaf_bytes(&doc, false) } else { omf_bytes(&doc) };
        assert_eq!((aaf::sniff(&bytes), omf::sniff(&bytes)), (is_aaf, !is_aaf));
        let back = read_clean(&bytes, is_aaf);
        let s = &back.sources[0];
        assert_eq!(s.embedded.as_deref(), Some(&pcm[..]), "aaf {is_aaf}");
        assert_eq!((s.path.as_deref(), s.file_channels, s.sample_rate, s.bits), (None, 2, 48_000, 16), "aaf {is_aaf}");
        assert_eq!(s.length, Tick::from_units(480, 48_000), "aaf {is_aaf}");
        assert_eq!(shape(&back), shape(&doc), "aaf {is_aaf}");
    }
}

#[test]
fn omf_round_trips_sound_linked_to_wave_and_aiff_files() {
    for path in ["/m/a.wav", "/m/a.aif"] {
        let mut doc = sound_only(None);
        doc.sources[0].path = Some(path.into());
        let back = read_clean(&omf_bytes(&doc), false);
        assert_eq!(shape(&back), shape(&doc), "{path}");
        let s = &back.sources[0];
        assert_eq!((s.file_channels, s.sample_rate, s.bits, s.embedded.as_ref()), (2, 48_000, 16, None), "{path}");
        assert_eq!(back.compositions[0].rate, FrameRate::FPS_25);
    }
}

#[test]
fn omf_leaves_picture_out() {
    let doc = picture_and_sound(FrameRate::FPS_25);
    let back = read_clean(&omf_bytes(&doc), false);
    assert!(back.compositions[0].tracks.iter().all(|t| t.kind == CKind::Sound));
    assert_eq!(shape(&back).len(), 1);
}

#[test]
fn empty_and_foreign_documents() {
    let none = Document::default();
    assert_eq!(aaf::write(&none, &aaf::WriteOptions { identity: IDENTITY, small_sectors: false }, &mut Report::default()), Err(Error::Empty));
    assert_eq!(omf::write(&none, &IDENTITY, &mut Report::default()), Err(Error::Empty));
    let mut empty = sound_only(None);
    empty.compositions[0].tracks.clear();
    empty.sources.clear();
    for small in [false, true] {
        assert_eq!(read_clean(&aaf_bytes(&empty, small), true).compositions.len(), 1);
    }
    assert_eq!(read_clean(&omf_bytes(&empty), false).compositions.len(), 1);
    assert!(matches!(aaf::read(b"not an aaf", &mut Report::default()), Err(Error::Parse { format: "AAF", .. })));
    assert!(matches!(omf::read(b"not an omf", &mut Report::default()), Err(Error::Parse { format: "OMF", .. })));
    assert!(!aaf::sniff(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 but short"));
    assert!(!omf::sniff(b""));
}

/// Truncation and random corruption: an error or a document, never a panic.
#[test]
fn damaged_documents_never_panic() {
    let aaf_file = aaf_bytes(&picture_and_sound(FrameRate::FPS_25), true);
    let omf_file = omf_bytes(&sound_only(Some(pcm_ramp(100, 2, 1))));
    for (bytes, is_aaf) in [(&aaf_file, true), (&omf_file, false)] {
        let read = |b: &[u8]| {
            let mut report = Report::default();
            let _ = if is_aaf { aaf::read(b, &mut report) } else { omf::read(b, &mut report) };
        };
        for cut in (0..bytes.len()).step_by(97) {
            read(&bytes[..cut]);
        }
        let mut seed = 0xA5A5_1234_5678_9ABCu64;
        for _ in 0..400 {
            let mut g = bytes.clone();
            for _ in 0..8 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let at = (seed as usize) % g.len();
                g[at] = (seed >> 40) as u8;
            }
            read(&g);
        }
    }
}

/// A time that is not on a whole edit unit is rounded, and the report says so.
#[test]
fn off_grid_times_are_rounded_and_reported() {
    let mut doc = picture_and_sound(FrameRate::FPS_25);
    if let Some(CItem::Clip(c)) = doc.compositions[0].tracks[0].items.first_mut() {
        c.start += Tick(7);
    }
    let mut report = Report::default();
    aaf::write(&doc, &aaf::WriteOptions { identity: IDENTITY, small_sectors: false }, &mut report).unwrap();
    assert!(report.mentions("rounded"), "{report}");
}

/// A composition that uses itself (a damaged file) is a gap with a warning, not a recursion.
#[test]
fn a_composition_nested_in_itself_is_a_gap() {
    let rate = FrameRate::FPS_25;
    let mut doc = picture_and_sound(rate);
    let mut inner = doc.compositions[0].clone();
    inner.name = "Inner".into();
    let mut nest = doc.sources[0].clone();
    (nest.key, nest.path, nest.nested, nest.start_tc) = ("seq:9".into(), None, Some(0), None);
    doc.sources.push(nest);
    let uses_inner = CItem::Clip(crate::model::CClip { len: rate.tick_of(20), source: 2, start: Tick::ZERO, gain: None, name: "Inner".into() });
    inner.tracks[0].items = vec![uses_inner.clone()];
    doc.nested.push(inner);
    doc.compositions[0].tracks[0].items = vec![uses_inner];
    let bytes = aaf::write(&doc, &aaf::WriteOptions { identity: IDENTITY, small_sectors: false }, &mut Report::default()).unwrap();
    let mut report = Report::default();
    let back = aaf::read(&bytes, &mut report).unwrap();
    assert!(report.mentions("nested in itself"), "{report}");
    assert_eq!(back.nested.len(), 1);
    assert!(matches!(back.nested[0].tracks[0].items[..], [CItem::Filler(_)]), "{:?}", back.nested[0].tracks[0].items);
}
