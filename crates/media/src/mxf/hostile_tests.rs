//! Hostile input: written files with single fields of their metadata, index tables and partition
//! packs set to extreme values, KLV lengths that lie, and the public parsers on arbitrary bytes.
//! Errors, warnings and empty tables are fine; a panic (an index out of range, an overflow or a
//! giant allocation in a debug build) is not.

use std::io::Cursor;

use super::write::{
    CodedKind, FrameInfo, MxfWriter, OpAtomPcm, PackageIds, Pattern, PictureCoding, PictureDesc, SoundDesc, StartTimecode, WriterConfig,
    write_opatom_pcm,
};
use super::{Error, IndexSegment, Rational, SoundFormat, aes3_element_samples, decode_pcm, klv, open, sniff};

/// Deterministic xorshift.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// OP1a: long-GOP AVC (temporal offsets, key and B flags) with stereo 16-bit and mono 24-bit sound.
fn op1a_avc() -> Vec<u8> {
    let rate = Rational::new(30_000, 1001);
    let mut cfg = WriterConfig::new(Pattern::Op1a, rate, PackageIds::from_seed("hostile", "Hostile"));
    cfg.picture = Some(PictureDesc::new(PictureCoding::Avc { profile_idc: 100, intra: false }, 64, 36));
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 16 }, SoundDesc { sample_rate: 48_000, channels: 1, bits: 24 }];
    cfg.timecode = Some(StartTimecode { frames: 107_892, rate, drop_frame: true });
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let order =
        [(0, CodedKind::Intra, true), (3, CodedKind::Predicted, false), (1, CodedKind::Bidirectional, false), (2, CodedKind::Bidirectional, false)];
    for gop in 0..3i64 {
        for (i, (d, kind, key)) in order.iter().enumerate() {
            let au = [&[0, 0, 0, 1, if *key { 0x65 } else { 0x41 }][..], &[i as u8; 40]].concat();
            w.push_picture(&au, FrameInfo { key: *key, kind: *kind, display: Some(gop * 4 + d) }).unwrap();
            w.push_sound(0, &vec![i as u8; 1602 * 4]).unwrap();
            w.push_sound(1, &vec![gop as u8; 1602 * 3]).unwrap();
        }
    }
    w.finish().unwrap().into_inner()
}

/// OP-Atom: a clip-wrapped VC-3 picture file.
fn atom_vc3() -> Vec<u8> {
    let mut cfg = WriterConfig::new(Pattern::OpAtom, Rational::new(24, 1), PackageIds::from_seed("atom", "Atom"));
    cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 64, 36));
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..9u8 {
        w.push_picture(&[&[0, 0, 2, 0x80, 1][..], &vec![i; 300 + 7 * i as usize]].concat(), FrameInfo::intra()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// OP-Atom PCM, with the sample rate or a frame rate as the edit rate.
fn atom_pcm(edit_rate: Option<Rational>) -> Vec<u8> {
    let opts = OpAtomPcm { sample_rate: 48_000, bits: 24, channels: 2, edit_rate, ids: PackageIds::from_seed("pcm", "PCM"), timecode: None };
    write_opatom_pcm(&opts, &(0..6000).collect::<Vec<i32>>()).unwrap()
}

fn bases() -> Vec<Vec<u8>> {
    vec![op1a_avc(), atom_vc3(), atom_pcm(None), atom_pcm(Some(Rational::new(25, 1)))]
}

/// Opens and uses everything the API offers. Returns whether the file opened.
fn exercise(data: &[u8]) -> bool {
    let Ok(m) = open(data) else { return false };
    let _ = (m.operational_pattern.name(), m.timecode.map(|t| t.format()));
    for p in &m.partitions {
        let _ = (p.closed(), p.complete());
    }
    for (ti, t) in m.tracks.iter().enumerate() {
        let _ = (t.codec_params(), t.first_edit_unit(), t.stored_sample_frames(), t.samples_per_edit_unit(), t.codec.name());
        let _ = t.picture.as_ref().map(|p| (p.frame_width(), p.frame_height(), p.full_range()));
        let _ = t.sound.as_ref().map(|s| s.frame_bytes());
        let n = t.samples.len();
        for i in (0..n).step_by((n / 32).max(1)).chain(n.checked_sub(1)) {
            let _ = m.read_sample(data, ti, i);
            let _ = t.sync_before(i);
        }
        let _ = (t.sync_before(usize::MAX), t.sample_at(-1), t.sample_at(i64::MAX), t.sample_at(0));
        // sound: the start, a request crossing chunks, one past the end, and absurd ones
        for (start, frames) in
            [(0u64, 100usize), (1500, 4000), (t.stored_sample_frames(), 16), (u64::MAX, 1), (0, usize::MAX), (u64::MAX - 3, 1 << 20)]
        {
            let _ = m.read_pcm(data, ti, start, frames);
        }
    }
    let _ = (m.read_sample(data, m.tracks.len(), 0), m.read_pcm(data, m.tracks.len(), 0, 1));
    true
}

/// The KLV packets of a written file: (key offset, value offset, value length).
fn klvs(f: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 17 <= f.len() {
        let Some((len, n)) = klv::ber_length(&f[pos + 16..]) else { break };
        let v = pos + 16 + n;
        if v + len as usize > f.len() {
            break;
        }
        out.push((pos, v, len as usize));
        pos = v + len as usize;
    }
    out
}

/// The items of a local set value: (value offset, value length) of each 2-byte tag / 2-byte length.
fn items(f: &[u8], at: usize, len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut p = at;
    while p + 4 <= at + len {
        let n = usize::from(u16::from_be_bytes([f[p + 2], f[p + 3]]));
        if p + 4 + n > at + len {
            break;
        }
        out.push((p + 4, n));
        p += 4 + n;
    }
    out
}

/// Overwrite `f[at..at + len]` with one of the extremes of an integer, a rational, a batch header
/// or a reference.
fn extreme(f: &mut [u8], at: usize, len: usize, r: &mut Rng) {
    let field = &mut f[at..at + len];
    match r.below(8) {
        0 => field.fill(0),
        1 => field.fill(0xFF),
        2 => {
            field.fill(0xFF);
            field[0] = 0x7F;
        }
        3 => {
            field.fill(0);
            field[0] = 0x80;
        }
        // a batch header that promises more items, or longer ones, than there are bytes
        4 if len >= 8 => field[..8].copy_from_slice(&[0x7F, 0xFF, 0xFF, 0xFF, 0, 0, 0, 1]),
        5 if len >= 8 => field[..8].copy_from_slice(&[0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xF0]),
        6 => {
            let i = r.below(len);
            field[i] ^= 1 << r.below(8);
        }
        _ => field.iter_mut().for_each(|b| *b = r.next() as u8),
    }
}

/// Every metadata item, index table item and partition pack field of every kind of written file,
/// set to an extreme value, a few at a time.
#[test]
fn extreme_metadata_index_and_partition_fields_never_panic() {
    let mut r = Rng(0x00C0_FFEE_1234_5678);
    let mut opened = 0;
    for base in bases() {
        assert!(exercise(&base), "the unchanged file opens");
        // local sets (header metadata, index table segments) and partition packs
        let sets: Vec<(usize, usize, usize)> =
            klvs(&base).into_iter().filter(|&(k, _, len)| len > 0 && base[k + 4] == 0x02 && matches!(base[k + 5], 0x53 | 0x05)).collect();
        assert!(sets.len() > 10, "{} packets to damage", sets.len());
        for round in 0..600 {
            let mut f = base.clone();
            for _ in 0..1 + round % 3 {
                let (k, v, len) = sets[r.below(sets.len())];
                if base[k + 5] == 0x53 {
                    let it = items(&base, v, len);
                    let (at, n) = it[r.below(it.len())];
                    if n > 0 {
                        extreme(&mut f, at, n, &mut r);
                    }
                } else {
                    // a fixed pack (partition, primer): a whole 4- or 8-byte field
                    let n = [4, 8][r.below(2)].min(len);
                    extreme(&mut f, v + r.below(len - n + 1), n, &mut r);
                }
            }
            opened += usize::from(exercise(&f));
        }
    }
    assert!(opened > 1200, "most damaged files still open and are read: {opened}");
}

/// KLV lengths that lie (longer than the file, the largest BER lengths, zero) and keys that stop
/// being SMPTE labels.
#[test]
fn lying_klv_lengths_and_lost_keys_never_panic() {
    let mut r = Rng(0xBAD0_BE12_3456_789A);
    for base in bases() {
        let all = klvs(&base);
        for _ in 0..300 {
            let mut f = base.clone();
            for _ in 0..1 + r.below(2) {
                let (k, v, _) = all[r.below(all.len())];
                match r.below(5) {
                    0 => f[k + 16..v].fill(0xFF),
                    1 => f[k + 16..v].fill(0),
                    // an 8-byte BER length of 2^63 and more, written over the packet's start
                    2 if k + 25 <= f.len() => {
                        f[k + 16..k + 25].copy_from_slice(&[0x88, 0x80 | r.next() as u8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
                    }
                    3 => f[k + r.below(16)] ^= 0xFF,
                    _ => f[v - 1] = f[v - 1].wrapping_add(1 + r.below(200) as u8),
                }
            }
            if r.below(3) == 0 {
                f.truncate(r.below(f.len()));
            }
            exercise(&f);
        }
    }
}

#[test]
fn parsers_never_panic_on_any_bytes() {
    let mut r = Rng(0x5EED_0000_0000_0001);
    for _ in 0..20_000 {
        let n = r.below(96);
        let b: Vec<u8> = (0..n).map(|_| [0u8, 0xFF, 0x80, 0x7F, r.next() as u8, r.next() as u8][r.below(6)]).collect();
        let seg = IndexSegment::parse(&b);
        for e in &seg.entries {
            let _ = (e.random_access(), e.is_b());
        }
        let _ = (
            sniff(&b),
            aes3_element_samples(&b),
            klv::ber_length(&b),
            klv::batch_of(&b).len(),
            klv::utf16_of(&b),
            klv::int_of(&b, n.is_multiple_of(2)),
        );
        for (format, channels, bits, frame_bytes) in [
            (SoundFormat::Pcm, r.below(300), r.below(70) as u32, r.below(40)),
            (SoundFormat::Aes3Element, r.below(12), 24, 0),
            (SoundFormat::Pcm, usize::MAX, u32::MAX, usize::MAX),
            (SoundFormat::Other, 2, 16, 4),
        ] {
            if let Ok(planes) = decode_pcm(&b, format, channels, bits, frame_bytes) {
                assert!(planes.iter().all(|p| p.len() <= b.len()), "more samples than bytes");
            }
        }
    }
}

/// A run-in may hold at most 65 535 bytes before the header partition pack.
#[test]
fn a_header_partition_behind_too_long_a_run_in_is_not_found() {
    let file = atom_pcm(None);
    let behind = |n: usize| [vec![0x55u8; n], file.clone()].concat();
    for n in [1, 4096, 65_535] {
        let f = behind(n);
        assert!(sniff(&f[..(n + 64).min(f.len())]), "run-in of {n}");
        let m = open(&f).unwrap();
        assert_eq!(m.run_in, n as u64);
    }
    let f = behind(65_536 + 17);
    assert!(!sniff(&f));
    assert!(matches!(open(&f), Err(Error::NotMxf(_))));
}

/// A sound track has no picture samples: asking it for a random-access sample is not a panic.
#[test]
fn sync_before_on_a_track_without_samples_is_zero() {
    let f = atom_pcm(None);
    let m = open(&f).unwrap();
    assert!(m.tracks[0].samples.is_empty());
    assert_eq!((m.tracks[0].sync_before(0), m.tracks[0].sync_before(7)), (0, 0));
    let f = op1a_avc();
    let m = open(&f).unwrap();
    let v = &m.tracks[0];
    assert_eq!((v.sync_before(3), v.sync_before(4), v.sync_before(usize::MAX)), (0, 4, 8));
}

/// A start timecode at the end of the 64-bit range (a damaged timecode component) formats.
#[test]
fn a_drop_frame_timecode_at_the_end_of_the_range_formats() {
    use super::Timecode;
    for base in [30, 60] {
        for start in [i64::MAX, i64::MAX - 1, i64::MAX / 2, i64::MIN] {
            let text = Timecode { start, rounded_base: base, drop_frame: true }.format();
            assert!(text.contains(';'), "{text}");
        }
    }
    assert_eq!(Timecode { start: 107_892, rounded_base: 30, drop_frame: true }.format(), "01:00:00;00");
}

/// A source that is `.1` long and zero after its real bytes, and counts the bytes read.
struct Sparse(Vec<u8>, u64, std::cell::Cell<u64>);

impl super::ByteSource for Sparse {
    fn len(&self) -> u64 {
        self.1
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.2.set(self.2.get() + buf.len() as u64);
        buf.fill(0);
        let real = self.0.get(offset.min(self.0.len() as u64) as usize..).unwrap_or_default();
        let n = real.len().min(buf.len());
        buf[..n].copy_from_slice(&real[..n]);
        Ok(())
    }
}

/// The header of a written file up to and including its primer pack, then the key of its first
/// metadata set.
fn header_then_a_set_key() -> Vec<u8> {
    let f = atom_pcm(None);
    let all = klvs(&f);
    let primer = all.iter().position(|&(k, _, _)| klv::classify(&klv::Ul(f[k..k + 16].try_into().unwrap())) == klv::KeyClass::Primer).unwrap();
    let (next, _, _) = all[primer + 1];
    assert_eq!(klv::classify(&klv::Ul(f[next..next + 16].try_into().unwrap())), klv::KeyClass::LocalSet);
    f[..next + 16].to_vec()
}

/// A metadata value of more than 64 MiB is refused before it is read or allocated.
#[test]
fn a_metadata_value_past_the_limit_is_refused_unread() {
    let limit = 64u32 << 20;
    let with_len = |len: u32| {
        let mut f = header_then_a_set_key();
        f.push(0x84);
        f.extend(len.to_be_bytes());
        let total = f.len() as u64 + u64::from(len);
        Sparse(f, total, Default::default())
    };
    let src = with_len(limit + 1);
    match open(&src) {
        Err(Error::Invalid(text)) => assert!(text.contains("metadata value"), "{text}"),
        other => panic!("expected the value to be refused, got {:?}", other.map(|m| m.warnings)),
    }
    assert!(src.2.get() < 1 << 20, "{} bytes read", src.2.get());
    // at the limit it is read
    let src = with_len(limit);
    let _ = open(&src);
    assert!(src.2.get() >= u64::from(limit));
}

/// After a key that is no SMPTE label, the next partition pack is looked for in at most 16 MiB.
#[test]
fn lost_sync_scans_a_bounded_stretch_for_the_next_partition() {
    // a written file cut after the key of its essence element, that key damaged
    let f = atom_pcm(None);
    let (essence, _, _) =
        klvs(&f).into_iter().find(|&(k, _, _)| klv::classify(&klv::Ul(f[k..k + 16].try_into().unwrap())) == klv::KeyClass::Essence).unwrap();
    let mut f = f[..essence + 16].to_vec();
    f[essence..].fill(0x11);
    let src = Sparse(f, 1 << 30, Default::default());
    // nothing follows, so the file has no essence: what matters is how much was read to find out
    assert!(matches!(open(&src), Err(Error::Unsupported(_))));
    assert!(src.2.get() > 15 << 20, "the scan ran: {} bytes read", src.2.get());
    assert!(src.2.get() < 18 << 20, "{} bytes read of a 1 GiB file", src.2.get());
}

/// Everything read while opening shares one budget (256 MiB): a file made of metadata sets, each
/// under the 64 MiB value limit, is refused once they pass it instead of being read whole.
#[test]
fn metadata_past_the_total_budget_is_refused() {
    use super::file::walk_within;
    let f = atom_pcm(None);
    let read: u64 = klvs(&f)
        .iter()
        .filter(|&&(k, _, _)| {
            use klv::KeyClass::{IndexSegment, LocalSet, Partition, Primer};
            matches!(klv::classify(&klv::Ul(f[k..k + 16].try_into().unwrap())), Partition { .. } | Primer | LocalSet | IndexSegment)
        })
        .map(|&(_, _, len)| len as u64)
        .sum();
    assert!(read > 1000, "{read} bytes of metadata");
    // exactly what the file holds is enough; one byte less is not
    assert!(walk_within(&f, read).is_ok());
    match walk_within(&f, read - 1) {
        Err(Error::Invalid(text)) => assert!(text.contains("past the limit"), "{text}"),
        other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
    }
    assert!(open(&f).is_ok());
}

/// The identification of a written file is the caller's on every entry point, and the product
/// UID follows the product name.
#[test]
fn every_writer_entry_point_takes_the_identification() {
    use super::write::write_opatom_pcm_as;
    let opts = OpAtomPcm { sample_rate: 48_000, bits: 16, channels: 1, edit_rate: None, ids: PackageIds::from_seed("id", "Id"), timecode: None };
    let named = write_opatom_pcm_as(&opts, &[1, 2, 3], "Acme", "Editor", "9.9").unwrap();
    let m = open(&named).unwrap();
    assert_eq!((m.company_name.as_deref(), m.product_name.as_deref()), (Some("Acme"), Some("Editor")));
    // the defaults, and the same bytes when they are passed explicitly
    let default = write_opatom_pcm(&opts, &[1, 2, 3]).unwrap();
    let m = open(&default).unwrap();
    assert_eq!((m.company_name.as_deref(), m.product_name.as_deref()), (Some("craft-media"), Some("craft-media")));
    assert_eq!(default, write_opatom_pcm_as(&opts, &[1, 2, 3], "craft-media", "craft-media", env!("CARGO_PKG_VERSION")).unwrap());
    // two products of one length differ in the product UID as well as in the name
    let other = write_opatom_pcm_as(&opts, &[1, 2, 3], "Acme", "Edited", "9.9").unwrap();
    assert_eq!(named.len(), other.len());
    let differing = named.iter().zip(&other).filter(|(a, b)| a != b).count();
    assert!(differing > 4, "only the name's {differing} bytes differ");
    // the streaming writer
    let mut cfg = WriterConfig::new(Pattern::OpAtom, Rational::new(24, 1), PackageIds::from_seed("id", "Id"));
    cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 64, 36));
    (cfg.company, cfg.product) = ("Acme".into(), "Editor".into());
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    w.push_picture(&[0, 0, 2, 0x80, 1, 9, 9, 9], FrameInfo::intra()).unwrap();
    let m = open(&w.finish().unwrap().into_inner()).unwrap();
    assert_eq!((m.company_name.as_deref(), m.product_name.as_deref()), (Some("Acme"), Some("Editor")));
}
