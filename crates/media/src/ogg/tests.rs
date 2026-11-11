//! Synthetic Ogg streams: a small page writer (RFC 3533) and Opus-like packets.

use super::*;

#[test]
fn recovery_charges_only_bytes_read_again() {
    let mut r = Recovery { budget: 10, seen: 0 };
    assert!(r.charge(0, 100));
    assert_eq!(r.budget, 10, "a first read is free");
    assert!(r.charge(95, 105));
    assert_eq!((r.budget, r.seen), (5, 105));
    assert!(!r.charge(0, 6));
    assert_eq!(r.budget, 0);
    // a scan of bytes not read before costs nothing; one over bytes already read is charged and
    // says when the budget ran out
    let source = b"junkOggS".to_vec();
    let mut r = Recovery { budget: 0, seen: 0 };
    assert_eq!(find_capture(&source, 0, 8, &mut r).unwrap(), Capture::At(4));
    let mut r = Recovery { budget: 7, seen: 8 };
    assert_eq!(find_capture(&source, 0, 8, &mut r).unwrap(), Capture::BudgetSpent);
    let mut r = Recovery { budget: 8, seen: 8 };
    assert_eq!(find_capture(&source, 0, 8, &mut r).unwrap(), Capture::At(4));
    let mut r = Recovery::new(16);
    assert_eq!(find_capture(&b"junk, no pattern".to_vec(), 0, 16, &mut r).unwrap(), Capture::End);
}

/// A source that counts the bytes read from it.
struct Counted(Vec<u8>, std::cell::Cell<u64>);

impl ByteSource for Counted {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.1.set(self.1.get() + buf.len() as u64);
        self.0.read_at(offset, buf)
    }
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
    opus_file_with(npk, pre_skip, |done, last| done as i64 * 960 - if last { trim } else { 0 })
}

/// Like `opus_file`, with the granule of each data page chosen by `granule(packets_done, is_last_page)`.
fn opus_file_with(npk: usize, pre_skip: u16, granule: impl Fn(usize, bool) -> i64) -> Vec<u8> {
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
        let g = granule(done, last);
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
    // Under an unstated count, a packet is a metadata block only when it has a 4-byte block header
    // and the length that header states: a 1-byte packet ends the metadata.
    let head = open(&ogg_flac(0, false, 0x04)[..]).unwrap().streams[0].headers[0].clone();
    let flac_file = |pages: &[Vec<&[u8]>]| {
        let mut file = Vec::new();
        let (lacing, body) = lace(&[&head[..]], false);
        page(&mut file, 2, 0, 9, 0, &lacing, &body);
        for (seq, packets) in (1..).zip(pages) {
            let (lacing, body) = lace(packets, false);
            page(&mut file, 0, 0, 9, seq, &lacing, &body);
        }
        file
    };
    let ogg = open(&flac_file(&[vec![&[0x04][..], &[0x04, 0, 0, 0][..]]])[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (1, 2));
    // Reserved block types (7–126) are metadata blocks too.
    let ogg = open(&flac_file(&[vec![&[0x07, 0, 0, 1, 9][..], &[0x84, 0, 0, 0][..]]])[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (3, 0));
    // Many small valid blocks: the first MAX_HEADERS are kept, the rest read as audio.
    let empty_padding = [0x01u8, 0, 0, 0];
    let pages: Vec<Vec<&[u8]>> = (0..200).map(|_| vec![&empty_padding[..]; 250]).collect();
    let ogg = open(&flac_file(&pages)[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (MAX_HEADERS, 200 * 250 + 1 - MAX_HEADERS));
    assert!(ogg.warnings.iter().any(|w| w.contains("header packets beyond")), "{:?}", ogg.warnings);
    // STREAMINFO alone, flagged last: the next packet is audio.
    let ogg = open(&ogg_flac(0, true, 0xFF)[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (1, 2));
}

/// Opus header pages for `serial` (with the beginning-of-stream flag), then data pages of one
/// 960-sample packet of `pk_len` bytes each; sequence numbers continue from the header pages.
fn opus_pages(f: &mut Vec<u8>, serial: u32, npk: usize, pk_len: usize, eos: bool) {
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(f, 0x02, 0, serial, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags\x00\x00\x00\x00\x00\x00\x00\x00"], false);
    page(f, 0, 0, serial, 1, &l, &b);
    for i in 0..npk {
        let p = pkt(i, pk_len);
        let (l, b) = lace(&[&p], false);
        let last = eos && i + 1 == npk;
        page(f, if last { 0x04 } else { 0 }, (i as i64 + 1) * 960, serial, i as u32 + 2, &l, &b);
    }
}

fn page_starts(f: &[u8]) -> Vec<usize> {
    f.windows(4).enumerate().filter(|(_, w)| *w == b"OggS").map(|(i, _)| i).collect()
}

#[test]
fn many_damaged_pages_do_not_exhaust_the_recovery_budget() {
    // 6 KB packets: a 2.4 MB file, so a scan that charged a whole 1 MiB read per damaged page
    // would spend the 64 MiB budget long before the end
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 400, 6000, true);
    let starts = page_starts(&f);
    // damage every fourth data page's body: 100 bad pages
    for k in (2..402).step_by(4) {
        f[starts[k] + 30] ^= 0xFF;
    }
    let o = open(&f).unwrap();
    assert_eq!(o.streams[0].packets.len(), 300);
    assert_eq!(o.warnings.iter().filter(|w| w.contains("bad CRC")).count(), 100);
    assert!(!o.warnings.iter().any(|w| w.contains("recovery budget")), "{:?}", o.warnings);
}

#[test]
fn a_page_running_past_the_end_is_skipped_not_the_end_of_the_file() {
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 2, 40, false);
    // a damaged page whose lacing values claim far more body than the file has left
    page(&mut f, 0, 2880, 3, 4, &[255; 5], &[0; 10]);
    let p = pkt(9, 40);
    let (l, b) = lace(&[&p], false);
    page(&mut f, 0x04, 3840, 3, 5, &l, &b);
    let o = open(&f).unwrap();
    assert!(o.warnings.iter().any(|w| w.contains("runs past the end")), "{:?}", o.warnings);
    assert_eq!(o.read_packet(&f, 0, o.streams[0].packets.len() - 1).unwrap(), p);
}

#[test]
fn a_lost_middle_page_drops_the_packet_it_continued() {
    let mut f = Vec::new();
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 1, 1, &l, &b);
    // one packet over three pages: 255 bytes, 255 bytes, then the last 90 with a whole packet
    let big = pkt(0, 600);
    page(&mut f, 0, -1, 1, 2, &[255], &big[..255]);
    page(&mut f, 0x01, -1, 1, 3, &[255], &big[255..510]);
    let small = pkt(1, 30);
    let (l, b) = lace(&[&big[510..], &small], false);
    page(&mut f, 0x01 | 0x04, 1920, 1, 4, &l, &b);
    let mut g = f.clone();
    g[page_starts(&f)[3] + 40] ^= 0xFF; // the middle page fails its CRC
    let o = open(&g).unwrap();
    assert!(o.warnings.iter().any(|w| w.contains("pages missing")), "{:?}", o.warnings);
    assert_eq!(o.streams[0].packets.len(), 1);
    assert_eq!(o.read_packet(&g, 0, 0).unwrap(), small);
}

#[test]
fn chained_links_that_reuse_a_serial_are_separate_streams() {
    let mut f = Vec::new();
    opus_pages(&mut f, 0, 3, 40, true);
    opus_pages(&mut f, 0, 3, 40, true);
    let o = open(&f).unwrap();
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_eq!(o.streams.len(), 2);
    for st in &o.streams {
        assert_eq!((st.codec, st.headers.len(), st.packets.len()), (Codec::Opus, 2, 3));
        assert!(st.headers[0].starts_with(b"OpusHead"));
    }
}

#[test]
fn opus_single_page_clip_is_trimmed_at_the_end() {
    let mut f = Vec::new();
    let head = opus_head(312);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 5, 0, &l, &b);
    let (l, b) = lace(&[b"OpusTags"], false);
    page(&mut f, 0, 0, 5, 1, &l, &b);
    let pkts: Vec<Vec<u8>> = (0..3).map(|i| pkt(i, 40)).collect();
    let refs: Vec<&[u8]> = pkts.iter().map(Vec::as_slice).collect();
    let (l, b) = lace(&refs, false);
    page(&mut f, 0x04, 2000, 5, 2, &l, &b);
    let t = OpusTiming::of(&open(&f).unwrap().streams[0], 312);
    assert_eq!(t.starts, vec![-312, 648, 1608]);
    assert_eq!(t.total, 2000 - 312); // 3 × 960 − 2000 = 880 samples trimmed at the end
}

#[test]
fn hostile_granules_do_not_overflow_opus_timing() {
    for g in [i64::MIN, i64::MIN + 1, i64::MAX, -2] {
        for eos in [false, true] {
            let mut f = Vec::new();
            let head = opus_head(u16::MAX);
            let (l, b) = lace(&[&head], false);
            page(&mut f, 0x02, 0, 5, 0, &l, &b);
            let (l, b) = lace(&[b"OpusTags"], false);
            page(&mut f, 0, 0, 5, 1, &l, &b);
            let (l, b) = lace(&[&pkt(0, 40), &pkt(1, 40)], false);
            page(&mut f, 0, g, 5, 2, &l, &b);
            let (l, b) = lace(&[&pkt(2, 40)], false);
            page(&mut f, if eos { 0x04 } else { 0 }, g, 5, 3, &l, &b);
            let st = &open(&f).unwrap().streams[0];
            let t = OpusTiming::of(st, u32::MAX);
            assert_eq!(t.starts.len(), 3);
            assert!(t.total >= 0);
        }
    }
}

#[test]
fn empty_packets_are_capped_by_the_file_size() {
    // pages of 255 zero lacing values: 255 empty packets in 282 bytes, enough pages to pass the
    // packet allowance of a file this size
    let mut f = Vec::new();
    let head = opus_head(0);
    let (l, b) = lace(&[&head], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let pages = MIN_PACKET_ALLOWANCE / 255 + 10;
    for seq in 1..=pages as u32 {
        page(&mut f, 0, -1, 1, seq, &[0; 255], &[]);
    }
    let o = open(&f).unwrap();
    let kept = o.streams[0].headers.len() + o.streams[0].packets.len();
    assert!(kept <= MIN_PACKET_ALLOWANCE.max(f.len() / 8), "{kept}");
    assert!(o.warnings.iter().any(|w| w.contains("more packets than")), "{:?}", o.warnings);
}

#[test]
fn speex_extra_headers_and_codec_params() {
    let mut first = b"Speex   1.2".to_vec();
    first.resize(80, 0);
    first[36..40].copy_from_slice(&16_000u32.to_le_bytes());
    first[48..52].copy_from_slice(&1u32.to_le_bytes());
    first[68..72].copy_from_slice(&1u32.to_le_bytes()); // one extra header after the comments
    let mut f = Vec::new();
    for (seq, (flags, p)) in [(0x02, &first[..]), (0, b"comments"), (0, b"extra"), (0x04, b"audio")].into_iter().enumerate() {
        let (l, b) = lace(&[p], false);
        page(&mut f, flags, 0, 4, seq as u32, &l, &b);
    }
    let st = &open(&f).unwrap().streams[0];
    assert_eq!((st.codec, st.headers.len(), st.packets.len()), (Codec::Speex, 3, 1));
    let p = st.codec_params();
    assert_eq!((p.codec, p.sample_rate, p.channels), (crate::CodecId::Speex, Some(16_000), Some(1)));
    assert_eq!(p.extradata, first);
}

#[test]
fn theora_codec_params_carry_the_laced_headers() {
    let headers: [&[u8]; 3] = [b"\x80theora-id", b"\x81theora-comments", b"\x82theora-setup"];
    let mut f = Vec::new();
    for (seq, h) in headers.iter().enumerate() {
        let (l, b) = lace(&[h], false);
        page(&mut f, if seq == 0 { 0x02 } else { 0 }, 0, 6, seq as u32, &l, &b);
    }
    let (l, b) = lace(&[b"frame"], false);
    page(&mut f, 0x04, 1, 6, 3, &l, &b);
    let st = &open(&f).unwrap().streams[0];
    let p = st.codec_params();
    assert_eq!(p.codec, crate::CodecId::Theora);
    assert_eq!(Some(p.extradata), crate::xiph_lace(&st.headers));
}

#[test]
fn flac_extradata_flags_exactly_the_last_block() {
    // comment block not flagged last (an audio frame ended the metadata), and flagged last
    for comment_flags in [0x04, 0x84] {
        for count in [0, 1] {
            let p = open(&ogg_flac(count, false, comment_flags)[..]).unwrap().streams[0].codec_params();
            assert_eq!(p.extradata[4] & 0x80, 0, "STREAMINFO is not last");
            assert_eq!(p.extradata[4 + 4 + 34], 0x84, "the comment block is last");
        }
    }
    // STREAMINFO alone is the last block
    let p = open(&ogg_flac(0, true, 0xFF)[..]).unwrap().streams[0].codec_params();
    assert_eq!((p.extradata.len(), p.extradata[4] & 0x80), (4 + 4 + 34, 0x80));
}

#[test]
fn pages_that_fail_their_crc_are_charged_to_the_recovery_budget() {
    // after one good page, fake page headers every 282 bytes, each claiming a 64 KB body: every
    // one is read and hashed before it fails its CRC, so without charging the overlapping reads
    // the file would cost gigabytes of hashing
    let mut f = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    let mut fake = b"OggS\0\0".to_vec();
    fake.resize(26, 0);
    fake.push(255);
    fake.extend_from_slice(&[255; 255]);
    for _ in 0..1500 {
        f.extend_from_slice(&fake);
    }
    let o = open(&f).unwrap();
    assert!(o.warnings.iter().any(|w| w.contains("past the recovery budget")), "{:?}", o.warnings.last());
    let bad = o.warnings.iter().filter(|w| w.contains("bad CRC on page")).count();
    let budget = 2 * f.len() + RECOVERY_ALLOWANCE as usize;
    assert!(bad <= budget / (255 * 255 - 282) + 1, "{bad}");
    assert!(o.warnings.len() <= MAX_DAMAGE_WARNINGS + 2, "{}", o.warnings.len());
}

#[test]
fn damage_warnings_are_capped() {
    let mut w = Warnings::default();
    for i in 0..MAX_DAMAGE_WARNINGS + 10 {
        w.damage(|| format!("damage {i}"));
    }
    w.push("budget".into());
    let list = w.finish();
    assert_eq!(list.len(), MAX_DAMAGE_WARNINGS + 2);
    assert_eq!(list[MAX_DAMAGE_WARNINGS], "budget");
    assert_eq!(list.last().unwrap(), "10 more damaged regions not listed");
}

#[test]
fn damage_in_a_large_file_costs_no_more_than_in_a_small_one() {
    // 1200 pages of 60 KB, every other one with a bad CRC: 72 MB, more than any fixed budget of
    // 64 MiB covers once each damaged page has been read and scanned through
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 1200, 60_000, true);
    let starts = page_starts(&f);
    for k in (2..1202).step_by(2) {
        f[starts[k] + 300] ^= 0xFF;
    }
    let o = open(&f).unwrap();
    assert_eq!(o.streams[0].packets.len(), 600);
    assert!(!o.warnings.iter().any(|w| w.contains("recovery budget")), "{:?}", o.warnings.last());
    // a partial download: 65 MiB of zeros between two runs of pages
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 20, 40, true);
    let cut = page_starts(&f)[12];
    f.splice(cut..cut, std::iter::repeat_n(0, 65 << 20));
    let o = open(&f).unwrap();
    assert_eq!(o.streams[0].packets.len(), 20, "{:?}", o.warnings);
}

#[test]
fn recovery_reads_stay_in_proportion_to_the_file() {
    // 100 KB of fake capture patterns every 5 bytes after one good page: each claims a page whose
    // body overlaps the next ones'
    let mut f = Vec::new();
    let (l, b) = lace(&[&opus_head(0)], false);
    page(&mut f, 0x02, 0, 1, 0, &l, &b);
    for _ in 0..20_000 {
        f.extend_from_slice(b"OggS\0");
    }
    let src = Counted(f, Default::default());
    open(&src).unwrap();
    let read = src.1.get();
    assert!(read <= 3 * src.len() + RECOVERY_ALLOWANCE + (64 << 10), "{read} bytes read from {}", src.len());
}

#[test]
fn a_stray_beginning_of_stream_flag_does_not_split_a_stream() {
    let mut f = Vec::new();
    opus_pages(&mut f, 3, 0, 0, false);
    for i in 0..6u32 {
        let p = pkt(i as usize, 40);
        let (l, b) = lace(&[&p], false);
        let flags = match i {
            2 => 0x02,
            5 => 0x04,
            _ => 0,
        };
        page(&mut f, flags, (i64::from(i) + 1) * 960, 3, i + 2, &l, &b);
    }
    let o = open(&f).unwrap();
    assert_eq!(o.streams.len(), 1);
    assert_eq!((o.streams[0].headers.len(), o.streams[0].packets.len()), (2, 6));
    assert!(o.warnings.iter().any(|w| w.contains("stray beginning-of-stream")), "{:?}", o.warnings);
    // a link that ends without its end-of-stream page, then the next link with the same serial:
    // two streams, and a warning
    let mut f = Vec::new();
    opus_pages(&mut f, 0, 3, 40, false);
    opus_pages(&mut f, 0, 3, 40, true);
    let o = open(&f).unwrap();
    assert_eq!(o.streams.len(), 2);
    assert!(o.warnings.iter().any(|w| w.contains("starts again")), "{:?}", o.warnings);
}

#[test]
fn stated_header_counts_are_capped() {
    // Speex stating u32::MAX extra headers, then 1100 short packets
    let mut first = b"Speex   1.2".to_vec();
    first.resize(80, 0);
    first[68..72].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut f = Vec::new();
    let (l, b) = lace(&[&first], false);
    page(&mut f, 0x02, 0, 4, 0, &l, &b);
    for seq in 1..=5 {
        let (l, b) = lace(&[&b"pk"[..]; 220], false);
        page(&mut f, 0, 0, 4, seq, &l, &b);
    }
    let src = Counted(f, Default::default());
    let o = open(&src).unwrap();
    assert_eq!((o.streams[0].headers.len(), o.streams[0].packets.len()), (MAX_HEADERS, 1101 - MAX_HEADERS));
    assert!(o.warnings.iter().any(|w| w.contains("header packets beyond")), "{:?}", o.warnings);
    // each page once, and each header once more: the first packet is not read twice, and the
    // packet past the cap is not read at all
    let headers: usize = o.streams[0].headers.iter().map(Vec::len).sum();
    assert_eq!(src.1.get(), src.len() + headers as u64);
}

#[test]
fn header_bytes_hold_two_of_the_largest_flac_metadata_blocks() {
    const { assert!(MAX_HEADER_BYTES >= 2 * (4 + (1 << 24) - 1) + (64 << 10)) }
}

#[test]
fn a_scan_span_of_4_gib_still_advances() {
    // zeros but for a capture pattern at 1000, in a source 2^32 + 3 bytes long: a span cast to
    // `usize` before it was clamped would be 0 on a 32-bit target, and the scan would not move
    struct Sparse;
    impl ByteSource for Sparse {
        fn len(&self) -> u64 {
            (1 << 32) + 3
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = b"OggS".get((offset + i as u64).wrapping_sub(1000) as usize).copied().unwrap_or(0);
            }
            Ok(())
        }
    }
    let mut r = Recovery::new(Sparse.len());
    assert_eq!(find_capture(&Sparse, 0, Sparse.len(), &mut r).unwrap(), Capture::At(1000));
}

#[test]
fn theora_codec_params_carry_the_picture_size() {
    // identification header: version 3.2.1, 120×68 macroblocks, a 1920×1080 picture
    let mut id = b"\x80theora\x03\x02\x01".to_vec();
    id.extend_from_slice(&120u16.to_be_bytes());
    id.extend_from_slice(&68u16.to_be_bytes());
    id.extend_from_slice(&1920u32.to_be_bytes()[1..]);
    id.extend_from_slice(&1080u32.to_be_bytes()[1..]);
    id.resize(42, 0);
    let mut f = Vec::new();
    for (seq, h) in [&id[..], b"\x81theora", b"\x82theora"].into_iter().enumerate() {
        let (l, b) = lace(&[h], false);
        page(&mut f, if seq == 0 { 0x02 } else { 0 }, 0, 6, seq as u32, &l, &b);
    }
    let p = open(&f).unwrap().streams[0].codec_params();
    assert_eq!((p.width, p.height), (Some(1920), Some(1080)));
}

/// Bit-at-a-time CRC of RFC 3533 §6 (polynomial 0x04C11DB7, initial value 0, no reflection, no final XOR).
fn crc32_bitwise(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

#[test]
fn crc_matches_bitwise_reference_on_every_length_and_alignment() {
    let mut seed = 7u64;
    let buf: Vec<u8> = (0..70_100)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u8
        })
        .collect();
    // every short length (covers the 16-byte main loop and every tail length), at several start offsets
    for start in 0..4 {
        for len in 0..=64 {
            let d = &buf[start..start + len];
            assert_eq!(crc32(d), crc32_bitwise(0, d), "start {start} len {len}");
        }
    }
    for len in [65, 255, 4096, 65_307, 70_000] {
        for start in [0, 1, 3] {
            let d = &buf[start..start + len];
            assert_eq!(crc32(d), crc32_bitwise(0, d), "start {start} len {len}");
        }
    }
    // all-zero and all-ones input
    assert_eq!(crc32(&[0u8; 100]), 0);
    assert_eq!(crc32(&[0xFFu8; 100]), crc32_bitwise(0, &[0xFFu8; 100]));
}

#[test]
fn crc_update_continues_across_any_split() {
    let data: Vec<u8> = (0..200u32).map(|i| (i * 31 + 5) as u8).collect();
    let whole = crc32(&data);
    for cut in 0..=data.len() {
        let (a, b) = data.split_at(cut);
        assert_eq!(crc32_update(crc32_update(0, a), b), whole, "cut {cut}");
        assert_eq!(crc32_update(crc32_bitwise(0, a), b), whole, "cut {cut} (reference head)");
    }
}

#[test]
fn read_packet_checks_its_ranges_against_the_source_before_allocating() {
    let f = opus_file(10, 312, 500);
    let o = open(&f[..]).unwrap();
    let s = o.stream_of(Codec::Opus).unwrap();
    // The last packet lies past the end of a source cut short: an error, not a read of missing bytes.
    let short = &f[..f.len() / 2];
    assert!(matches!(o.read_packet(short, s, 9), Err(Error::Invalid(_))));
}

#[test]
fn opus_timing_includes_packets_after_the_last_known_granule() {
    let f = opus_file(6, 312, 0);
    let o = open(&f[..]).unwrap();
    let mut stream = o.streams[0].clone();
    // Missing granules on complete-packet pages are non-conforming. The recovery fallback still
    // assigns their packet positions; the total must include the same samples.
    stream.packets.last_mut().unwrap().granule = None;
    let timing = OpusTiming::of(&stream, 312);
    assert_eq!(timing.starts, vec![-312, 648, 1608, 2568, 3528, 4488]);
    assert_eq!(timing.total, 6 * 960 - 312);

    // With no usable granule, every complete packet still contributes its TOC duration.
    for packet in &mut stream.packets {
        packet.granule = None;
    }
    let timing = OpusTiming::of(&stream, 312);
    assert_eq!(timing.starts, vec![-312, 648, 1608, 2568, 3528, 4488]);
    assert_eq!(timing.total, 6 * 960 - 312);
}

/// The timing must be defined, bounded and panic-free for any granule a file can carry.
fn assert_sane_timing(t: &OpusTiming, npk: usize) {
    assert_eq!(t.starts.len(), npk);
    // treated as unknown granules: the packets simply follow one another from the start
    for (k, s) in t.starts.iter().enumerate() {
        assert_eq!(*s, k as i64 * 960 - 312, "packet {k}");
    }
    // as for a truncated file with no usable granule, the total stays 0: it never exceeds what the packets add up to
    assert!((0..=npk as i64 * 960).contains(&t.total), "total {}", t.total);
}

#[test]
fn opus_timing_ignores_a_granule_near_i64_max() {
    // panicked with "attempt to add with overflow" in debug builds, wrapped in release
    let f = opus_file_with(10, 312, |done, _| i64::MAX - 10 + done as i64 / 3);
    let o = open(&f).unwrap();
    assert_sane_timing(&OpusTiming::of(&o.streams[0], 312), 10);
}

#[test]
fn opus_timing_ignores_a_granule_of_i64_min() {
    // panicked with "attempt to subtract with overflow" in debug builds
    for g in [i64::MIN, i64::MIN + 1, -2] {
        let f = opus_file_with(10, 312, |_, _| g);
        let o = open(&f).unwrap();
        assert_sane_timing(&OpusTiming::of(&o.streams[0], 312), 10);
    }
}

#[test]
fn opus_timing_with_a_bad_granule_on_one_page_recovers_on_the_next() {
    // pages of 3 packets: the second page's granule is absurd, the others are right
    let f = opus_file_with(9, 0, |done, _| if done == 6 { i64::MAX } else { done as i64 * 960 });
    let o = open(&f).unwrap();
    let t = OpusTiming::of(&o.streams[0], 0);
    let expect: Vec<i64> = (0..9).map(|k| k * 960).collect();
    assert_eq!(t.starts, expect);
    assert_eq!(t.total, 9 * 960);
}

#[test]
fn largest_trusted_granule_with_a_huge_pre_skip_stays_in_range() {
    let f = opus_file_with(6, 0, |done, _| MAX_OPUS_GRANULE - 6 * 960 + done as i64 * 960);
    let o = open(&f).unwrap();
    let t = OpusTiming::of(&o.streams[0], u32::MAX);
    // shift = base (the first raw start, 2^62 - 5760) + u32::MAX; every start is relative to it
    let expect: Vec<i64> = (0..6).map(|k| k * 960 - u32::MAX as i64).collect();
    assert_eq!(t.starts, expect);
    // the end (2^62) lies before the shift, so nothing is playable
    assert_eq!(t.total, 0);
}
