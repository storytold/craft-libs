//! SubStation Alpha / Advanced SubStation Alpha: reading, writing, and ffmpeg as the reference
//! (oracle; skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).

use craft_captions::{Cue, Document, Format, Tick, WriteOptions, detect, parse, write};
use std::process::Command;

const T: i64 = craft_captions::TICKS_PER_SECOND;

fn cs(n: i64) -> Tick {
    Tick(n * (T / 100))
}

const SAMPLE: &str = "\u{feff}[Script Info]\n; a comment\nTitle: Test\nScriptType: v4.00+\n\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: Default,Arial\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nComment: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,not a cue\nDialogue: 0,0:00:01.50,0:00:03.25,Default,Bob,0,0,0,,Hello, {\\i1}world{\\i0}\\Nsecond {\\b1}line{\\b0}\nDialogue: 0,1:02:03.04,1:02:04.5,Default,,0,0,0,,{\\pos(10,20)\\c&H0000FF&}Red\\hspace\\nsoft\nDialogue: 0,0:00:09.00,0:00:10.00,Default,,0,0,0,,{\\p1}m 0 0 l 100 0 100 100{\\p0}\n";

#[test]
fn reads_dialogue_lines_with_commas_overrides_and_line_breaks() {
    let doc = parse(SAMPLE.as_bytes(), Format::Ass).unwrap();
    assert_eq!(doc.cues.len(), 2);
    assert_eq!(doc.cues[0].start, cs(150));
    assert_eq!(doc.cues[0].end, cs(325));
    assert_eq!(doc.cues[0].text, "Hello, <i>world</i>\nsecond <b>line</b>");
    assert_eq!(doc.cues[0].speaker.as_deref(), Some("Bob"));
    // 1:02:03.04, a one-digit fraction, an override block dropped, \h and \n.
    assert_eq!(doc.cues[1].start, Tick((3600 + 120 + 3) * T + 4 * (T / 100)));
    assert_eq!(doc.cues[1].end, Tick((3600 + 120 + 4) * T + T / 2));
    assert_eq!(doc.cues[1].text, "Red\u{a0}space soft");
    assert!(doc.warnings.iter().any(|w| w.contains("vector drawing")), "{:?}", doc.warnings);
}

#[test]
fn ssa_v4_files_read_through_their_own_format_line() {
    let ssa = "[Script Info]\nScriptType: v4.00\n\n[V4 Styles]\nFormat: Name\n\n[Events]\nFormat: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: Marked=0,0:00:01.00,0:00:02.00,*Default,,0000,0000,0000,,Old style\n";
    let doc = parse(ssa.as_bytes(), Format::Ass).unwrap();
    assert_eq!((doc.cues[0].start, doc.cues[0].end, doc.cues[0].text.as_str()), (cs(100), cs(200), "Old style"));
}

#[test]
fn write_then_read_keeps_times_text_and_speaker() {
    let cues = vec![
        Cue { start: cs(100), end: cs(250), text: "Line one\n<i>line two</i>".into(), speaker: Some("Ann".into()), ..Default::default() },
        Cue {
            start: Tick(7200 * T + 99 * (T / 100)),
            end: Tick(7203 * T),
            text: "<b>bold</b> and <u>under</u> {braces}".into(),
            ..Default::default()
        },
    ];
    let doc = Document { cues, ..Default::default() };
    let bytes = write(&doc, Format::Ass, WriteOptions::default());
    assert_eq!(detect(&bytes, Some("ass")), Some(Format::Ass));
    let back = parse(&bytes, Format::Ass).unwrap();
    assert_eq!(back.cues[0], doc.cues[0]);
    assert_eq!(back.cues[1].start, doc.cues[1].start);
    // Braces are written full-width, as ASS reads `{` as the start of an override block.
    assert_eq!(back.cues[1].text, "<b>bold</b> and <u>under</u> \u{ff5b}braces\u{ff5d}");
}

#[test]
fn files_without_an_events_section_are_not_ass() {
    assert!(parse(b"[Script Info]\nTitle: x\n", Format::Ass).is_err());
    assert!(parse(b"hello", Format::Ass).is_err());
    assert_eq!(detect(b"hello", Some("ass")), None);
}

#[test]
fn bad_lines_are_skipped_with_a_warning() {
    let text = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,garbage,0:00:02.00,Default,,0,0,0,,x\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,ok\n";
    let doc = parse(text.as_bytes(), Format::Ass).unwrap();
    assert_eq!(doc.cues.len(), 1);
    assert_eq!(doc.warnings.len(), 1);
}

fn ffmpeg_run(ff: &std::path::Path, args: &[&str]) {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

const SRT: &str = "1\n00:00:01,500 --> 00:00:03,250\nHello, <i>world</i>\nsecond <b>line</b>\n\n2\n00:00:04,000 --> 00:00:05,990\nPlain, with a comma\n\n3\n01:00:00,010 --> 01:00:02,999\n<u>Under</u> and <i>italic <b>both</b></i>\n";

#[test]
fn we_read_what_ffmpeg_writes() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("captions_ass");
    let (srt, ass) = (dir.join("in.srt"), dir.join("ffmpeg.ass"));
    std::fs::write(&srt, SRT).expect("write");
    ffmpeg_run(&ff, &["-i", srt.to_str().expect("path"), ass.to_str().expect("path")]);
    let from_ass = parse(&std::fs::read(&ass).expect("read"), Format::Ass).expect("parse ass");
    let from_srt = parse(SRT.as_bytes(), Format::Srt).expect("parse srt");
    assert_eq!(from_ass.cues.len(), from_srt.cues.len());
    for (a, s) in from_ass.cues.iter().zip(&from_srt.cues) {
        // ASS times are centiseconds; the SRT ones round to them.
        assert_eq!(a.start, cs((s.start.0 + T / 200) / (T / 100)), "start of {:?}", s.text);
        assert_eq!(a.end, cs((s.end.0 + T / 200) / (T / 100)), "end of {:?}", s.text);
        assert_eq!(a.text, s.text);
    }
}

#[test]
fn ffmpeg_reads_what_we_write() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("captions_ass");
    let mut doc = parse(SRT.as_bytes(), Format::Srt).expect("parse srt");
    doc.cues.push(Cue { start: cs(400_000), end: cs(400_100), text: r"C:\New\notes\hello and \N \n \h".into(), ..Default::default() });
    let (ass, srt) = (dir.join("ours.ass"), dir.join("back.srt"));
    std::fs::write(&ass, write(&doc, Format::Ass, WriteOptions::default())).expect("write");
    ffmpeg_run(&ff, &["-i", ass.to_str().expect("path"), srt.to_str().expect("path")]);
    let theirs = parse(&std::fs::read(&srt).expect("read"), Format::Srt).expect("parse back");
    assert_eq!(theirs.cues.len(), doc.cues.len());
    for (t, d) in theirs.cues.iter().zip(&doc.cues) {
        assert_eq!(t.start, cs((d.start.0 + T / 200) / (T / 100)));
        assert_eq!(t.end, cs((d.end.0 + T / 200) / (T / 100)));
        assert_eq!(t.text, d.text, "text of the cue at {:?}", d.start);
    }
}

#[test]
fn literal_backslashes_survive_ass_round_trip() {
    let text = r"C:\New\notes\hello and \N \n \h";
    let doc = Document { cues: vec![Cue { start: cs(100), end: cs(200), text: text.into(), ..Default::default() }], ..Default::default() };
    let bytes = write(&doc, Format::Ass, WriteOptions::default());
    assert_eq!(parse(&bytes, Format::Ass).unwrap().cues[0].text, text);
}

#[test]
fn carriage_returns_cannot_create_dialogue_records() {
    let doc = Document {
        cues: vec![Cue {
            start: cs(100),
            end: cs(200),
            text: "first\rsecond\r\nthird".into(),
            speaker: Some("Ann\rOther".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let bytes = write(&doc, Format::Ass, WriteOptions::default());
    assert!(!bytes.contains(&b'\r'));
    let back = parse(&bytes, Format::Ass).unwrap();
    assert_eq!(back.cues.len(), 1);
    assert_eq!(back.cues[0].text, "first\nsecond\nthird");
    assert_eq!(back.cues[0].speaker.as_deref(), Some("Ann Other"));
}
