//! Synthetic Ogg streams: a small page writer (RFC 3533) and Opus-like packets.

use super::*;

#[test]
fn recovery_scan_charges_what_it_examines_and_says_why_it_stopped() {
    let source = b"junkOggS".to_vec();
    let len = source.len() as u64;
    let mut budget = 3;
    assert_eq!(find_capture(&source, 0, len, &mut budget).unwrap(), Capture::BudgetSpent);
    assert_eq!(budget, 0);
    // found on the last position the budget allows: a capture, not a spent budget
    let mut budget = 5;
    assert_eq!(find_capture(&source, 0, len, &mut budget).unwrap(), Capture::At(4));
    assert_eq!(budget, 0);
    // a nearby capture costs the positions up to it, not a whole read
    let mut budget = 1000;
    assert_eq!(find_capture(&source, 0, len, &mut budget).unwrap(), Capture::At(4));
    assert_eq!(budget, 995);
    let mut budget = 1000;
    assert_eq!(find_capture(&b"junk, no pattern".to_vec(), 0, 16, &mut budget).unwrap(), Capture::End);
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
    // Many small valid blocks: the first MAX_UNSTATED_HEADERS are kept, the rest read as audio.
    let empty_padding = [0x01u8, 0, 0, 0];
    let pages: Vec<Vec<&[u8]>> = (0..200).map(|_| vec![&empty_padding[..]; 250]).collect();
    let ogg = open(&flac_file(&pages)[..]).unwrap();
    assert_eq!((ogg.streams[0].headers.len(), ogg.streams[0].packets.len()), (MAX_UNSTATED_HEADERS, 200 * 250 + 1 - MAX_UNSTATED_HEADERS));
    assert!(ogg.warnings.iter().any(|w| w.contains("FLAC metadata beyond")), "{:?}", ogg.warnings);
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
