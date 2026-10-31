//! Synthetic Ogg streams: a small page writer (RFC 3533) and Opus-like packets.

use super::*;

#[test]
fn recovery_scan_stops_when_its_shared_budget_is_spent() {
    let source = b"junkOggS".to_vec();
    let mut budget = 3;
    assert_eq!(find_capture(&source, 0, source.len() as u64, &mut budget).unwrap(), None);
    let mut budget = source.len();
    assert_eq!(find_capture(&source, 0, source.len() as u64, &mut budget).unwrap(), Some(4));
}

/// Write one page holding `segments` (lacing already computed by `lace`).
fn page(out: &mut Vec<u8>, flags: u8, granule: i64, serial: u32, seq: u32, lacing: &[u8], body: &[u8]) {
    let start = out.len();
    out.extend_from_slice(b"OggS");
    out.push(0);
    out.push(flags);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&serial.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.push(lacing.len() as u8);
    out.extend_from_slice(lacing);
    out.extend_from_slice(body);
    let crc = crc32(&out[start..]);
    out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
}

fn lace(packets: &[&[u8]], last_continues: bool) -> (Vec<u8>, Vec<u8>) {
    let mut l = Vec::new();
    let mut b = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        let mut n = p.len();
        while n >= 255 {
            l.push(255);
            n -= 255;
        }
        if !(last_continues && i + 1 == packets.len()) {
            l.push(n as u8);
        }
        b.extend_from_slice(p);
    }
    (l, b)
}

fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut h = b"OpusHead".to_vec();
    h.push(1);
    h.push(2);
    h.extend_from_slice(&pre_skip.to_le_bytes());
    h.extend_from_slice(&48_000u32.to_le_bytes());
    h.extend_from_slice(&0i16.to_le_bytes());
    h.push(0);
    h
}

/// A CELT 20 ms packet (960 samples): TOC config 31, code 0.
fn pkt(i: usize, len: usize) -> Vec<u8> {
    let mut p = vec![0xF8];
    p.extend((0..len.saturating_sub(1)).map(|k| (i * 7 + k) as u8));
    p
}

/// Header pages, then pages of 3 packets each; the last page's granule trims `trim` samples.
fn opus_file(npk: usize, pre_skip: u16, trim: i64) -> Vec<u8> {
    let mut f = Vec::new();
    let head = opus_head(pre_skip);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 9, 0, &l, &b);
    let tags = b"OpusTags\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
    let (l, b) = lace(&[&tags], false);
    page(&mut f, 0, 0, 9, 1, &l, &b);
    let pkts: Vec<Vec<u8>> = (0..npk).map(|i| pkt(i, 40 + i % 5)).collect();
    let mut done = 0;
    for (seq, chunk) in (2..).zip(pkts.chunks(3)) {
        done += chunk.len();
        let last = done == npk;
        let mut g = done as i64 * 960;
        if last {
            g -= trim;
        }
        let refs: Vec<&[u8]> = chunk.iter().map(Vec::as_slice).collect();
        let (l, b) = lace(&refs, false);
        page(&mut f, if last { 0x04 } else { 0 }, g, 9, seq, &l, &b);
    }
    f
}

#[test]
fn crc_matches_known_value() {
    // CRC-32/CKSUM parameters without its final XOR (check value 0x765E7680 ^ 0xFFFFFFFF)
    assert_eq!(crc32(b"123456789"), 0x89A1_897F);
    assert_eq!(crc32(b""), 0);
    assert_ne!(crc32(b"OggS"), crc32(b"OggT"));
}

#[test]
fn opus_pages_packets_and_timing() {
    let f = opus_file(10, 312, 500);
    assert!(sniff(&f));
    let o = open(&f).unwrap();
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    let s = o.stream_of(Codec::Opus).unwrap();
    let st = &o.streams[s];
    assert_eq!(st.headers.len(), 2);
    assert_eq!(st.packets.len(), 10);
    assert!(st.saw_eos);
    assert_eq!(o.read_packet(&f, s, 4).unwrap(), pkt(4, 44));
    let t = OpusTiming::of(st, 312);
    assert_eq!(t.durations, vec![960; 10]);
    assert_eq!(t.starts[0], -312);
    assert_eq!(t.starts[9], 9 * 960 - 312);
    // end trimming: the last page granule is 500 short
    assert_eq!(t.total, 10 * 960 - 500 - 312);
}

#[test]
fn packets_spanning_pages() {
    let mut f = Vec::new();
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    let big = pkt(0, 700);
    // first 510 bytes on one page (two 255 lacing values, packet continues), rest on the next
    let (l, b) = (vec![255, 255], big[..510].to_vec());
    page(&mut f, 0, -1, 1, 2, &l, &b);
    let rest = &big[510..];
    let small = pkt(1, 30);
    let (l, b) = lace(&[rest, &small], false);
    page(&mut f, 0x01 | 0x04, 1920, 1, 3, &l, &b);
    let o = open(&f).unwrap();
    let st = &o.streams[0];
    assert_eq!(st.packets.len(), 2);
    assert_eq!(st.packets[0].parts.len(), 2);
    assert_eq!(o.read_packet(&f, 0, 0).unwrap(), big);
    assert_eq!(o.read_packet(&f, 0, 1).unwrap(), small);
    let t = OpusTiming::of(st, 0);
    assert_eq!(t.starts, vec![0, 960]);
    assert_eq!(t.total, 1920);
}

#[test]
fn corrupt_pages_are_skipped_and_truncation_is_safe() {
    let f = opus_file(12, 312, 0);
    // flip a byte inside the third data page's body: that page fails its CRC and is dropped
    let mut g = f.clone();
    let third = g.windows(4).enumerate().filter(|(_, w)| *w == b"OggS").map(|(i, _)| i).nth(4).unwrap();
    g[third + 40] ^= 0xFF;
    let o = open(&g).unwrap();
    assert!(o.warnings.iter().any(|w| w.contains("CRC")), "{:?}", o.warnings);
    assert_eq!(o.streams[0].packets.len(), 9);
    let t = OpusTiming::of(&o.streams[0], 312);
    // the granules after the gap put the later packets at their true positions
    assert_eq!(*t.starts.last().unwrap(), 11 * 960 - 312);
    for cut in 0..f.len() {
        if let Ok(o) = open(&f[..cut].to_vec()) {
            for st in &o.streams {
                let t = OpusTiming::of(st, 312);
                assert_eq!(t.starts.len(), st.packets.len());
            }
        }
    }
    let mut seed = 99u64;
    for _ in 0..300 {
        let mut g = f.clone();
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let at = (seed >> 33) as usize % g.len();
        g[at] = (seed >> 20) as u8;
        if let Ok(o) = open(&g) {
            for st in &o.streams {
                let _ = OpusTiming::of(st, 312);
            }
        }
    }
    assert!(matches!(open(&b"not ogg at all, no capture pattern".to_vec()), Err(Error::NotOgg)));
}

#[test]
fn opus_toc_durations() {
    assert_eq!(opus_packet_samples([0x00, 0], 10), Some(480)); // SILK NB 10 ms
    assert_eq!(opus_packet_samples([0x18, 0], 10), Some(2880)); // SILK NB 60 ms
    assert_eq!(opus_packet_samples([0x78 | 1, 0], 10), Some(1920)); // hybrid 20 ms × 2
    assert_eq!(opus_packet_samples([0x80 | 3, 6], 10), Some(720)); // CELT 2.5 ms × 6
    assert_eq!(opus_packet_samples([0xF8 | 3, 7], 10), None); // 140 ms: invalid
    assert_eq!(opus_packet_samples([0xF8, 0], 0), None);
}

/// An Ogg FLAC file: the first packet (stating `count` more header packets), a VORBIS_COMMENT
/// metadata packet (`comment_flags`: its block header byte), and one audio frame.
fn ogg_flac(count: u16, streaminfo_last: bool, comment_flags: u8) -> Vec<u8> {
    let mut first = vec![0x7F, b'F', b'L', b'A', b'C', 1, 0];
    first.extend_from_slice(&count.to_be_bytes());
    first.extend_from_slice(b"fLaC");
    first.extend_from_slice(&[if streaminfo_last { 0x80 } else { 0 }, 0, 0, 34]);
    let mut info = vec![0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0];
    // 44.1 kHz (0x0AC44), 6 channels (5 = channels less one), 24-bit.
    let packed: u64 = (0x0AC44 << 44) | (5 << 41) | (23 << 36);
    info.extend_from_slice(&packed.to_be_bytes());
    info.extend_from_slice(&[0; 16]);
    first.extend_from_slice(&info);
    let comment = [comment_flags, 0, 0, 8, 4, 0, 0, 0, b'c', b'r', b'a', b'f'];
    let audio = [0xFF, 0xF8, 0x69, 0x08, 0x00];
    let mut file = Vec::new();
    for (i, (flags, packet)) in [(2u8, &first[..]), (0, &comment[..]), (4, &audio[..])].into_iter().enumerate() {
        let (lacing, body) = lace(&[packet], false);
        page(&mut file, flags, 0, 9, i as u32, &lacing, &body);
    }
    file
}

#[test]
fn flac_codec_params_carry_rate_channels_and_every_metadata_block() {
    // The count stated in the first packet, and an unstated count (0) ended by the last-block flag.
    for file in [ogg_flac(1, false, 0x84), ogg_flac(0, false, 0x84)] {
        let ogg = open(&file[..]).unwrap();
        let s = &ogg.streams[0];
        assert_eq!((s.headers.len(), s.packets.len()), (2, 1));
        let p = s.codec_params();
        assert_eq!((p.codec, p.sample_rate, p.channels), (crate::CodecId::Flac, Some(44_100), Some(6)));
        // `fLaC`, STREAMINFO with its header, then the comment block.
        assert_eq!(&p.extradata[..4], b"fLaC");
        assert_eq!(p.extradata.len(), 4 + 4 + 34 + 12);
        assert!(p.extradata.ends_with(b"craf"));
    }
    // An unstated count and no block flagged last: the audio frame ends the headers, so the
    // stream's audio is never held as header packets.
    let ogg = open(&ogg_flac(0, false, 0x04)[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (2, 1));
    // Many tiny metadata packets under an unstated count: the size cap is kept as a running total,
    // so opening stays linear in the packets (summing the kept headers per packet would not).
    let mut file = Vec::new();
    let head = {
        let f = ogg_flac(0, false, 0x04);
        open(&f[..]).unwrap().streams[0].headers[0].clone()
    };
    let (lacing, body) = lace(&[&head[..]], false);
    page(&mut file, 2, 0, 9, 0, &lacing, &body);
    let tiny = [0x04u8];
    for seq in 1..=200u32 {
        let packets: Vec<&[u8]> = vec![&tiny[..]; 250];
        let (lacing, body) = lace(&packets, false);
        page(&mut file, 0, 0, 9, seq, &lacing, &body);
    }
    let (lacing, body) = lace(&[&[0xFF, 0xF8, 0x69, 0x08, 0x00][..]], false);
    page(&mut file, 4, 0, 9, 201, &lacing, &body);
    let ogg = open(&file[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (1 + 200 * 250, 1));
    // STREAMINFO alone, flagged last: the next packet is audio.
    let ogg = open(&ogg_flac(0, true, 0xFF)[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (1, 2));
}
