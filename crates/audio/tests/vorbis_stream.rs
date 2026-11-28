//! Ogg Vorbis through `Decoder`, `decode` and `probe`: packet by packet with the native decoder,
//! timed by the pages' granule positions (Vorbis I §A.2), seekable, and with a damaged packet
//! concealed without moving the audio after it.

use craft_audio::vorbis::decoder::{Decoder as PacketDecoder, decode_ogg};
use craft_audio::{AudioBuffer, ChannelLayout, CodecId, Container, DecodeOptions, Decoder, OggVorbisOptions, decode, encode_ogg_vorbis, probe};
use craft_media::ogg::PageWriter;

/// Vorbis order (§4.3.9) to WAV order for six channels: output channel → stream channel.
const WAV_ORDER_6: [usize; 6] = [0, 2, 1, 5, 3, 4];
/// The same for eight channels (7.1).
const WAV_ORDER_8: [usize; 8] = [0, 2, 1, 7, 5, 6, 3, 4];

fn signal(rate: u32, channels: usize, frames: usize) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|c| {
            (0..frames)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    let tone = (t * (220.0 + 110.0 * c as f32) * std::f32::consts::TAU).sin() * 0.3;
                    // clicks every 3000 frames give the encoder short blocks to switch to
                    tone + if i % 3000 < 6 { 0.5 } else { 0.0 }
                })
                .collect()
        })
        .collect()
}

fn encode(rate: u32, channels: usize, frames: usize, quality: f32) -> Vec<u8> {
    encode_ogg_vorbis(
        &AudioBuffer { sample_rate: rate, channels: signal(rate, channels, frames) },
        &OggVorbisOptions::default().with_quality(quality),
    )
    .expect("encode")
}

/// Every block of a decoder, joined.
fn drain(dec: &mut Decoder<'_>) -> Vec<Vec<f32>> {
    let mut out: Vec<Vec<f32>> = Vec::new();
    while let Some(block) = dec.next_block().expect("block") {
        if out.is_empty() {
            out = vec![Vec::new(); block.channels.len()];
        }
        for (all, c) in out.iter_mut().zip(block.channels) {
            all.extend(c);
        }
    }
    out
}

/// A file's headers, its audio packets and the frames each packet decodes to.
struct Parts {
    headers: Vec<Vec<u8>>,
    packets: Vec<Vec<u8>>,
    frames: Vec<usize>,
    last_granule: i64,
}

fn parts(ogg: &[u8]) -> Parts {
    let file = craft_media::ogg::open(ogg).expect("open");
    let s = &file.streams[0];
    let packets: Vec<Vec<u8>> = (0..s.packets.len()).map(|i| file.read_packet(ogg, 0, i).expect("packet")).collect();
    let mut dec = PacketDecoder::new(&s.headers[0], &s.headers[1], &s.headers[2]).expect("headers");
    let frames = packets.iter().map(|p| dec.decode_packet(p).expect("packet decodes")[0].len()).collect();
    Parts { headers: s.headers.clone(), packets, frames, last_granule: s.last_granule.expect("granule") }
}

/// An Ogg file of `p`'s packets, `per_page` to a page, each page's granule position the frames
/// decoded through it less `shift`, and the last page's `last`.
fn remux(p: &Parts, packets: &[Vec<u8>], per_page: usize, shift: i64, last: i64) -> Vec<u8> {
    let mut w = PageWriter::new(7).with_page_target(1 << 20, per_page);
    let mut out = Vec::new();
    for h in &p.headers {
        out.extend(w.header(h));
    }
    let mut end = 0i64;
    for (packet, &n) in packets.iter().zip(&p.frames) {
        end += n as i64;
        out.extend(w.packet(packet, end - shift));
    }
    out.extend(w.finish(Some(last)));
    out
}

#[test]
fn ogg_vorbis_streams_packet_by_packet_to_the_whole_file_decode() {
    for (rate, channels, quality) in [(44_100u32, 2usize, 5.0f32), (48_000, 1, 2.0), (32_000, 6, 4.0), (32_000, 8, 3.0)] {
        let ogg = encode(rate, channels, 40_000, quality);
        let whole = decode_ogg(&ogg).expect("decode_ogg");
        // the native decoder's output, in WAV order
        let expected: Vec<Vec<f32>> = match channels {
            6 => WAV_ORDER_6.iter().map(|&c| whole.channels[c].clone()).collect(),
            8 => WAV_ORDER_8.iter().map(|&c| whole.channels[c].clone()).collect(),
            _ => whole.channels.clone(),
        };
        let mut dec = Decoder::from_bytes(&ogg, None, &DecodeOptions::default()).expect("open");
        let info = dec.info().clone();
        assert_eq!((info.container, info.codec, info.sample_rate, usize::from(info.channels)), (Container::Ogg, CodecId::Vorbis, rate, channels));
        assert!(dec.length_known());
        assert_eq!(info.frames, 40_000);
        match channels {
            6 => assert_eq!(info.layout, ChannelLayout::SURROUND_5_1),
            8 => assert_eq!(info.layout, ChannelLayout::SURROUND_7_1),
            _ => {}
        }
        assert_eq!(drain(&mut dec), expected, "{rate} Hz, {channels} ch");
        let (decoded_info, buf) = decode(&ogg, Some("ogg")).expect("decode");
        assert_eq!((decoded_info.frames, decoded_info.damaged), (40_000, 0));
        assert_eq!(buf.channels, expected);
        assert_eq!(probe(&ogg, None).expect("probe").frames, 40_000);
    }
}

#[test]
fn a_seek_lands_on_the_frame_asked_for() {
    let ogg = encode(44_100, 2, 50_000, 5.0);
    let whole = decode_ogg(&ogg).expect("decode_ogg").channels;
    let mut dec = Decoder::from_bytes(&ogg, None, &DecodeOptions::default()).expect("open");
    for target in [0u64, 1, 700, 12_345, 31_000, 49_990, 50_000, 3, 60_000] {
        let at = dec.seek(target).expect("seek");
        assert_eq!(at, target.min(50_000));
        let rest = drain(&mut dec);
        let from = at as usize;
        let tail: Vec<Vec<f32>> = whole.iter().map(|c| c[from..].to_vec()).collect();
        if tail[0].is_empty() {
            assert!(rest.is_empty() || rest[0].is_empty(), "nothing after {target}");
        } else {
            assert_eq!(rest, tail, "after a seek to {target}");
        }
    }
}

/// A first page whose granule position is below the frames decoded through it trims the start; a
/// last page's lower granule position trims the end.
#[test]
fn granule_positions_trim_the_start_and_the_end() {
    let ogg = encode(44_100, 2, 30_000, 4.0);
    let p = parts(&ogg);
    // every decoded frame, untrimmed
    let all: Vec<Vec<f32>> = {
        let mut dec = PacketDecoder::new(&p.headers[0], &p.headers[1], &p.headers[2]).expect("headers");
        let mut out = vec![Vec::new(); 2];
        for packet in &p.packets {
            for (a, c) in out.iter_mut().zip(dec.decode_packet(packet).expect("decodes")) {
                a.extend(c);
            }
        }
        out
    };
    let (start, end_cut) = (300i64, 200i64);
    let last = p.last_granule - start - end_cut;
    let trimmed = remux(&p, &p.packets, 4, start, last);
    let expected: Vec<Vec<f32>> = all.iter().map(|c| c[start as usize..(p.last_granule - end_cut) as usize].to_vec()).collect();
    let mut dec = Decoder::from_bytes(&trimmed, None, &DecodeOptions::default()).expect("open");
    assert_eq!(dec.info().frames, last as u64);
    assert_eq!(drain(&mut dec), expected);
    // seeking counts from the trimmed start
    dec.seek(1000).expect("seek");
    let rest = drain(&mut dec);
    assert_eq!(rest[0], expected[0][1000..]);
    // without trimming, the same pages decode to the encoder's length
    let plain = remux(&p, &p.packets, 4, 0, p.last_granule);
    assert_eq!(decode(&plain, None).expect("decode").1.channels, decode_ogg(&ogg).expect("decode_ogg").channels);
}

/// A damaged packet is concealed at its own length: the audio two packets on is the same as
/// without the damage, and the length does not change.
#[test]
fn a_damaged_packet_keeps_the_audio_after_it_in_place() {
    let ogg = encode(44_100, 2, 60_000, 5.0);
    let p = parts(&ogg);
    // a packet between two long blocks, so its own block is long too
    let longest = *p.frames.iter().max().expect("packets");
    let k = (2..p.frames.len() - 3).find(|&k| p.frames[k] == longest && p.frames[k + 1] == longest).expect("a run of long blocks");
    let mut packets = p.packets.clone();
    packets[k][0] |= 1; // not an audio packet any more
    let damaged = remux(&p, &packets, 8, 0, p.last_granule);
    let clean = decode_ogg(&ogg).expect("decode_ogg").channels;
    let (info, buf) = decode(&damaged, None).expect("decode");
    assert_eq!(info.damaged, 1);
    assert_eq!(buf.frames(), clean[0].len());
    let before: usize = p.frames[..k].iter().sum();
    let after: usize = p.frames[..k + 2].iter().sum();
    for (ours, theirs) in buf.channels.iter().zip(&clean) {
        assert_eq!(ours[..before], theirs[..before]);
        assert_eq!(ours[after..], theirs[after..]);
    }
    // A damaged first packet counts as a short block, in the timing as in its concealment: the
    // declared length is the decoded length, and seeking lands where asked.
    let mut packets = p.packets.clone();
    packets[0][0] |= 1;
    let first_damaged = remux(&p, &packets, 8, 0, p.last_granule);
    let (info, buf) = decode(&first_damaged, None).expect("decode");
    assert_eq!(info.damaged, 1);
    assert_eq!(probe(&first_damaged, None).expect("probe").frames, buf.frames() as u64);
    let mut dec = Decoder::from_bytes(&first_damaged, None, &DecodeOptions::default()).expect("open");
    dec.seek(20_000).expect("seek");
    assert_eq!(drain(&mut dec), buf.channels.iter().map(|c| c[20_000..].to_vec()).collect::<Vec<_>>());
}

/// A seek into the packet after one or two damaged packets lands where asked: the damaged packets
/// are concealed at the long block size of the packet before them, in the timing as after the
/// seek, so the audio after the seek is the whole decode's.
#[test]
fn a_seek_past_damaged_packets_keeps_the_audio_in_place() {
    let ogg = encode(44_100, 2, 60_000, 5.0);
    let p = parts(&ogg);
    let longest = *p.frames.iter().max().expect("packets");
    // packets k - 2 to k + 1 all long blocks
    let k = (3..p.frames.len() - 3).find(|&k| (k - 1..=k + 1).all(|j| p.frames[j] == longest)).expect("a run of long blocks");
    for damage in [&[k][..], &[k - 1, k]] {
        let mut packets = p.packets.clone();
        for &d in damage {
            packets[d][0] |= 1; // not an audio packet any more
        }
        let damaged = remux(&p, &packets, 8, 0, p.last_granule);
        let (info, buf) = decode(&damaged, None).expect("decode");
        assert_eq!(info.damaged, damage.len() as u64);
        // a frame inside packet k + 1
        let target = p.frames[..=k].iter().sum::<usize>() + 10;
        let mut dec = Decoder::from_bytes(&damaged, None, &DecodeOptions::default()).expect("open");
        assert_eq!(dec.seek(target as u64).expect("seek"), target as u64);
        assert_eq!(drain(&mut dec), buf.channels.iter().map(|c| c[target..].to_vec()).collect::<Vec<_>>(), "damaged packets {damage:?}");
    }
}

/// A lost page leaves silence in its place: the packets after it keep the places their granule
/// positions give them, the decoded length is the declared one, and a seek past the gap lands
/// where asked.
#[test]
fn a_lost_page_is_silence_and_the_audio_after_it_stays_in_place() {
    let ogg = encode(44_100, 2, 60_000, 5.0);
    let p = parts(&ogg);
    let clean = decode_ogg(&ogg).expect("decode_ogg").channels;
    // eight packets to a page: the page of packets 16 to 23 is lost
    let (lost, per_page) = (16..24, 8);
    let mut w = PageWriter::new(7).with_page_target(1 << 20, per_page);
    let mut file = Vec::new();
    for h in &p.headers {
        file.extend(w.header(h));
    }
    let mut end = 0i64;
    for (k, (packet, &n)) in p.packets.iter().zip(&p.frames).enumerate() {
        end += n as i64;
        if !lost.contains(&k) {
            file.extend(w.packet(packet, end));
        }
    }
    file.extend(w.finish(Some(p.last_granule)));
    let (info, buf) = decode(&file, None).expect("decode");
    assert_eq!(info.frames, p.last_granule as u64);
    assert_eq!(buf.frames(), clean[0].len());
    // the audio before the lost page, and from the second packet after it on (the first overlaps
    // the block before the gap), is the clean decode's
    let before: usize = p.frames[..lost.start].iter().sum();
    let gap_end: usize = p.frames[..lost.end].iter().sum();
    let after: usize = p.frames[..lost.end + 1].iter().sum();
    for (ours, theirs) in buf.channels.iter().zip(&clean) {
        assert_eq!(ours[..before], theirs[..before]);
        assert!(ours[before..gap_end].iter().all(|&sample| sample == 0.0));
        assert_eq!(ours[after..], theirs[after..]);
    }
    let mut dec = Decoder::from_bytes(&file, None, &DecodeOptions::default()).expect("open");
    for target in [before + 100, after + 100] {
        assert_eq!(dec.seek(target as u64).expect("seek"), target as u64);
        assert_eq!(drain(&mut dec), buf.channels.iter().map(|c| c[target..].to_vec()).collect::<Vec<_>>(), "after a seek to {target}");
    }
}

/// A granule position that jumps far past what the file could hold (here by 2^40 frames) is
/// damage, not a lost page: the jump counts as one damaged packet, no silence is made for it, and
/// the audio after it follows on, in the length, the decode and a seek. That holds whether the
/// pages after the jump keep it or come back down, and when the first page jumps. A packet after a
/// jump that is damaged too counts once.
#[test]
fn a_granule_jump_past_what_the_file_holds_is_damage() {
    let ogg = encode(44_100, 2, 60_000, 5.0);
    let p = parts(&ogg);
    let clean = decode_ogg(&ogg).expect("decode_ogg").channels;
    let jump = 1i64 << 40;
    // eight packets to a page: the pages of the packets in `jumped` carry granule positions
    // `jump` higher (the end of the range past the last packet: the last page too)
    let file = |packets: &[Vec<u8>], jumped: &std::ops::Range<usize>| {
        let mut w = PageWriter::new(7).with_page_target(1 << 20, 8);
        let mut file = Vec::new();
        for h in &p.headers {
            file.extend(w.header(h));
        }
        let mut end = 0i64;
        for (k, (packet, &n)) in packets.iter().zip(&p.frames).enumerate() {
            end += n as i64;
            file.extend(w.packet(packet, if jumped.contains(&k) { end + jump } else { end }));
        }
        let last = if jumped.contains(&packets.len()) { p.last_granule + jump } else { p.last_granule };
        file.extend(w.finish(Some(last)));
        file
    };
    // the page of packets 16 to 23 jumps, and every page after it too, or none, or every page but
    // the last; or the first page jumps and none after it does
    let last_page = 8 * ((p.packets.len() - 1) / 8);
    for jumped in [16..usize::MAX, 16..24, 16..last_page, 0..8] {
        let file = file(&p.packets, &jumped);
        let mut dec = Decoder::from_bytes(&file, None, &DecodeOptions::default()).expect("open");
        assert_eq!(dec.info().frames, p.last_granule as u64, "jump on packets {jumped:?}");
        assert_eq!(dec.seek(30_000).expect("seek"), 30_000);
        assert_eq!(drain(&mut dec), clean.iter().map(|c| c[30_000..].to_vec()).collect::<Vec<_>>(), "jump on packets {jumped:?}");
        let (info, buf) = decode(&file, None).expect("decode");
        assert_eq!((info.frames, info.damaged), (p.last_granule as u64, 1), "jump on packets {jumped:?}");
        assert_eq!(buf.channels, clean, "jump on packets {jumped:?}");
    }
    // every page `jump` higher is a stream cut from a longer one: presented from its first packet,
    // with nothing damaged
    let (info, buf) = decode(&file(&p.packets, &(0..usize::MAX)), None).expect("decode");
    assert_eq!((info.frames, info.damaged), (p.last_granule as u64, 0));
    assert_eq!(buf.channels, clean);
    let mut packets = p.packets.clone();
    packets[16][0] |= 1; // not an audio packet any more
    let (info, _) = decode(&file(&packets, &(16..usize::MAX)), None).expect("decode");
    assert_eq!((info.frames, info.damaged), (p.last_granule as u64, 1));
}

#[test]
fn damaged_and_cut_files_are_errors_or_results() {
    let ogg = encode(44_100, 2, 20_000, 3.0);
    // cut anywhere: a result no longer than the file, or an error
    for cut in [ogg.len() / 2, ogg.len() - 1, 4000, 200] {
        if let Ok((_, buf)) = decode(&ogg[..cut], None) {
            assert!(buf.frames() <= 20_000, "cut at {cut}");
        }
    }
    // a setup header of garbage is not decoded
    let p = parts(&ogg);
    let mut broken = Parts { headers: p.headers.clone(), packets: Vec::new(), frames: p.frames.clone(), last_granule: p.last_granule };
    broken.headers[2] = [&b"\x05vorbis"[..], &[0xFF; 64]].concat();
    assert!(decode(&remux(&broken, &p.packets, 8, 0, p.last_granule), None).is_err());
    // every audio packet damaged but the first (which decodes to no frames): an error when
    // decoding reaches the declared length, with or without a lost page's silence before the last
    // page
    let mut damaged = p.packets.clone();
    for packet in damaged.iter_mut().skip(1) {
        packet[0] |= 1; // not an audio packet any more
    }
    for last in [p.last_granule, p.last_granule + 50_000] {
        assert!(decode(&remux(&p, &damaged, 8, 0, last), None).is_err(), "last granule position {last}");
    }
    // headers and no audio
    let empty = remux(&broken, &[], 8, 0, 0);
    let _ = decode(&empty, None);
    // headers and no audio packets, or only the first (which decodes to no frames): no audio
    let headers_only = remux(&p, &[], 8, 0, 0);
    assert!(decode(&headers_only, None).is_err());
    let primer_only = remux(&p, &p.packets[..1], 8, 0, 0);
    assert!(decode(&primer_only, None).is_err());
    let mut decoder = Decoder::from_bytes(&primer_only, None, &DecodeOptions::default()).expect("open primer");
    assert_eq!(decoder.seek(0).expect("seek zero"), 0);
    assert!(decoder.next_block().is_err(), "seeking zero cannot make a primer-only stream valid");
}

/// Two audio pages, the second the last: a first page whose granule position is above the last
/// page's is damaged, not trimmed by the last page. The first page is then placed from 0, the last
/// page keeps its end trimming, and the whole stream decodes.
#[test]
fn a_raised_first_granule_before_the_last_page_is_damage() {
    let ogg = encode(44_100, 2, 20_000, 4.0);
    let p = parts(&ogg);
    let clean = decode_ogg(&ogg).expect("decode_ogg").channels;
    let first_page = p.packets.len() / 2;
    let mut w = PageWriter::new(7).with_page_target(1 << 20, p.packets.len());
    let mut file = Vec::new();
    for h in &p.headers {
        file.extend(w.header(h));
    }
    let mut end = 0i64;
    for (k, (packet, &n)) in p.packets.iter().zip(&p.frames).enumerate() {
        if k == first_page {
            file.extend(w.flush());
        }
        end += n as i64;
        // the first page's granule position is raised past the last page's
        file.extend(w.packet(packet, if k < first_page { end + p.last_granule } else { end }));
    }
    file.extend(w.finish(Some(p.last_granule)));
    let pages = craft_media::ogg::open(&file[..]).expect("open").streams[0].packets.iter().filter(|q| q.granule.is_some()).count();
    assert_eq!(pages, 2);
    let (info, buf) = decode(&file, None).expect("decode");
    assert_eq!((info.frames, info.damaged), (p.last_granule as u64, 1));
    assert_eq!(buf.channels, clean);
}
