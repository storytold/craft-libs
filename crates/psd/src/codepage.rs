//! Guessing the legacy code page of 8-bit names.
//!
//! Some formats store names as bytes in the writer's system ANSI code page: an `.atn` file's
//! event and dictionary names are CP936 (GBK) from Chinese Photoshop, CP932 (Shift_JIS) from
//! Japanese, Mac Roman from French Photoshop on a Mac, and so on. Nothing in the file says
//! which. When the file also stores some of those names as Unicode, [`match_known`] finds the
//! code page that turns the bytes into them. Otherwise [`guess`] infers one code page per file
//! from the names that are not valid UTF-8:
//!
//! 1. Candidates: Shift_JIS, GBK, Big5, EUC-KR, windows-1252, windows-1251, macintosh (Mac
//!    Roman). A candidate that cannot decode every name without errors is dropped.
//! 2. Each survivor is scored by the share of non-ASCII bytes that fall in its common-text
//!    ranges (GB2312 hanzi, JIS level-1 kanji and kana, Big5 level 1, KS X 1001 Hangul,
//!    letters, each page's punctuation, the degree sign, and ¡ opening an exclamation: "¡Hola!"). The
//!    highest share wins. Two single-byte readings don't count: the letters windows-1252 alone
//!    has at 0x80–0x9F (Š š Ž ž Ÿ ƒ ˆ), where Mac Roman keeps its accented lowercase (Mac
//!    "Opacité" is 1252 "OpacitŽ"), unless one starts a word ("Škoda", "šuma") in a file whose
//!    own Unicode text doesn't write the Mac Roman letter on that byte instead (a French "état",
//!    a German "über", "öffnen");
//!    and a windows-1251 Cyrillic letter glued into an ASCII word (Mac "Höhe" is 1251 "Hшhe"),
//!    unless the name's letters are mostly Cyrillic ("CMYKканал", "Копiя") or the letter is in a
//!    Cyrillic word of its own ("Layer Слой", "Слой1").
//! 3. A tie is broken by the script of a hint (text the file stores as UTF-16, so it decodes
//!    fine): kana → Shift_JIS; Hangul → EUC-KR; Cyrillic → windows-1251; Han → GBK, then
//!    Big5, then Shift_JIS; Latin letters → windows-1252, then Mac Roman.
//! 4. Still tied: windows-1252 if it is among them, otherwise the first highest-scoring candidate,
//!    flagged [`Guess::uncertain`].
//! 5. Mac Roman must also beat windows-1252 when the Latin-1 symbols that are no letter in
//!    Mac Roman (© ° « ± …) count as common text (1252 "Größe »" is Mac Roman "Grˆﬂe ª").
//!    Otherwise the names are windows-1252, with step 3's hint and step 4's warning.
//!
//! Only the first [`GUESS_SAMPLE`] bytes of names are examined.

use encoding_rs::{
    BIG5, DecoderResult, EUC_KR, Encoding, GBK, ISO_2022_JP, MACINTOSH, REPLACEMENT, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1251,
    WINDOWS_1252, X_USER_DEFINED,
};

use crate::error::{PsdError, Result};

/// At most this many bytes of legacy names are examined by [`guess`].
pub(crate) const GUESS_SAMPLE: usize = 64 * 1024;
/// At most this many `(bytes, text)` pairs are examined by [`match_known`].
pub(crate) const KNOWN_PAIRS: usize = 256;

/// The code page [`guess`] settled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Guess {
    /// The encoding to decode every legacy name with.
    pub(crate) encoding: &'static Encoding,
    /// Nothing settled the highest-scoring candidates' tie; `encoding` is a fallback among them.
    pub(crate) uncertain: bool,
}

/// Candidates in tie order (encoding_rs exposes encodings as statics, hence a function).
fn candidates() -> [&'static Encoding; 7] {
    [SHIFT_JIS, GBK, BIG5, EUC_KR, WINDOWS_1252, WINDOWS_1251, MACINTOSH]
}

/// The candidate that decodes the most of `pairs` (a name's bytes and the same name as Unicode)
/// exactly; ties go to the earlier candidate. `None` when it decodes none of them. Only the first
/// [`KNOWN_PAIRS`] pairs are examined.
pub(crate) fn match_known(pairs: &[(&[u8], &str)]) -> Option<&'static Encoding> {
    let pairs = pairs.get(..KNOWN_PAIRS).unwrap_or(pairs);
    let mut best = None;
    let mut best_count = 0usize;
    for enc in candidates() {
        let count =
            pairs.iter().filter(|(bytes, text)| enc.decode_without_bom_handling_and_without_replacement(bytes).is_some_and(|d| d == *text)).count();
        if count > best_count {
            best = Some(enc);
            best_count = count;
        }
    }
    best
}

/// Guesses the one code page that `legacy` (names that are not valid UTF-8, in file order) were
/// written in. `hint` is text from the same file whose script points at the writer's locale.
pub(crate) fn guess(legacy: &[&[u8]], hint: &str) -> Guess {
    let sample = sample(legacy);
    if sample.is_empty() {
        return Guess { encoding: UTF_8, uncertain: false };
    }
    // The accented letters the file's own Unicode text uses: what its writer's language writes.
    let hint_letters: std::collections::BTreeSet<char> =
        hint.chars().filter(|c| !c.is_ascii() && c.is_alphabetic()).flat_map(char::to_lowercase).collect();
    let mut best: Vec<&'static Encoding> = Vec::new();
    let mut best_score = (0u64, 1u64);
    for enc in candidates() {
        if !sample.iter().all(|(name, truncated)| decodes_cleanly(enc, name, *truncated)) {
            continue;
        }
        let score = sample.iter().fold((0u64, 0u64), |(c, h), (name, _)| {
            let (nc, nh) = score(enc, name, &hint_letters);
            (c.saturating_add(nc), h.saturating_add(nh))
        });
        let score = if score.1 == 0 { (0, 1) } else { score };
        // Compare common/high shares without division: a/b vs c/d ⇔ a·d vs c·b.
        let lhs = u128::from(score.0) * u128::from(best_score.1);
        let rhs = u128::from(best_score.0) * u128::from(score.1);
        if best.is_empty() || lhs > rhs {
            best = vec![enc];
            best_score = score;
        } else if lhs == rhs {
            best.push(enc);
        }
    }
    let picked = match best.as_slice() {
        [only] => Guess { encoding: only, uncertain: false },
        tied => match hinted(hint).iter().find(|enc| tied.contains(enc)) {
            Some(enc) => Guess { encoding: enc, uncertain: false },
            // A fallback must not discard the scores and choose a lower-scoring page.
            None => {
                let encoding = if tied.contains(&WINDOWS_1252) { WINDOWS_1252 } else { tied.first().copied().unwrap_or(WINDOWS_1252) };
                Guess { encoding, uncertain: true }
            }
        },
    };
    if picked.encoding != MACINTOSH {
        return picked;
    }
    // windows-1252 maps every byte, so it is always a survivor here.
    let latin = |enc| {
        sample.iter().fold((0u64, 0u64), |(c, h), (name, _)| {
            let (nc, nh) = single_byte_score(enc, name, is_latin_common);
            (c.saturating_add(nc), h.saturating_add(nh))
        })
    };
    let (windows, mac) = (latin(WINDOWS_1252), latin(MACINTOSH));
    let lhs = u128::from(windows.0) * u128::from(mac.1.max(1));
    let rhs = u128::from(mac.0) * u128::from(windows.1.max(1));
    if lhs < rhs {
        picked
    } else if lhs > rhs || hinted(hint).contains(&WINDOWS_1252) {
        Guess { encoding: WINDOWS_1252, uncertain: false }
    } else {
        Guess { encoding: WINDOWS_1252, uncertain: true }
    }
}

/// Decodes `bytes` with `enc`; invalid sequences become U+FFFD.
pub(crate) fn decode(bytes: &[u8], enc: &'static Encoding) -> String {
    enc.decode_without_bom_handling(bytes).0.into_owned()
}

/// Whether `name` decodes cleanly in the double-byte page `enc` and every non-ASCII byte of it is
/// common text there. A name in such a page can be valid UTF-8 by accident (GBK 去色 is
/// `C8 A5 C9 AB`, "ȥɫ"); real UTF-8 rarely passes, as its continuation bytes fall outside the
/// common ranges. Single-byte pages are never asked: their letters cover UTF-8's lead and
/// continuation bytes ("Weiß" is "WeiÃŸ" in windows-1252), and a legacy name there is valid UTF-8
/// by accident only for unlikely pairs such as "Ã©".
pub(crate) fn reads_as_common(name: &[u8], enc: &'static Encoding) -> bool {
    if ![SHIFT_JIS, GBK, BIG5, EUC_KR].contains(&enc) {
        return false;
    }
    let (common, high) = score(enc, name, &std::collections::BTreeSet::new());
    high > 0 && common == high && decodes_cleanly(enc, name, false)
}

/// Resolves an encoding label (`shift_jis`, `gbk`, `big5`, `euc-kr`, `windows-1252`, `utf-8`…;
/// case and surrounding whitespace are ignored). Labels for encodings that cannot hold an 8-bit
/// name are rejected, and so is ISO-2022-JP: its names are 7-bit escape sequences, which would
/// pass through as ASCII.
pub(crate) fn resolve_label(label: &str) -> Result<&'static Encoding> {
    match Encoding::for_label(label.as_bytes()) {
        Some(enc) if ![REPLACEMENT, UTF_16LE, UTF_16BE, X_USER_DEFINED, ISO_2022_JP].contains(&enc) => Ok(enc),
        _ => Err(PsdError::Unsupported(format!("text encoding `{label}`: use a label such as shift_jis, gbk, big5, euc-kr, windows-1252 or utf-8"))),
    }
}

/// The names [`guess`] examines, each with whether it was cut short: names in order while they
/// fit in [`GUESS_SAMPLE`] bytes, and at least the first (truncated if it alone is longer).
fn sample<'a>(legacy: &[&'a [u8]]) -> Vec<(&'a [u8], bool)> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for name in legacy {
        let next = total.saturating_add(name.len());
        if next > GUESS_SAMPLE {
            if out.is_empty() {
                out.push((name.get(..GUESS_SAMPLE).unwrap_or(name), true));
            }
            break;
        }
        out.push((*name, false));
        total = next;
    }
    out
}

/// Whether `enc` decodes `bytes` without a single error. A `truncated` name may end inside a
/// character, which is not held against it.
fn decodes_cleanly(enc: &'static Encoding, bytes: &[u8], truncated: bool) -> bool {
    let mut decoder = enc.new_decoder_without_bom_handling();
    // Every candidate decodes one byte to at most three UTF-8 bytes, so this is linear in
    // `bytes` (a sampled name, or one name the file already holds).
    let mut out = String::with_capacity(bytes.len().saturating_mul(3).saturating_add(16));
    let mut rest = bytes;
    loop {
        let (result, read) = decoder.decode_to_string_without_replacement(rest, &mut out, !truncated);
        match result {
            DecoderResult::InputEmpty => return true,
            DecoderResult::Malformed(..) => return false,
            DecoderResult::OutputFull => {
                rest = rest.get(read..).unwrap_or_default();
                out.clear();
            }
        }
    }
}

/// `(common, high)`: how many of a name's non-ASCII bytes `enc` reads as common text, and how
/// many non-ASCII bytes it has. `hint_letters` are the accented letters the file's own Unicode
/// text writes (lowercase), which settle a word-initial letter of windows-1252's 0x80–0x9F.
fn score(enc: &'static Encoding, name: &[u8], hint_letters: &std::collections::BTreeSet<char>) -> (u64, u64) {
    if enc == WINDOWS_1252 || enc == WINDOWS_1251 || enc == MACINTOSH {
        // Letters, typographic punctuation, the degree sign ("Rotation 90°") and ¡ opening an
        // exclamation ("¡Hola!") are common, as the double-byte pages' punctuation rows are, with two readings
        // held back: the letters windows-1252 alone has at 0x80–0x9F, where Mac Roman keeps its
        // accented lowercase (Mac "Opacité" is 1252 "OpacitŽ"), unless one starts a word ("Škoda",
        // "ŠKODA", "šuma") and the file's own text doesn't write the Mac Roman letter instead (a
        // French "état", a German "öffnen"); and a windows-1251 Cyrillic letter glued into an ASCII
        // word (Mac "Höhe" is 1251 "Hшhe"), unless the name's letters are mostly Cyrillic
        // ("CMYKканал", Ukrainian "Копiя") or the letter is in a Cyrillic word of its own ("Layer
        // Слой"); a lone one right after a digit (Mac "90°" alone is 1251 "90Ў") never counts.
        let decoded = Name::new(&decode(name, enc));
        let chars = &decoded.chars;
        let cyrillic_name = chars.iter().filter(|c| is_cyrillic(**c)).count() >= chars.iter().filter(|c| c.is_ascii_alphabetic()).count();
        let (mut common, mut high) = (0u64, 0u64);
        for (i, &ch) in chars.iter().enumerate() {
            if ch.is_ascii() {
                continue;
            }
            high += 1;
            let (after_ascii, letters_after) = position(&decoded, i);
            let counts = if enc == WINDOWS_1252 && RARE_1252_LETTERS.contains(&ch) {
                // Opening a word, capital or not ("Škoda", "šuma"), before an ASCII letter of
                // either case: Mac Roman's lowercase ä before a capital ("äKODA") is no word
                // either.
                let word_initial = !after_ascii && letters_after >= 1;
                // The same byte as Mac Roman reads it: a letter the file's text writes, where it
                // never writes this one, says the writer meant that ("état", not "Žtat").
                let written = |c: char| c.to_lowercase().all(|l| hint_letters.contains(&l));
                let mac = name.get(i..=i).map(|b| decode(b, MACINTOSH)).and_then(|s| s.chars().next());
                word_initial && !(mac.is_some_and(|m| m.is_alphabetic() && written(m)) && !written(ch))
            } else if enc == WINDOWS_1251 && is_cyrillic(ch) {
                (cyrillic_name && !lone_after_digit(&decoded, i)) || cyrillic_word(&decoded, i)
            } else {
                ch.is_alphabetic() || TYPOGRAPHIC.contains(&ch) || ch == '°' || (ch == '¡' && opens_exclamation(&decoded, i))
            };
            common += u64::from(counts);
        }
        return (common, high);
    }
    let (mut common, mut high) = (0u64, 0u64);
    let mut i = 0usize;
    while let Some(&lead) = name.get(i) {
        if lead < 0x80 {
            i += 1;
            continue;
        }
        // Shift_JIS reads 0x80 and 0xA0–0xDF (half-width katakana, rare in real text) as one
        // byte; elsewhere a lone non-ASCII byte is an error the decoder already rejected.
        let single = if enc == SHIFT_JIS { lead == 0x80 || (0xA0..=0xDF).contains(&lead) || lead >= 0xFD } else { lead == 0x80 || lead == 0xFF };
        if single {
            high += 1;
            i += 1;
            continue;
        }
        // A lead byte with nothing after it is where the sample was cut (the decoder rejects one
        // at the real end of a name), so it is not scored.
        let Some(&trail) = name.get(i + 1) else { break };
        // GB18030 four-byte sequences (second byte a digit) are never common text.
        let len = if enc == GBK && trail.is_ascii_digit() { 4 } else { 2 };
        let len = len.min(name.len().saturating_sub(i));
        high += len as u64;
        if len == 2 && is_common(enc, lead, trail) {
            common += 2;
        }
        i += len;
    }
    (common, high)
}

/// [`score`] for a single-byte page, where one byte is one character and a non-ASCII byte is a
/// non-ASCII character: `common` decides which of them count.
///
/// windows-1251 has Cyrillic letters (І і Ј д) where windows-1252 has ² ³ £ ä, so a windows-1252
/// name like "Fläche m²" or "Preis £5" would score higher as windows-1251 ("Flдche mІ", "Preis
/// Ј5"); [`score`] counts a Cyrillic letter only in a name whose letters are mostly Cyrillic,
/// which keeps ordinary Russian names such as "CMYKканал" or Ukrainian "Копiя" (Latin i).
fn single_byte_score(enc: &'static Encoding, name: &[u8], common: impl Fn(&Name, usize) -> bool) -> (u64, u64) {
    let name = Name::new(&decode(name, enc));
    (0..name.chars.len())
        .filter(|i| name.chars.get(*i).is_some_and(|c| !c.is_ascii()))
        .fold((0, 0), |(c, h), i| (c.saturating_add(u64::from(common(&name, i))), h.saturating_add(1)))
}

/// A decoded name with what the position checks of [`score`] need, each found in one pass, so a
/// long name (a hostile file's 64 KiB of one letter) costs linear time rather than a rescan of
/// the rest of the name from every character.
struct Name {
    chars: Vec<char>,
    /// `letters_from[k]`: how many letters run from `k` on (0 one past the end).
    letters_from: Vec<usize>,
    /// Where the last "!" is.
    last_bang: Option<usize>,
    /// For each Cyrillic letter, the run of Cyrillic letters around it (`start..end`); `i..i + 1`
    /// for any other character.
    cyrillic_runs: Vec<(usize, usize)>,
}

impl Name {
    fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        let mut letters_from = vec![0usize; n + 1];
        let mut run = 0usize;
        for (slot, c) in letters_from.iter_mut().zip(&chars).rev() {
            run = if c.is_alphabetic() { run.saturating_add(1) } else { 0 };
            *slot = run;
        }
        let last_bang = chars.iter().rposition(|c| *c == '!');
        let mut cyrillic_runs: Vec<(usize, usize)> = (0..n).map(|i| (i, i + 1)).collect();
        let mut start = 0;
        while start < n {
            if !chars.get(start).is_some_and(|c| is_cyrillic(*c)) {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while chars.get(end).is_some_and(|c| is_cyrillic(*c)) {
                end += 1;
            }
            for slot in cyrillic_runs.get_mut(start..end).into_iter().flatten() {
                *slot = (start, end);
            }
            start = end;
        }
        Self { chars, letters_from, last_bang, cyrillic_runs }
    }
}

/// Whether the character at `i` of `chars` is common in Western names, for weighing windows-1252
/// against Mac Roman: letters (but not those windows-1252 has only at 0x80–0x9F, where Mac Roman
/// keeps its accented lowercase: Mac "Größe" is 1252 "Grš§e"), typographic punctuation,
/// [`LATIN_SYMBOLS`], and ¡ opening an exclamation ("¡Hola!"), as in [`score`].
fn is_latin_common(name: &Name, i: usize) -> bool {
    let Some(&ch) = name.chars.get(i) else { return false };
    (ch.is_alphabetic() && !RARE_1252_LETTERS.contains(&ch) && ch != 'Œ' && ch != 'œ')
        || TYPOGRAPHIC.contains(&ch)
        || LATIN_SYMBOLS.contains(&ch)
        || (ch == '¡' && opens_exclamation(name, i))
}

/// Where the character at `i` of `name` sits: right after an ASCII letter or digit (inside a word
/// or a number), and how many letters (accented ones too: "¡Ánimo!") follow it directly.
fn position(name: &Name, i: usize) -> (bool, usize) {
    let after_ascii = i.checked_sub(1).and_then(|k| name.chars.get(k)).is_some_and(|c| c.is_ascii_alphanumeric());
    let letters_after = name.letters_from.get(i.saturating_add(1)).copied().unwrap_or(0);
    (after_ascii, letters_after)
}

/// Whether ¡ at `i` opens a Spanish exclamation ("¡Hola!", "Capa ¡Atención!", "¡Ánimo!"): no letter
/// or digit right before it, two letters or more after it, and the closing "!" later in the name.
/// Mac Roman's degree sign on the same byte follows a number ("Rotation 90°") or stands before a
/// unit ("20 °C", "20 °Celsius"), with no "!" to close.
fn opens_exclamation(name: &Name, i: usize) -> bool {
    let (after_ascii, letters_after) = position(name, i);
    !after_ascii && letters_after >= 2 && name.last_bang.is_some_and(|bang| bang > i)
}

/// The letters windows-1252 has at 0x80–0x9F and Mac Roman doesn't: there Mac Roman keeps its
/// accented lowercase (ä é ö ü ...), so these never decide against it. Œ and œ (0x8C, 0x9C: Mac
/// å, ú) are real French letters and keep counting in [`score`]; [`is_latin_common`] leaves them
/// out as well.
const RARE_1252_LETTERS: [char; 7] = ['Š', 'š', 'Ž', 'ž', 'Ÿ', 'ƒ', 'ˆ'];

/// A Cyrillic letter (windows-1251's 0xC0–0xFF and the letters among 0x80–0xBF).
fn is_cyrillic(ch: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&ch) && ch.is_alphabetic()
}

/// Whether the Cyrillic letter at `i` is in a Cyrillic word of its own: a run of two or more,
/// not glued to an ASCII letter, and not right after a digit ("Слой" beside "Layer", "Слой1";
/// not "OpacitЋ", nor "90ЎЋ", which is Mac Roman "90°é": a digit ends a number, not a word, so
/// what follows it without a space is its unit or suffix).
fn cyrillic_word(name: &Name, i: usize) -> bool {
    let (start, end) = cyrillic_run(name, i);
    let glued_before = start.checked_sub(1).and_then(|k| name.chars.get(k)).is_some_and(|c| c.is_ascii_alphanumeric());
    let glued_after = name.chars.get(end).is_some_and(|c| c.is_ascii_alphabetic());
    end.saturating_sub(start) >= 2 && !glued_before && !glued_after
}

/// The run of Cyrillic letters around the Cyrillic letter at `i` (`start..end`).
fn cyrillic_run(name: &Name, i: usize) -> (usize, usize) {
    name.cyrillic_runs.get(i).copied().unwrap_or((i, i.saturating_add(1)))
}

/// Whether the Cyrillic letter at `i` stands alone right after a digit: a unit or a symbol misread
/// (Mac Roman "90°" is windows-1251 "90Ў"), whatever the rest of the name; a run after a digit is
/// a Russian unit ("5шт") and still counts.
fn lone_after_digit(name: &Name, i: usize) -> bool {
    let (start, end) = cyrillic_run(name, i);
    end.saturating_sub(start) == 1 && start.checked_sub(1).and_then(|k| name.chars.get(k)).is_some_and(|c| c.is_ascii_digit())
}

/// Latin-1 symbols people type in names (windows-1252 0xA2–0xB7) whose byte is not a letter in
/// Mac Roman. Left out because Mac Roman has a letter there: § ® ¹ » ¼ ½ ¾ ¿ (ß Æ π ª º Ω æ ø,
/// as in Mac "Weiß", "Størrelse", "Æble"), € and ™ (0x80, 0x99: Ä, ô), and ¥ (0xA5, Mac Roman's
/// bullet in [`TYPOGRAPHIC`]: Mac "Ebene • Kopie"); and ¡ (0xA1), where Mac Roman has its degree
/// sign, the commoner of the two in names (Mac "Rotation 90°", "20 °C"), which counts only where
/// it opens an exclamation closed by "!" ("¡Hola!"; [`opens_exclamation`], in [`score`] and
/// [`is_latin_common`]). So a windows-1252 name whose only non-ASCII character is one of the
/// others ("Logo ®", "¿Tamaño?", "Preis ¥100") still reads as Mac Roman ("Logo Æ").
const LATIN_SYMBOLS: [char; 10] = ['¢', '£', '©', '«', '°', '±', '²', '³', '¶', '·'];

/// Quotes, dashes, ellipsis and bullet: windows-1252 and windows-1251 share them at 0x82–0x97;
/// Mac Roman has them too.
const TYPOGRAPHIC: [char; 10] = ['‘', '’', '‚', '“', '”', '„', '–', '—', '…', '•'];

/// Whether the two-byte character `lead trail` is in `enc`'s common-text ranges.
fn is_common(enc: &'static Encoding, lead: u8, trail: u8) -> bool {
    let both = u16::from_be_bytes([lead, trail]);
    if enc == GBK {
        // GB2312 hanzi, and the punctuation rows 0xA1 and 0xA3.
        (0xA1..=0xFE).contains(&trail) && ((0xB0..=0xF7).contains(&lead) || lead == 0xA1 || lead == 0xA3)
    } else if enc == BIG5 {
        // Level-1 hanzi and punctuation.
        (0xA4..=0xC6).contains(&lead) || lead == 0xA1
    } else if enc == SHIFT_JIS {
        // Kana, level-1 kanji and punctuation.
        (0x829F..=0x8396).contains(&both) || (0x889F..=0x9872).contains(&both) || (0x8140..=0x81FC).contains(&both)
    } else if enc == EUC_KR {
        // KS X 1001 Hangul and punctuation.
        trail >= 0xA1 && ((0xB0..=0xC8).contains(&lead) || lead == 0xA1)
    } else {
        false
    }
}

/// The candidates the script of `hint` points at, strongest first.
fn hinted(hint: &str) -> Vec<&'static Encoding> {
    let has = |lo: char, hi: char| hint.chars().any(|c| (lo..=hi).contains(&c) && c.is_alphabetic());
    if has('\u{3040}', '\u{30FF}') {
        vec![SHIFT_JIS]
    } else if has('\u{AC00}', '\u{D7A3}') {
        vec![EUC_KR]
    } else if has('\u{0400}', '\u{04FF}') {
        vec![WINDOWS_1251]
    } else if has('\u{4E00}', '\u{9FFF}') {
        vec![GBK, BIG5, SHIFT_JIS]
    } else if has('\u{00C0}', '\u{024F}') {
        vec![WINDOWS_1252, MACINTOSH]
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GBK_ISSUE: &[&[u8]] = &[b"\xbf\xc9\xd1\xa1\xd1\xd5\xc9\xab"]; // 可选颜色 (from photocraft#6743)
    const SJIS_COMMENT: &[&[u8]] = &[
        b"\x91\x49\x91\xf0\x20\x3a", // 選択 :
        b"\x90\xdd\x92\xe8\x20\x3a", // 設定 :
        b"\x83\x52\x83\x73\x81\x5b", // コピー
        b"\x95\xdb\x91\xb6",         // 保存
    ];

    fn encoded(enc: &'static Encoding, words: &[&str]) -> Vec<Vec<u8>> {
        words.iter().map(|w| enc.encode(w).0.into_owned()).collect()
    }

    fn guess_words(enc: &'static Encoding, words: &[&str], hint: &str) -> Guess {
        let bytes = encoded(enc, words);
        let refs: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
        guess(&refs, hint)
    }

    #[test]
    fn issue_gbk_bytes_read_as_gbk() {
        let g = guess(GBK_ISSUE, "调白");
        assert_eq!(g.encoding, GBK);
        assert!(!g.uncertain);
        assert_eq!(decode(GBK_ISSUE[0], g.encoding), "可选颜色");
    }

    #[test]
    fn kanji_only_hint_still_reads_shift_jis() {
        let g = guess(SJIS_COMMENT, "保存");
        assert_eq!(g.encoding, SHIFT_JIS);
        assert_eq!(decode(SJIS_COMMENT[0], g.encoding), "選択 :");
    }

    #[test]
    fn big5_reads_as_big5() {
        assert_eq!(guess_words(BIG5, &["色彩範圍", "移動", "選取", "儲存"], "").encoding, BIG5);
    }

    #[test]
    fn hangul_hint_reads_euc_kr() {
        let g = guess_words(EUC_KR, &["선택", "이동", "저장", "색상 범위"], "동작 1");
        assert_eq!(g.encoding, EUC_KR);
        assert!(!g.uncertain);
    }

    #[test]
    fn cyrillic_hint_reads_1251() {
        let g = guess_words(WINDOWS_1251, &["Выделить", "Переместить", "Сохранить"], "Набор 1");
        assert_eq!(g.encoding, WINDOWS_1251);
        assert!(!g.uncertain);
    }

    #[test]
    fn latin_hint_reads_1252() {
        let g = guess_words(WINDOWS_1252, &["Größe"], "Größe ändern");
        assert_eq!(g.encoding, WINDOWS_1252);
        assert!(!g.uncertain);
    }

    #[test]
    fn tie_without_hint_is_1252_and_uncertain() {
        let g = guess_words(WINDOWS_1252, &["Größe"], "Set 1");
        assert_eq!(g.encoding, WINDOWS_1252);
        assert!(g.uncertain);
    }

    #[test]
    fn han_tie_prefers_gbk() {
        // GBK 保存: GBK, Big5 and EUC-KR all read both characters as common text.
        let names: &[&[u8]] = &[b"\xb1\xa3\xb4\xe6"];
        let g = guess(names, "动作");
        assert_eq!(g.encoding, GBK);
        assert!(!g.uncertain);
        assert_eq!(decode(names[0], g.encoding), "保存");
        assert_eq!(guess(names, "Set 1"), Guess { encoding: GBK, uncertain: true });
    }

    #[test]
    fn typographic_punctuation_reads_as_1252() {
        // ’ (0x92) then a letter is also a level-1 kanji pair in Shift_JIS.
        let names: &[&[u8]] = &[b"Taille de l\x92image"];
        let g = guess(names, "Actions par défaut");
        assert_eq!(g.encoding, WINDOWS_1252);
        assert!(!g.uncertain);
        assert_eq!(decode(names[0], g.encoding), "Taille de l’image");
        assert_eq!(guess(names, "Set 1"), Guess { encoding: WINDOWS_1252, uncertain: true });
    }

    #[test]
    fn utf8_lookalikes_read_as_common_text() {
        // GBK 去色 and EUC-KR 칠 are also valid UTF-8 ("ȥɫ", "ĥ").
        assert!(reads_as_common(b"\xc8\xa5\xc9\xab", GBK));
        assert!(reads_as_common(b"\xc4\xa5", EUC_KR));
        // Real UTF-8 text does not: its continuation bytes fall outside the common ranges.
        assert!(!reads_as_common("高斯模糊".as_bytes(), GBK));
        assert!(!reads_as_common("Gaussian Blur ✓".as_bytes(), GBK));
        assert!(!reads_as_common("Größe".as_bytes(), WINDOWS_1252));
        assert!(!reads_as_common(b"Blur", GBK), "ASCII has nothing to re-read");
        // Nor does UTF-8 in a single-byte page, though "Weiß" is Ã and Ÿ there and "Привет" is
        // all Р, С and letters: accidental UTF-8 is a double-byte page's problem.
        assert!(!reads_as_common("Weiß".as_bytes(), WINDOWS_1252));
        assert!(!reads_as_common("Привет".as_bytes(), WINDOWS_1251));
        assert!(!reads_as_common("Weiß".as_bytes(), MACINTOSH));
    }

    #[test]
    fn latin1_symbols_read_as_1252_not_mac_roman() {
        // Mac Roman has letters where windows-1252 has some symbols: 1252 "Größe »" is Mac Roman
        // "Grˆﬂe ª", "50 °" is "50 ∞". Latin-1 (what a code-page-blind reader shows) wins.
        for words in [&["Größe »"][..], &["50 °"], &["Contraste « auto »", "© Größe", "± 5"]] {
            let g = guess_words(WINDOWS_1252, words, "Größe ändern");
            assert_eq!(g, Guess { encoding: WINDOWS_1252, uncertain: false }, "{words:?}");
            assert_eq!(guess_words(WINDOWS_1252, words, "Set 1").encoding, WINDOWS_1252, "{words:?}");
        }
    }

    #[test]
    fn mac_roman_accents_on_1252_letter_bytes_stay_mac_roman() {
        // photocraft#10774: Mac Roman é ä ö ü Ä (0x8E 0x8A 0x9A 0x9F 0x80) are windows-1252 Ž Š š Ÿ € and
        // windows-1251 Ћ Љ ш џ Ђ. The rare 1252 letters don't count, and a Cyrillic letter in a
        // Latin word doesn't either, so Mac Roman wins with a Latin hint and without one.
        let cases: [(&[&str], &str); 6] = [
            (&["Opacité 50%"], "Définir"),
            (&["Opacité 50%"], "Set 1"),
            (&["Sélection", "Créer un calque", "Luminosité/Contraste"], "Définir"),
            (&["Höhe", "Grün", "Bär"], "Größe ändern"),
            (&["Höhe"], "Set 1"),
            (&["Ändern"], "Set 1"),
        ];
        for (words, hint) in cases {
            let g = guess_words(MACINTOSH, words, hint);
            assert_eq!(g, Guess { encoding: MACINTOSH, uncertain: false }, "{words:?} with hint {hint:?}");
            let bytes = encoded(MACINTOSH, words);
            for (b, w) in bytes.iter().zip(words) {
                assert_eq!(decode(b, g.encoding), *w);
            }
        }
    }

    #[test]
    fn mac_roman_degree_sign_is_not_cyrillic() {
        // photocraft#10774: Mac Roman ° (0xA1) is windows-1251 Ў and windows-1252 ¡; the degree sign is
        // common text, a lone Cyrillic letter among Latin ones is not.
        // "Layer 90°é" (review): ° and é are windows-1251 Ў and Ћ, two Cyrillic letters in a row,
        // but glued to the number, so no word of their own.
        let cases: [(&[&str], &str); 6] = [
            (&["Rotation 90°"], "Définir"),
            (&["Rotation 90°"], "Set 1"),
            (&["Rotation 90°"], "Größe ändern"),
            (&["Rotation 90°", "Arrière-plan"], "Définir"),
            (&["Layer 90°é"], "Set 1"),
            (&["Layer 90°é"], "Définir"),
        ];
        for (words, hint) in cases {
            let g = guess_words(MACINTOSH, words, hint);
            assert_eq!(g, Guess { encoding: MACINTOSH, uncertain: false }, "{words:?} with hint {hint:?}");
        }
    }

    #[test]
    fn the_same_names_in_windows_1252_stay_windows_1252() {
        for words in [&["Opacité 50%"][..], &["Höhe", "Grün", "Bär"], &["Rotation 90°"], &["Ändern"], &["Fläche m²"], &["Preis £5"]] {
            assert_eq!(guess_words(WINDOWS_1252, words, "Set 1").encoding, WINDOWS_1252, "{words:?}");
            assert_eq!(guess_words(WINDOWS_1252, words, "Größe ändern"), Guess { encoding: WINDOWS_1252, uncertain: false }, "{words:?}");
        }
    }

    #[test]
    fn cyrillic_names_in_latin_hinted_files_and_cyrillic_words_beside_latin_ones_stay_1251() {
        // A Cyrillic step name in a set with a French name reads as it did; a Cyrillic word beside
        // a Latin one counts as Cyrillic, and the hint settles the tie with windows-1252's letters.
        assert_eq!(guess_words(WINDOWS_1251, &["Ёлка"], "Définir"), Guess { encoding: WINDOWS_1251, uncertain: false });
        assert_eq!(guess_words(WINDOWS_1251, &["Layer Слой"], "Набор 1"), Guess { encoding: WINDOWS_1251, uncertain: false });
        assert_eq!(
            guess_words(WINDOWS_1251, &["Layer Слой1"], "Набор 1"),
            Guess { encoding: WINDOWS_1251, uncertain: false },
            "a digit after the word"
        );
        assert_eq!(guess_words(WINDOWS_1251, &["Layer Слой", "Ёлка"], "Set 1"), Guess { encoding: WINDOWS_1251, uncertain: false });
        // Alone and without a hint, "Layer Слой" is as ambiguous as before (every letter is one in
        // windows-1252 too): the flagged fallback.
        assert_eq!(guess_words(WINDOWS_1251, &["Layer Слой"], "Set 1"), Guess { encoding: WINDOWS_1252, uncertain: true });
    }

    #[test]
    fn a_rare_letter_starting_a_word_keeps_windows_1252() {
        // windows-1252 Š Ž Ÿ š ž (0x8A 0x8E 0x9F 0x9A 0x9E) are Mac Roman ä é ü ö û: one starting
        // a word ("Škoda", all caps "ŠKODA", lowercase "šuma") counts, an accent inside a word (Mac
        // "Opacité" as "OpacitŽ") does not.
        for words in [&["Škoda"][..], &["ŠKODA"], &["Žaba"], &["Šibenik", "Žaba"], &["ŽUPA", "Set"], &["šuma"], &["žaba"]] {
            assert_eq!(guess_words(WINDOWS_1252, words, "Nastavení"), Guess { encoding: WINDOWS_1252, uncertain: false }, "{words:?}");
            assert_eq!(guess_words(WINDOWS_1252, words, "Set 1").encoding, WINDOWS_1252, "{words:?}");
        }
        // In a file whose own text writes ä and never Š (a German Photoshop), the byte reads as
        // the Mac Roman ä it would be there: the file's alphabet decides what the bytes can't.
        assert_eq!(guess_words(WINDOWS_1252, &["Škoda"], "Größe ändern").encoding, MACINTOSH);
        assert_eq!(guess_words(WINDOWS_1252, &["šuma"], "Größe ändern").encoding, MACINTOSH, "the file writes ö and never š");
    }

    #[test]
    fn an_inverted_exclamation_opening_an_exclamation_reads_as_windows_1252() {
        // windows-1252 ¡ (0xA1) is Mac Roman °: ¡ opening an exclamation, two letters or more
        // after it (accented ones too) and the closing "!" later ("¡Hola!", "¡Ánimo!"), counts for
        // windows-1252 (review); after a number ("90°"), before a unit ("20 °C", "20 °Celsius") or
        // without a "!" the byte stays the degree sign.
        for words in [&["¡Hola!"][..], &["Capa ¡Atención!"], &["¡Ánimo!"]] {
            assert_eq!(guess_words(WINDOWS_1252, words, "Configuración"), Guess { encoding: WINDOWS_1252, uncertain: false }, "{words:?}");
            assert_eq!(guess_words(WINDOWS_1252, words, "Set 1").encoding, WINDOWS_1252, "{words:?}");
        }
        // (With a Latin hint: without any, a lone 0xA1 before an ASCII letter is also a Big5
        // punctuation pair, a tie the fallback flags.)
        for words in [&["Rotation 90°"][..], &["Temperatur 20 °C"], &["20 °Celsius"]] {
            for hint in ["Configuración", "Größe ändern"] {
                assert_eq!(guess_words(MACINTOSH, words, hint), Guess { encoding: MACINTOSH, uncertain: false }, "{words:?} with hint {hint:?}");
            }
        }
        assert_eq!(guess_words(MACINTOSH, &["Rotation 90°"], "Set 1"), Guess { encoding: MACINTOSH, uncertain: false });
    }

    #[test]
    fn a_lone_cyrillic_letter_after_a_digit_is_no_cyrillic_name() {
        // Mac Roman "90°" alone has no ASCII letter, so its one windows-1251 letter (Ў) would make
        // the name "mostly Cyrillic" (review): a lone Cyrillic letter right after a digit never
        // counts. A one-letter Cyrillic name and a Russian unit after a digit still do.
        for hint in ["Set 1", "Définir"] {
            assert_eq!(guess_words(MACINTOSH, &["90°"], hint), Guess { encoding: MACINTOSH, uncertain: false }, "hint {hint:?}");
        }
        assert_eq!(guess_words(WINDOWS_1251, &["Я"], "Набор 1"), Guess { encoding: WINDOWS_1251, uncertain: false });
        assert_eq!(guess_words(WINDOWS_1251, &["5шт"], "Набор 1"), Guess { encoding: WINDOWS_1251, uncertain: false });
    }

    #[test]
    fn word_initial_mac_roman_accents_read_as_mac_roman_when_the_file_writes_them() {
        // Mac é (0x8E) and ü (0x9F) are windows-1252 Ž and Ÿ, capitals that could start a word:
        // in a file whose own text has é or ü and no Ž or Ÿ, they are the Mac Roman letters.
        assert_eq!(guess_words(MACINTOSH, &["état"], "Définir"), Guess { encoding: MACINTOSH, uncertain: false });
        assert_eq!(guess_words(MACINTOSH, &["über Ebene", "Höhe"], "Größe ändern"), Guess { encoding: MACINTOSH, uncertain: false });
        // Without such text the capital reading stands, as before: the bytes alone can't tell.
        assert_eq!(guess_words(MACINTOSH, &["éclair"], "Set 1").encoding, WINDOWS_1252);
    }

    #[test]
    fn cyrillic_names_with_latin_words_still_read_as_1251() {
        // Mostly Cyrillic letters count; a Latin brand name or a Latin i in Ukrainian doesn't
        // turn the set into something else.
        let g = guess_words(WINDOWS_1251, &["CMYKканал", "Копiя", "Слой 1"], "Набор 1");
        assert_eq!(g, Guess { encoding: WINDOWS_1251, uncertain: false });
        let bytes = encoded(WINDOWS_1251, &["CMYKканал"]);
        assert_eq!(decode(&bytes[0], g.encoding), "CMYKканал");
    }

    #[test]
    fn symbols_on_mac_roman_letters_stay_ambiguous() {
        // Known limitation: 1252 "Logo ®" is Mac Roman "Logo Æ", and ® is not counted for
        // windows-1252 because Mac "Æble" and "Weiß" use those bytes, so the name reads as Mac
        // Roman whatever the hint, as it did before the symbols counted at all.
        let logo: &[&[u8]] = &[b"Logo \xae"];
        assert_eq!(guess(logo, "Größe ändern"), Guess { encoding: MACINTOSH, uncertain: false });
        assert_eq!(guess(logo, "Set 1"), Guess { encoding: MACINTOSH, uncertain: false });
    }

    #[test]
    fn mac_roman_letters_still_read_as_mac_roman() {
        // Mac Roman "Größe" is 1252 "Grš§e", "Ändern" "€ndern", "Contrôle" "Contr™le": Mac Roman
        // still wins once § counts as common text in windows-1252.
        // Mac Roman ß ø æ Æ are 1252 § ¿ ¾ ®, which windows-1252 must not count as common.
        let words = [
            "Größe",
            "Ändern",
            "Contrôle",
            "Weiß",
            "Schwarzweiß",
            "Gaußscher Weichzeichner",
            "Schließen",
            "Størrelse",
            "Lærredstørrelse",
            "Fjern støy",
            "Æble",
        ];
        for word in words {
            let g = guess_words(MACINTOSH, &[word], "Définir l’arrière-plan");
            assert_eq!(g, Guess { encoding: MACINTOSH, uncertain: false }, "{word}");
        }
    }

    #[test]
    fn latin_hint_reads_mac_roman_when_1252_does_not_fit() {
        // Mac Roman "Arrière-plan": è is 0x8F, a control character in windows-1252.
        let names: &[&[u8]] = &[b"Arri\x8fre-plan"];
        let g = guess(names, "Définir l’arrière-plan");
        assert_eq!(g.encoding, MACINTOSH);
        assert!(!g.uncertain);
        assert_eq!(decode(names[0], g.encoding), "Arrière-plan");
    }

    #[test]
    fn known_names_pick_the_code_page() {
        // Every step of the corpus/atn sets that has a non-ASCII panel name also stores it as
        // Unicode (the descriptor's class name); here, Mac Roman "Définir".
        let pairs: &[(&[u8], &str)] = &[(b"D\x8efinir", "Définir")];
        assert_eq!(match_known(pairs), Some(MACINTOSH));
        // GBK 保存 is a three-way tie for the statistics; the known name settles it.
        assert_eq!(match_known(&[(b"\xb1\xa3\xb4\xe6", "保存")]), Some(GBK));
    }

    #[test]
    fn most_matching_names_win() {
        let pairs: &[(&[u8], &str)] = &[(b"\xb1\xa3\xb4\xe6", "保存"), (b"D\x8efinir", "Définir"), (b"Arri\x8fre-plan", "Arrière-plan")];
        assert_eq!(match_known(pairs), Some(MACINTOSH));
    }

    #[test]
    fn known_name_ties_take_candidate_order() {
        // Curly quotes are 0x93/0x94 in both windows-1252 and windows-1251.
        assert_eq!(match_known(&[(b"\x93x\x94", "“x”")]), Some(WINDOWS_1252));
    }

    #[test]
    fn known_names_that_match_nothing_decide_nothing() {
        assert_eq!(match_known(&[(b"D\x8efinir", "Set")]), None);
        assert_eq!(match_known(&[]), None);
        // Past the cap, pairs are not examined.
        let mut pairs = vec![(&b"D\x8efinir"[..], "x"); KNOWN_PAIRS];
        pairs.push((b"\xb1\xa3\xb4\xe6", "保存"));
        assert_eq!(match_known(&pairs), None);
    }

    #[test]
    fn bytes_no_cjk_page_accepts_still_decode() {
        // 0xFF is invalid in every double-byte candidate; windows-1251 (Ђ я Ђ) and Mac Roman
        // (Ä ˇ Ä) read all three bytes as letters, windows-1252 only one (€ ÿ €). With nothing to
        // break the tie, the first highest-scoring candidate still decodes them.
        let names: &[&[u8]] = &[b"\x80\xff\x80"];
        let g = guess(names, "");
        assert_eq!(g, Guess { encoding: WINDOWS_1251, uncertain: true });
        assert_eq!(decode(names[0], g.encoding), "ЂяЂ");
        assert_eq!(guess(names, "Набор").encoding, WINDOWS_1251);
    }

    #[test]
    fn uncertain_fallback_stays_among_the_highest_scoring_candidates() {
        // Mac Roman "éäé" (0x8E 0x8A 0x8E) is windows-1251 "ЋЉЋ", three Cyrillic letters in a
        // row, a word of their own: both pages score every non-ASCII byte as a letter (an odd
        // count of bytes, so no double-byte page reads it). Windows-1252 reads "ŽŠŽ", no
        // word-initial capital among them, so it must not win their unresolved tie. (Mac Roman
        // "Ändern" no longer ties: its lone Cyrillic letter glued into a Latin word, 1251
        // "Ђndern", doesn't count, photocraft#10774.)
        let names: &[&[u8]] = &[b"\x8e\x8a\x8e"];
        for hint in ["Set 1", "保存"] {
            let g = guess(names, hint);
            assert!([MACINTOSH, WINDOWS_1251].contains(&g.encoding), "{hint}: {g:?}");
            assert!(g.uncertain, "{hint}: {g:?}");
            assert_eq!(score(g.encoding, names[0], &Default::default()), (3, 3));
        }
        assert_eq!(guess(&[b"\x80ndern"], "Set 1"), Guess { encoding: MACINTOSH, uncertain: false }, "Mac Roman \"Ändern\" is certain now");
    }

    #[test]
    fn empty_or_utf8_input_is_utf8() {
        let g = guess(&[], "");
        assert_eq!(g.encoding, UTF_8);
        assert!(!g.uncertain);
    }

    #[test]
    fn mixed_code_pages_pick_one_and_never_fail() {
        let names: &[&[u8]] = &[GBK_ISSUE[0], SJIS_COMMENT[0]];
        let g = guess(names, "");
        for name in names {
            assert!(!decode(name, g.encoding).is_empty());
        }
    }

    #[test]
    fn huge_input_is_sampled() {
        let names = vec![GBK_ISSUE[0]; 10_000];
        assert_eq!(guess(&names, "").encoding, GBK);
    }

    #[test]
    fn one_name_over_the_sample_cap_is_truncated_not_rejected() {
        // "a" then 可 repeated past the cap, so the cut lands inside a two-byte character.
        let mut name = b"a".to_vec();
        name.extend(b"\xbf\xc9".repeat(GUESS_SAMPLE / 2 + 1));
        assert_eq!(guess(&[&name], "可").encoding, GBK);
    }

    #[test]
    fn labels_resolve_and_reject() {
        for ok in ["shift_jis", "gbk", "big5", "euc-kr", "windows-1252", "utf-8"] {
            assert!(resolve_label(ok).is_ok(), "{ok}");
        }
        for bad in ["nope", "replacement", "utf-16le", "x-user-defined", "", "iso-2022-kr", "iso-2022-jp", "csISO2022JP"] {
            let Err(e) = resolve_label(bad) else { panic!("{bad} should be rejected") };
            assert!(e.to_string().contains("shift_jis"), "{e}");
        }
    }

    #[test]
    fn labels_match_loosely() {
        assert_eq!(resolve_label("Shift_JIS").ok(), Some(SHIFT_JIS));
        assert_eq!(resolve_label(" gbk ").ok(), Some(GBK));
        assert_eq!(resolve_label("gb2312").ok(), Some(GBK));
    }
    #[test]
    fn a_name_of_one_long_letter_run_is_scored_in_linear_time() {
        // 64 KiB of one letter (é in windows-1252, й in windows-1251, Mac Roman's È), of
        // alternating Cyrillic letters, and of "¡ab" with no "!": every character used to rescan
        // the rest of the name, which took minutes.
        for unit in [&[0xe9u8][..], &[0xe9, 0xea][..], &[0xa1, b'a', b'b'][..]] {
            let name = unit.repeat(GUESS_SAMPLE / unit.len());
            let started = std::time::Instant::now();
            let _ = guess(&[&name], "");
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?} took {:?}", unit, started.elapsed());
        }
    }
}
