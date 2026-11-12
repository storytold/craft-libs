//! Avid Log Exchange (`.ale`): a tab-delimited clip log.
//!
//! Written from Avid's published description of the format: three sections introduced by a line
//! holding only `Heading`, `Column` or `Data`, separated by blank lines.
//!
//! ```text
//! Heading
//! FIELD_DELIM  TABS
//! VIDEO_FORMAT  1080
//! AUDIO_FORMAT  48khz
//! FPS  23.976
//!
//! Column
//! (fields are separated by tab characters)
//! Name  Tracks  Start  End  Tape  Source File  Description  …
//!
//! Data
//! Beach  VA1A2  01:00:00:00  01:00:10:00  …
//! ```
//!
//! `Tracks` lists the clip's tracks (`V`, `A1`…`A16`); `Start` and `End` are SMPTE timecode at the
//! heading's `FPS`, `End` exclusive. Values never contain tabs or line breaks (they are replaced by
//! spaces). Lines end with CR LF (Avid's own files); the reader accepts LF and CR too.

use crate::time::{FrameRate, Tick, parse_timecode};

/// A parsed or to-be-written ALE file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AleDoc {
    /// Heading fields in order (`FIELD_DELIM`, `VIDEO_FORMAT`, `AUDIO_FORMAT`, `FPS`, …).
    pub heading: Vec<(String, String)>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl AleDoc {
    pub fn heading(&self, key: &str) -> Option<&str> {
        self.heading.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }
    /// The value of `column` in row `row`.
    pub fn get(&self, row: usize, column: &str) -> Option<&str> {
        let c = self.columns.iter().position(|x| x.eq_ignore_ascii_case(column))?;
        self.rows.get(row)?.get(c).map(String::as_str)
    }
    /// The heading's frame rate (`FPS`).
    pub fn rate(&self) -> Option<FrameRate> {
        let f = self.heading("FPS").and_then(|f| f.trim().parse::<f64>().ok())?;
        // Reject NaN, infinite, non-positive and absurdly large rates (`from_f64` saturates or yields 0/1 for them).
        if !f.is_finite() || f <= 0.0 || f > 100_000.0 {
            return None;
        }
        let r = FrameRate::from_f64(f);
        (r.num > 0 && r.den > 0).then_some(r)
    }
}

/// The columns every log starts with, in order. `Tape` is the tape or reel name.
pub const STANDARD_COLUMNS: [&str; 6] = ["Name", "Tracks", "Start", "End", "Tape", "Source File"];

/// Metadata columns commonly written after [`STANDARD_COLUMNS`].
pub const EXTRA_COLUMNS: [&str; 6] = ["Description", "Scene", "Take", "Shot", "Log Note", "Comment"];

/// A value as it is written: tabs and line breaks become spaces.
pub fn clean(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ")
}

/// `FPS` heading text for a rate.
pub fn fps_text(r: FrameRate) -> String {
    let f = r.as_f64();
    if (f - f.round()).abs() < 1e-6 { format!("{}", f.round() as i64) } else { format!("{:.3}", f).trim_end_matches('0').to_string() }
}

/// `VIDEO_FORMAT` heading text for a frame height.
pub fn video_format(height: u32) -> &'static str {
    match height {
        1080 => "1080",
        720 => "720",
        480 | 486 => "NTSC",
        576 => "PAL",
        _ => "CUSTOM",
    }
}

/// The heading of a log at `rate` whose picture is `height` lines high and whose sound is sampled at
/// `sample_rate` Hz: `FIELD_DELIM`, `VIDEO_FORMAT`, `AUDIO_FORMAT` and `FPS`.
pub fn standard_heading(rate: FrameRate, height: u32, sample_rate: u32) -> Vec<(String, String)> {
    vec![
        ("FIELD_DELIM".to_string(), "TABS".to_string()),
        ("VIDEO_FORMAT".to_string(), video_format(height).to_string()),
        ("AUDIO_FORMAT".to_string(), if sample_rate == 44_100 { "44khz" } else { "48khz" }.to_string()),
        ("FPS".to_string(), fps_text(rate)),
    ]
}

/// The `Tracks` value of a clip: `V` when it has picture, then `A1`… for each sound channel (at most 16).
pub fn tracks_text(video: bool, audio_channels: u32) -> String {
    let mut tracks = String::new();
    if video {
        tracks.push('V');
    }
    for c in 1..=audio_channels.min(16) {
        tracks.push_str(&format!("A{c}"));
    }
    tracks
}

/// Serialise (CR LF line ends).
pub fn write(doc: &AleDoc) -> String {
    let mut out = String::from("Heading\r\n");
    for (k, v) in &doc.heading {
        out.push_str(&format!("{}\t{}\r\n", clean(k), clean(v)));
    }
    out.push_str("\r\nColumn\r\n");
    out.push_str(&doc.columns.iter().map(|c| clean(c)).collect::<Vec<_>>().join("\t"));
    out.push_str("\r\n\r\nData\r\n");
    for r in &doc.rows {
        out.push_str(&r.iter().map(|c| clean(c)).collect::<Vec<_>>().join("\t"));
        out.push_str("\r\n");
    }
    out
}

/// Read an ALE file.
pub fn parse(text: &str) -> Result<AleDoc, String> {
    #[derive(PartialEq)]
    enum Sec {
        None,
        Heading,
        Column,
        Data,
    }
    let mut doc = AleDoc::default();
    let mut sec = Sec::None;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    for line in text.lines() {
        match line.trim() {
            "Heading" => {
                sec = Sec::Heading;
                continue;
            }
            "Column" => {
                sec = Sec::Column;
                continue;
            }
            "Data" => {
                sec = Sec::Data;
                continue;
            }
            "" => continue,
            _ => {}
        }
        match sec {
            Sec::None => return Err("not an ALE file (no Heading section)".into()),
            Sec::Heading => {
                let mut it = line.splitn(2, '\t');
                let k = it.next().unwrap_or_default().trim().to_string();
                doc.heading.push((k, it.next().unwrap_or_default().trim().to_string()));
            }
            Sec::Column => doc.columns.extend(line.split('\t').map(|c| c.trim().to_string())),
            Sec::Data => doc.rows.push(line.split('\t').map(str::to_string).collect()),
        }
    }
    if doc.columns.is_empty() {
        return Err("the ALE file has no Column section".into());
    }
    Ok(doc)
}

/// Start and end (exclusive) of a data row, in ticks at the heading's rate.
pub fn row_range(doc: &AleDoc, row: usize) -> Option<(Tick, Tick)> {
    let rate = doc.rate()?;
    let a = parse_timecode(doc.get(row, "Start")?, rate, false, 0).ok()?;
    let b = parse_timecode(doc.get(row, "End")?, rate, false, 0).ok()?;
    Some((rate.tick_of(a), rate.tick_of(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::format_timecode_frames;

    #[test]
    fn utf8_bom_is_ignored() {
        let doc = parse("\u{feff}Heading\r\nFPS\t24\r\n\r\nColumn\r\nName\tStart\r\n\r\nData\r\nA\t01:00:00:00\r\n").unwrap();
        assert_eq!(doc.columns, ["Name", "Start"]);
        assert_eq!(doc.rows.len(), 1);
    }

    #[test]
    fn non_positive_or_non_finite_fps_is_rejected() {
        for fps in ["0", "-24", "nan", "inf", "1e300", "0.0001"] {
            let doc = parse(&format!("Heading\nFPS\t{fps}\n\nColumn\nName\tStart\tEnd\n\nData\nA\t01:00:00:00\t01:00:10:00\n")).unwrap();
            assert!(doc.rate().is_none(), "FPS {fps} accepted as {:?}", doc.rate());
        }
    }

    #[test]
    fn writes_the_three_sections_and_round_trips() {
        let r = FrameRate::FPS_23_976;
        let mut columns: Vec<String> = STANDARD_COLUMNS.iter().map(|s| s.to_string()).collect();
        columns.extend(EXTRA_COLUMNS.iter().map(|s| s.to_string()));
        let row = |name: &str, start: i64, end: i64, tape: &str, scene: &str| {
            let mut row = vec![
                clean(name),
                tracks_text(true, 2),
                format_timecode_frames(start, r, false),
                format_timecode_frames(end, r, false),
                tape.to_string(),
                format!("/media/{}", clean(name)),
            ];
            row.extend(EXTRA_COLUMNS.iter().map(|k| if *k == "Scene" { scene.to_string() } else { String::new() }));
            row
        };
        let doc = AleDoc {
            heading: standard_heading(r, 1080, 48_000),
            columns,
            rows: vec![row("Beach\tWide.mp4", 86_400, 86_640, "A001", "4"), row("Beach Sub", 86_424, 86_472, "", "")],
        };
        let text = write(&doc);
        assert!(
            text.starts_with(
                "Heading\r\nFIELD_DELIM\tTABS\r\nVIDEO_FORMAT\t1080\r\nAUDIO_FORMAT\t48khz\r\nFPS\t23.976\r\n\r\nColumn\r\nName\tTracks\tStart\tEnd\tTape"
            ),
            "{text}"
        );
        let back = parse(&text).unwrap();
        assert_eq!(back, doc);
        assert_eq!(back.get(0, "Name"), Some("Beach Wide.mp4"));
        assert_eq!(back.get(0, "Tracks"), Some("VA1A2"));
        assert_eq!(back.get(0, "Start"), Some("01:00:00:00"));
        assert_eq!(back.get(0, "End"), Some("01:00:10:00"));
        assert_eq!(back.get(0, "Tape"), Some("A001"));
        assert_eq!(back.get(0, "Source File"), Some("/media/Beach Wide.mp4"));
        assert_eq!(back.get(0, "Scene"), Some("4"));
        assert_eq!(back.get(1, "Start"), Some("01:00:01:00"));
        assert_eq!(back.get(1, "End"), Some("01:00:03:00"));
        let (s, e) = row_range(&back, 1).unwrap();
        assert_eq!(e - s, r.tick_of(48));
        assert_eq!(back.rate(), Some(r));
        // LF-only files read the same
        assert_eq!(parse(&text.replace("\r\n", "\n")).unwrap(), doc);
        assert!(parse("hello").is_err());
    }

    #[test]
    fn fps_and_formats() {
        assert_eq!(fps_text(FrameRate::FPS_29_97), "29.97");
        assert_eq!(fps_text(FrameRate::FPS_25), "25");
        assert_eq!(fps_text(FrameRate::FPS_59_94), "59.94");
        assert_eq!(video_format(576), "PAL");
        assert_eq!(video_format(2160), "CUSTOM");
        assert_eq!(tracks_text(false, 40).matches('A').count(), 16);
    }
}
