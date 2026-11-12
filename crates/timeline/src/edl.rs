//! CMX 3600 edit decision lists.
//!
//! Read: `TITLE:`, `FCM:` (drop/non-drop, may change mid-list), cut / dissolve (`D nnn`) /
//! wipe (`Wnnn nnn`) / key (`K B`, `K`, `K O`) events, audio channels (`V`, `A`, `A2`, `AA`, `B`,
//! `AA/V`, `A2/V`, `A3`, `A4`, `AUD 3 4` lines), `M2` motion (speed, reverse, freeze) lines, reel
//! names of any length, and the comment extensions `* FROM CLIP NAME:`, `* TO CLIP NAME:`,
//! `* SOURCE FILE:`, `* EFFECT NAME:` and `* LOC:` markers.
//!
//! [`parse`] gives the list as written ([`Edl`]: events of lines with their timecode fields); what
//! an event means on a timeline (which media a reel is, how a dissolve overlaps) is the caller's
//! mapping. A list carries no frame rate: [`guess_rate`] guesses one, and [`Timecode::frames`]
//! counts at the rate the caller settles on. [`write`] lays out events the caller built.

use std::collections::BTreeSet;

use crate::report::Report;
use crate::time::{FrameRate, fields_to_frames, format_timecode_frames};

/// Heuristic: does this text look like an EDL?
pub fn looks_like_edl(text: &str) -> bool {
    text.lines().take(40).any(|l| {
        let l = l.trim_start();
        l.starts_with("TITLE:") || l.starts_with("FCM:") || parse_event_line(l).is_some()
    })
}

/// A timecode as written: its four fields, and whether its separators say drop-frame (`;` or `,`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timecode {
    pub h: i64,
    pub m: i64,
    pub s: i64,
    pub f: i64,
    pub df: bool,
}

/// Read `HH:MM:SS:FF` (separators `:`, `;`, `.` or `,`). Fields beyond `i64` are refused.
pub fn parse_tc(s: &str) -> Option<Timecode> {
    let parts: Vec<&str> = s.split([':', ';', '.', ',']).collect();
    if parts.len() != 4 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let n: Vec<i64> = parts.iter().map(|p| p.parse().ok()).collect::<Option<_>>()?;
    Some(Timecode { h: n[0], m: n[1], s: n[2], f: n[3], df: s.contains(';') || s.contains(',') })
}

/// The edit type of an event line (`C`, `D`, `Wnnn`, `K B`, `K`, `K O`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Cut,
    Dissolve,
    Wipe(u32),
    KeyBackground,
    Key,
    KeyOut,
}

/// A channel an event records to: picture, or audio channel `n` (1-based).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    V,
    A(usize),
}

/// The channels of a channel field (`V`, `A`, `A2`, `AA`, `B`, `AA/V`, `A2/V`, `A3`, `NONE`, …); audio channels are 1..=99.
pub fn parse_channels(s: &str) -> Option<Vec<Channel>> {
    let u = s.to_ascii_uppercase();
    match u.as_str() {
        "B" => return Some(vec![Channel::V, Channel::A(1)]),
        "NONE" => return Some(vec![]),
        _ => {}
    }
    let mut out = Vec::new();
    for part in u.split('/') {
        match part {
            "V" => out.push(Channel::V),
            "A" | "A1" => out.push(Channel::A(1)),
            "AA" => out.extend([Channel::A(1), Channel::A(2)]),
            p if p.starts_with('A') && p[1..].bytes().all(|b| b.is_ascii_digit()) && p[1..].parse::<usize>().is_ok_and(|n| (1..=99).contains(&n)) => {
                out.push(Channel::A(p[1..].parse().ok()?))
            }
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// One event line: `num reel channels type [duration] src-in src-out rec-in rec-out`.
#[derive(Clone, Debug)]
pub struct Line {
    pub num: String,
    pub reel: String,
    pub chans: Vec<Channel>,
    pub trans: Transition,
    /// Transition duration in frames (0 for a cut).
    pub dur: i64,
    /// Source in, source out, record in, record out.
    pub tcs: [Timecode; 4],
}

/// Read one event line; `None` when the line is not one.
pub fn parse_event_line(l: &str) -> Option<Line> {
    let toks: Vec<&str> = l.split_whitespace().collect();
    if toks.len() < 8 || !toks[0].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = toks.len();
    let tcs = [parse_tc(toks[n - 4])?, parse_tc(toks[n - 3])?, parse_tc(toks[n - 2])?, parse_tc(toks[n - 1])?];
    let reel = toks[1].to_string();
    let chans = parse_channels(toks[2])?;
    let mid = &toks[3..n - 4];
    let first = mid.first()?.to_ascii_uppercase();
    let num_at = |i: usize| mid.get(i).and_then(|t| t.parse::<i64>().ok());
    let (trans, dur) = match first.as_str() {
        "C" => (Transition::Cut, 0),
        "D" => (Transition::Dissolve, num_at(1).unwrap_or(0)),
        "K" => match mid.get(1).map(|s| s.to_ascii_uppercase()) {
            Some(ref s) if s == "B" => (Transition::KeyBackground, 0),
            Some(ref s) if s == "O" => (Transition::KeyOut, num_at(2).unwrap_or(0)),
            _ => (Transition::Key, num_at(1).unwrap_or(0)),
        },
        "KB" => (Transition::KeyBackground, 0),
        "KO" => (Transition::KeyOut, num_at(1).unwrap_or(0)),
        w if w.starts_with('W') => (Transition::Wipe(w[1..].parse().unwrap_or(1)), num_at(1).unwrap_or(0)),
        _ => return None,
    };
    Some(Line { num: toks[0].to_string(), reel, chans, trans, dur, tcs })
}

/// An event: the lines sharing one event number, and the comment and motion lines after them.
#[derive(Clone, Debug, Default)]
pub struct Event {
    pub lines: Vec<Line>,
    pub from_clip: Option<String>,
    pub to_clip: Option<String>,
    pub files: Vec<String>,
    pub effect: Option<String>,
    /// `M2` motion lines: (reel, frames per second the source plays at).
    pub m2: Vec<(String, f64)>,
    /// `* LOC:` locators: (timecode, colour word or empty, name).
    pub locs: Vec<(Timecode, String, String)>,
    /// FCM in force for this event.
    pub df: bool,
}

/// A parsed list.
#[derive(Clone, Debug, Default)]
pub struct Edl {
    pub title: Option<String>,
    pub events: Vec<Event>,
    /// Whether any `FCM:` line or timecode says drop-frame.
    pub any_df: bool,
    /// The largest frame field of any event timecode.
    pub max_ff: i64,
}

/// Read a list. Never fails: lines that cannot be read are named in `report` and skipped.
pub fn parse(text: &str, report: &mut Report) -> Edl {
    let mut p = Edl { title: None, events: Vec::new(), any_df: false, max_ff: 0 };
    let mut df = false;
    let mut skip_metadata = false;
    for raw in text.lines() {
        let l = raw.trim();
        if l.is_empty() {
            continue;
        }
        let upper = l.to_ascii_uppercase();
        if let Some(t) = l.strip_prefix("TITLE:") {
            p.title = Some(t.trim().to_string());
            continue;
        }
        if upper.starts_with("FCM:") {
            df = upper.contains("DROP") && !upper.contains("NON");
            p.any_df |= df;
            continue;
        }
        if skip_metadata && (l.starts_with('*') || upper.starts_with("M2") || upper.starts_with("AUD")) {
            continue;
        }
        if let Some(c) = l.strip_prefix('*') {
            let c = c.trim();
            let Some(ev) = p.events.last_mut() else { continue };
            let cu = c.to_ascii_uppercase();
            let value = |prefix: &str| c[prefix.len()..].trim().to_string();
            if cu.starts_with("FROM CLIP NAME:") {
                ev.from_clip = Some(value("FROM CLIP NAME:"));
            } else if cu.starts_with("TO CLIP NAME:") {
                ev.to_clip = Some(value("TO CLIP NAME:"));
            } else if cu.starts_with("SOURCE FILE:") {
                ev.files.push(value("SOURCE FILE:"));
            } else if cu.starts_with("EFFECT NAME:") {
                ev.effect = Some(value("EFFECT NAME:"));
            } else if cu.starts_with("LOC:") {
                let rest = value("LOC:");
                let mut it = rest.splitn(3, char::is_whitespace).filter(|s| !s.is_empty());
                if let Some(tc) = it.next().and_then(parse_tc) {
                    let rest2: Vec<&str> = rest.split_whitespace().skip(1).collect();
                    let (color, name) = match rest2.split_first() {
                        Some((c, n)) if is_locator_color(c) => (c.to_string(), n.join(" ")),
                        _ => (String::new(), rest2.join(" ")),
                    };
                    ev.locs.push((tc, color, name));
                }
            }
            continue;
        }
        if upper.starts_with("M2") {
            let toks: Vec<&str> = l.split_whitespace().collect();
            if toks.len() >= 3 {
                // `M2 REEL fps tc`; reels with spaces are joined back.
                let fps_i = toks.iter().rposition(|t| t.parse::<f64>().is_ok() && parse_tc(t).is_none()).unwrap_or(2);
                let reel = toks[1..fps_i.max(2)].join(" ");
                if let Ok(fps) = toks[fps_i].parse::<f64>()
                    && let Some(ev) = p.events.last_mut()
                {
                    ev.m2.push((reel, fps));
                }
            }
            continue;
        }
        if upper.starts_with("AUD") {
            let mut extra: Vec<Channel> = Vec::new();
            for n in l.split_whitespace().skip(1).filter_map(|t| t.parse::<usize>().ok()) {
                if (1..=99).contains(&n) {
                    extra.push(Channel::A(n));
                } else {
                    report.warn(format!("EDL AUD channel {n} is outside 1..99; ignored"));
                }
            }
            if let Some(line) = p.events.last_mut().and_then(|e| e.lines.last_mut()) {
                line.chans.extend(extra);
            }
            continue;
        }
        if upper.starts_with("SPLIT:") {
            report.warn("EDL split (L-cut/J-cut) lines are not supported; the split was ignored");
            continue;
        }
        if let Some(line) = parse_event_line(l) {
            for tc in &line.tcs {
                p.max_ff = p.max_ff.max(tc.f);
                p.any_df |= tc.df;
            }
            if let Some(e) = p.events.last_mut().filter(|e| !skip_metadata && e.lines.last().is_some_and(|last| last.num == line.num)) {
                e.lines.push(line);
            } else {
                p.events.push(Event { lines: vec![line], df, ..Default::default() });
            }
            skip_metadata = false;
            continue;
        }
        if upper.starts_with(">>>") || upper.starts_with("REM") {
            continue;
        }
        if l.split_whitespace().next().is_some_and(|token| token.bytes().all(|b| b.is_ascii_digit())) {
            // Comments and auxiliary lines after a rejected event do not belong to the previous one.
            skip_metadata = true;
        }
        report.warn(format!("unrecognised EDL line ignored: {}", l.chars().take(60).collect::<String>()));
    }
    p
}

/// The frame rate a list most likely runs at (a list does not say): 29.97 or 59.94 for drop-frame
/// lists, else the lowest of 24, 25, 30 and 60 that its largest frame field fits.
pub fn guess_rate(p: &Edl) -> FrameRate {
    if p.any_df {
        if p.max_ff >= 30 { FrameRate::FPS_59_94 } else { FrameRate::FPS_29_97 }
    } else if p.max_ff >= 30 {
        FrameRate::FPS_60
    } else if p.max_ff >= 25 {
        FrameRate::FPS_30
    } else if p.max_ff >= 24 {
        FrameRate::FPS_25
    } else {
        FrameRate::FPS_24
    }
}

impl Timecode {
    /// The frame count at `rate`; drop-frame when the timecode's separators or `drop_frame` (the
    /// `FCM:` in force) say so and the rate has drop-frame timecode.
    pub fn frames(self, rate: FrameRate, drop_frame: bool) -> i64 {
        fields_to_frames(self.h, self.m, self.s, self.f, rate, self.df || drop_frame)
    }
}

/// The colour words a `* LOC:` locator can carry between its timecode and its name.
pub const LOCATOR_COLORS: [&str; 8] = ["RED", "GREEN", "BLUE", "CYAN", "MAGENTA", "YELLOW", "WHITE", "BLACK"];

/// Whether `word` is one of [`LOCATOR_COLORS`] (in any case).
pub fn is_locator_color(word: &str) -> bool {
    LOCATOR_COLORS.iter().any(|c| c.eq_ignore_ascii_case(word))
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// The channel field for a set of channels, when one field can name it (`V`, `B`, `AA/V`, `A2/V`,
/// `A`, `A2`, `AA`, `A3`, `A4`).
pub fn channel_code(set: &BTreeSet<Channel>) -> Option<&'static str> {
    let v: Vec<Channel> = set.iter().copied().collect();
    Some(match v.as_slice() {
        [Channel::V] => "V",
        [Channel::V, Channel::A(1)] => "B",
        [Channel::V, Channel::A(1), Channel::A(2)] => "AA/V",
        [Channel::V, Channel::A(2)] => "A2/V",
        [Channel::A(1)] => "A",
        [Channel::A(2)] => "A2",
        [Channel::A(1), Channel::A(2)] => "AA",
        [Channel::A(3)] => "A3",
        [Channel::A(4)] => "A4",
        _ => return None,
    })
}

/// The channel field for one channel.
pub fn channel_name(channel: Channel) -> String {
    match channel {
        Channel::V => "V".to_string(),
        Channel::A(1) => "A".to_string(),
        Channel::A(n) => format!("A{n}"),
    }
}

/// A reel name from free text: ASCII letters (upper-cased), digits, `_` and `-`, anything else as
/// `_`, at most `max` characters; `AX` when nothing is left.
pub fn sanitize_reel(s: &str, max: usize) -> String {
    let r: String =
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_uppercase() } else { '_' }).take(max).collect();
    if r.is_empty() { "AX".into() } else { r }
}

/// An event line to write. Times are frame counts (timecode frames at the list's rate).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutLine {
    pub reel: String,
    /// The channel field (see [`channel_code`]).
    pub chan: String,
    /// The edit type field (`C`, `D`, `W001`, …).
    pub trans: String,
    /// Transition duration in frames; `None` for a cut.
    pub dur: Option<i64>,
    /// Source in and out.
    pub src: (i64, i64),
    /// Record in and out.
    pub rec: (i64, i64),
}

/// An event to write: its lines, then the motion line, comments and locators that follow them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutEvent {
    pub lines: Vec<OutLine>,
    /// `M2` motion line: reel, frames per second (negative plays backwards), source entry frame.
    pub m2: Option<(String, f64, i64)>,
    /// Comment lines, without the leading `* ` (`FROM CLIP NAME: a.mov`).
    pub comments: Vec<String>,
    /// `* LOC:` locators: record frame, colour word (see [`LOCATOR_COLORS`]), name.
    pub locators: Vec<(i64, String, String)>,
}

/// The head of a list to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutHeader {
    pub title: String,
    pub rate: FrameRate,
    /// Drop-frame timecode (`FCM: DROP FRAME`, `;` separators); only at rates that have it.
    pub drop_frame: bool,
    /// Width of the reel column: 8 (strict CMX 3600) or more (at least 8 is used).
    pub reel_len: usize,
}

/// Lay out a list: the title and frame code mode, then the events, numbered from 001.
pub fn write(header: &OutHeader, events: &[OutEvent]) -> String {
    let (rate, df) = (header.rate, header.drop_frame);
    let tc = |frames: i64| format_timecode_frames(frames, rate, df);
    let mut out = String::new();
    out.push_str(&format!("TITLE: {}\n", header.title));
    out.push_str(if df { "FCM: DROP FRAME\n" } else { "FCM: NON-DROP FRAME\n" });
    let w = header.reel_len.max(8);
    for (i, ev) in events.iter().enumerate() {
        out.push('\n');
        let num = i + 1;
        for l in &ev.lines {
            let dur = l.dur.map(|d| format!("{d:03}")).unwrap_or_default();
            out.push_str(&format!(
                "{num:03}  {:<w$} {:<5} {:<4} {:>3} {} {} {} {}\n",
                l.reel,
                l.chan,
                l.trans,
                dur,
                tc(l.src.0),
                tc(l.src.1),
                tc(l.rec.0),
                tc(l.rec.1),
            ));
        }
        if let Some((reel, fps, entry)) = &ev.m2
            && !ev.lines.is_empty()
        {
            let sign = if *fps < 0.0 { "-" } else { "" };
            out.push_str(&format!("M2   {:<w$}       {sign}{:05.1}                {}\n", reel, fps.abs(), tc(*entry)));
        }
        for c in &ev.comments {
            out.push_str(&format!("* {c}\n"));
        }
        for (at, color, name) in &ev.locators {
            out.push_str(&format!("* LOC: {} {:<7} {}\n", tc(*at), color, name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels() {
        assert_eq!(parse_channels("B"), Some(vec![Channel::V, Channel::A(1)]));
        assert_eq!(parse_channels("AA/V"), Some(vec![Channel::A(1), Channel::A(2), Channel::V]));
        assert_eq!(parse_channels("A4"), Some(vec![Channel::A(4)]));
        assert_eq!(parse_channels("X"), None);
        assert_eq!(parse_channels("A+4"), None);
    }

    #[test]
    fn timecode_rejects_fields_too_large_for_i64() {
        for tc in ["9223372036854775808:00:00:00", "00:9223372036854775808:00:00", "00:00:9223372036854775808:00", "00:00:00:9223372036854775808"] {
            assert!(parse_tc(tc).is_none(), "accepted {tc}");
        }
        assert_eq!(parse_tc("01:02:03:04"), Some(Timecode { h: 1, m: 2, s: 3, f: 4, df: false }));
    }

    #[test]
    fn rejected_event_metadata_does_not_change_previous_event() {
        let text = "TITLE: Rejected event\n\
001 A V C 00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00\n\
* FROM CLIP NAME: First\n\
* SOURCE FILE: first.mov\n\
002 B V C 9223372036854775808:00:00:00 00:00:01:00 01:00:01:00 01:00:02:00\n\
* FROM CLIP NAME: Rejected\n\
* SOURCE FILE: rejected.mov\n\
* LOC: 01:00:01:00 RED Rejected marker\n\
M2 B 12.0 00:00:00:00\n\
AUD 2\n\
003 C V C 00:00:00:00 00:00:01:00 01:00:02:00 01:00:03:00\n\
* FROM CLIP NAME: Third\n";
        for number in ["001", "003"] {
            let text = text.replace("003 C", &format!("{number} C"));
            let mut report = Report::default();
            let parsed = parse(&text, &mut report);
            assert_eq!(parsed.events.len(), 2);
            let first = &parsed.events[0];
            assert_eq!(first.from_clip.as_deref(), Some("First"));
            assert_eq!(first.files, ["first.mov"]);
            assert!(first.locs.is_empty());
            assert!(first.m2.is_empty());
            assert_eq!(first.lines[0].chans, [Channel::V]);
            assert_eq!(parsed.events[1].from_clip.as_deref(), Some("Third"));
        }
    }

    #[test]
    fn event_lines() {
        let l = parse_event_line("002  REEL_B   V     D    030 02:00:10:00 02:00:15:00 01:00:05:00 01:00:10:00").unwrap();
        assert_eq!(l.trans, Transition::Dissolve);
        assert_eq!(l.dur, 30);
        let k = parse_event_line("003  KEY      V     K B      00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00").unwrap();
        assert_eq!(k.trans, Transition::KeyBackground);
        let w = parse_event_line("004  R        V     W001 015 00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00").unwrap();
        assert_eq!(w.trans, Transition::Wipe(1));
        assert!(parse_event_line("* comment").is_none());
        assert!(parse_tc("01;00;00;02").unwrap().df);
    }

    #[test]
    fn aud_line_out_of_range_channel_is_ignored() {
        for n in ["0", "100", "3000000"] {
            let text = format!("TITLE: t\n001  AX       V     C        00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00\nAUD {n}\n");
            let mut report = Report::default();
            let edl = parse(&text, &mut report);
            assert_eq!(edl.events[0].lines[0].chans, [Channel::V], "AUD {n}");
            assert!(report.mentions("AUD"), "AUD {n} should be reported");
        }
        let mut report = Report::default();
        let edl = parse("001 AX V C 00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00\nAUD 3 4\n", &mut report);
        assert_eq!(edl.events[0].lines[0].chans, [Channel::V, Channel::A(3), Channel::A(4)]);
        assert!(report.is_empty(), "{report}");
    }

    #[test]
    fn hostile_m2_speeds_are_read_as_written() {
        // the caller decides what a speed means; the parser must only not fail on them
        for v in ["nan", "inf", "-inf", "0.00001", "1e300"] {
            let text = format!(
                "TITLE: t\nFCM: NON-DROP FRAME\n001  AX  V  C  01:00:00:00 01:00:02:00 01:00:00:00 01:00:02:00\nM2   AX   {v}   01:00:00:00\n"
            );
            let edl = parse(&text, &mut Report::default());
            assert_eq!(edl.events[0].m2.len(), 1, "{v}");
            assert_eq!(edl.events[0].m2[0].0, "AX");
        }
    }

    #[test]
    fn frame_counts_and_rate_guess() {
        let ndf = parse_tc("01:00:00:00").unwrap();
        assert_eq!(ndf.frames(FrameRate::FPS_24, false), 86_400);
        assert_eq!(ndf.frames(FrameRate::FPS_29_97, false), 108_000);
        assert_eq!(ndf.frames(FrameRate::FPS_29_97, true), 107_892);
        assert_eq!(parse_tc("01;00;00;00").unwrap().frames(FrameRate::FPS_29_97, false), 107_892);
        let rate = |text: &str| guess_rate(&parse(text, &mut Report::default()));
        let ev = |ff: &str| format!("001 AX V C 00:00:00:00 00:00:01:{ff} 01:00:00:00 01:00:01:00\n");
        assert_eq!(rate(&ev("23")), FrameRate::FPS_24);
        assert_eq!(rate(&ev("24")), FrameRate::FPS_25);
        assert_eq!(rate(&ev("29")), FrameRate::FPS_30);
        assert_eq!(rate(&ev("59")), FrameRate::FPS_60);
        assert_eq!(rate(&format!("FCM: DROP FRAME\n{}", ev("29"))), FrameRate::FPS_29_97);
        assert_eq!(rate(&format!("FCM: DROP FRAME\n{}", ev("59"))), FrameRate::FPS_59_94);
        assert_eq!(rate(&format!("FCM: NON-DROP FRAME\n{}", ev("10"))), FrameRate::FPS_24);
    }

    #[test]
    fn locators_take_a_colour_word_only_from_the_known_set() {
        let text = "001 AX V C 00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00\n* LOC: 01:00:00:10 red    Slate\n* LOC: 01:00:00:20 Take two\n";
        let edl = parse(text, &mut Report::default());
        let locs: Vec<(i64, &str, &str)> = edl.events[0].locs.iter().map(|(t, c, n)| (t.f, c.as_str(), n.as_str())).collect();
        assert_eq!(locs, [(10, "red", "Slate"), (20, "", "Take two")]);
    }

    #[test]
    fn written_lists_have_the_column_layout_and_read_back() {
        let header = OutHeader { title: "Cut 1".into(), rate: FrameRate::FPS_29_97, drop_frame: true, reel_len: 8 };
        let events = [
            OutEvent {
                lines: vec![OutLine { reel: "A001".into(), chan: "B".into(), trans: "C".into(), dur: None, src: (0, 30), rec: (107_892, 107_922) }],
                m2: Some(("A001".into(), -12.0, 0)),
                comments: vec!["FROM CLIP NAME: a.mov".into(), "SOURCE FILE: /m/a.mov".into()],
                locators: vec![(107_900, "RED".into(), "Slate".into())],
            },
            OutEvent {
                lines: vec![
                    OutLine { reel: "A001".into(), chan: "V".into(), trans: "C".into(), dur: None, src: (30, 30), rec: (107_922, 107_922) },
                    OutLine { reel: "B002".into(), chan: "V".into(), trans: "D".into(), dur: Some(15), src: (60, 90), rec: (107_922, 107_952) },
                ],
                comments: vec!["EFFECT NAME: CROSS DISSOLVE".into()],
                ..Default::default()
            },
        ];
        let text = write(&header, &events);
        let expected = "TITLE: Cut 1\n\
FCM: DROP FRAME\n\
\n\
001  A001     B     C        00;00;00;00 00;00;01;00 01;00;00;00 01;00;01;00\n\
M2   A001           -012.0                00;00;00;00\n\
* FROM CLIP NAME: a.mov\n\
* SOURCE FILE: /m/a.mov\n\
* LOC: 01;00;00;08 RED     Slate\n\
\n\
002  A001     V     C        00;00;01;00 00;00;01;00 01;00;01;00 01;00;01;00\n\
002  B002     V     D    015 00;00;02;00 00;00;03;00 01;00;01;00 01;00;02;00\n\
* EFFECT NAME: CROSS DISSOLVE\n";
        assert_eq!(text, expected);
        assert!(looks_like_edl(&text));
        let mut report = Report::default();
        let back = parse(&text, &mut report);
        assert!(report.is_empty(), "{report}");
        assert_eq!(back.title.as_deref(), Some("Cut 1"));
        assert!(back.any_df);
        assert_eq!(back.events.len(), 2);
        assert_eq!(back.events[0].lines[0].chans, [Channel::V, Channel::A(1)]);
        assert_eq!(back.events[0].m2, [("A001".to_string(), -12.0)]);
        assert_eq!(back.events[0].files, ["/m/a.mov"]);
        assert_eq!(back.events[0].locs[0].0.frames(header.rate, true), 107_900);
        assert_eq!(back.events[1].lines.len(), 2);
        assert_eq!((back.events[1].lines[1].trans, back.events[1].lines[1].dur), (Transition::Dissolve, 15));
        assert_eq!(back.events[1].effect.as_deref(), Some("CROSS DISSOLVE"));
        // strict lists pad the reel to 8; a wider reel column is kept
        let wide = write(&OutHeader { reel_len: 32, ..header }, &events[..1]);
        assert!(wide.contains(&format!("001  {:<32} B ", "A001")), "{wide}");
        assert_eq!(sanitize_reel("my clip.mov", 8), "MY_CLIP_");
        assert_eq!(sanitize_reel("", 8), "AX");
        assert_eq!(channel_code(&BTreeSet::from([Channel::V, Channel::A(1), Channel::A(2)])), Some("AA/V"));
        assert_eq!(channel_code(&BTreeSet::from([Channel::A(1), Channel::A(3)])), None);
        assert_eq!(channel_name(Channel::A(7)), "A7");
    }
}
