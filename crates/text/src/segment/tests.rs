use super::*;
use craft_ui::testing::trial;

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

/// あカッ breaks before ッ with Soft and None (a small kana may start a line) and not with Hard; a
/// full stop never starts a line, Soft or not.
#[trial]
fn kinsoku_mode_decides_whether_a_small_kana_may_start_a_line() {
    assert_eq!(starts("あカッ", &opts(Kinsoku::Hard)), ['カ']);
    assert_eq!(starts("あカッ", &opts(Kinsoku::Soft)), ['カ', 'ッ']);
    assert_eq!(starts("あカッ", &opts(Kinsoku::None)), ['カ', 'ッ']);
    assert_eq!(starts("あカ。", &opts(Kinsoku::Soft)), ['カ']);
    assert_eq!(starts("あカ。", &opts(Kinsoku::None)), ['カ', '。']);
}

/// The East Asian breaks that None and Soft add to UAX #14 never split a cluster: not after a
/// joiner, not before a combining mark or variation selector.
#[trial]
fn no_kinsoku_mode_breaks_inside_a_cluster() {
    for k in [Kinsoku::None, Kinsoku::Soft, Kinsoku::Hard] {
        let o = opts(k);
        assert_eq!(pieces("日本\u{200D}語", &o), ["日", "本\u{200D}語"], "{k:?}");
        assert_eq!(pieces("あカ\u{200D}ッ", &o), ["あ", "カ\u{200D}ッ"], "{k:?}");
        assert_eq!(pieces("葛\u{E0100}城", &o), ["葛\u{E0100}", "城"], "{k:?}");
        assert_eq!(pieces("字\u{3099}\u{FE00}字", &o), ["字\u{3099}\u{FE00}", "字"], "{k:?}");
    }
}
