//! Hostile input: streams of every codec filled with start codes and sync words at random,
//! damaged containers, and the public frame-header helpers on arbitrary bytes. Errors and empty
//! tables are fine; a panic (an index out of range, an overflow in a debug build) is not.

use super::tests::{TsMux, pes};
use super::*;

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

/// Random bytes with the patterns the splitters look for scattered through them: start codes of
/// every kind, MPEG audio / ADTS / LOAS / AC-3 sync words, with header bytes at the extremes.
/// At least one byte, at most about `max`.
fn soup(r: &mut Rng, max: usize) -> Vec<u8> {
    let n = 1 + r.below(max);
    let mut v = Vec::with_capacity(n + 16);
    while v.len() < n {
        let edge = [0x00u8, 0xFF, 0x80, 0x7F, r.next() as u8][r.below(5)];
        match r.below(12) {
            0 => v.extend([0, 0, 1, r.next() as u8]),
            1 => v.extend([0, 0, 1, [0x00, 0xB3, 0xB5, 0xB8, 0x09, 0x65, 0x41, 0x06, 0x26, 0x40, 0x46][r.below(11)], edge, edge]),
            2 => v.extend([0, 0, 0, 1, r.next() as u8, edge]),
            3 => v.extend([0xFF, 0xF0 | (r.next() as u8 & 0x0F), r.next() as u8, edge, edge, edge, edge]),
            4 => v.extend([0x0B, 0x77, edge, edge, r.next() as u8, r.next() as u8, edge]),
            5 => v.extend([0x56, 0xE0 | (r.next() as u8 & 0x1F), r.next() as u8]),
            6 => v.extend([edge; 3]),
            _ => v.push(r.next() as u8),
        }
    }
    v
}

/// Opens and reads; returns how many units the tables hold.
fn open_and_read_all(data: &[u8]) -> usize {
    let Ok(f) = open(data) else { return 0 };
    let _ = (f.format.name(), f.pcr_range);
    for (s, st) in f.streams.iter().enumerate() {
        let _ = (st.codec_params(), st.codec.name(), st.pts_range());
        let n = st.units.len();
        // every unit of a small table, a spread of a large one
        for i in (0..n).step_by((n / 64).max(1)).chain(n.checked_sub(1)) {
            let _ = f.read_unit(data, s, i);
        }
    }
    let _ = f.read_unit(data, f.streams.len(), 0);
    f.streams.iter().map(|s| s.units.len()).sum()
}

/// Every stream type the PMT maps, each on its own PID.
const TYPES: [u8; 16] = [0x01, 0x02, 0x03, 0x04, 0x0F, 0x11, 0x1B, 0x24, 0x80, 0x81, 0x83, 0x84, 0x87, 0x90, 0x06, 0x42];

fn soup_ts(r: &mut Rng, m2ts: bool, packets: usize) -> Vec<u8> {
    let mut mux = TsMux::new(m2ts);
    let streams: Vec<(u8, u16)> = TYPES.iter().enumerate().map(|(i, &t)| (t, 0x200 + i as u16)).collect();
    mux.psi(&streams);
    for _ in 0..packets {
        let (_, pid) = streams[r.below(streams.len())];
        let payload = soup(r, 700);
        let pts = (r.below(3) > 0).then(|| r.next() as i64 & ((1 << 33) - 1));
        let dts = pts.filter(|_| r.below(2) == 0).map(|p| p.wrapping_sub(3600) & ((1 << 33) - 1));
        mux.pes(pid, &pes([0xE0, 0xC0, 0xBD, 0xFD][r.below(4)], pts, dts, &payload, r.below(2) == 0));
    }
    mux.out
}

fn soup_ps(r: &mut Rng, mpeg1: bool, packets: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..packets {
        if mpeg1 {
            out.extend([0, 0, 1, 0xBA, 0x21, 0, 1, 0, 1, 0x80, 0, 1]);
        } else {
            out.extend([0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 1, 0x89, 0xC3, 0xF8 | r.below(8) as u8]);
            out.extend(std::iter::repeat_n(0xFF, r.below(8)));
        }
        let mut payload = soup(r, 1500);
        let id = [0xE0, 0xE1, 0xC0, 0xC1, 0xBD, 0xBD, 0xBD, 0xBB, 0xBC, 0xBE, 0xBF][r.below(11)];
        if id == 0xBD {
            // DVD sub-streams: AC-3, DTS, LPCM (every format byte), sub-pictures
            payload[0] = [0x80, 0x88, 0xA0, 0xA1, 0x20, r.next() as u8][r.below(6)];
        }
        let pts = (r.below(3) > 0).then(|| r.next() as i64 & ((1 << 33) - 1));
        out.extend(pes(id, pts, None, &payload, true));
    }
    out.extend([0, 0, 1, 0xB9]);
    out
}

#[test]
fn streams_of_start_codes_and_sync_words_never_panic() {
    let mut r = Rng(0x1234_5678_9ABC_DEF1);
    let (mut ts, mut ps) = (0, 0);
    for round in 0..40 {
        ts += open_and_read_all(&soup_ts(&mut r, round % 2 == 1, 150));
        ps += open_and_read_all(&soup_ps(&mut r, round % 2 == 1, 60));
    }
    // the streams reach the splitters: they are cut into units, not rejected up front
    assert!(ts > 2000 && ps > 500, "{ts} transport stream units, {ps} program stream units");
}

#[test]
fn damaged_containers_never_panic() {
    let mut r = Rng(0xDEAD_BEEF_0BAD_F00D);
    let bases = [soup_ts(&mut r, false, 60), soup_ts(&mut r, true, 60), soup_ps(&mut r, false, 40), soup_ps(&mut r, true, 40)];
    for base in &bases {
        for _ in 0..300 {
            let mut d = base.clone();
            for _ in 0..1 + r.below(24) {
                let at = r.below(d.len());
                // header bytes at their extremes, lengths that run past the packet, lost sync
                d[at] = [0x00, 0xFF, 0x47, 0x01, 0x80, 0xBA, r.next() as u8][r.below(7)];
            }
            match r.below(4) {
                0 => d.truncate(r.below(d.len())),
                // a run cut out of the middle: the packet grid shifts
                1 => {
                    let at = r.below(d.len());
                    d.drain(at..(at + r.below(300)).min(d.len()));
                }
                _ => {}
            }
            open_and_read_all(&d);
        }
    }
}

/// Program-specific information with every length field wrong.
#[test]
fn damaged_psi_sections_never_panic() {
    let mut r = Rng(0x0F0F_1234_5555_AAAA);
    for _ in 0..3000 {
        let mut mux = TsMux::new(false);
        // sections the muxer signs with a valid CRC, so the parsers see the damage
        let mut pat = vec![0x00, 0xB0, 13, 0, 1, 0xC1, 0, 0, 0, 1, 0xE1, 0x00];
        let mut pmt = vec![0x02, 0xB0, 0, 0, 1, 0xC1, 0, 0, 0xE1, 0x00, 0xF0, 0];
        for _ in 0..r.below(6) {
            pmt.extend([TYPES[r.below(TYPES.len())], 0xE2, r.next() as u8, 0xF0 | r.below(16) as u8, r.next() as u8]);
            pmt.extend(soup(&mut r, 12));
        }
        let len = pmt.len() - 3 + 4;
        pmt[1] = 0xB0 | (len >> 8) as u8;
        pmt[2] = len as u8;
        for sec in [&mut pat, &mut pmt] {
            for _ in 0..r.below(3) {
                let at = r.below(sec.len());
                sec[at] = [0x00, 0xFF, 0x0F, 0xF0, r.next() as u8][r.below(5)];
            }
        }
        mux.section(0, &pat);
        mux.section(0x100, &pmt);
        for i in 0..8u16 {
            mux.pes(0x200 + i, &pes(0xE0, Some(0), None, &soup(&mut r, 200), false));
        }
        // enough packets for the sniffer
        open_and_read_all(&mux.out);
    }
}

#[test]
fn frame_header_helpers_never_panic_on_any_bytes() {
    let mut r = Rng(0x5EED_5EED_5EED_5EED);
    let codecs = [Codec::MpegAudio, Codec::AacAdts, Codec::AacLatm, Codec::Ac3, Codec::Eac3, Codec::H264, Codec::LpcmDvd];
    for _ in 0..20_000 {
        let b = soup(&mut r, 24);
        for cut in 0..=b.len() {
            let b = &b[..cut];
            for c in &codecs {
                if let Some(n) = frame_bytes(c, b) {
                    assert!(n > 0, "{c:?}: an empty frame would never advance");
                }
                let _ = frame_info(c, b);
            }
            let _ = (sei_has_recovery_point(b), pes::parse(b), sniff(b));
            let _ = ts::codec_of(b.first().copied().unwrap_or(6), b, cut % 2 == 0);
        }
    }
    // every MPEG audio, ADTS and AC-3 header value in its first four bytes
    for h in 0..=0xFFFFu32 {
        for sync in [[0xFFu8, 0xE0], [0x0B, 0x77]] {
            let b = [sync[0], sync[1] | (h >> 12) as u8, (h >> 4) as u8, (h << 4) as u8, h as u8, (h >> 8) as u8, 0xFF, 0x00];
            for c in &codecs[..5] {
                let _ = (frame_bytes(c, &b), frame_info(c, &b));
            }
        }
    }
}

/// A pack header is looked for behind at most 64 zero bytes.
#[test]
fn a_pack_header_behind_too_many_zero_bytes_is_not_sniffed() {
    let pack = [0u8, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 1, 0x89, 0xC3, 0xF8];
    let behind = |zeros: usize| [vec![0u8; zeros], pack.to_vec()].concat();
    assert_eq!(sniff(&behind(0)), Some(Format::Ps { mpeg1: false }));
    assert_eq!(sniff(&behind(62)), Some(Format::Ps { mpeg1: false }));
    assert_eq!(sniff(&behind(63)), None);
    assert_eq!(sniff(&behind(5000)), None);
    assert!(matches!(open(&behind(5000)), Err(Error::NotMpeg)));
}
