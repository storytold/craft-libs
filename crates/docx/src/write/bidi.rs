//! Run direction for mixed right-to-left and left-to-right text (#1543).
//!
//! WordCraft lays a paragraph out with the Unicode Bidirectional Algorithm (UAX #9) from its
//! text and direction alone, so a run need not say its direction. Word orders a run's characters
//! by the run's `w:rtl` (ECMA-376 Part 1 §17.3.2.30): text in a run without it is laid out left
//! to right, so Hebrew words on either side of a Latin name, and the punctuation between them,
//! change places. A run that doesn't say its direction is therefore split where the resolved
//! level changes parity, and the right-to-left pieces are written `w:rtl`, as Word writes text
//! typed in a right-to-left paragraph. Runs that say their direction, and paragraphs without
//! right-to-left characters, are written as they are.
//!
//! Word gives right-to-left text without a bidi language (`w:lang w:bidi`, §17.3.2.20) its
//! default complex-script language, Arabic, so Hebrew would proof and show as Arabic. The pieces
//! marked `w:rtl` therefore also say their language, unless the run or its styles already do.

use std::borrow::Cow;
use std::ops::Range;

use craft_wordmodel::props::CharProps;
use craft_wordmodel::styles::StyleSheet;

/// The language Word expects for right-to-left text of `c`'s script, for the scripts that have
/// one usual language.
fn script_lang(c: char) -> Option<&'static str> {
    match u32::from(c) {
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Some("he-IL"),
        0x0600..=0x06FF | 0x0750..=0x077F | 0x0870..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Some("ar-SA"),
        0x0700..=0x074F => Some("syr-SY"),
        0x0780..=0x07BF => Some("dv-MV"),
        _ => None,
    }
}

/// The language of the first right-to-left letter of `text` with a usual language.
fn text_lang(text: &str) -> Option<&'static str> {
    text.chars().find_map(script_lang)
}

/// A language tag of a right-to-left language (`he-IL`, `yi`, `fa-IR`, `ar-EG`…)?
fn is_rtl_lang(tag: &str) -> bool {
    let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    matches!(primary.as_str(), "he" | "iw" | "yi" | "ar" | "fa" | "ur" | "ps" | "sd" | "ug" | "ckb" | "syr" | "dv" | "ku")
}

/// Does a paragraph of style `para_style` inherit a bidi language from the document defaults or
/// its style chain? (Resolved once per paragraph.)
pub(super) fn para_inherits_lang_bidi(styles: &StyleSheet, para_style: Option<&str>) -> bool {
    styles.para_style_props(para_style.unwrap_or("Normal")).1.lang_bidi.is_some()
}

/// Does `c`'s character style chain give it a bidi language?
pub(super) fn char_style_has_lang_bidi(styles: &StyleSheet, c: &CharProps) -> bool {
    c.style.as_deref().is_some_and(|s| styles.chain(s).iter().any(|st| st.chr.lang_bidi.is_some()))
}

/// Add `rtl` and, when given, the bidi language `lang` to `c` where it doesn't say them.
fn direct(c: &mut CharProps, lang: Option<&String>) {
    c.rtl.get_or_insert(true);
    if let Some(l) = lang
        && c.lang_bidi.is_none()
    {
        c.lang_bidi = Some(l.clone());
    }
}

/// `runs` (byte ranges of `text` with their props), split where the bidi level of `text` in a
/// paragraph of direction `rtl_para` changes parity; right-to-left pieces get `rtl: Some(true)`
/// and, unless they have one or `inherits_lang` says it comes from a style, a bidi language.
pub(super) fn direction_spans<'a>(
    text: &str,
    rtl_para: bool,
    runs: impl Iterator<Item = (Range<usize>, &'a CharProps)>,
    inherits_lang: impl Fn(&CharProps) -> bool,
) -> Vec<(Range<usize>, Cow<'a, CharProps>)> {
    if !craft_wordmodel::bidi::has_rtl(text) {
        return runs.map(|(r, p)| (r, Cow::Borrowed(p))).collect();
    }
    let level = if rtl_para { unicode_bidi::Level::rtl() } else { unicode_bidi::Level::ltr() };
    let info = unicode_bidi::ParagraphBidiInfo::new(text, Some(level));
    let odd = |at: usize| info.levels.get(at).is_some_and(|l| l.is_rtl());
    let para_lang = text_lang(text);
    let mut out = Vec::new();
    for (range, props) in runs {
        let Some(run) = text.get(range.clone()).filter(|_| props.rtl.is_none()) else {
            out.push((range, Cow::Borrowed(props)));
            continue;
        };
        let named = props.lang_bidi.is_some() || inherits_lang(props);
        let piece = |r: Range<usize>, rtl: bool| {
            if !rtl {
                return (r, Cow::Borrowed(props));
            }
            let lang = if named {
                None
            } else {
                let own = props.lang.clone().filter(|l| is_rtl_lang(l));
                own.or_else(|| text.get(r.clone()).and_then(text_lang).or(para_lang).map(str::to_string))
            };
            let mut p = props.clone();
            direct(&mut p, lang.as_ref());
            // A tracked formatting change's "before" copy gets the same, so the change lists
            // nothing the author didn't do.
            if let Some(ch) = p.fmt_change.as_mut() {
                direct(&mut ch.old, lang.as_ref());
            }
            (r, Cow::Owned(p))
        };
        let mut start = range.start;
        let mut cur: Option<bool> = None;
        for (i, _) in run.char_indices() {
            let at = range.start.saturating_add(i);
            let rtl = odd(at);
            match cur {
                Some(c) if c != rtl => {
                    out.push(piece(start..at, c));
                    start = at;
                }
                _ => {}
            }
            cur = Some(rtl);
        }
        out.push(piece(start..range.end, cur.unwrap_or(false)));
    }
    out
}

/// A Thai letter, mark or digit?
fn is_thai(c: char) -> bool {
    matches!(u32::from(c), 0x0E00..=0x0E7F)
}

/// How Word marks `c` when it is typed into Thai text: complex script (`Some(true)`), not
/// (`Some(false)`), or like the text before it (`None`). Measured in Word for Windows.
fn typed_mark(c: char) -> Option<bool> {
    if is_thai(c) {
        Some(true)
    } else if c.is_alphanumeric() || matches!(c, ',' | ';' | '?') {
        Some(false)
    } else {
        None
    }
}

/// Each character of `text` (byte offset, complex script?) as Word marks typed Thai text:
/// punctuation and spaces follow the text before them; at the start, spaces follow the text after
/// them and punctuation is not complex script.
fn typed_marks(text: &str) -> Vec<(usize, bool)> {
    let mut out: Vec<(usize, bool)> = Vec::new();
    let mut prev: Option<bool> = None;
    let mut leading_spaces = Vec::new();
    for (i, c) in text.char_indices() {
        let mark = match (typed_mark(c), prev) {
            (Some(m), _) | (None, Some(m)) => m,
            (None, None) if c.is_whitespace() => {
                leading_spaces.push(out.len());
                out.push((i, false));
                continue;
            }
            (None, None) => false,
        };
        if prev.is_none() {
            for &k in &leading_spaces {
                if let Some(m) = out.get_mut(k) {
                    m.1 = mark;
                }
            }
        }
        prev = Some(mark);
        out.push((i, mark));
    }
    out
}

/// Fill in `c`, a piece now written complex script, the complex-script font, size, bold and italic
/// it was drawn with where `m` (all its formatting, merged as the resolver merges it) sets none:
/// Word draws a `w:cs` run with those values alone (10 pt without a `w:szCs` anywhere), while
/// WordCraft drew the text with the ordinary ones there. What the run sets itself is kept.
fn fill_complex(c: &mut CharProps, m: &CharProps) {
    if m.font_cs.is_none() && m.font_cs_theme.is_none() {
        c.font_cs = m.font.clone();
    }
    if m.size_cs.is_none() {
        c.size_cs = Some(m.size.unwrap_or(craft_wordmodel::props::DEFAULT_FONT_SIZE));
    }
    if m.bold_cs.is_none() && m.bold == Some(true) {
        c.bold_cs = Some(true);
    }
    if m.italic_cs.is_none() && m.italic == Some(true) {
        c.italic_cs = Some(true);
    }
}

/// `spans` with `w:cs` written, in one pass, where Word needs it to draw and break a run's text as
/// WordCraft does. `table` is the run formatting the table style gives the cell the paragraph is
/// in, so a run's formatting is resolved there as in the table.
///
/// - Word breaks lines inside Thai words (by its dictionary) only in `w:cs` runs, and marks Thai
///   as it is typed: Thai letters and digits complex script, Latin letters, Western digits and
///   `,` `;` `?` not, other punctuation and spaces with the text before them. A run's Thai is
///   written that way, punctuation and spaces with it only where they look the same either way.
/// - Word draws a run's characters with the complex-script font, size, bold and italic only when
///   the run is complex script or right to left by its own properties, its paragraph style, its
///   table style or the run defaults; a character style saying so doesn't count. Anywhere else it
///   draws them in the run's other font and `w:sz`, Thai included (measured in Word for Windows).
///   WordCraft also draws Thai, Lao and Indic characters with the complex-script formatting, and
///   counts a character style's `w:cs`. So where the two disagree the text is written `w:cs` (or
///   `w:cs w:val="0"`) as WordCraft draws it.
///
/// A piece written `w:cs` gets the complex-script values it was drawn with where nothing sets
/// them ([`fill_complex`]); a tracked formatting change's "before" copy is marked by its own
/// formatting. Runs that say whether they are complex script or right to left pass through.
pub(super) fn complex_script_spans<'a>(
    text: &str,
    spans: Vec<(Range<usize>, Cow<'a, CharProps>)>,
    styles: &StyleSheet,
    para_style: Option<&str>,
    table: Option<&CharProps>,
) -> Vec<(Range<usize>, Cow<'a, CharProps>)> {
    let typed = text.chars().any(is_thai).then(|| typed_marks(text));
    let typed_at = |at: usize| typed.as_ref().is_some_and(|m| m.binary_search_by_key(&at, |x| x.0).ok().and_then(|k| m.get(k)).is_some_and(|x| x.1));
    let mut out = Vec::with_capacity(spans.len());
    for (range, props) in spans {
        let Some(run) = text.get(range.clone()) else {
            out.push((range, props));
            continue;
        };
        let need = marks(styles, para_style, table, &props);
        // A tracked formatting change's "before" copy, where it doesn't say either, is marked as
        // WordCraft draws it, by its own character style: rejecting the change in Word then
        // brings back the look it had. Where the run is marked, the before copy is marked too, so
        // the change lists the mark only where the look changed; elsewhere it is marked by what
        // it needs itself.
        let old = props.fmt_change.as_ref().map(|ch| &ch.old).filter(|o| o.cs.is_none() && o.rtl.is_none());
        let old_rc = old.map(|o| styles.resolve_char_in(para_style, table, o));
        let old_need = old.and_then(|o| marks(styles, para_style, table, o));
        let key = |at: usize, c: char| {
            let thai = typed_at(at);
            let n = need.as_ref().and_then(|f| f(c, thai));
            let o = if n.is_some() { old_rc.as_ref().map(|r| r.uses_complex(c)) } else { old_need.as_ref().and_then(|f| f(c, thai)) };
            (n, o)
        };
        if run.char_indices().all(|(i, c)| key(range.start.saturating_add(i), c) == (None, None)) {
            out.push((range, props));
            continue;
        }
        let merged = styles.char_props_in(para_style, table, &props);
        let old_merged = old.map(|o| styles.char_props_in(para_style, table, o));
        let piece = |(n, old_cs): (Option<bool>, Option<bool>)| {
            if n.is_none() && old_cs.is_none() {
                return props.clone();
            }
            let mut p = props.clone().into_owned();
            if n.is_some() {
                p.cs = n;
                if n == Some(true) {
                    fill_complex(&mut p, &merged);
                }
            }
            if let (Some(ch), Some(o)) = (p.fmt_change.as_mut(), old_cs) {
                ch.old.cs = Some(o);
                if let (true, Some(m)) = (o, old_merged.as_ref()) {
                    fill_complex(&mut ch.old, m);
                }
            }
            Cow::Owned(p)
        };
        let mut start = range.start;
        let mut cur: Option<(Option<bool>, Option<bool>)> = None;
        for (i, c) in run.char_indices() {
            let at = range.start.saturating_add(i);
            let n = key(at, c);
            if let Some(was) = cur
                && was != n
            {
                out.push((start..at, piece(was)));
                start = at;
            }
            cur = Some(n);
        }
        out.push((start..range.end, piece(cur.unwrap_or((None, None)))));
    }
    out
}

/// For [`complex_script_spans`]: what a character `c` of a run of formatting `p` needs written,
/// given whether Word marks it complex script when typed (`typed_thai`): `Some(complex)` for typed
/// Thai WordCraft draws in the complex-script formatting (or where both formattings look the
/// same), so Word breaks it by word, and where Word would draw it otherwise than WordCraft does;
/// `None` for a run that says whether it is complex script or right to left. A run that is right
/// to left by its styles stays complex script with `w:cs w:val="0"`, so that isn't written.
fn marks(
    styles: &StyleSheet,
    para_style: Option<&str>,
    table: Option<&CharProps>,
    p: &CharProps,
) -> Option<impl Fn(char, bool) -> Option<bool> + use<>> {
    if p.cs.is_some() || p.rtl.is_some() {
        return None;
    }
    let rc = styles.resolve_char_in(para_style, table, p);
    let cs = rc.complex();
    let same = rc.font == cs.font && rc.size == cs.size && rc.bold == cs.bold && rc.italic == cs.italic;
    let word = styles.resolve_char_in(para_style, table, &CharProps { style: None, ..p.clone() });
    let (word_cs, word_rtl) = (word.cs || word.rtl, word.rtl);
    Some(move |c: char, typed_thai: bool| {
        let draws = rc.uses_complex(c);
        if typed_thai && !word_cs && (draws || same) {
            return Some(true);
        }
        if same {
            return None;
        }
        (draws != word_cs && (draws || !word_rtl)).then_some(draws)
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_ranges_never_panic() {
        let p = CharProps::default();
        let text = "עברית WordCraft";
        let runs = vec![(0..3, &p), (3..1000, &p), (usize::MAX - 1..usize::MAX, &p), (2..text.len(), &p)];
        let out = direction_spans(text, true, runs.into_iter(), |_| false);
        assert_eq!(out.len(), 5, "bad ranges pass through unsplit: {out:?}");
    }

    #[test]
    fn languages_by_script() {
        assert_eq!(text_lang("(שלום"), Some("he-IL"));
        assert_eq!(text_lang("سلام"), Some("ar-SA"));
        assert_eq!(text_lang("ܫܠܡܐ"), Some("syr-SY"));
        assert_eq!(text_lang(". ,"), None);
        assert!(is_rtl_lang("he-IL") && is_rtl_lang("yi") && is_rtl_lang("FA_ir") && !is_rtl_lang("en-US") && !is_rtl_lang(""));
    }

    /// `text` as one plain run through [`complex_script_spans`] (built-in styles: both
    /// formattings look the same), written `[cs]{not}`.
    fn split(text: &str) -> String {
        let p = CharProps::default();
        let styles = StyleSheet::default();
        complex_script_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], &styles, None, None)
            .iter()
            .map(|(r, c)| {
                let t = &text[r.clone()];
                if c.cs == Some(true) { format!("[{t}]") } else { format!("{{{t}}}") }
            })
            .collect()
    }

    /// Where both formattings look the same, Thai is split as Word marks typed text (Word for
    /// Windows, text typed through its object model and saved).
    #[test]
    fn thai_is_split_as_word_marks_typed_text() {
        for (text, word) in [
            ("ภาษาไทยล้วน", "[ภาษาไทยล้วน]"),
            ("ไทย ปน English 123 และ ๑๒๓ ด้วย", "[ไทย ปน ]{English 123 }[และ ๑๒๓ ด้วย]"),
            ("วันที่ 29 ตุลาคม 2569 (ทดสอบ), \"อ้างอิง\".", "[วันที่ ]{29 }[ตุลาคม ]{2569 (}[ทดสอบ)]{, \"}[อ้างอิง\".]"),
            ("ก, ข", "[ก]{, }[ข]"),
            ("ก ,ข", "[ก ]{,}[ข]"),
            ("(ก)", "{(}[ก)]"),
            ("ก (a)", "[ก (]{a)}"),
            ("ก1ข", "[ก]{1}[ข]"),
            (" ก", "[ ก]"),
            ("\"ก\"", "{\"}[ก\"]"),
            ("ก) ข", "[ก) ข]"),
            ("ก?", "[ก]{?}"),
            ("ก; ข", "[ก]{; }[ข]"),
            ("ก: ข", "[ก: ข]"),
            ("ก ๆ ข", "[ก ๆ ข]"),
            ("1ก", "{1}[ก]"),
            ("ก 1.5", "[ก ]{1.5}"),
        ] {
            assert_eq!(split(text), word, "{text}");
        }
        // No Thai: unchanged.
        let text = "English only 2026";
        let p = CharProps::default();
        let out = complex_script_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], &StyleSheet::default(), None, None);
        assert!(matches!(out.as_slice(), [(r, Cow::Borrowed(_))] if *r == (0..text.len())));
    }

    /// A marked piece keeps the font, size, bold and italic it was drawn with: Word draws a `w:cs`
    /// run with the complex-script values alone, so the ordinary ones are written there where
    /// nothing sets complex-script values; ones the styles set are left to them. Punctuation
    /// between Thai joins it only where both formattings look the same.
    #[test]
    fn marked_pieces_keep_the_font_size_bold_and_italic_they_were_drawn_with() {
        let text = "ไทย ABC";
        let p = CharProps { font: Some("Leelawadee UI".into()), size: Some(16.0), bold: Some(true), italic: Some(true), ..CharProps::default() };
        let styles = StyleSheet::default();
        let out = complex_script_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], &styles, None, None);
        let c = &out[0].1;
        assert_eq!(
            (c.cs, c.font_cs.as_deref(), c.size_cs, c.bold_cs, c.italic_cs),
            (Some(true), Some("Leelawadee UI"), Some(16.0), Some(true), Some(true))
        );
        assert_eq!((out[1].1.cs, out[1].1.font_cs.as_deref(), out[1].1.size_cs), (None, None, None), "the Latin piece is left as it was");
        // Complex-script values the styles give are kept, not overwritten; the space after the
        // Thai, drawn in the other formatting, stays out of the marked piece.
        let mut styles = StyleSheet::default();
        let cs = CharProps {
            font_cs: Some("TH SarabunPSK".into()),
            size_cs: Some(18.0),
            bold_cs: Some(false),
            italic_cs: Some(false),
            ..Default::default()
        };
        styles.default_chr.overlay(&cs);
        let out = complex_script_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], &styles, None, None);
        assert_eq!(&text[out[0].0.clone()], "ไทย");
        let c = &out[0].1;
        assert_eq!((c.cs, c.font_cs.as_deref(), c.size_cs, c.bold_cs, c.italic_cs), (Some(true), None, None, None, None));
    }
}
