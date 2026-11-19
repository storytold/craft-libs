//! The Layer III decoder against ffmpeg's float MP3 decoder as the reference, sample for sample.
//!
//! - LAME through ffmpeg (`libmp3lame`): MPEG-1, MPEG-2 and MPEG-2.5 sampling frequencies, mono,
//!   stereo and joint stereo, CBR from 8 to 320 kbit/s and VBR, transients (block switching).
//! - The `lame` command, when installed: CRC-protected frames, forced mid / side and no bit
//!   reservoir.
//! - This crate's own encoder.
//! - What no encoder at hand writes, made by editing LAME's frames so that they stay valid:
//!   intensity stereo (the `mode_extension` bit turned on, which MPEG-1 scalefactors do not
//!   depend on) and mixed blocks (`mixed_block_flag` set on short blocks whose scalefactors take
//!   no bits, so the granule's `part2_3_length` still holds).
//!
//! Each stream's side information is read here as well, to prove what it covers (short blocks,
//! mid / side, CRC).
//!
//! Streams are written without a Xing / Info frame, so ffmpeg decodes every frame and trims
//! nothing: both decoders' outputs line up from the first sample.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use craft_audio::mp3::{Config, Decoder, Encoder, InfoTag, StereoMode};
use craft_audio::mpa::{Mode, parse_header};

const TONES: &str = "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t)";
/// Clicks over a quiet tone: an encoder switches to short blocks around each click.
const CLICKS: &str = "aevalsrc=exprs=if(lt(mod(t\\,0.2)\\,0.004)\\,0.9*sin(2*PI*2500*t)\\,0.05*sin(2*PI*330*t))|if(lt(mod(t+0.1\\,0.2)\\,0.004)\\,0.8*sin(2*PI*1800*t)\\,0.04*sin(2*PI*220*t))";
const NOISE: &str = "anoisesrc=color=pink:amplitude=0.4:seed=7";
/// Two nearly equal channels: an encoder codes them as mid / side.
const CORRELATED: &str = "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.05*sin(2*PI*5000*t)|0.28*sin(2*PI*441*t+0.1)+0.05*sin(2*PI*5000*t)";

/// A stream from ffmpeg's LAME encoder.
struct Lame {
    name: &'static str,
    input: &'static str,
    rate: u32,
    channels: usize,
    /// Encoder arguments after `-c:a libmp3lame`.
    enc: &'static [&'static str],
}

const LAME: &[Lame] = &[
    Lame { name: "joint_44k_128k", input: TONES, rate: 44_100, channels: 2, enc: &["-b:a", "128k"] },
    Lame { name: "joint_44k_correlated", input: CORRELATED, rate: 44_100, channels: 2, enc: &["-b:a", "96k"] },
    Lame { name: "joint_44k_clicks", input: CLICKS, rate: 44_100, channels: 2, enc: &["-b:a", "160k"] },
    Lame { name: "stereo_48k_320k", input: NOISE, rate: 48_000, channels: 2, enc: &["-b:a", "320k", "-joint_stereo", "0"] },
    Lame { name: "mono_32k_48k", input: CLICKS, rate: 32_000, channels: 1, enc: &["-b:a", "48k"] },
    Lame { name: "vbr_44k_noise", input: NOISE, rate: 44_100, channels: 2, enc: &["-q:a", "2"] },
    Lame { name: "lsf_22k_64k", input: CLICKS, rate: 22_050, channels: 2, enc: &["-b:a", "64k"] },
    Lame { name: "lsf_24k_mono_32k", input: TONES, rate: 24_000, channels: 1, enc: &["-b:a", "32k"] },
    Lame { name: "lsf_16k_vbr", input: NOISE, rate: 16_000, channels: 2, enc: &["-q:a", "4"] },
    Lame { name: "mpeg25_12k_24k", input: CLICKS, rate: 12_000, channels: 2, enc: &["-b:a", "24k"] },
    Lame { name: "mpeg25_11k_mono", input: TONES, rate: 11_025, channels: 1, enc: &["-b:a", "16k"] },
    Lame { name: "mpeg25_8k_8k", input: CLICKS, rate: 8_000, channels: 1, enc: &["-b:a", "8k"] },
    // Low bit rates: many short blocks with scalefac_compress 0, for the mixed block edits.
    Lame { name: "mixed_44k_32k", input: CLICKS, rate: 44_100, channels: 2, enc: &["-b:a", "32k"] },
    Lame { name: "mixed_32k_mono_32k", input: CLICKS, rate: 32_000, channels: 1, enc: &["-b:a", "32k"] },
    Lame { name: "mixed_lsf_22k_32k", input: CLICKS, rate: 22_050, channels: 2, enc: &["-b:a", "32k"] },
    // 16 kHz has the band tables of MPEG-2.5's 11.025 and 12 kHz, whose streams give LAME few
    // short blocks to edit.
    Lame { name: "mixed_lsf_16k_32k", input: CLICKS, rate: 16_000, channels: 2, enc: &["-b:a", "32k"] },
];

/// The minimum SNR against ffmpeg. Both decoders compute in f32 with different IMDCT and
/// synthesis factorizations: they agree to 125-132 dB on encoded and edited streams. A wrong
/// table entry, band boundary or stereo factor costs far more: the 8 kHz short-block region
/// boundary at 36 lines instead of 72 measured -5 dB.
const MIN_SNR: f64 = 100.0;

fn generate(out: &Path, cmd: impl FnOnce(&Path) -> Command) -> Option<PathBuf> {
    crate::testkit::fixtures::generate(out, |tmp| cmd(tmp).stdin(Stdio::null()).status().is_ok_and(|s| s.success()))
}

/// The LAME stream for `s`, or `None` (after saying so) when this ffmpeg has no `libmp3lame`; a
/// failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`.
fn lame_stream(ff: &Path, s: &Lame) -> Option<PathBuf> {
    let out = crate::testkit::fixtures_dir("mp3dec").join(format!("{}.mp3", s.name));
    generate(&out, |tmp| {
        let mut c = Command::new(ff);
        c.args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("{}:sample_rate={}", s.input, s.rate))
            .args(["-t", "2", "-ac", &s.channels.to_string(), "-c:a", "libmp3lame"])
            .args(s.enc)
            .args(["-write_xing", "0", "-id3v2_version", "0", "-f", "mp3"])
            .arg(tmp);
        c
    })
    .or_else(|| {
        assert!(!craft_media_testkit::oracles_required(), "could not generate {} (ffmpeg without libmp3lame?)", s.name);
        eprintln!("SKIPPED {}: ffmpeg could not encode it (no libmp3lame?)", s.name);
        None
    })
}

/// The frames of a raw stream.
fn frames(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut p = 0;
    while let Some(h) = data.get(p..).and_then(|b| parse_header(b).ok()) {
        let Some(f) = data.get(p..p + h.frame_bytes).filter(|_| h.frame_bytes > 0) else { break };
        out.push(f);
        p += h.frame_bytes;
    }
    out
}

/// ffmpeg's float decode of `file`, interleaved.
fn ffmpeg_decode(ff: &Path, file: &Path) -> Vec<f32> {
    let o = Command::new(ff).args(["-v", "error", "-c:a", "mp3float", "-f", "mp3", "-i"]).arg(file).args(["-f", "f32le", "-"]).output().unwrap();
    assert!(o.status.success(), "{}: {}", file.display(), String::from_utf8_lossy(&o.stderr));
    o.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

/// Our decode of every frame of `data`, planar.
fn decode_all(data: &[u8], rate: u32, channels: usize) -> Vec<Vec<f32>> {
    let mut dec = Decoder::new();
    let mut out: Vec<Vec<f32>> = vec![Vec::new(); channels];
    for f in frames(data) {
        let d = dec.decode(f).unwrap_or_else(|e| panic!("frame {}: {e}", out[0].len()));
        assert_eq!((d.header.sample_rate, d.channels.len()), (rate, channels));
        for (o, c) in out.iter_mut().zip(d.channels) {
            o.extend(c);
        }
    }
    out
}

/// The worst channel's SNR of `ours` against ffmpeg's decode, which must be as long.
fn snr(ours: &[Vec<f32>], want: &[f32], what: &str) -> f64 {
    let channels = ours.len();
    assert_eq!(want.len() / channels, ours[0].len(), "{what}: sample count");
    let mut worst = f64::INFINITY;
    for (c, ours) in ours.iter().enumerate() {
        let (mut sig, mut err) = (0f64, 0f64);
        for (i, &a) in ours.iter().enumerate() {
            let b = f64::from(want[i * channels + c]);
            sig += b * b;
            err += (f64::from(a) - b).powi(2);
        }
        worst = worst.min(10.0 * (sig / err.max(1e-30)).log10());
    }
    worst
}

fn check(ff: &Path, file: &Path, rate: u32, channels: usize, what: &str) {
    let data = std::fs::read(file).unwrap();
    assert!(frames(&data).len() > 10, "{what}: too few frames");
    let ours = decode_all(&data, rate, channels);
    let snr = snr(&ours, &ffmpeg_decode(ff, file), what);
    println!("{what}: SNR vs ffmpeg {snr:.1} dB");
    assert!(snr >= MIN_SNR, "{what}: SNR {snr:.1} dB < {MIN_SNR}");
}

#[test]
fn lame_streams_match_ffmpeg() {
    let ff = require_ffmpeg!();
    for s in LAME {
        let Some(f) = lame_stream(&ff, s) else { continue };
        check(&ff, &f, s.rate, s.channels, s.name);
    }
}

/// The `lame` command's options ffmpeg does not reach: CRC, forced mid / side, no reservoir.
#[test]
fn lame_cli_streams_match_ffmpeg() {
    let ff = require_ffmpeg!();
    let Some(lame) = crate::testkit::oracle::find_tool("lame") else {
        eprintln!("lame not installed, skipped");
        return;
    };
    let cases: &[(&str, &[&str], u32, usize)] = &[
        ("crc_48k_mono", &["-p", "-b", "96", "-m", "m"], 48_000, 1),
        ("nores_32k", &["--nores", "-b", "80"], 32_000, 2),
        ("forced_ms_44k", &["-m", "f", "-b", "96"], 44_100, 2),
    ];
    for &(name, args, rate, channels) in cases {
        let dir = crate::testkit::fixtures_dir("mp3dec");
        let wav = dir.join(format!("{name}.wav"));
        let input = if channels == 1 { "aevalsrc=exprs=if(lt(mod(t\\,0.2)\\,0.004)\\,0.9\\,0.05)*sin(2*PI*700*t)" } else { CLICKS };
        generate(&wav, |tmp| {
            let mut c = Command::new(&ff);
            c.args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i"])
                .arg(format!("{input}:sample_rate={rate}"))
                .args(["-t", "2", "-c:a", "pcm_s16le", "-f", "wav"])
                .arg(tmp);
            c
        })
        .unwrap();
        let out = dir.join(format!("{name}.mp3"));
        let f = generate(&out, |tmp| {
            let mut c = Command::new(&lame);
            c.args(["--quiet", "-t", "--resample"]).arg(format!("{}", f64::from(rate) / 1000.0)).args(args).arg(&wav).arg(tmp);
            c
        })
        .unwrap_or_else(|| panic!("lame could not encode {name}"));
        if name.starts_with("crc") {
            assert!(parse_header(&std::fs::read(&f).unwrap()).unwrap().crc, "{name}: no CRC");
        }
        check(&ff, &f, rate, channels, name);
    }
}

#[test]
fn our_encoder_streams_match_ffmpeg() {
    let ff = require_ffmpeg!();
    let dir = crate::testkit::fixtures_dir("mp3dec");
    for (rate, channels, kbps, stereo) in [
        (44_100, 2, 192, StereoMode::JointStereo),
        (48_000, 2, 128, StereoMode::Stereo),
        (32_000, 1, 64, StereoMode::Stereo),
        (24_000, 2, 64, StereoMode::JointStereo),
    ] {
        let input: Vec<Vec<f32>> = (0..channels)
            .map(|c| {
                (0..rate as usize).map(|i| 0.4 * (i as f32 * (0.03 + 0.02 * c as f32)).sin() * (1.0 + (i as f32 * 0.0007).sin()) / 2.0).collect()
            })
            .collect();
        let refs: Vec<&[f32]> = input.iter().map(Vec::as_slice).collect();
        let mut enc = Encoder::new(Config { stereo, ..Config::cbr(rate, channels, kbps) }).unwrap();
        let mut data = enc.encode(&refs);
        data.extend(enc.flush());
        let path = dir.join(format!("ours_{rate}_{channels}ch_{kbps}k.mp3"));
        std::fs::write(&path, &data).unwrap();
        check(&ff, &path, rate, channels, &format!("our encoder {rate} Hz {channels} ch {kbps} kbit/s"));
    }
}

/// The streams cover what they are meant to: short blocks (LAME switches on the clicks, at
/// every sampling frequency family), mid / side and CRC.
#[test]
fn streams_cover_short_blocks_and_mid_side() {
    let ff = require_ffmpeg!();
    for s in LAME.iter().filter(|s| s.input == CLICKS) {
        let Some(f) = lame_stream(&ff, s) else { continue };
        let data = std::fs::read(f).unwrap();
        let short = frames(&data).iter().flat_map(|f| granules(f)).filter(|g| g.block_type == 2).count();
        assert!(short > 5, "{}: {short} short-block granules", s.name);
    }
    let s = LAME.iter().find(|s| s.name == "joint_44k_correlated").unwrap();
    let Some(f) = lame_stream(&ff, s) else { return };
    let data = std::fs::read(f).unwrap();
    let ms = frames(&data).iter().filter(|f| parse_header(f).is_ok_and(|h| h.mode == Mode::JointStereo && h.mode_extension & 2 != 0)).count();
    assert!(ms > 10, "{ms} mid/side frames");
}

/// What the side information says of one granule of one channel, and where its
/// `mixed_block_flag` is (a bit offset in the frame) when it switches windows.
struct GranuleSide {
    block_type: u8,
    scalefac_compress: u32,
    mixed_bit: Option<usize>,
}

/// Read the side information of a Layer III frame (ISO/IEC 11172-3 §2.4.1.7): each granule and
/// channel is 59 bits in MPEG-1, 63 in MPEG-2.
fn granules(frame: &[u8]) -> Vec<GranuleSide> {
    let h = parse_header(frame).unwrap();
    let lsf = h.sample_rate < 32_000;
    let nch = h.channels();
    let bit = |p: usize| (frame[p / 8] >> (7 - p % 8)) & 1;
    let field = |p: usize, n: usize| (p..p + n).fold(0u32, |v, q| v << 1 | u32::from(bit(q)));
    let mut p = 32 + if h.crc { 16 } else { 0 };
    p += if lsf { 8 + if nch == 1 { 1 } else { 2 } } else { 9 + if nch == 1 { 5 } else { 3 } + 4 * nch };
    let mut out = Vec::new();
    for _ in 0..if lsf { 1 } else { 2 } {
        for _ in 0..nch {
            let scalefac_compress = field(p + 29, if lsf { 9 } else { 4 });
            let ws = p + if lsf { 38 } else { 33 };
            let (block_type, mixed_bit) = if bit(ws) == 1 { (field(ws + 1, 2) as u8, Some(ws + 3)) } else { (0, None) };
            out.push(GranuleSide { block_type, scalefac_compress, mixed_bit });
            p += if lsf { 63 } else { 59 };
        }
    }
    out
}

/// A granule the mixed block test may edit: its frame, where its `mixed_block_flag` is, whether
/// it is a short block, and whether its scalefactors take no bits.
#[derive(Clone, Copy)]
struct Candidate {
    frame: usize,
    bit: Option<usize>,
    short: bool,
    editable: bool,
}

/// Write `frames` (edited copies) as a stream and compare it with ffmpeg.
fn check_edited(ff: &Path, frames: Vec<Vec<u8>>, rate: u32, channels: usize, name: &str) {
    let path = crate::testkit::fixtures_dir("mp3dec").join(format!("{name}.mp3"));
    std::fs::write(&path, frames.concat()).unwrap();
    check(ff, &path, rate, channels, name);
}

/// Intensity stereo: LAME's MPEG-1 joint stereo frames with the intensity bit of
/// `mode_extension` set. The right channel's zero bands above its last non-zero line become
/// intensity coded with the positions its scalefactors give, in long and short blocks.
#[test]
fn intensity_stereo_matches_ffmpeg() {
    let ff = require_ffmpeg!();
    for name in ["joint_44k_clicks", "joint_44k_correlated", "joint_44k_128k"] {
        let s = LAME.iter().find(|s| s.name == name).unwrap();
        let Some(f) = lame_stream(&ff, s) else { continue };
        let data = std::fs::read(f).unwrap();
        let mut edited = 0;
        let fr: Vec<Vec<u8>> = frames(&data)
            .into_iter()
            .map(|f| {
                let mut f = f.to_vec();
                if parse_header(&f).unwrap().mode == Mode::JointStereo {
                    f[3] |= 0x10;
                    edited += 1;
                }
                f
            })
            .collect();
        assert!(edited > 10, "{name}: {edited} joint stereo frames");
        check_edited(&ff, fr, s.rate, s.channels, &format!("{name}_intensity"));
    }
}

/// Mixed blocks: LAME's short-block granules with `mixed_block_flag` set, at MPEG-1 and MPEG-2
/// sampling frequencies (16 kHz shares its band tables with MPEG-2.5's 11.025 and 12 kHz).
///
/// - Only granules whose scalefactors take no bits (`scalefac_compress` 0) are edited: the mixed
///   layout reads the same zero bits, so the stream stays valid. Editing others would shift every
///   later field of the granule past its `part2_3_length`, and decoders differ on such damage.
/// - Not a granule whose channel's next granule is a short block that stays unmixed. ffmpeg's
///   short-block IMDCT drops the last 6 overlap values of each subband, which is exact after a
///   start or short window (whose tails are zero) but not after a mixed block's long subbands
///   (the normal window's tail is not); ISO/IEC 11172-3 adds the whole overlap, as this decoder
///   does.
/// - Not at 8 kHz, where ffmpeg does not implement mixed blocks.
#[test]
fn mixed_blocks_match_ffmpeg() {
    let ff = require_ffmpeg!();
    for name in ["mixed_44k_32k", "mixed_32k_mono_32k", "mixed_lsf_22k_32k", "mixed_lsf_16k_32k"] {
        let s = LAME.iter().find(|s| s.name == name).unwrap();
        let Some(f) = lame_stream(&ff, s) else { continue };
        let data = std::fs::read(f).unwrap();
        let mut fr: Vec<Vec<u8>> = frames(&data).into_iter().map(<[u8]>::to_vec).collect();
        // each channel's granules in order
        let mut channels: Vec<Vec<Candidate>> = vec![Vec::new(); s.channels];
        for (frame, f) in fr.iter().enumerate() {
            for (i, g) in granules(f).into_iter().enumerate() {
                let short = g.block_type == 2;
                let editable = short && g.scalefac_compress == 0;
                channels[i % s.channels].push(Candidate { frame, bit: g.mixed_bit, short, editable });
            }
        }
        let mut mixed = 0;
        for ch in &channels {
            // from the last granule back, so whether the next one is mixed is already decided
            let mut next_pure_short = false;
            for c in ch.iter().rev() {
                let edit = c.editable && !next_pure_short;
                if let (true, Some(p)) = (edit, c.bit) {
                    fr[c.frame][p / 8] |= 0x80 >> (p % 8);
                    mixed += 1;
                }
                next_pure_short = c.short && !edit;
            }
        }
        println!("{name}: {mixed} granules made mixed");
        assert!(mixed > 5, "{name}: {mixed} mixed granules");
        check_edited(&ff, fr, s.rate, s.channels, &format!("{name}_mixed"));
    }
}

/// A Xing / Info frame with a LAME tag gives the frame count, encoder delay and padding.
#[test]
fn info_tag_of_a_lame_stream() {
    let ff = require_ffmpeg!();
    let out = crate::testkit::fixtures_dir("mp3dec").join("tagged_44k.mp3");
    let f = generate(&out, |tmp| {
        let mut c = Command::new(&ff);
        c.args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=500:sample_rate=44100", "-t", "1"])
            .args(["-c:a", "libmp3lame", "-b:a", "128k", "-id3v2_version", "0", "-f", "mp3"])
            .arg(tmp);
        c
    })
    .or_else(|| {
        assert!(!craft_media_testkit::oracles_required(), "could not write a LAME-tagged stream (ffmpeg without libmp3lame?)");
        None
    });
    let Some(f) = f else {
        eprintln!("SKIPPED: ffmpeg could not encode a LAME-tagged stream (no libmp3lame?)");
        return;
    };
    let data = std::fs::read(f).unwrap();
    let fr = frames(&data);
    let tag = InfoTag::parse(fr[0]).expect("Info frame");
    assert_eq!(tag.frames, Some(fr.len() as u32 - 1));
    let (delay, padding) = tag.delay_padding.expect("LAME tag");
    // 1 s at 44.1 kHz: the frames hold the delay, the audio and the padding
    assert_eq!(u64::from(delay) + 44_100 + u64::from(padding), (fr.len() as u64 - 1) * 1152);
    assert!(InfoTag::parse(fr[1]).is_none());
}
