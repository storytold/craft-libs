//! ADPCM (feature `adpcm`): every family decodes what ffmpeg's encoder writes to ffmpeg's own
//! samples, and ffmpeg decodes what our encoder writes to our own decoder's samples (oracle;
//! skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`); plus the
//! round trip through our own encoder and decoder, and damaged input.

use craft_audio::adpcm::{ima, ms, swf, yamaha};
use std::path::Path;
use std::process::Command;

const RATE: u32 = 22_050;

/// Interleaved test audio: tones, a sweep, noise bursts, silence and a full-scale square.
fn signal(channels: usize, frames: usize) -> Vec<i16> {
    let mut seed = 0x2545_F491u32;
    let mut out = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        for c in 0..channels {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let t = i as f64;
            let v = match (i / 1500) % 5 {
                0 => (t * (0.03 + 0.011 * c as f64)).sin() * 9000.0,
                1 => (t * 0.002 * (1.0 + t / 3000.0)).sin() * 14000.0,
                2 => f64::from(seed % 12000) - 6000.0,
                3 => 0.0,
                _ => {
                    if (i / 40) % 2 == 0 {
                        20000.0
                    } else {
                        -20000.0
                    }
                }
            };
            out.push(v as i16);
        }
    }
    out
}

fn snr(a: &[i16], b: &[i16]) -> f64 {
    let (mut s, mut n) = (0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        s += f64::from(*x).powi(2);
        n += (f64::from(*x) - f64::from(*y)).powi(2);
    }
    10.0 * (s / n.max(1.0)).log10()
}

fn max_diff(a: &[i16], b: &[i16]) -> i32 {
    a.iter().zip(b).map(|(x, y)| (i32::from(*x) - i32::from(*y)).abs()).max().unwrap_or(0)
}

#[test]
fn encode_then_decode_is_close_to_the_input() {
    for channels in [1usize, 2] {
        let x = signal(channels, 20_000);
        // (name, decoded, minimum SNR dB, a few dB under what each reaches): the signal has noise bursts
        // and full-scale square waves, which codes this small follow loosely
        let cases: Vec<(&str, Vec<i16>, f64)> = vec![
            ("ima", ima::decode(&ima::encode(&x, channels, 512).unwrap(), channels, 512).unwrap(), 5.0),
            ("ms", ms::decode(&ms::encode(&x, channels, 512).unwrap(), channels, 512).unwrap(), 18.0),
            ("yamaha", yamaha::decode(&yamaha::encode(&x, channels).unwrap(), channels).unwrap(), 5.0),
            ("swf2", swf::decode(&swf::encode(&x, channels, 2).unwrap(), channels).unwrap(), 1.0),
            ("swf3", swf::decode(&swf::encode(&x, channels, 3).unwrap(), channels).unwrap(), 3.0),
            ("swf4", swf::decode(&swf::encode(&x, channels, 4).unwrap(), channels).unwrap(), 5.0),
            ("swf5", swf::decode(&swf::encode(&x, channels, 5).unwrap(), channels).unwrap(), 7.0),
        ];
        for (name, y, min) in cases {
            assert!(y.len() >= x.len(), "{name}: {} samples for {}", y.len(), x.len());
            let s = snr(&x, &y[..x.len()]);
            println!("{name} {channels}ch: SNR {s:.1} dB");
            assert!(s >= min, "{name} {channels}ch: SNR {s:.1} dB under {min}");
        }
    }
}

#[test]
fn damaged_input_is_an_error_or_decodes_never_panics() {
    let x = signal(2, 4000);
    let streams: Vec<(&str, Vec<u8>)> = vec![
        ("ima", ima::encode(&x, 2, 256).unwrap()),
        ("ms", ms::encode(&x, 2, 256).unwrap()),
        ("yamaha", yamaha::encode(&x, 2).unwrap()),
        ("swf", swf::encode(&x, 2, 4).unwrap()),
    ];
    let mut seed = 99u32;
    for (name, s) in &streams {
        for cut in (0..s.len()).step_by(7) {
            let part = &s[..cut];
            let _ = match *name {
                "ima" => ima::decode(part, 2, 256).map(drop),
                "ms" => ms::decode(part, 2, 256).map(drop),
                "yamaha" => yamaha::decode(part, 2).map(drop),
                _ => swf::decode(part, 2).map(drop),
            };
        }
        for _ in 0..300 {
            let mut d = s.clone();
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let at = seed as usize % d.len();
            d[at] ^= (seed >> 8) as u8 | 1;
            let _ = match *name {
                "ima" => ima::decode(&d, 2, 256).map(drop),
                "ms" => ms::decode(&d, 2, 256).map(drop),
                "yamaha" => yamaha::decode(&d, 2).map(drop),
                _ => swf::decode(&d, 2).map(drop),
            };
        }
    }
    // the impossible layouts
    assert!(ima::decode(&[0; 10], 2, 10).is_err());
    assert!(ima::decode(&[0; 8], 9, 8).is_err());
    assert!(ms::decode(&[0; 8], 3, 8).is_err());
    assert!(ima::encode(&[0; 3], 2, 256).is_err());
    assert!(ima::encode_block(&[0; 3], 1, 256).is_err());
    assert!(ms::encode_block(&[0; 3], 1, 256).is_err());
    // a block whose step index or predictor is out of range
    let mut bad = ima::encode_block(&vec![0; 505], 1, 256).unwrap();
    bad[2] = 200;
    assert!(ima::decode_block(&bad, 1, &mut Vec::new()).is_err());
    let mut bad = swf::encode(&[0; 10], 1, 4).unwrap();
    bad[2] = 0xFF;
    let _ = swf::decode(&bad, 1);
}

#[test]
fn huge_layouts_and_big_jumps_are_handled() {
    // block sizes whose sample counts do not fit in a usize
    assert_eq!(ima::samples_per_block(usize::MAX - 3, 1), None);
    assert_eq!(ms::samples_per_block(usize::MAX, 1), None);
    assert!(ima::encode(&[], 1, usize::MAX - 3).is_err());
    assert!(ms::encode(&[], 1, usize::MAX).is_err());
    // a Flash stream's header holds the step index in six bits: a jump from silence to full scale
    // still decodes to what the encoder's own state says
    for bits in [2u32, 3, 4, 5] {
        let x = [0i16, 32767, -32768, 0, 0];
        let y = swf::decode(&swf::encode(&x, 1, bits).unwrap(), 1).unwrap();
        // the stream has no length: up to seven padding bits can read as one more code
        assert!((x.len()..=x.len() + 3).contains(&y.len()), "{bits} bits: {} samples", y.len());
        assert!(y[1] > 0 && y[2] < y[1], "{bits} bits: {y:?}");
    }
    // padding bits at the end of a stream are not a code: no incomplete stereo frame
    for bits in [2u32, 3, 4, 5] {
        let y = swf::decode(&swf::encode(&[0, 0, 5, -5, 9, 9], 2, bits).unwrap(), 2).unwrap();
        assert_eq!(y.len() % 2, 0, "{bits} bits: {y:?}");
        assert!((6..=8).contains(&y.len()), "{bits} bits: {} samples", y.len());
    }
}

#[test]
fn partial_final_blocks_are_handled() {
    let x = signal(1, 1000);
    // IMA: 256-byte blocks hold 505 samples; 1000 samples is two blocks, the second padded
    let enc = ima::encode(&x, 1, 256).unwrap();
    assert_eq!(enc.len(), 2 * 256);
    assert_eq!(ima::decode(&enc, 1, 256).unwrap().len(), 1010);
    // a stream cut inside the last block decodes as far as it goes
    let cut = ima::decode(&enc[..256 + 4 + 40], 1, 256).unwrap();
    assert_eq!(cut.len(), 505 + 1 + 80);
    let enc = ms::encode(&x, 1, 256).unwrap();
    assert_eq!(ms::decode(&enc, 1, 256).unwrap().len(), 1000);
    assert!(ima::encode(&[], 1, 256).unwrap().is_empty());
    assert!(ms::encode(&[], 1, 256).unwrap().is_empty());
    assert!(yamaha::encode(&[], 1).unwrap().is_empty());
    assert_eq!(yamaha::decode(&yamaha::encode(&[5, 6, 7], 1).unwrap(), 1).unwrap().len(), 4);
}

// ---- the other rounding some ffmpeg versions use for IMA and Flash ADPCM
//
// The IMA recommended practices build the step difference from `step >> 3` and the shifted steps
// of the magnitude bits that are set; some decoders (including older ffmpeg releases) compute
// `((2 * magnitude + 1) * step) >> (bits - 1)` instead, which rounds differently by a few units
// and so drifts apart through the predictor. A stream decodes to ours or to this one.

fn alt_diff(mag: i32, step: i32, bits: u32) -> i32 {
    ((2 * mag + 1) * step) >> (bits - 1)
}

const INDEX_4: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];
const INDEX_5: [i32; 16] = [-1, -1, -1, -1, -1, -1, -1, -1, 1, 2, 4, 6, 8, 10, 13, 16];
const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190,
    209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499,
    2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350,
    22385, 24623, 27086, 29794, 32767,
];

fn index_table(bits: u32) -> &'static [i32] {
    match bits {
        2 => &[-1, 2],
        3 => &[-1, -1, 2, 4],
        4 => &INDEX_4,
        _ => &INDEX_5,
    }
}

/// `ima::decode` with the other rounding.
fn alt_ima_decode(data: &[u8], channels: usize, block_align: usize) -> Vec<i16> {
    let per = ima::samples_per_block(block_align, channels).expect("layout");
    let mut out = Vec::new();
    for block in data.chunks_exact(block_align) {
        let mut pred = vec![0i32; channels];
        let mut idx = vec![0i32; channels];
        let base = out.len();
        out.resize(base + per * channels, 0);
        for ch in 0..channels {
            pred[ch] = i32::from(i16::from_le_bytes([block[4 * ch], block[4 * ch + 1]]));
            idx[ch] = i32::from(block[4 * ch + 2]);
            out[base + ch] = pred[ch] as i16;
        }
        for (g, group) in block[4 * channels..].chunks_exact(4 * channels).enumerate() {
            for (ch, bytes) in group.as_chunks::<4>().0.iter().enumerate() {
                for (k, b) in bytes.iter().enumerate() {
                    for (half, nib) in [b & 0xF, b >> 4].into_iter().enumerate() {
                        let step = STEPS[idx[ch] as usize];
                        let d = alt_diff(i32::from(nib & 7), step, 4);
                        pred[ch] = (pred[ch] + if nib & 8 != 0 { -d } else { d }).clamp(-32768, 32767);
                        idx[ch] = (idx[ch] + INDEX_4[usize::from(nib & 7)]).clamp(0, 88);
                        out[base + (1 + g * 8 + k * 2 + half) * channels + ch] = pred[ch] as i16;
                    }
                }
            }
        }
    }
    out
}

/// `swf::decode` with the other rounding (one stream).
fn alt_swf_decode(data: &[u8], channels: usize) -> Vec<i16> {
    let mut bitpos = 0usize;
    let mut get = |n: usize| -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *data.get(bitpos / 8)?;
            v = (v << 1) | u32::from((byte >> (7 - bitpos % 8)) & 1);
            bitpos += 1;
        }
        Some(v)
    };
    let bits = get(2).expect("header") + 2;
    let mut out = Vec::new();
    loop {
        let mut pred = Vec::new();
        let mut idx = Vec::new();
        for _ in 0..channels {
            let (Some(p), Some(i)) = (get(16), get(6)) else { return out };
            out.push(p as u16 as i16);
            pred.push(i32::from(p as u16 as i16));
            idx.push(i as i32);
        }
        for _ in 1..4096 {
            for ch in 0..channels {
                let Some(code) = get(bits as usize) else { return out };
                let mag = (code & ((1 << (bits - 1)) - 1)) as i32;
                let d = alt_diff(mag, STEPS[idx[ch] as usize], bits);
                pred[ch] = (pred[ch] + if code >> (bits - 1) != 0 { -d } else { d }).clamp(-32768, 32767);
                idx[ch] = (idx[ch] + index_table(bits)[mag as usize]).clamp(0, 88);
                out.push(pred[ch] as i16);
            }
        }
    }
}

/// Assert `theirs` is `ours` or `alt` over their common length (and say which).
fn same_as_either(what: &str, ours: &[i16], alt: &[i16], theirs: &[i16]) {
    let n = ours.len().min(theirs.len());
    let (a, b) = (max_diff(&ours[..n], &theirs[..n]), max_diff(&alt[..n.min(alt.len())], &theirs[..n.min(alt.len())]));
    println!("{what}: largest difference from ours {a}, from the other rounding {b}");
    assert!(a == 0 || b == 0, "{what}: ffmpeg's decode is neither ours ({a}) nor the other rounding ({b})");
}

/// A name, the WAV file around our stream, our decode of it and the other rounding's.
type Stream = (&'static str, Vec<u8>, Vec<i16>, Vec<i16>);

// ---- ffmpeg as the reference

fn ffmpeg(ff: &Path, args: &[&str]) {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn s(p: &Path) -> String {
    p.to_str().expect("path").to_string()
}

fn write_pcm(path: &Path, x: &[i16]) {
    std::fs::write(path, x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).expect("write");
}

fn read_pcm(path: &Path) -> Vec<i16> {
    std::fs::read(path).expect("read").as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

/// The `fmt ` chunk's fields and the `data` chunk of a WAV file.
fn wav_parts(bytes: &[u8]) -> (u16, usize, usize, Vec<u8>) {
    let mut at = 12;
    let (mut tag, mut channels, mut align) = (0, 0, 0);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        if id == b"fmt " {
            tag = u16::from_le_bytes([body[0], body[1]]);
            channels = usize::from(u16::from_le_bytes([body[2], body[3]]));
            align = usize::from(u16::from_le_bytes([body[12], body[13]]));
        } else if id == b"data" {
            return (tag, channels, align, body.to_vec());
        }
        at += 8 + len + (len & 1);
    }
    panic!("no data chunk");
}

/// A WAV file around `data`.
fn wav(tag: u16, channels: usize, align: usize, bits: u16, extra: &[u8], data: &[u8]) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend(tag.to_le_bytes());
    fmt.extend((channels as u16).to_le_bytes());
    fmt.extend(RATE.to_le_bytes());
    fmt.extend((RATE * align as u32 / 500).to_le_bytes());
    fmt.extend((align as u16).to_le_bytes());
    fmt.extend(bits.to_le_bytes());
    if !extra.is_empty() || tag == 0x11 || tag == 2 {
        fmt.extend((extra.len() as u16).to_le_bytes());
        fmt.extend(extra);
    }
    let mut out = b"RIFF".to_vec();
    out.extend((4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend((fmt.len() as u32).to_le_bytes());
    out.extend(fmt);
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

#[test]
fn we_decode_what_ffmpeg_encodes_to_the_samples_ffmpeg_decodes() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("adpcm");
    for channels in [1usize, 2] {
        let x = signal(channels, 30_000);
        let pcm = dir.join(format!("in{channels}.pcm"));
        write_pcm(&pcm, &x);
        let input = ["-f", "s16le", "-ar", "22050", "-ac", if channels == 1 { "1" } else { "2" }, "-i"];
        for (name, codec, align) in [("ima", "adpcm_ima_wav", 512usize), ("ms", "adpcm_ms", 512), ("yamaha", "adpcm_yamaha", 0)] {
            let file = dir.join(format!("{name}{channels}.wav"));
            let mut args: Vec<String> = input.iter().map(|a| a.to_string()).collect();
            args.extend([s(&pcm), "-c:a".into(), codec.into()]);
            if align > 0 {
                args.extend(["-block_size".into(), align.to_string()]);
            }
            args.push(s(&file));
            ffmpeg(&ff, &args.iter().map(String::as_str).collect::<Vec<_>>());
            let back = dir.join(format!("{name}{channels}.out.pcm"));
            ffmpeg(&ff, &["-i", &s(&file), "-f", "s16le", "-acodec", "pcm_s16le", &s(&back)]);
            let theirs = read_pcm(&back);
            let (tag, ch, block, data) = wav_parts(&std::fs::read(&file).expect("read"));
            assert_eq!(ch, channels);
            let ours = match name {
                "ima" => {
                    assert_eq!(tag, 0x11);
                    ima::decode(&data, ch, block).expect("decode")
                }
                "ms" => {
                    assert_eq!(tag, 2);
                    ms::decode(&data, ch, block).expect("decode")
                }
                _ => yamaha::decode(&data, ch).expect("decode"),
            };
            let n = ours.len().min(theirs.len());
            assert!(n >= 29_000 * channels, "{name} {channels}ch: {} ours, {} theirs", ours.len(), theirs.len());
            if name == "ima" {
                same_as_either(&format!("{name} {channels}ch"), &ours, &alt_ima_decode(&data, ch, block), &theirs);
            } else {
                let d = max_diff(&ours[..n], &theirs[..n]);
                println!("{name} {channels}ch: {n} samples, largest difference from ffmpeg's decode {d}");
                assert_eq!(d, 0, "{name} {channels}ch");
            }
            // our encoder is as good as ffmpeg's on the same audio, to within 3 dB
            let mine = match name {
                "ima" => ima::decode(&ima::encode(&x, ch, block).expect("encode"), ch, block).expect("decode"),
                "ms" => ms::decode(&ms::encode(&x, ch, block).expect("encode"), ch, block).expect("decode"),
                _ => yamaha::decode(&yamaha::encode(&x, ch).expect("encode"), ch).expect("decode"),
            };
            let m = x.len().min(mine.len()).min(theirs.len());
            let (a, b) = (snr(&x[..m], &mine[..m]), snr(&x[..m], &theirs[..m]));
            println!("{name} {channels}ch: SNR {a:.1} dB ours, {b:.1} dB ffmpeg's");
            assert!(a + 3.0 >= b, "{name} {channels}ch: our encoder {a:.1} dB against ffmpeg's {b:.1} dB");
        }
    }
}

#[test]
fn ffmpeg_decodes_what_we_encode_to_the_samples_we_decode() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("adpcm");
    for channels in [1usize, 2] {
        let x = signal(channels, 12_000);
        let coefs: Vec<u8> = ms::COEFFICIENTS.iter().flat_map(|&(a, b)| [a as i16, b as i16].into_iter().flat_map(i16::to_le_bytes)).collect();
        let streams: Vec<Stream> = {
            let e = ima::encode(&x, channels, 512).unwrap();
            let per = ima::samples_per_block(512, channels).unwrap() as u16;
            let d = ima::decode(&e, channels, 512).unwrap();
            let d_alt = alt_ima_decode(&e, channels, 512);
            let ima_wav = wav(0x11, channels, 512, 4, &per.to_le_bytes(), &e);
            let e = ms::encode(&x, channels, 512).unwrap();
            let per = ms::samples_per_block(512, channels).unwrap() as u16;
            let d2 = ms::decode(&e, channels, 512).unwrap();
            let mut extra = per.to_le_bytes().to_vec();
            extra.extend(7u16.to_le_bytes());
            extra.extend(&coefs);
            let ms_wav = wav(2, channels, 512, 4, &extra, &e);
            let e = yamaha::encode(&x, channels).unwrap();
            let d3 = yamaha::decode(&e, channels).unwrap();
            let y_wav = wav(0x20, channels, 512 * channels, 4, &[], &e);
            vec![("ima", ima_wav, d, d_alt), ("ms", ms_wav, d2.clone(), d2), ("yamaha", y_wav, d3.clone(), d3)]
        };
        for (name, bytes, ours, alt) in streams {
            let file = dir.join(format!("our_{name}{channels}.wav"));
            std::fs::write(&file, bytes).expect("write");
            let back = dir.join(format!("our_{name}{channels}.pcm"));
            ffmpeg(&ff, &["-i", &s(&file), "-f", "s16le", "-acodec", "pcm_s16le", &s(&back)]);
            let theirs = read_pcm(&back);
            let n = ours.len().min(theirs.len());
            assert!(n + 2048 >= ours.len(), "{name} {channels}ch: ffmpeg decoded {} of {}", theirs.len(), ours.len());
            same_as_either(&format!("{name} {channels}ch, our stream"), &ours, &alt, &theirs);
        }
    }
}

/// The audio tags of an FLV file: each tag's body without its flags byte.
fn flv_audio(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut at = 13;
    let mut out = Vec::new();
    while at + 11 <= bytes.len() {
        let kind = bytes[at];
        let len = (usize::from(bytes[at + 1]) << 16) | (usize::from(bytes[at + 2]) << 8) | usize::from(bytes[at + 3]);
        let body = &bytes[at + 11..(at + 11 + len).min(bytes.len())];
        if kind == 8 && body.len() > 1 {
            assert_eq!(body[0] >> 4, 1, "SWF ADPCM tag");
            out.push(body[1..].to_vec());
        }
        at += 11 + len + 4;
    }
    out
}

/// An FLV file of 44.1 kHz audio tags with these ADPCM bodies.
fn flv(stereo: bool, tags: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"FLV\x01\x04\0\0\0\x09\0\0\0\0".to_vec();
    let flags = (1u8 << 4) | (3 << 2) | (1 << 1) | u8::from(stereo);
    for (i, t) in tags.iter().enumerate() {
        let len = t.len() + 1;
        let ts = (i * 4096 * 1000 / 44_100) as u32;
        out.extend([
            8,
            (len >> 16) as u8,
            (len >> 8) as u8,
            len as u8,
            (ts >> 16) as u8,
            (ts >> 8) as u8,
            ts as u8,
            (ts >> 24) as u8,
            0,
            0,
            0,
            flags,
        ]);
        out.extend(t);
        out.extend((len as u32 + 11).to_be_bytes());
    }
    out
}

#[test]
fn flash_adpcm_matches_ffmpeg_in_both_directions() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("adpcm");
    for channels in [1usize, 2] {
        let x = signal(channels, 4096 * 3);
        let pcm = dir.join(format!("swf_in{channels}.pcm"));
        write_pcm(&pcm, &x);
        // ffmpeg encodes (it needs 44.1 kHz for FLV)
        let file = dir.join(format!("swf{channels}.flv"));
        ffmpeg(&ff, &["-f", "s16le", "-ar", "44100", "-ac", &channels.to_string(), "-i", &s(&pcm), "-c:a", "adpcm_swf", "-f", "flv", &s(&file)]);
        let tags = flv_audio(&std::fs::read(&file).expect("read"));
        assert!(tags.len() >= 3, "{} tags", tags.len());
        let back = dir.join(format!("swf{channels}.out.pcm"));
        ffmpeg(&ff, &["-i", &s(&file), "-f", "s16le", "-acodec", "pcm_s16le", &s(&back)]);
        let theirs = read_pcm(&back);
        let ours: Vec<i16> = tags.iter().flat_map(|t| swf::decode(t, channels).expect("decode")).collect();
        assert_eq!(ours.len(), 4096 * 3 * channels);
        assert_eq!(theirs.len(), 4096 * 3 * channels);
        let alt: Vec<i16> = tags.iter().flat_map(|t| alt_swf_decode(t, channels)).collect();
        same_as_either(&format!("swf {channels}ch"), &ours, &alt, &theirs);
        // ffmpeg decodes ours, one 4096-sample stream per tag
        for bits in [2u32, 3, 4, 5] {
            let tags: Vec<Vec<u8>> = x.chunks(4096 * channels).map(|c| swf::encode(c, channels, bits).expect("encode")).collect();
            let ours: Vec<i16> = tags.iter().flat_map(|t| swf::decode(t, channels).expect("decode")).collect();
            let file = dir.join(format!("our_swf{channels}_{bits}.flv"));
            std::fs::write(&file, flv(channels == 2, &tags)).expect("write");
            let back = dir.join(format!("our_swf{channels}_{bits}.pcm"));
            ffmpeg(&ff, &["-i", &s(&file), "-f", "s16le", "-acodec", "pcm_s16le", &s(&back)]);
            let theirs = read_pcm(&back);
            assert_eq!(ours.len(), 4096 * 3 * channels);
            assert_eq!(theirs.len(), 4096 * 3 * channels);
            let alt: Vec<i16> = tags.iter().flat_map(|t| alt_swf_decode(t, channels)).collect();
            same_as_either(&format!("swf {channels}ch {bits} bits, our stream"), &ours, &alt, &theirs);
        }
    }
}
