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
