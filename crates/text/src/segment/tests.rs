use super::kinsoku::{no_line_end, no_line_start};
use super::*;

fn opts(kinsoku: Kinsoku) -> Options {
    Options { kinsoku, ..Options::default() }
}

/// Byte offsets of the breaks before the end of the text.
fn offsets(text: &str, options: &Options) -> Vec<usize> {
    line_breaks(text, options).iter().map(|b| b.offset).filter(|&o| o < text.len()).collect()
}

/// The text from each break before the end to the next break.
fn pieces(text: &str, options: &Options) -> Vec<String> {
    let mut from = 0;
    let mut out = Vec::new();
    for b in line_breaks(text, options) {
        out.push(text.get(from..b.offset).unwrap_or_default().to_string());
        from = b.offset;
    }
    out
}

/// The character each line after the first could start with.
fn starts(text: &str, options: &Options) -> Vec<char> {
    offsets(text, options).iter().filter_map(|&o| text.get(o..).and_then(|t| t.chars().next())).collect()
}

#[test]
fn kinsoku_keeps_closing_marks_off_line_starts_and_opening_ones_off_line_ends() {
    for (a, b) in
        [('字', '。'), ('弧', '」'), ('」', '、'), ('ャ', 'ー'), ('カ', 'ッ'), ('「', 'か'), ('（', '雅'), ('字', '〞'), ('１', '％'), ('＄', '１')]
    {
        assert!(!kinsoku_allows(a, Some(b), Kinsoku::Hard), "{a}{b}");
    }
    for (a, b) in [('。', '雅'), ('」', 'は'), ('字', '「'), ('の', '演')] {
        assert!(kinsoku_allows(a, Some(b), Kinsoku::Hard), "{a}{b}");
    }
    assert!(kinsoku_allows('。', None, Kinsoku::Hard));
    // No East Asian character on either side: Latin rules apply.
    assert!(kinsoku_allows(' ', Some('–'), Kinsoku::Hard));
    assert!(!kinsoku_allows('字', Some(')'), Kinsoku::Hard));
}

/// Soft kinsoku lets 々, ー and small kana (JLREQ cl-09's 々, cl-10, cl-11) start a line and keeps
/// the rest of the set (half-width forms aren't in those classes); None allows any break.
#[test]
fn soft_kinsoku_lets_small_kana_and_the_prolonged_sound_mark_start_a_line() {
    for (a, b) in [('ャ', 'ー'), ('カ', 'ッ'), ('時', '々'), ('ト', 'ㇰ'), ('か', 'ゃ')] {
        assert!(!kinsoku_allows(a, Some(b), Kinsoku::Hard), "{a}{b}");
        assert!(kinsoku_allows(a, Some(b), Kinsoku::Soft), "{a}{b}");
    }
    for (a, b) in [('字', '。'), ('弧', '」'), ('「', 'か'), ('ｶ', 'ｯ'), ('ｶ', 'ｰ'), ('字', 'ゝ')] {
        assert!(!kinsoku_allows(a, Some(b), Kinsoku::Soft), "{a}{b}");
        assert!(kinsoku_allows(a, Some(b), Kinsoku::None), "{a}{b}");
    }
}

#[test]
fn no_break_puts_a_closing_mark_at_a_line_start_or_an_opening_one_at_a_line_end() {
    let text = "あいうえお。かきくけ「こさしすせそ」";
    let breaks = offsets(text, &Options::default());
    assert!(breaks.len() > 10, "{breaks:?}");
    for o in breaks {
        let (head, tail) = text.split_at(o);
        let (Some(last), Some(first)) = (head.chars().next_back(), tail.chars().next()) else { continue };
        assert!(!no_line_start(first) && !no_line_end(last), "break at {o}: {last}|{first}");
    }
}

/// あカッ breaks before ッ with Soft and None (a small kana may start a line) and not with Hard; a
/// full stop never starts a line, Soft or not.
#[test]
fn kinsoku_mode_decides_whether_a_small_kana_may_start_a_line() {
    assert_eq!(starts("あカッ", &opts(Kinsoku::Hard)), ['カ']);
    assert_eq!(starts("あカッ", &opts(Kinsoku::Soft)), ['カ', 'ッ']);
    assert_eq!(starts("あカッ", &opts(Kinsoku::None)), ['カ', 'ッ']);
    assert_eq!(starts("あカ。", &opts(Kinsoku::Soft)), ['カ']);
    assert_eq!(starts("あカ。", &opts(Kinsoku::None)), ['カ', '。']);
}

#[test]
fn kinsoku_adjusts_uax14_next_to_east_asian_text_only() {
    let ja = "日本語、「漢字」。テスト";
    let on = starts(ja, &opts(Kinsoku::Hard));
    for c in ['、', '」', '。', '漢'] {
        assert!(!on.contains(&c), "kinsoku: no line starts with {c}: {on:?}");
    }
    assert!(on.contains(&'「') && on.contains(&'テ'), "{on:?}");
    let off = starts(ja, &opts(Kinsoku::None));
    for c in ['、', '」', '。', '漢'] {
        assert!(off.contains(&c), "no kinsoku: a line may start with {c}: {off:?}");
    }
    let en = "Hello wonderful (world), 50%.";
    for k in [Kinsoku::None, Kinsoku::Soft] {
        assert_eq!(starts(en, &opts(k)), starts(en, &opts(Kinsoku::Hard)));
    }
    assert_eq!(starts(en, &Options::default()), ['w', '(', '5']);
}

#[test]
fn breaks_are_byte_offsets_and_newlines_are_mandatory() {
    let b = line_breaks("a b\nc", &Options::default());
    let got: Vec<(usize, bool)> = b.iter().map(|b| (b.offset, b.mandatory)).collect();
    assert_eq!(got, [(2, false), (4, true), (5, true)]);
    assert!(line_breaks("", &Options::default()).is_empty());
    // A variation selector belongs to its ideograph: no break before it, kinsoku reads the base.
    assert_eq!(pieces("字\u{FE00}字\u{FE00}。", &Options::default()), ["字\u{FE00}", "字\u{FE00}。"]);
}

/// The East Asian breaks that None and Soft add to UAX #14 never split a cluster: not after a
/// joiner, not before a combining mark or variation selector.
#[test]
fn no_kinsoku_mode_breaks_inside_a_cluster() {
    for k in [Kinsoku::None, Kinsoku::Soft, Kinsoku::Hard] {
        let o = opts(k);
        assert_eq!(pieces("日本\u{200D}語", &o), ["日", "本\u{200D}語"], "{k:?}");
        assert_eq!(pieces("あカ\u{200D}ッ", &o), ["あ", "カ\u{200D}ッ"], "{k:?}");
        assert_eq!(pieces("葛\u{E0100}城", &o), ["葛\u{E0100}", "城"], "{k:?}");
        assert_eq!(pieces("字\u{3099}\u{FE00}字", &o), ["字\u{3099}\u{FE00}", "字"], "{k:?}");
    }
}

#[test]
fn korean_words_break_at_spaces_only() {
    let text = "한국어 문장(文章)입니다";
    assert_eq!(pieces(text, &Options::default()), ["한", "국", "어 ", "문", "장", "(文", "章)", "입", "니", "다"]);
    let words = Options { korean_words: true, ..Options::default() };
    assert_eq!(pieces(text, &words), ["한국어 ", "문장", "(文章)", "입니다"]);
}

#[cfg(not(feature = "thai"))]
#[test]
fn thai_without_its_feature_breaks_as_uax14() {
    assert_eq!(pieces("ภาษาไทยภาษาไทย ไทย", &Options::default()), ["ภาษาไทยภาษาไทย ", "ไทย"]);
}

/// Each model available in this build, for the complex-script tests.
#[cfg(any(feature = "thai", feature = "lao", feature = "khmer", feature = "burmese"))]
fn models() -> Vec<(&'static str, complex::Model)> {
    use icu_segmenter::LineSegmenter;
    #[allow(unused_mut)]
    let mut models: Vec<(&str, complex::Model)> = vec![("dictionary", LineSegmenter::new_dictionary)];
    #[cfg(feature = "lstm")]
    models.push(("lstm", LineSegmenter::new_lstm));
    models
}

/// Breaks every model puts in `words.concat()` fall between two of `words`, and there are at least
/// `min` of them. `words` splits compounds where a model may break them.
#[cfg(any(feature = "thai", feature = "lao", feature = "khmer", feature = "burmese"))]
fn assert_word_breaks(words: &[&str], min: usize) {
    let text = words.concat();
    let bounds: Vec<usize> = words
        .iter()
        .scan(0, |end, w| {
            *end += w.len();
            Some(*end)
        })
        .collect();
    for (name, model) in models() {
        let breaks = breaks_with_words(&text, &Options::default(), complex::word_breaks(&text, model));
        let inside: Vec<usize> = breaks.iter().map(|b| b.offset).filter(|&o| o < text.len()).collect();
        let shown: Vec<&str> = std::iter::once(0)
            .chain(inside.iter().copied())
            .zip(inside.iter().copied().chain([text.len()]))
            .filter_map(|(a, b)| text.get(a..b))
            .collect();
        assert!(inside.iter().all(|o| bounds.contains(o)), "{name}: {shown:?}");
        assert!(inside.len() >= min, "{name}: {shown:?}");
    }
}

#[cfg(feature = "thai")]
#[test]
fn thai_breaks_between_words() {
    assert_word_breaks(&["Thai: ", "ภาษา", "ไทย", "เป็น", "ภาษา", "ราชการ", "ของ", "ประเทศ", "ไทย."], 6);
}

#[cfg(feature = "lao")]
#[test]
fn lao_breaks_between_words() {
    assert_word_breaks(&["ພາສາ", "ລາວ", "ເປັນ", "ພາສາ", "ລາດ", "ຊະ", "ການ", "ຂອງ", "ປະເທດ", "ລາວ"], 5);
}

#[cfg(feature = "khmer")]
#[test]
fn khmer_breaks_between_words() {
    assert_word_breaks(&["ភាសា", "ខ្មែរ", "ជា", "ភាសា", "ផ្លូវ", "ការ", "នៃ", "ប្រទេស", "កម្ពុជា"], 5);
}

/// Burmese may break between any two syllables, and the LSTM model splits some words there
/// (မြန်|မာ): the expected breaks are the syllable boundaries.
#[cfg(feature = "burmese")]
#[test]
fn burmese_breaks_between_syllables() {
    let syllables = ["မြန်", "မာ", "ဘာ", "သာ", "စ", "ကား", "သည်", "မြန်", "မာ", "နိုင်", "ငံ", "၏", "ရုံး", "သုံး", "ဘာ", "သာ", "စ", "ကား", "ဖြစ်", "သည်"];
    assert_word_breaks(&syllables, 5);
}

/// Chinese full-width punctuation: no line starts with a closing mark, none ends with an opening
/// one.
#[test]
fn chinese_kinsoku_keeps_punctuation_with_its_text() {
    let text = "他说：“《红楼梦》是一部小说，（很长）。”";
    let o = Options::for_language(Some("zh-Hans"));
    let breaks = offsets(text, &o);
    assert!(breaks.len() > 8, "{:?}", pieces(text, &o));
    for b in breaks {
        let (head, tail) = text.split_at(b);
        let (Some(last), Some(first)) = (head.chars().next_back(), tail.chars().next()) else { continue };
        assert!(!"：”》，）。".contains(first), "line starts with {first}: {:?}", pieces(text, &o));
        assert!(!"“《（".contains(last), "line ends with {last}: {:?}", pieces(text, &o));
    }
}

#[test]
fn language_picks_korean_words() {
    for tag in ["ko", "ko-KR", "KO_kr"] {
        assert!(Options::for_language(Some(tag)).korean_words, "{tag}");
    }
    for tag in [Some("ja"), Some("kok"), Some("en-KO"), None] {
        assert_eq!(Options::for_language(tag), Options::default(), "{tag:?}");
    }
}

/// Emergency breaks fall between grapheme clusters: never inside an emoji ZWJ sequence, a flag, a
/// skin-tone modifier or a letter with its combining marks.
#[test]
fn emergency_breaks_keep_grapheme_clusters_whole() {
    let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    let text = format!("ab{family}e\u{301}🇯🇵👍🏽");
    let breaks = emergency_breaks(&text, 0..text.len());
    let a = family.len() + 2;
    let b = a + "e\u{301}".len();
    let c = b + "🇯🇵".len();
    assert_eq!(breaks, [1, 2, a, b, c]);
    // Offsets are into `text`; a range off a char boundary has none.
    assert_eq!(emergency_breaks(&text, 1..b), [2, a]);
    assert!(emergency_breaks(&text, 3..b).is_empty());
    assert!(emergency_breaks("", 0..0).is_empty());
}

/// WHY: Baseline sanity check. Single-byte ASCII strings must exhibit identical 1-byte stepping behavior.
/// DANGER: Naive underflow on cursor 0 or overflow on cursor text.len() causes panic in unchecked dec/inc.
/// CONTRACT:
/// - prev_grapheme_offset(text, 0) == 0
/// - next_grapheme_offset(text, text.len()) == text.len()
/// - For all 0 < i <= len: prev_grapheme_offset(text, i) == i - 1
/// - For all 0 <= i < len: next_grapheme_offset(text, i) == i + 1
#[test]
fn grapheme_offsets_empty_and_ascii() {
    assert_eq!(prev_grapheme_offset("", 0), 0);
    assert_eq!(next_grapheme_offset("", 0), 0);

    let text = "hello";
    // Moving backwards from any offset
    assert_eq!(prev_grapheme_offset(text, 0), 0);
    assert_eq!(prev_grapheme_offset(text, 1), 0);
    assert_eq!(prev_grapheme_offset(text, 2), 1);
    assert_eq!(prev_grapheme_offset(text, 3), 2);
    assert_eq!(prev_grapheme_offset(text, 4), 3);
    assert_eq!(prev_grapheme_offset(text, 5), 4);

    // Moving forwards from any offset
    assert_eq!(next_grapheme_offset(text, 0), 1);
    assert_eq!(next_grapheme_offset(text, 1), 2);
    assert_eq!(next_grapheme_offset(text, 2), 3);
    assert_eq!(next_grapheme_offset(text, 3), 4);
    assert_eq!(next_grapheme_offset(text, 4), 5);
    assert_eq!(next_grapheme_offset(text, 5), 5);
}

/// WHY: Precomposed Vietnamese text (NFC) contains multibyte codepoints (e.g. 'ệ' = U+1EC7, 3 bytes).
/// DANGER: Naive byte stepping splits a 3-byte sequence into invalid UTF-8 chunks, panicking downstream code.
/// CONTRACT: Cursor leaps across the entire 3-byte glyph in a single step (range 2..5).
#[test]
fn grapheme_offsets_vietnamese_nfc() {
    // NFC string: 'v' (1B, 0..1), 'i' (1B, 1..2), 'ệ' (3B, 2..5), 'n' (1B, 5..6)
    let nfc = "viện";
    assert_eq!(nfc.len(), 6);
    assert_eq!(prev_grapheme_offset(nfc, 0), 0);
    assert_eq!(prev_grapheme_offset(nfc, 1), 0);
    assert_eq!(prev_grapheme_offset(nfc, 2), 1);
    assert_eq!(prev_grapheme_offset(nfc, 5), 2); // After 'ệ', backspace removes 'ệ' (range 2..5)
    assert_eq!(prev_grapheme_offset(nfc, 6), 5); // After 'n'

    assert_eq!(next_grapheme_offset(nfc, 0), 1);
    assert_eq!(next_grapheme_offset(nfc, 1), 2);
    assert_eq!(next_grapheme_offset(nfc, 2), 5); // Forward past 'ệ'
    assert_eq!(next_grapheme_offset(nfc, 5), 6); // Forward past 'n'
    assert_eq!(next_grapheme_offset(nfc, 6), 6);
}

/// WHY: Canonical decomposed Vietnamese (NFD) stacks multiple combining marks in Canonical Combining Class order:
///      Base vowel + ccc 220 (combining dot below `\u{0323}`) + ccc 230 (combining circumflex `\u{0302}`).
/// DANGER: Backspacing on 'e\u{0323}\u{0302}' with char-by-char deletion peels `\u{0302}`, leaving an invalid orphan tone.
/// CONTRACT:
/// - Backspacing from after the cluster (offset 7) returns cluster start (offset 2).
/// - Deleting from before the cluster (offset 2) returns cluster end (offset 7).
/// - Cursors landing inside the cluster (offsets 3 and 5) snap defensively to bounds (2 and 7).
#[test]
fn grapheme_offsets_vietnamese_nfd_canonical_ordering() {
    // NFD string: 'v' (1B, 0..1), 'i' (1B, 1..2), 'e' (1B, 2..3)
    // + Combining Dot Below (2B, U+0323, ccc 220, 3..5)
    // + Combining Circumflex (2B, U+0302, ccc 230, 5..7)
    // + 'n' (1B, 7..8)
    let nfd = "vie\u{0323}\u{0302}n";
    assert_eq!(nfd.len(), 8);

    // Backward from cluster boundaries
    assert_eq!(prev_grapheme_offset(nfd, 0), 0);
    assert_eq!(prev_grapheme_offset(nfd, 1), 0);
    assert_eq!(prev_grapheme_offset(nfd, 2), 1);
    assert_eq!(prev_grapheme_offset(nfd, 7), 2); // Backspace removes the entire decomposed vowel cluster
    assert_eq!(prev_grapheme_offset(nfd, 8), 7);

    // Forward from cluster boundaries
    assert_eq!(next_grapheme_offset(nfd, 0), 1);
    assert_eq!(next_grapheme_offset(nfd, 1), 2);
    assert_eq!(next_grapheme_offset(nfd, 2), 7); // Delete removes the entire decomposed vowel cluster
    assert_eq!(next_grapheme_offset(nfd, 7), 8);
    assert_eq!(next_grapheme_offset(nfd, 8), 8);

    // Snap-to-boundary recovery for cursors landed inside the decomposed cluster:
    // Offset 3: between base 'e' and dot below '\u{0323}'
    assert_eq!(prev_grapheme_offset(nfd, 3), 2);
    assert_eq!(next_grapheme_offset(nfd, 3), 7);
    // Offset 5: between dot below '\u{0323}' and circumflex '\u{0302}'
    assert_eq!(prev_grapheme_offset(nfd, 5), 2);
    assert_eq!(next_grapheme_offset(nfd, 5), 7);
}

/// WHY: Verifies arbitrary combining mark sequences as specified in docs/vietnamese-text-reliability.md.
/// DANGER: Synthetic or unusual mark combinations must not disrupt extended grapheme cluster boundaries.
/// CONTRACT: In-cluster cursors (offsets 2, 4) snap to cluster start (1) and cluster end (6).
#[test]
fn grapheme_offsets_synthetic_compound_cluster() {
    // Synthetic decomposed cluster: "vi\u{0323}\u{0302}n" (3 clusters: 'v' [0..1], 'i\u{0323}\u{0302}' [1..6], 'n' [6..7])
    let synth = "vi\u{0323}\u{0302}n";
    assert_eq!(synth.len(), 7);
    assert_eq!(prev_grapheme_offset(synth, 6), 1);
    assert_eq!(next_grapheme_offset(synth, 1), 6);
    assert_eq!(prev_grapheme_offset(synth, 2), 1);
    assert_eq!(next_grapheme_offset(synth, 2), 6);
    assert_eq!(prev_grapheme_offset(synth, 4), 1);
    assert_eq!(next_grapheme_offset(synth, 4), 6);
}

/// WHY: Core invariant of Tier 2: Backspace/Delete mutation must be isomorphic between NFC and NFD strings.
/// DANGER: Asymmetric deletion causes desynchronized document models and ghost undo history.
/// CONTRACT: Step-by-step backward deletion produces identical visual letters at every stage without leaving orphan marks.
#[test]
fn grapheme_offsets_isomorphic_atomic_deletion() {
    let nfc = "viện";
    let nfd = "vie\u{0323}\u{0302}n";

    fn delete_backwards(text: &str) -> Vec<String> {
        let mut cur_text = text.to_string();
        let mut steps = Vec::new();
        while !cur_text.is_empty() {
            let cursor = cur_text.len();
            let prev = prev_grapheme_offset(&cur_text, cursor);
            cur_text.drain(prev..cursor);
            steps.push(cur_text.clone());
        }
        steps
    }

    let nfc_steps = delete_backwards(nfc);
    let nfd_steps = delete_backwards(nfd);

    assert_eq!(nfc_steps.len(), 4);
    assert_eq!(nfd_steps.len(), 4);

    // Step 1: removes 'n'
    assert_eq!(nfc_steps[0], "việ");
    assert_eq!(nfd_steps[0], "vie\u{0323}\u{0302}");

    // Step 2: removes the vowel letter atomically (never leaves orphan combining marks)
    assert_eq!(nfc_steps[1], "vi");
    assert_eq!(nfd_steps[1], "vi");

    // Step 3: removes 'i'
    assert_eq!(nfc_steps[2], "v");
    assert_eq!(nfd_steps[2], "v");

    // Step 4: removes 'v'
    assert_eq!(nfc_steps[3], "");
    assert_eq!(nfd_steps[3], "");
}

/// WHY: UAX #29 specifies that CRLF (\r\n) is a single indivisible grapheme cluster.
/// DANGER: Splitting \r and \n corrupts newline handling and text rendering.
/// CONTRACT: Cursor inside CRLF (between \r and \n) snaps to cluster start or end.
#[test]
fn grapheme_offsets_crlf_and_newlines() {
    let text = "a\r\nb";
    assert_eq!(text.len(), 4);
    // CRLF is a single cluster: [1..3]
    assert_eq!(prev_grapheme_offset(text, 3), 1);
    assert_eq!(next_grapheme_offset(text, 1), 3);

    // Cursor inside CRLF (offset 2, between \r and \n) snaps to boundaries
    assert_eq!(prev_grapheme_offset(text, 2), 1);
    assert_eq!(next_grapheme_offset(text, 2), 3);
}

/// WHY: Extended grapheme clusters encompass emoji ZWJ sequences, skin-tone modifiers, and Regional Indicator flag pairs.
/// DANGER: Peeling a ZWJ modifier renders disjoint raw emojis or missing-glyph fallback boxes.
/// CONTRACT: Entire sequences remain atomic; any interior cursor snaps to the full sequence bounds.
#[test]
fn grapheme_offsets_emoji_zwj_modifiers_and_flags() {
    // Emoji ZWJ family sequence
    let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    let len = family.len();
    assert_eq!(prev_grapheme_offset(family, len), 0);
    assert_eq!(next_grapheme_offset(family, 0), len);

    // Inside the ZWJ sequence, cursor snaps to cluster bounds
    for (offset, _) in family.char_indices().skip(1) {
        assert_eq!(prev_grapheme_offset(family, offset), 0);
        assert_eq!(next_grapheme_offset(family, offset), len);
    }

    // Flag (Regional Indicator pair)
    let flag = "🇻🇳";
    let flag_len = flag.len();
    assert_eq!(prev_grapheme_offset(flag, flag_len), 0);
    assert_eq!(next_grapheme_offset(flag, 0), flag_len);
    // Between regional indicators
    let mid = flag.chars().next().map_or(0, |c| c.len_utf8());
    assert_eq!(prev_grapheme_offset(flag, mid), 0);
    assert_eq!(next_grapheme_offset(flag, mid), flag_len);
}

/// WHY: Exhaustive verification of memory safety and boundary invariants across mixed-script text.
/// DANGER: Untrusted documents or hostile user input must never trigger panics or invalid slices.
/// CONTRACT: Every returned offset is guaranteed to be a valid char boundary bounded within [0, text.len()].
#[test]
fn grapheme_offsets_defensive_invariants_and_metamorphic_sweep() {
    let text = "vi\u{0323}\u{0302}n 🇻🇳 👍🏽 \r\n 漢字";
    let len = text.len();

    // Out-of-bounds cursors clamp safely without panicking
    assert!(prev_grapheme_offset(text, len + 100) <= len);
    assert_eq!(next_grapheme_offset(text, len + 100), len);

    // Non-char boundary cursors snap safely without panicking
    let multi_byte = "ệ"; // 3 bytes
    assert_eq!(prev_grapheme_offset(multi_byte, 1), 0);
    assert_eq!(prev_grapheme_offset(multi_byte, 2), 0);
    assert_eq!(next_grapheme_offset(multi_byte, 1), 3);
    assert_eq!(next_grapheme_offset(multi_byte, 2), 3);

    // Metamorphic sweep: verify invariants across all valid char boundaries
    for (offset, _) in text.char_indices().chain(std::iter::once((len, ' '))) {
        let prev = prev_grapheme_offset(text, offset);
        let next = next_grapheme_offset(text, offset);
        let (start, end) = grapheme_bounds(text, offset);

        assert!(prev <= offset, "prev ({prev}) must be <= offset ({offset})");
        assert!(next >= offset, "next ({next}) must be >= offset ({offset})");
        assert!(next <= len, "next ({next}) must be <= len ({len})");
        assert!(start <= offset, "start ({start}) must be <= offset ({offset})");
        assert!(offset <= end, "offset ({offset}) must be <= end ({end})");
        assert!(end <= len, "end ({end}) must be <= len ({len})");
        assert!(text.is_char_boundary(prev), "prev ({prev}) must be char boundary");
        assert!(text.is_char_boundary(next), "next ({next}) must be char boundary");
        assert!(text.is_char_boundary(start), "start ({start}) must be char boundary");
        assert!(text.is_char_boundary(end), "end ({end}) must be char boundary");
    }
}

/// WHY: Double-click word/character selection and block cursor rendering require the exact
///      enclosing extended grapheme cluster bounds (start, end) around a cursor position.
/// DANGER: Computing bounds naively via prev/next causes off-by-one errors when cursor is exactly
///         at cluster start (where prev moves to the preceding cluster).
/// CONTRACT:
/// - For empty text: returns (0, 0).
/// - For cursor inside or at start of cluster i: returns (s_i, e_i).
/// - For cursor at text.len(): returns bounds of the last cluster (or (0, 0) if empty).
/// - Invariant: 0 <= start <= cursor <= end <= text.len() holds for all valid cursors.
#[test]
fn grapheme_offsets_bounds_enclosing_cluster() {
    assert_eq!(grapheme_bounds("", 0), (0, 0));
    assert_eq!(grapheme_bounds("", 5), (0, 0));

    let text = "hello";
    assert_eq!(grapheme_bounds(text, 0), (0, 1));
    assert_eq!(grapheme_bounds(text, 1), (1, 2));
    assert_eq!(grapheme_bounds(text, 2), (2, 3));
    assert_eq!(grapheme_bounds(text, 3), (3, 4));
    assert_eq!(grapheme_bounds(text, 4), (4, 5));
    assert_eq!(grapheme_bounds(text, 5), (4, 5));

    // Vietnamese NFD: 'v' (0..1), 'i' (1..2), 'e\u{0323}\u{0302}' (2..7), 'n' (7..8)
    let nfd = "vie\u{0323}\u{0302}n";
    assert_eq!(grapheme_bounds(nfd, 0), (0, 1));
    assert_eq!(grapheme_bounds(nfd, 1), (1, 2));
    assert_eq!(grapheme_bounds(nfd, 2), (2, 7));
    assert_eq!(grapheme_bounds(nfd, 3), (2, 7));
    assert_eq!(grapheme_bounds(nfd, 5), (2, 7));
    assert_eq!(grapheme_bounds(nfd, 6), (2, 7));
    assert_eq!(grapheme_bounds(nfd, 7), (7, 8));
    assert_eq!(grapheme_bounds(nfd, 8), (7, 8));

    // Emoji ZWJ sequence
    let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    let flen = family.len();
    for (offset, _) in family.char_indices() {
        assert_eq!(grapheme_bounds(family, offset), (0, flen));
    }
    assert_eq!(grapheme_bounds(family, flen), (0, flen));
}
