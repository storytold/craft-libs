//! Integration and FFmpeg oracle tests for PJS, RealText, and JACOsub legacy caption formats.

use craft_captions::{Cue, Document, Format, TICKS_PER_SECOND, Tick, WriteOptions, detect, parse, write};

fn ms(millis: i64) -> Tick {
    Tick(millis.saturating_mul(TICKS_PER_SECOND / 1000))
}

#[test]
fn test_detect_legacy_formats() {
    let pjs = b"10,50,\"Hello world\"\n60,120,\"Line one|Line two\"\n";
    assert_eq!(detect(pjs, Some("pjs")), Some(Format::Pjs));
    assert_eq!(detect(pjs, None), Some(Format::Pjs));

    let rt = b"<window type=\"generic\" width=\"320\" height=\"60\">\n<time begin=\"00:01.00\" end=\"00:03.00\"/>Hello\n</window>\n";
    assert_eq!(detect(rt, Some("rt")), Some(Format::RealText));
    assert_eq!(detect(rt, None), Some(Format::RealText));

    let jss = b"# JACOsub file\n#TimeRes 100\n0:00:01.00 0:00:03.00 D0 Hello\n";
    assert_eq!(detect(jss, Some("jss")), Some(Format::JacoSub));
    assert_eq!(detect(jss, Some("js")), Some(Format::JacoSub));
    assert_eq!(detect(jss, None), Some(Format::JacoSub));
}

#[test]
fn test_pjs_roundtrip_and_edge_cases() {
    let doc = Document {
        cues: vec![
            Cue { start: ms(1000), end: ms(4500), text: "First caption".into(), ..Default::default() },
            Cue { start: ms(6000), end: ms(10200), text: "Line 1\nLine 2".into(), ..Default::default() },
        ],
        ..Default::default()
    };

    let bytes = write(&doc, Format::Pjs, WriteOptions::default());
    let parsed = parse(&bytes, Format::Pjs).expect("parse pjs");
    assert_eq!(parsed.cues.len(), 2);
    assert_eq!(parsed.cues[0].start, ms(1000));
    assert_eq!(parsed.cues[0].end, ms(4500));
    assert_eq!(parsed.cues[0].text, "First caption");
    assert_eq!(parsed.cues[1].start, ms(6000));
    assert_eq!(parsed.cues[1].end, ms(10200));
    assert_eq!(parsed.cues[1].text, "Line 1\nLine 2");
}

#[test]
fn test_realtext_roundtrip_and_edge_cases() {
    let doc = Document {
        cues: vec![
            Cue { start: ms(2000), end: ms(5500), text: "Welcome to RealText".into(), ..Default::default() },
            Cue { start: ms(7000), end: ms(12000), text: "Multi-line\nRealText caption".into(), ..Default::default() },
        ],
        ..Default::default()
    };

    let bytes = write(&doc, Format::RealText, WriteOptions::default());
    let parsed = parse(&bytes, Format::RealText).expect("parse realtext");
    assert_eq!(parsed.cues.len(), 2);
    assert_eq!(parsed.cues[0].start, ms(2000));
    assert_eq!(parsed.cues[0].end, ms(5500));
    assert_eq!(parsed.cues[0].text, "Welcome to RealText");
    assert_eq!(parsed.cues[1].start, ms(7000));
    assert_eq!(parsed.cues[1].end, ms(12000));
    assert_eq!(parsed.cues[1].text, "Multi-line\nRealText caption");
}

#[test]
fn test_jacosub_roundtrip_and_edge_cases() {
    let doc = Document {
        cues: vec![
            Cue { start: ms(1000), end: ms(4000), text: "First JACOsub cue".into(), ..Default::default() },
            Cue { start: ms(5500), end: ms(9200), text: "Line A\nLine B".into(), ..Default::default() },
        ],
        ..Default::default()
    };

    let bytes = write(&doc, Format::JacoSub, WriteOptions::default());
    let parsed = parse(&bytes, Format::JacoSub).expect("parse jacosub");
    assert_eq!(parsed.cues.len(), 2);
    assert_eq!(parsed.cues[0].start, ms(1000));
    assert_eq!(parsed.cues[0].end, ms(4000));
    assert_eq!(parsed.cues[0].text, "First JACOsub cue");
    assert_eq!(parsed.cues[1].start, ms(5500));
    assert_eq!(parsed.cues[1].end, ms(9200));
    assert_eq!(parsed.cues[1].text, "Line A\nLine B");
}

#[test]
fn test_hostile_inputs_never_panic() {
    for format in [Format::Pjs, Format::RealText, Format::JacoSub] {
        let _ = parse(b"", format);
        let _ = parse(b"   \n\r\n\t  ", format);
        let _ = parse(b"invalid text without any headers or timestamps\nfoo bar baz", format);
        let _ = parse(b"9999999999999999999999999999999999,99999999999999999999,\"overflow\"", format);
        let _ = parse(b"<window><time begin=\"invalid\" end=\"NaN\"/>unclosed", format);
        let _ = parse(b"#TimeRes 0\n@9999999999999999999999 @0 D0 test", format);

        for len in [1, 5, 20, 100, 500] {
            let garbage = vec![0xFE; len];
            let _ = parse(&garbage, format);
        }
    }
}

#[test]
fn test_ffmpeg_oracle_pjs_realtext_jacosub() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("legacy text subtitles oracle") else { return };

    let temp_dir = std::env::temp_dir().join("craft_captions_legacy_test");
    let _ = std::fs::create_dir_all(&temp_dir);

    // 1. PJS Oracle: write PJS -> decode with FFmpeg -> compare cues
    let pjs_doc = Document {
        cues: vec![
            Cue { start: ms(1000), end: ms(4000), text: "PJS oracle cue 1".into(), ..Default::default() },
            Cue { start: ms(5000), end: ms(9000), text: "PJS cue 2 line 1\nPJS cue 2 line 2".into(), ..Default::default() },
        ],
        ..Default::default()
    };
    let pjs_path = temp_dir.join("test.pjs");
    std::fs::write(&pjs_path, write(&pjs_doc, Format::Pjs, WriteOptions::default())).expect("write pjs");

    let ff_pjs_srt =
        std::process::Command::new(&ff).args(["-y", "-i", pjs_path.to_str().expect("path"), "-f", "srt", "-"]).output().expect("ffmpeg pjs").stdout;
    let ff_pjs_doc = parse(&ff_pjs_srt, Format::Srt).expect("parse ff pjs srt");
    assert_eq!(ff_pjs_doc.cues.len(), pjs_doc.cues.len());
    for i in 0..pjs_doc.cues.len() {
        assert_eq!(ff_pjs_doc.cues[i].start, pjs_doc.cues[i].start);
        assert_eq!(ff_pjs_doc.cues[i].end, pjs_doc.cues[i].end);
        assert_eq!(ff_pjs_doc.cues[i].text.trim(), pjs_doc.cues[i].text.trim());
    }

    // 2. RealText Oracle: write RealText -> decode with FFmpeg -> compare cues
    let rt_doc = Document {
        cues: vec![
            Cue { start: ms(1000), end: ms(3000), text: "RealText oracle cue 1".into(), ..Default::default() },
            Cue { start: ms(4500), end: ms(8000), text: "RealText cue 2 line 1\nRealText cue 2 line 2".into(), ..Default::default() },
        ],
        ..Default::default()
    };
    let rt_path = temp_dir.join("test.rt");
    std::fs::write(&rt_path, write(&rt_doc, Format::RealText, WriteOptions::default())).expect("write rt");

    let ff_rt_srt =
        std::process::Command::new(&ff).args(["-y", "-i", rt_path.to_str().expect("path"), "-f", "srt", "-"]).output().expect("ffmpeg rt").stdout;
    let ff_rt_doc = parse(&ff_rt_srt, Format::Srt).expect("parse ff rt srt");
    assert_eq!(ff_rt_doc.cues.len(), rt_doc.cues.len());
    for i in 0..rt_doc.cues.len() {
        assert_eq!(ff_rt_doc.cues[i].start, rt_doc.cues[i].start);
        assert_eq!(ff_rt_doc.cues[i].end, rt_doc.cues[i].end);
        assert_eq!(ff_rt_doc.cues[i].text.trim(), rt_doc.cues[i].text.trim());
    }

    // 3. JACOsub Oracle: write JACOsub -> decode with FFmpeg -> compare cues
    let jss_doc = Document {
        cues: vec![
            Cue { start: ms(1000), end: ms(4000), text: "JACOsub oracle cue 1".into(), ..Default::default() },
            Cue { start: ms(5000), end: ms(9000), text: "JACOsub cue 2 line 1\nJACOsub cue 2 line 2".into(), ..Default::default() },
        ],
        ..Default::default()
    };
    let jss_path = temp_dir.join("test.jss");
    std::fs::write(&jss_path, write(&jss_doc, Format::JacoSub, WriteOptions::default())).expect("write jss");

    let ff_jss_srt =
        std::process::Command::new(&ff).args(["-y", "-i", jss_path.to_str().expect("path"), "-f", "srt", "-"]).output().expect("ffmpeg jss").stdout;
    let ff_jss_doc = parse(&ff_jss_srt, Format::Srt).expect("parse ff jss srt");
    assert_eq!(ff_jss_doc.cues.len(), jss_doc.cues.len());
    for i in 0..jss_doc.cues.len() {
        assert_eq!(ff_jss_doc.cues[i].start, jss_doc.cues[i].start);
        assert_eq!(ff_jss_doc.cues[i].end, jss_doc.cues[i].end);
        assert_eq!(ff_jss_doc.cues[i].text.trim(), jss_doc.cues[i].text.trim());
    }
}
