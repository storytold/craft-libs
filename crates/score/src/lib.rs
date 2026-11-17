#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! MusicXML and SVG score writing over application-independent notation data.

use std::fmt::Write as _;

const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024; // 256 MiB
fn over_budget(buf: &str, budget: usize) -> Result<(), String> {
    if buf.len() > budget {
        return Err(format!("score too large: over {} bytes", budget));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoreSetup {
    /// Software name recorded in the MusicXML encoding metadata.
    pub software: String,
    pub title: String,
    pub composer: String,
    pub bars_per_system: u32,
    pub show_track_names: bool,
}

/// One measure: its meter and its items (notes, chords or rests), in sixteenths.
#[derive(Debug, Clone, PartialEq)]
pub struct Measure {
    pub numerator: u32,
    pub denominator: u32,
    pub items: Vec<Item>,
}

/// A notehead group (chord) or a rest (no pitches).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub pitches: Vec<u8>,
    /// Duration in sixteenths; always one of the supported duration values.
    pub units: u32,
    /// Pitches continuing into the next item.
    pub tie_start: Vec<u8>,
    /// Pitches continuing from the previous item.
    pub tie_stop: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    /// True when the part reads best in bass clef.
    pub bass: bool,
    pub measures: Vec<Measure>,
}

fn spell(pitch: u8) -> (char, i32, i32) {
    const STEPS: [(char, i32); 12] =
        [('C', 0), ('C', 1), ('D', 0), ('D', 1), ('E', 0), ('F', 0), ('F', 1), ('G', 0), ('G', 1), ('A', 0), ('A', 1), ('B', 0)];
    let (step, alter) = STEPS[usize::from(pitch % 12)];
    (step, alter, i32::from(pitch / 12) - 1)
}

fn xml_escape(s: &str) -> String {
    // Keep the score export's existing literal whitespace and quote spelling. The
    // shared attribute writer owns XML legality/escaping; only its generated
    // whitespace references are restored (input ampersands were escaped first).
    craft_xml::esc_attr(s).replace("&#9;", "\t").replace("&#10;", "\n").replace("&#13;", "\r")
}

fn note_type(units: u32) -> (&'static str, bool) {
    match units {
        16 => ("whole", false),
        12 => ("half", true),
        8 => ("half", false),
        6 => ("quarter", true),
        4 => ("quarter", false),
        3 => ("eighth", true),
        2 => ("eighth", false),
        _ => ("16th", false),
    }
}

/// MusicXML 4.0 (partwise) for `parts`.
pub fn musicxml(parts: &[Part], setup: &ScoreSetup) -> Result<String, String> {
    musicxml_budgeted(parts, setup, MAX_OUTPUT_BYTES)
}

/// MusicXML with an explicit byte budget, capped at 256 MiB.
pub fn musicxml_budgeted(parts: &[Part], setup: &ScoreSetup, budget: usize) -> Result<String, String> {
    validate(parts, setup)?;
    let budget = budget.min(MAX_OUTPUT_BYTES);
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
    x.push_str("<!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\" \"http://www.musicxml.org/dtds/partwise.dtd\">\n");
    x.push_str("<score-partwise version=\"4.0\">\n");
    // The header (and an oversized title/composer, escaped) can itself exceed the budget even with
    // no measures, so check before the per-measure loop too.
    over_budget(&x, budget)?;
    let _ = writeln!(x, "  <work><work-title>{}</work-title></work>", xml_escape(&setup.title));
    x.push_str("  <identification>\n");
    if !setup.composer.is_empty() {
        let _ = writeln!(x, "    <creator type=\"composer\">{}</creator>", xml_escape(&setup.composer));
    }
    let _ = writeln!(x, "    <encoding><software>{}</software></encoding>", xml_escape(&setup.software));
    x.push_str("  </identification>\n  <part-list>\n");
    for (i, p) in parts.iter().enumerate() {
        let _ = writeln!(x, "    <score-part id=\"P{}\"><part-name>{}</part-name></score-part>", i + 1, xml_escape(&p.name));
    }
    x.push_str("  </part-list>\n");
    over_budget(&x, budget)?;
    for (i, p) in parts.iter().enumerate() {
        let _ = writeln!(x, "  <part id=\"P{}\">", i + 1);
        let mut meter = (0, 0);
        for (mi, m) in p.measures.iter().enumerate() {
            let _ = writeln!(x, "    <measure number=\"{}\">", mi + 1);
            if mi == 0 || meter != (m.numerator, m.denominator) {
                x.push_str("      <attributes>");
                if mi == 0 {
                    x.push_str("<divisions>4</divisions><key><fifths>0</fifths></key>");
                }
                let _ = write!(x, "<time><beats>{}</beats><beat-type>{}</beat-type></time>", m.numerator, m.denominator);
                if mi == 0 {
                    x.push_str(if p.bass { "<clef><sign>F</sign><line>4</line></clef>" } else { "<clef><sign>G</sign><line>2</line></clef>" });
                }
                x.push_str("</attributes>\n");
                meter = (m.numerator, m.denominator);
            }
            for it in &m.items {
                let (ty, dot) = note_type(it.units);
                if it.pitches.is_empty() {
                    let _ = writeln!(
                        x,
                        "      <note><rest/><duration>{}</duration><voice>1</voice><type>{ty}</type>{}</note>",
                        it.units,
                        if dot { "<dot/>" } else { "" }
                    );
                    continue;
                }
                for (k, pitch) in it.pitches.iter().enumerate() {
                    let (step, alter, octave) = spell(*pitch);
                    x.push_str("      <note>");
                    if k > 0 {
                        x.push_str("<chord/>");
                    }
                    let _ = write!(x, "<pitch><step>{step}</step>");
                    if alter != 0 {
                        let _ = write!(x, "<alter>{alter}</alter>");
                    }
                    let _ = write!(x, "<octave>{octave}</octave></pitch><duration>{}</duration>", it.units);
                    if it.tie_stop.contains(pitch) {
                        x.push_str("<tie type=\"stop\"/>");
                    }
                    if it.tie_start.contains(pitch) {
                        x.push_str("<tie type=\"start\"/>");
                    }
                    let _ = write!(x, "<voice>1</voice><type>{ty}</type>");
                    if dot {
                        x.push_str("<dot/>");
                    }
                    if alter != 0 {
                        x.push_str("<accidental>sharp</accidental>");
                    }
                    if it.tie_stop.contains(pitch) || it.tie_start.contains(pitch) {
                        x.push_str("<notations>");
                        if it.tie_stop.contains(pitch) {
                            x.push_str("<tied type=\"stop\"/>");
                        }
                        if it.tie_start.contains(pitch) {
                            x.push_str("<tied type=\"start\"/>");
                        }
                        x.push_str("</notations>");
                    }
                    x.push_str("</note>\n");
                }
            }
            x.push_str("    </measure>\n");
            over_budget(&x, budget)?;
        }
        x.push_str("  </part>\n");
    }
    x.push_str("</score-partwise>\n");
    over_budget(&x, budget)?;
    Ok(x)
}

/// Staff position of `pitch` in half-spaces above the bottom line (treble E4 / bass G2).
fn staff_step(pitch: u8, bass: bool) -> i32 {
    let (step, _, octave) = spell(pitch);
    let idx = "CDEFGAB".find(step).map_or(0, |i| i as i32);
    let diatonic = octave * 7 + idx;
    diatonic - if bass { 2 * 7 + 4 } else { 4 * 7 + 2 }
}

/// An engraved score as a printable SVG page (A4 portrait width, as tall as needed).
pub fn svg(parts: &[Part], setup: &ScoreSetup) -> Result<String, String> {
    svg_budgeted(parts, setup, MAX_OUTPUT_BYTES)
}

/// SVG with an explicit byte budget, capped at 256 MiB.
pub fn svg_budgeted(parts: &[Part], setup: &ScoreSetup, budget: usize) -> Result<String, String> {
    validate(parts, setup)?;
    let budget = budget.min(MAX_OUTPUT_BYTES);
    const W: f64 = 794.0;
    const MARGIN: f64 = 48.0;
    const SP: f64 = 7.0; // staff space
    const STAFF_GAP: f64 = 70.0;
    const SYSTEM_GAP: f64 = 40.0;
    let n_measures = parts.iter().map(|p| p.measures.len()).max().unwrap_or(0);
    // The layout range the setup offers (1 to 16): a caller's own value can't multiply the
    // barlines drawn per system beyond it.
    let per = setup.bars_per_system.clamp(1, 16) as usize;
    let systems = n_measures.div_ceil(per).max(1);
    let label_w = if setup.show_track_names { 70.0 } else { 0.0 };
    let staff_x0 = MARGIN + label_w + 34.0;
    let staff_x1 = W - MARGIN;
    let sys_h = parts.len().max(1) as f64 * STAFF_GAP + SYSTEM_GAP;
    let top = 110.0;
    let h = top + systems as f64 * sys_h + MARGIN;
    let mut o = String::new();
    let _ =
        writeln!(o, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{W}\" height=\"{h:.0}\" viewBox=\"0 0 {W} {h:.0}\" font-family=\"serif\">");
    let _ = writeln!(o, "<rect width=\"100%\" height=\"100%\" fill=\"white\"/>");
    let _ = writeln!(o, "<text x=\"{}\" y=\"56\" font-size=\"24\" text-anchor=\"middle\">{}</text>", W / 2.0, xml_escape(&setup.title));
    if !setup.composer.is_empty() {
        let _ = writeln!(o, "<text x=\"{}\" y=\"84\" font-size=\"13\" text-anchor=\"end\">{}</text>", W - MARGIN, xml_escape(&setup.composer));
        over_budget(&o, budget)?;
    }
    over_budget(&o, budget)?;
    let bar_w = (staff_x1 - staff_x0) / per as f64;
    for sys in 0..systems {
        let y_sys = top + sys as f64 * sys_h;
        for (pi, p) in parts.iter().enumerate() {
            let y0 = y_sys + pi as f64 * STAFF_GAP; // top line
            let bottom = y0 + 4.0 * SP;
            if setup.show_track_names && sys == 0 {
                let _ = writeln!(o, "<text x=\"{MARGIN}\" y=\"{:.1}\" font-size=\"11\">{}</text>", y0 + 2.0 * SP + 4.0, xml_escape(&p.name));
            }
            for l in 0..5 {
                let y = y0 + f64::from(l) * SP;
                let _ = writeln!(
                    o,
                    "<line x1=\"{:.1}\" y1=\"{y:.1}\" x2=\"{staff_x1:.1}\" y2=\"{y:.1}\" stroke=\"black\" stroke-width=\"0.8\"/>",
                    staff_x0 - 30.0
                );
            }
            let clef = if p.bass { "𝄢" } else { "𝄞" };
            let _ = writeln!(
                o,
                "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{}\">{clef}</text>",
                staff_x0 - 29.0,
                bottom + if p.bass { -SP } else { SP * 0.6 },
                if p.bass { 26 } else { 36 }
            );
            for k in 0..per {
                let mi = sys * per + k;
                let x0 = staff_x0 + k as f64 * bar_w;
                let _ = writeln!(
                    o,
                    "<line x1=\"{:.1}\" y1=\"{y0:.1}\" x2=\"{:.1}\" y2=\"{bottom:.1}\" stroke=\"black\" stroke-width=\"1\"/>",
                    x0 + bar_w,
                    x0 + bar_w
                );
                let Some(m) = p.measures.get(mi) else { continue };
                if pi == 0 {
                    let _ = writeln!(o, "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\">{}</text>", x0 + 2.0, y0 - 8.0, mi + 1);
                }
                let total: u32 = m.items.iter().map(|i| i.units).sum::<u32>().max(1);
                let mut acc = 0u32;
                for it in &m.items {
                    let x = x0 + 10.0 + (bar_w - 16.0) * f64::from(acc) / f64::from(total);
                    acc += it.units;
                    let (_, dot) = note_type(it.units);
                    if it.pitches.is_empty() {
                        rest(&mut o, x, y0, SP, it.units);
                        if dot {
                            let _ = writeln!(o, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"1.3\"/>", x + 7.0, y0 + 1.5 * SP);
                        }
                        continue;
                    }
                    let steps: Vec<i32> = it.pitches.iter().map(|p2| staff_step(*p2, p.bass)).collect();
                    let filled = it.units < 8;
                    for (pitch, st) in it.pitches.iter().zip(&steps) {
                        let y = bottom - f64::from(*st) * SP / 2.0;
                        // Ledger lines.
                        let mut l = if *st < 0 { -2 } else { 10 };
                        while (*st < 0 && l >= *st) || (*st > 8 && l <= *st) {
                            let ly = bottom - f64::from(l) * SP / 2.0;
                            let _ = writeln!(
                                o,
                                "<line x1=\"{:.1}\" y1=\"{ly:.1}\" x2=\"{:.1}\" y2=\"{ly:.1}\" stroke=\"black\" stroke-width=\"0.8\"/>",
                                x - 7.0,
                                x + 7.0
                            );
                            l += if *st < 0 { -2 } else { 2 };
                        }
                        let fill = if filled { "black" } else { "white" };
                        let _ = writeln!(
                            o,
                            "<ellipse cx=\"{x:.1}\" cy=\"{y:.1}\" rx=\"4.6\" ry=\"3.3\" transform=\"rotate(-20 {x:.1} {y:.1})\" fill=\"{fill}\" stroke=\"black\" stroke-width=\"1.1\"/>"
                        );
                        if spell(*pitch).1 != 0 {
                            let _ = writeln!(o, "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"12\">♯</text>", x - 14.0, y + 4.0);
                        }
                        if dot {
                            let _ = writeln!(o, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"1.3\"/>", x + 8.0, y - 1.5);
                        }
                    }
                    if it.units < 16 {
                        let (lo, hi) = (steps.iter().copied().min().unwrap_or(4), steps.iter().copied().max().unwrap_or(4));
                        let up = (lo + hi) < 8;
                        let (sx, y_from, y_to) = if up {
                            (x + 4.3, bottom - f64::from(lo) * SP / 2.0, bottom - f64::from(hi) * SP / 2.0 - 3.5 * SP)
                        } else {
                            (x - 4.3, bottom - f64::from(hi) * SP / 2.0, bottom - f64::from(lo) * SP / 2.0 + 3.5 * SP)
                        };
                        let _ = writeln!(
                            o,
                            "<line x1=\"{sx:.1}\" y1=\"{y_from:.1}\" x2=\"{sx:.1}\" y2=\"{y_to:.1}\" stroke=\"black\" stroke-width=\"1\"/>"
                        );
                        let flags = match it.units {
                            1 => 2,
                            2 | 3 => 1,
                            _ => 0,
                        };
                        for f in 0..flags {
                            let fy = y_to + f64::from(f) * if up { 5.0 } else { -5.0 };
                            let dy = if up { 9.0 } else { -9.0 };
                            let _ = writeln!(
                                o,
                                "<path d=\"M{sx:.1} {fy:.1} q 7 {:.1} 5 {dy:.1}\" fill=\"none\" stroke=\"black\" stroke-width=\"1.4\"/>",
                                dy * 0.5
                            );
                        }
                    }
                    for pitch in &it.tie_start {
                        let above = it.pitches.first().is_some_and(|low| pitch > low);
                        let direction = if above { -1.0 } else { 1.0 };
                        let y = bottom - f64::from(staff_step(*pitch, p.bass)) * SP / 2.0 + direction * 6.0;
                        let _ = writeln!(
                            o,
                            "<path d=\"M{:.1} {y:.1} q 8 {} 16 0\" fill=\"none\" stroke=\"black\" stroke-width=\"0.9\"/>",
                            x + 3.0,
                            direction * 5.0
                        );
                    }
                }
                // Check each measure, so a run of rest-only or missing measures that only draws
                // bar lines cannot grow the output past the budget untouched.
                over_budget(&o, budget)?;
            }
            // The staff markup (lines, clef, track names) itself grows with the part count even
            // without any items, so check once per staff too.
            over_budget(&o, budget)?;
        }
        // System bracket.
        let y_last = y_sys + (parts.len().max(1) - 1) as f64 * STAFF_GAP + 4.0 * SP;
        let _ = writeln!(
            o,
            "<line x1=\"{:.1}\" y1=\"{y_sys:.1}\" x2=\"{:.1}\" y2=\"{y_last:.1}\" stroke=\"black\" stroke-width=\"1.5\"/>",
            staff_x0 - 30.0,
            staff_x0 - 30.0
        );
        over_budget(&o, budget)?;
    }
    o.push_str("</svg>\n");
    over_budget(&o, budget)?;
    Ok(o)
}

fn rest(o: &mut String, x: f64, y0: f64, sp: f64, units: u32) {
    let mid = y0 + 2.0 * sp;
    match units {
        16 => {
            let _ = writeln!(o, "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"9\" height=\"{:.1}\"/>", x - 4.5, y0 + sp, sp / 2.0);
        }
        8 | 12 => {
            let _ = writeln!(o, "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"9\" height=\"{:.1}\"/>", x - 4.5, mid - sp / 2.0, sp / 2.0);
        }
        4 | 6 => {
            let _ = writeln!(
                o,
                "<path d=\"M{:.1} {:.1} l 4 5 l -4 4 l 4 5 q -6 -2 -2 5\" fill=\"none\" stroke=\"black\" stroke-width=\"1.8\"/>",
                x - 2.0,
                mid - 9.0
            );
        }
        _ => {
            let _ = writeln!(o, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2\"/>", x - 2.0, mid - 3.0);
            let _ = writeln!(
                o,
                "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"black\" stroke-width=\"1.2\"/>",
                x + 2.5,
                mid - 4.0,
                x - 1.0,
                mid + 7.0
            );
            if units == 1 {
                let _ = writeln!(o, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2\"/>", x - 3.5, mid + 2.0);
            }
        }
    }
}

// Public notation data can come from callers other than SoundCraft's bounded layout.
// Validate before writing/escaping so malicious dimensions cannot overflow sums or create
// an unbounded temporary string before the output-budget checks run.
fn validate(parts: &[Part], setup: &ScoreSetup) -> Result<(), String> {
    let invalid = || "score exceeds supported notation limits".to_string();
    if parts.len() > 4096 {
        return Err(invalid());
    }
    let mut text = setup.title.len().checked_add(setup.composer.len()).and_then(|n| n.checked_add(setup.software.len())).ok_or_else(invalid)?;
    for part in parts {
        text = text.checked_add(part.name.len()).ok_or_else(invalid)?;
        if text > 16 * 1024 * 1024 || part.measures.len() > 2000 {
            return Err(invalid());
        }
        for measure in &part.measures {
            if measure.items.len() > 128 {
                return Err(invalid());
            }
            for item in &measure.items {
                if ![1, 2, 3, 4, 6, 8, 12, 16].contains(&item.units)
                    || item.pitches.len() > 16
                    || item.tie_start.len() > 16
                    || item.tie_stop.len() > 16
                    || item.pitches.iter().chain(&item.tie_start).chain(&item.tie_stop).any(|p| *p > 127)
                {
                    return Err(invalid());
                }
            }
        }
    }
    if text > 16 * 1024 * 1024 {
        return Err(invalid());
    }
    Ok(())
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    fn setup() -> ScoreSetup {
        ScoreSetup { software: "SoundCraft".into(), title: "A<&\u{1}".into(), composer: "B".into(), bars_per_system: 4, show_track_names: true }
    }
    fn parts() -> Vec<Part> {
        vec![Part {
            name: "Keys".into(),
            bass: false,
            measures: vec![Measure {
                numerator: 4,
                denominator: 4,
                items: vec![
                    Item { pitches: vec![60, 64], units: 4, tie_start: vec![64], tie_stop: vec![] },
                    Item { pitches: vec![64], units: 12, tie_start: vec![], tie_stop: vec![64] },
                ],
            }],
        }]
    }
    #[test]
    fn per_pitch_ties_and_escaping_survive_serialization() {
        let parts = parts();
        let xml = musicxml(&parts, &setup()).unwrap();
        assert!(xml.contains("<work-title>A&lt;&amp;</work-title>"));
        let notes: Vec<_> = xml.split("<note>").skip(1).map(|n| n.split("</note>").next().unwrap()).collect();
        assert!(notes[0].contains("<duration>4</duration>"));
        assert!(!notes[0].contains("<tie "));
        assert!(notes[1].contains("<tie type=\"start\"/>") && notes[1].contains("<chord/>"));
        assert!(notes[2].contains("<duration>12</duration>") && notes[2].contains("<tie type=\"stop\"/>"));
        assert_eq!(svg(&parts, &setup()).unwrap().matches("stroke-width=\"0.9\"").count(), 1);
    }
    #[test]
    fn byte_budgets_and_hostile_durations_fail() {
        assert!(musicxml_budgeted(&parts(), &setup(), 32).is_err());
        assert!(svg_budgeted(&parts(), &setup(), 32).is_err());
        let mut parts = parts();
        parts[0].measures[0].items[0].units = u32::MAX;
        assert!(musicxml(&parts, &setup()).is_err());
        assert!(svg(&parts, &setup()).is_err());
    }
    #[test]
    fn spelling_and_staff_steps() {
        assert_eq!(spell(60), ('C', 0, 4));
        assert_eq!(spell(61), ('C', 1, 4));
        assert_eq!(staff_step(64, false), 0); // E4: bottom treble line
        assert_eq!(staff_step(77, false), 8); // F5: top treble line
        assert_eq!(staff_step(43, true), 0); // G2: bottom bass line
    }
}
