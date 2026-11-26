//! The legacy text formats (MicroDVD, MPL2, SubViewer 2, VPlayer, SAMI): reading, writing, and
//! ffmpeg as the reference (oracle; skipped when ffmpeg is absent, a failure with
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1`).

use craft_captions::time::FrameRate;
use craft_captions::{Cue, Document, Format, TICKS_PER_SECOND, Tick, WriteOptions, detect, parse, plain_text, write};
use std::process::Command;

const T: i64 = TICKS_PER_SECOND;
const MS: i64 = T / 1000;

fn ms(v: i64) -> Tick {
    Tick(v * MS)
}

/// The five formats, with the ffmpeg format name each is read and written through.
const FORMATS: [(Format, &str, &str); 5] = [
    (Format::MicroDvd, "microdvd", "sub"),
    (Format::Mpl2, "mpl2", "txt"),
    (Format::SubViewer, "subviewer", "sbv"),
    (Format::VPlayer, "vplayer", "txt"),
    (Format::Sami, "sami", "smi"),
];

const MICRODVD: &str = "{1}{1}23.976\n{24}{72}Hello|{y:i}second bold\n{100}{150}{c:$0000ff}red\n{200}{250}a & b < c\n";
const MPL2: &str = "[10][35]Hello|/italic\n[40][60]plain\n[70][90]/a|/b\n";
const SUBVIEWER: &str = "[INFORMATION]\n[TITLE]x\n[END INFORMATION]\n[SUBTITLE]\n[COLF]&HFFFFFF,[STYLE]bd,[SIZE]18,[FONT]Arial\n00:00:01.00,00:00:03.50\nHello[br]world\n\n00:00:04.00,00:00:05.00\nplain\n";
const VPLAYER: &str = "00:00:01.00=Hello|world\n00:00:04.00=\n00:00:05:plain\n00:00:09 later\n";
const SAMI: &str = "<SAMI>\n<HEAD><TITLE>x</TITLE></HEAD>\n<BODY>\n<SYNC Start=1000><P Class=ENCC>Hello<br>world\n<SYNC Start=3500><P Class=ENCC>&nbsp;\n<SYNC Start=4000><P Class=ENCC><i>plain</i> &amp; more\n<SYNC Start=5000><P Class=ENCC>&nbsp;\n</BODY>\n</SAMI>\n";

fn samples() -> [(Format, &'static str); 5] {
    [(Format::MicroDvd, MICRODVD), (Format::Mpl2, MPL2), (Format::SubViewer, SUBVIEWER), (Format::VPlayer, VPLAYER), (Format::Sami, SAMI)]
}

fn run(ff: &std::path::Path, args: &[&str]) {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// What ffmpeg reads from `bytes` as `ffmpeg_format`, as cues.
fn ffmpeg_reads(ff: &std::path::Path, name: &str, ffmpeg_format: &str, ext: &str, bytes: &[u8]) -> Document {
    let dir = craft_media_testkit::fixture_subdir("captions_legacy");
    let (input, out) = (dir.join(format!("{name}.{ext}")), dir.join(format!("{name}.srt")));
    std::fs::write(&input, bytes).expect("write");
    run(ff, &["-f", ffmpeg_format, "-i", input.to_str().expect("path"), out.to_str().expect("path")]);
    parse(&std::fs::read(&out).expect("read"), Format::Srt).expect("parse ffmpeg's srt")
}

/// The lines of a cue's text as plain text, tags and entities gone.
fn plain(c: &Cue) -> String {
    c.text.lines().map(|l| plain_text(l).trim().to_string()).collect::<Vec<_>>().join("\n")
}

#[test]
fn we_read_what_ffmpeg_reads() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    for (format, text) in samples() {
        let (_, name, ext) = FORMATS.iter().copied().find(|f| f.0 == format).expect("format");
        let ours = parse(text.as_bytes(), format).expect("parse");
        let theirs = ffmpeg_reads(&ff, &format!("read_{name}"), name, ext, text.as_bytes());
        assert_eq!(ours.cues.len(), theirs.cues.len(), "{format:?}: {:?} against {:?}", ours.cues, theirs.cues);
        for (i, (o, t)) in ours.cues.iter().zip(&theirs.cues).enumerate() {
            // ffmpeg gives a last caption of a start-only format no end (an enormous one).
            let last = i + 1 == ours.cues.len();
            let open_ended = matches!(format, Format::VPlayer | Format::Sami) && last;
            assert!((o.start.0 - t.start.0).abs() <= MS, "{format:?} start of {:?}: {:?} against {:?}", o.text, o.start, t.start);
            if !open_ended {
                assert!((o.end.0 - t.end.0).abs() <= MS, "{format:?} end of {:?}: {:?} against {:?}", o.text, o.end, t.end);
            }
            assert_eq!(plain(o), plain(t), "{format:?} text of the cue at {:?}", o.start);
        }
    }
}

fn sample_document() -> Document {
    let cue = |a: i64, b: i64, text: &str| Cue { start: ms(a), end: ms(b), text: text.into(), ..Default::default() };
    Document {
        cues: vec![
            cue(1000, 3500, "Hello\n<i>second line</i>"),
            cue(3500, 5000, "straight after, with &amp; and &lt;"),
            cue(7000, 9000, "after a gap [br] { } |"),
            cue(3_600_000, 3_601_500, "an hour in"),
        ],
        ..Default::default()
    }
}

#[test]
fn ffmpeg_reads_what_we_write() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    // ffmpeg's own readers mangle the look-alike characters a writer uses for what a format cannot hold,
    // so the cue that has those is left out here (write_then_read covers it).
    let mut doc = sample_document();
    doc.cues[2].text = "after a gap".into();
    for (format, name, ext) in FORMATS {
        let bytes = write(&doc, format, WriteOptions::default());
        let theirs = ffmpeg_reads(&ff, &format!("write_{name}"), name, ext, &bytes);
        assert_eq!(theirs.cues.len(), doc.cues.len(), "{format:?}: {:?}", String::from_utf8_lossy(&bytes));
        for (t, d) in theirs.cues.iter().zip(&doc.cues) {
            let tolerance = if format == Format::MicroDvd { 45 * MS } else { 10 * MS };
            assert!((t.start.0 - d.start.0).abs() <= tolerance, "{format:?} start of {:?}: {:?}", d.text, t.start);
            // the characters a format cannot hold are written as look-alikes: compare without them
            let bare = |c: &Cue| {
                plain(c)
                    .replace('\u{ff3b}', "[")
                    .replace('\u{ff3d}', "]")
                    .replace("[br]", "")
                    .replace(['{', '}', '|', '\u{ff5b}', '\u{ff5d}', '\u{ff5c}'], "")
                    .replace("  ", " ")
            };
            assert_eq!(bare(t), bare(d), "{format:?}");
        }
    }
}

#[test]
fn write_then_read_keeps_times_and_text() {
    let doc = sample_document();
    for (format, _, _) in FORMATS {
        let bytes = write(&doc, format, WriteOptions::default());
        let back = parse(&bytes, format).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert_eq!(back.cues.len(), doc.cues.len(), "{format:?}");
        let tolerance = match format {
            Format::MicroDvd => 25 * MS,
            Format::Mpl2 => 50 * MS,
            _ => 5 * MS,
        };
        for (b, d) in back.cues.iter().zip(&doc.cues) {
            assert!((b.start.0 - d.start.0).abs() <= tolerance, "{format:?} start {:?} against {:?}", b.start, d.start);
            assert!((b.end.0 - d.end.0).abs() <= tolerance, "{format:?} end {:?} against {:?}", b.end, d.end);
        }
        assert_eq!(back.cues[0].text.lines().count(), 2, "{format:?} keeps the line break");
        assert_eq!(plain(&back.cues[1]), "straight after, with & and <", "{format:?}");
    }
}

#[test]
fn styles_survive_where_the_format_has_them() {
    let doc = Document {
        cues: vec![Cue { start: ms(0), end: ms(2000), text: "plain\n<i>slanted</i>".into(), ..Default::default() }],
        ..Default::default()
    };
    for format in [Format::MicroDvd, Format::Mpl2] {
        let back = parse(&write(&doc, format, WriteOptions::default()), format).expect("parse");
        assert_eq!(back.cues[0].text, "plain\n<i>slanted</i>", "{format:?}");
    }
    // SAMI keeps inline tags too.
    let doc = Document {
        cues: vec![Cue { start: ms(0), end: ms(2000), text: "a <b>bold</b> <i>and</i> <u>more</u>".into(), ..Default::default() }],
        ..Default::default()
    };
    assert_eq!(parse(&write(&doc, Format::Sami, WriteOptions::default()), Format::Sami).expect("parse").cues[0].text, doc.cues[0].text);
}

#[test]
fn microdvd_counts_frames_at_the_given_rate() {
    let doc = Document { cues: vec![Cue { start: Tick(2 * T), end: Tick(4 * T), text: "x".into(), ..Default::default() }], ..Default::default() };
    let bytes = write(&doc, Format::MicroDvd, WriteOptions { rate: Some(FrameRate::FPS_25), ..Default::default() });
    assert_eq!(String::from_utf8(bytes.clone()).expect("utf8"), "{1}{1}25.000\n{50}{100}x\n");
    let back = parse(&bytes, Format::MicroDvd).expect("parse");
    assert_eq!((back.cues[0].start, back.cues[0].end), (Tick(2 * T), Tick(4 * T)));

    for header in ["{1}{1}25.000", "{0}{0}25.000"] {
        let fixture = format!("{header}\n{{50}}{{100}}First cue\n{{100}}{{150}}23.976\n");
        let parsed = parse(fixture.as_bytes(), Format::MicroDvd).expect("parse rate header fixture");
        assert_eq!(parsed.cues.len(), 2, "{header}");
        assert_eq!((parsed.cues[0].start, parsed.cues[0].end), (Tick(2 * T), Tick(4 * T)), "{header}");
        assert_eq!((parsed.cues[1].start, parsed.cues[1].end), (Tick(4 * T), Tick(6 * T)), "{header}");
        assert_eq!(parsed.cues[1].text, "23.976", "{header}");
    }
}

#[test]
fn detection_finds_each_format() {
    for (format, text) in samples() {
        assert_eq!(detect(text.as_bytes(), None), Some(format), "{format:?}");
        let ext = FORMATS.iter().find(|f| f.0 == format).map(|f| f.2);
        assert_eq!(detect(text.as_bytes(), ext), Some(format), "{format:?} with its extension");
    }
    assert_eq!(detect(b"1\n00:00:01,000 --> 00:00:02,000\nx\n", None), Some(Format::Srt));
    // Cue text containing literal SubViewer markers must not hijack SRT detection
    assert_eq!(detect(b"1\n00:00:01,000 --> 00:00:02,000\nHere is a [SUBTITLE] tag\n", None), Some(Format::Srt));
    assert_eq!(detect(b"1\n00:00:01,000 --> 00:00:02,000\n[INFORMATION] about movie\n", None), Some(Format::Srt));
    assert_eq!(detect(b"1\n00:00:01,000 --> 00:00:02,000\n00:00:01.00,00:00:02.00\n", None), Some(Format::Srt));
    assert_eq!(detect(b"1\n00:00:01,000 --> 00:00:02,000\n00:00:01.00,00:00:02.00\n", Some("srt")), Some(Format::Srt));
    // TTML with namespaced sami element is not SAMI
    assert_ne!(detect(b"<sami:tt xmlns:sami=\"http://www.w3.org/ns/ttml\"><head></head><body></body></sami:tt>", None), Some(Format::Sami));
    // Genuine SubViewer with only [SUBTITLE] header
    assert_eq!(detect(b"[SUBTITLE]\n00:00:01.00,00:00:02.00\nText\n", None), Some(Format::SubViewer));
    // Genuine SubViewer with long/truncated [TITLE] line
    let truncated_sv = format!("[INFORMATION]\n[TITLE]{}", "A".repeat(5000));
    assert_eq!(detect(truncated_sv.as_bytes(), None), Some(Format::SubViewer));
    assert_eq!(detect(b"just some prose\n", Some("txt")), None);
}

#[test]
fn other_text_is_not_one_of_these_formats() {
    for (format, _, _) in FORMATS {
        assert!(parse(b"hello there\nthis is not a subtitle file\n", format).is_err(), "{format:?}");
        assert!(parse(b"", format).expect("empty").cues.is_empty(), "{format:?}");
    }
}

#[test]
fn hostile_input_is_an_error_or_a_warning_never_a_panic() {
    let mut inputs: Vec<Vec<u8>> = vec![
        "{".repeat(10_000).into_bytes(),
        "{1}{1}".repeat(2000).into_bytes(),
        "{999999999}{999999999}x\n{1}{99999999999999999999}y\n{5}{3}z\n".into(),
        "[999999999][999999999]x\n[1][2".into(),
        "99999:99:99.99,99999:99:99.99\nx\n\n00:00:01.00,00:00:00.00\ny\n".into(),
        "99999999:00:00=x\n00:60:00=y\n00:00:00.=z\n".into(),
        "<sync start=".repeat(5000).into_bytes(),
        "<SAMI><SYNC Start=999999999999999999999><P>x<SYNC Start=-5>y<SYNC Start=1><<<<&&&&&#99999999999;&#x;".into(),
        format!("<SYNC Start=1>{}", "<font color=\"#fff\">".repeat(10_000)).into_bytes(),
        format!("<SYNC Start=1>{}", "<".repeat(10_000)).into_bytes(),
        vec![0xff, 0xfe, 0x00, 0xd8, 0x00],
        vec![0u8; 1000],
    ];
    // deterministic noise
    let mut x = 0x9e37_79b9u32;
    for len in [10usize, 100, 1000] {
        let alphabet = b"{}[]<>:=,. \n|&#;0123456789abSYNC";
        let noise: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                alphabet[x as usize % alphabet.len()]
            })
            .collect();
        inputs.push(noise);
    }
    for (format, _, _) in FORMATS {
        for input in &inputs {
            if let Ok(doc) = parse(input, format) {
                // what was read writes and reads again
                let again = write(&doc, format, WriteOptions::default());
                parse(&again, format).expect("a written file reads back");
            }
        }
    }
}

#[test]
fn the_cue_limit_applies() {
    let mut text = String::new();
    for i in 0..200 {
        text.push_str(&format!("[{}][{}]x\n", i * 10, i * 10 + 5));
    }
    assert_eq!(parse(text.as_bytes(), Format::Mpl2).expect("parse").cues.len(), 200);
}
