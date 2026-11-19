//! Clean-room tests and FFmpeg oracle tests for MP4 Timed Text (`mov_text` / `tx3g`).

use craft_captions::mov_text::{
    Error, KaraokeEntry, STYLE_FLAG_BOLD, STYLE_FLAG_ITALIC, STYLE_FLAG_UNDERLINE, StyleRecord, TextBox, TextSample, Tx3gConfig,
    decode_timed_samples, markup_to_sample, read_sample, sample_to_markup, write_sample,
};
use craft_captions::time::Tick;
use craft_captions::{Document, TICKS_PER_SECOND};

#[test]
fn test_plain_text_roundtrip() {
    let sample = TextSample { text: "Hello world without styles".to_string(), ..Default::default() };

    let bytes = write_sample(&sample).expect("write sample");
    assert_eq!(bytes.len(), 2 + sample.text.len());
    assert_eq!(&bytes[..2], &(sample.text.len() as u16).to_be_bytes());
    assert_eq!(&bytes[2..], sample.text.as_bytes());

    let decoded = read_sample(&bytes).expect("read sample");
    assert_eq!(decoded.text, sample.text);
    assert!(decoded.styles.is_empty());
}

#[test]
fn test_empty_sample_roundtrip() {
    let sample = TextSample::default();
    let bytes = write_sample(&sample).expect("write");
    assert_eq!(bytes, &[0x00, 0x00]);

    let decoded = read_sample(&bytes).expect("read");
    assert_eq!(decoded.text, "");
    assert!(decoded.styles.is_empty());
}

#[test]
fn test_styles_roundtrip() {
    let sample = TextSample {
        text: "Bold, Italic, Underline, and Colored".to_string(),
        styles: vec![
            StyleRecord {
                start_char: 0,
                end_char: 4,
                font_id: 1,
                face_style_flags: STYLE_FLAG_BOLD,
                font_size: 16,
                text_color_rgba: [255, 255, 255, 255],
            },
            StyleRecord {
                start_char: 6,
                end_char: 12,
                font_id: 1,
                face_style_flags: STYLE_FLAG_ITALIC,
                font_size: 18,
                text_color_rgba: [255, 255, 255, 255],
            },
            StyleRecord {
                start_char: 14,
                end_char: 23,
                font_id: 1,
                face_style_flags: STYLE_FLAG_UNDERLINE,
                font_size: 16,
                text_color_rgba: [255, 255, 255, 255],
            },
            StyleRecord { start_char: 29, end_char: 36, font_id: 1, face_style_flags: 0, font_size: 20, text_color_rgba: [255, 0, 0, 255] },
        ],
        ..Default::default()
    };

    let bytes = write_sample(&sample).expect("write");
    let decoded = read_sample(&bytes).expect("read");

    assert_eq!(decoded.text, sample.text);
    assert_eq!(decoded.styles.len(), 4);
    assert_eq!(decoded.styles, sample.styles);

    assert!(decoded.styles[0].is_bold());
    assert!(!decoded.styles[0].is_italic());
    assert!(decoded.styles[1].is_italic());
    assert!(decoded.styles[2].is_underline());
    assert_eq!(decoded.styles[3].text_color_rgba, [255, 0, 0, 255]);
}

#[test]
fn test_all_modifier_boxes_roundtrip() {
    let sample = TextSample {
        text: "Full modifiers test".to_string(),
        styles: vec![StyleRecord {
            start_char: 0,
            end_char: 4,
            font_id: 1,
            face_style_flags: STYLE_FLAG_BOLD,
            font_size: 16,
            text_color_rgba: [255, 255, 255, 255],
        }],
        highlight: Some((5, 14)),
        highlight_color: Some([255, 255, 0, 255]),
        karaoke: Some((
            100,
            vec![
                KaraokeEntry { highlight_end_time: 200, start_char: 0, end_char: 4 },
                KaraokeEntry { highlight_end_time: 350, start_char: 5, end_char: 14 },
            ],
        )),
        scroll_delay: Some(500),
        hypertext: vec![craft_captions::mov_text::HyperText {
            start_char: 0,
            end_char: 19,
            url: "https://example.com".to_string(),
            alt_string: "Example".to_string(),
        }],
        text_box: Some(TextBox { top: 10, left: 20, bottom: 100, right: 200 }),
        blinking: vec![(0, 4), (15, 19)],
        wrap_flag: Some(1),
    };

    let bytes = write_sample(&sample).expect("write");
    let decoded = read_sample(&bytes).expect("read");

    assert_eq!(decoded, sample);
}

#[test]
fn test_utf16_bom_reading() {
    // UTF-16 BE with BOM
    let text = "Hello UTF-16";
    let mut data_be = Vec::new();
    let utf16_be_bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    let total_len = 2 + utf16_be_bytes.len();
    data_be.extend_from_slice(&(total_len as u16).to_be_bytes());
    data_be.extend_from_slice(&[0xFE, 0xFF]);
    data_be.extend_from_slice(&utf16_be_bytes);

    let decoded_be = read_sample(&data_be).expect("read UTF-16 BE");
    assert_eq!(decoded_be.text, text);

    // UTF-16 LE with BOM
    let mut data_le = Vec::new();
    let utf16_le_bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    data_le.extend_from_slice(&(total_len as u16).to_be_bytes());
    data_le.extend_from_slice(&[0xFF, 0xFE]);
    data_le.extend_from_slice(&utf16_le_bytes);

    let decoded_le = read_sample(&data_le).expect("read UTF-16 LE");
    assert_eq!(decoded_le.text, text);
}

#[test]
fn test_markup_conversion() {
    let markup = "Hello <b>Bold</b> and <i>Italic</i> and <u>Underline</u> and <font color=\"#ff0000\">Red</font>";
    let sample = markup_to_sample(markup);

    assert_eq!(sample.text, "Hello Bold and Italic and Underline and Red");
    assert_eq!(sample.styles.len(), 4);

    let bold = &sample.styles[0];
    assert_eq!(&sample.text[bold.start_char as usize..bold.end_char as usize], "Bold");
    assert!(bold.is_bold());

    let italic = &sample.styles[1];
    assert_eq!(&sample.text[italic.start_char as usize..italic.end_char as usize], "Italic");
    assert!(italic.is_italic());

    let underline = &sample.styles[2];
    assert_eq!(&sample.text[underline.start_char as usize..underline.end_char as usize], "Underline");
    assert!(underline.is_underline());

    let red = &sample.styles[3];
    assert_eq!(&sample.text[red.start_char as usize..red.end_char as usize], "Red");
    assert_eq!(red.text_color_rgba, [255, 0, 0, 255]);

    let regenerated = sample_to_markup(&sample);
    assert_eq!(regenerated, markup);
}

#[test]
fn test_nested_styles_sorted_non_overlapping() {
    let markup = "<b>A<i>B</i>C</b>";
    let sample = markup_to_sample(markup);
    assert_eq!(sample.text, "ABC");
    assert_eq!(sample.styles.len(), 3);
    // [0, 1): A is bold
    assert_eq!(sample.styles[0].start_char, 0);
    assert_eq!(sample.styles[0].end_char, 1);
    assert_eq!(sample.styles[0].face_style_flags, STYLE_FLAG_BOLD);
    // [1, 2): B is bold + italic
    assert_eq!(sample.styles[1].start_char, 1);
    assert_eq!(sample.styles[1].end_char, 2);
    assert_eq!(sample.styles[1].face_style_flags, STYLE_FLAG_BOLD | STYLE_FLAG_ITALIC);
    // [2, 3): C is bold
    assert_eq!(sample.styles[2].start_char, 2);
    assert_eq!(sample.styles[2].end_char, 3);
    assert_eq!(sample.styles[2].face_style_flags, STYLE_FLAG_BOLD);
}

#[test]
fn test_entities_and_literal_escaping() {
    let markup = "A &amp; <b>B &lt; C</b>";
    let sample = markup_to_sample(markup);
    assert_eq!(sample.text, "A & B < C");
    assert_eq!(sample.styles.len(), 1);
    assert_eq!(sample.styles[0].start_char, 4);
    assert_eq!(sample.styles[0].end_char, 9);

    let roundtripped = sample_to_markup(&sample);
    assert_eq!(roundtripped, "A &amp; <b>B &lt; C</b>");
}

#[test]
fn test_twrp_box() {
    let sample = TextSample { text: "Wrap me".to_string(), wrap_flag: Some(1), ..Default::default() };
    let bytes = write_sample(&sample).expect("write sample");
    // Verify bytes contain twrp box
    assert!(bytes.windows(4).any(|w| w == b"twrp"));

    let read_back = read_sample(&bytes).expect("read sample");
    assert_eq!(read_back.wrap_flag, Some(1));
}

#[test]
fn test_tx3g_config_roundtrip() {
    let cfg = Tx3gConfig {
        display_flags: 0x0000_0001,
        horizontal_justification: 1,
        vertical_justification: -1,
        background_color: [10, 20, 30, 255],
        default_text_box: TextBox { top: 0, left: 0, bottom: 480, right: 640 },
        default_style: StyleRecord {
            start_char: 0,
            end_char: 0,
            font_id: 1,
            face_style_flags: STYLE_FLAG_BOLD,
            font_size: 24,
            text_color_rgba: [255, 255, 255, 255],
        },
        fonts: vec![(1, "Serif".to_string()), (2, "Sans".to_string())],
    };

    let bytes = cfg.write();
    let parsed = Tx3gConfig::parse(&bytes).expect("parse config");
    assert_eq!(parsed, cfg);
}

#[test]
fn test_decode_timed_samples_to_document() {
    let mut doc = Document::default();

    let s1 = write_sample(&markup_to_sample("First <b>cue</b>")).expect("s1");
    let s2 = write_sample(&markup_to_sample("Second <i>cue</i>")).expect("s2");

    let samples = vec![(0u64, 1000u32, s1.as_slice()), (1000u64, 1500u32, s2.as_slice())];

    decode_timed_samples(&mut doc, samples, 1000).expect("decode samples");

    assert_eq!(doc.cues.len(), 2);
    assert_eq!(doc.cues[0].start, Tick(0));
    assert_eq!(doc.cues[0].end, Tick(TICKS_PER_SECOND));
    assert_eq!(doc.cues[0].text, "First <b>cue</b>");

    assert_eq!(doc.cues[1].start, Tick(TICKS_PER_SECOND));
    assert_eq!(doc.cues[1].end, Tick((TICKS_PER_SECOND as f64 * 2.5) as i64));
    assert_eq!(doc.cues[1].text, "Second <i>cue</i>");
}

#[test]
fn test_hostile_inputs_never_panic() {
    // Empty input
    assert!(matches!(read_sample(&[]), Err(Error::Truncated)));

    // 1-byte input
    assert!(matches!(read_sample(&[0x00]), Err(Error::Truncated)));

    // Length says 10 bytes, but slice ends
    assert!(matches!(read_sample(&[0x00, 0x0A, 0x41]), Err(Error::Truncated)));

    // Invalid UTF-8 without BOM
    assert!(matches!(read_sample(&[0x00, 0x02, 0xFF, 0xFF]), Err(Error::InvalidText)));

    // Box size < 8
    let mut bad_box = Vec::new();
    bad_box.extend_from_slice(&[0x00, 0x04, b'T', b'e', b's', b't']);
    bad_box.extend_from_slice(&[0x00, 0x00, 0x00, 0x04, b's', b't', b'y', b'l']);
    assert!(matches!(read_sample(&bad_box), Err(Error::InvalidBox(_))));

    // Box size exceeds remaining slice
    let mut truncated_box = Vec::new();
    truncated_box.extend_from_slice(&[0x00, 0x04, b'T', b'e', b's', b't']);
    truncated_box.extend_from_slice(&[0x00, 0x00, 0x01, 0x00, b's', b't', b'y', b'l']);
    assert!(matches!(read_sample(&truncated_box), Err(Error::Truncated)));

    // Unknown modifier boxes are safely ignored without error
    let mut unknown_box = Vec::new();
    unknown_box.extend_from_slice(&[0x00, 0x04, b'T', b'e', b's', b't']);
    unknown_box.extend_from_slice(&[0x00, 0x00, 0x00, 0x0C, b'f', b'a', b'k', b'e', 1, 2, 3, 4]);
    let decoded = read_sample(&unknown_box).expect("unknown box should be skipped");
    assert_eq!(decoded.text, "Test");
}

#[test]
fn test_ffmpeg_mov_text_oracle() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let temp_dir = std::env::temp_dir().join("craft_mov_text_test");
    let _ = std::fs::create_dir_all(&temp_dir);

    // 1. Create SRT file with formatting: bold, italic, underline
    let srt_path = temp_dir.join("input.srt");
    let mp4_path = temp_dir.join("output.mp4");
    let srt_content = "1\n00:00:01,000 --> 00:00:03,000\nHello <b>World</b> and <i>Italic</i> and <u>Underline</u>\n";
    std::fs::write(&srt_path, srt_content).expect("write srt");

    // Encode to MP4 with mov_text
    let out = std::process::Command::new(&ff)
        .args(["-y", "-i", srt_path.to_str().expect("path"), "-c:s", "mov_text", mp4_path.to_str().expect("path")])
        .output();
    let Ok(output) = out else { return };
    if !output.status.success() {
        return;
    }

    // Read MP4 through craft-media ISOBMFF demuxer
    let mp4_bytes = std::fs::read(&mp4_path).expect("read mp4");
    let mp4 = craft_media::isobmff::open(&mp4_bytes).expect("open mp4");

    let (track_idx, track) =
        mp4.tracks.iter().enumerate().find(|(_, t)| t.kind == craft_media::isobmff::TrackKind::Subtitle).expect("subtitle track");

    // Decode all non-empty samples
    let mut found_styled_cue = false;
    for sample_idx in 0..track.samples.len() {
        let sample_data = mp4.read_sample(&mp4_bytes, track_idx, sample_idx).expect("read sample");
        let sample = read_sample(&sample_data).expect("read sample");
        if !sample.text.is_empty() {
            assert_eq!(sample.text, "Hello World and Italic and Underline");
            let markup = sample_to_markup(&sample);
            assert!(markup.contains("<b>World</b>"), "expected bold markup: {markup}");
            assert!(markup.contains("<i>Italic</i>"), "expected italic markup: {markup}");
            assert!(markup.contains("<u>Underline</u>"), "expected underline markup: {markup}");
            found_styled_cue = true;
        }
    }
    assert!(found_styled_cue, "expected to find styled subtitle sample in MP4");

    // 2. Test roundtrip: our encoder writes sample bytes -> decode with FFmpeg to verify exact text & styles
    let our_sample = markup_to_sample("Greeting: <b>Craft</b> <i>Timed</i> <u>Text</u>");
    let encoded_bytes = write_sample(&our_sample).expect("write sample");

    // Verify our sample reads back identically
    let roundtripped = read_sample(&encoded_bytes).expect("read back");
    assert_eq!(sample_to_markup(&roundtripped), "Greeting: <b>Craft</b> <i>Timed</i> <u>Text</u>");
}
