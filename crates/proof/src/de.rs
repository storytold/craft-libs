//! German proofing helpers: coordinated compounds with ellipsis (*Ergänzungsstrich*, § 98
//! Amtliche Rechtschreibung).

use crate::{Issue, IssueKind, ProofLang};

#[cfg(test)]
thread_local! {
    static STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[inline]
fn count_step() {
    #[cfg(test)]
    STEPS.with(|s| s.set(s.get().saturating_add(1)));
}

#[cfg(test)]
fn count_steps<R>(f: impl FnOnce() -> R) -> (usize, R) {
    STEPS.with(|s| s.set(0));
    let res = f();
    let steps = STEPS.with(std::cell::Cell::get);
    (steps, res)
}

/// Maximum byte window searched on a partner word for candidate compound splits.
const MAX_PARTNER_WINDOW_BYTES: usize = 40;

/// Maximum number of lowercase modifiers allowed between a suspended fragment and its partner.
const MAX_MODIFIERS: usize = 2;

/// Whether `c` is a hyphen character (ASCII hyphen-minus, Unicode hyphen, non-breaking hyphen, or en-dash).
fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2011}' | '\u{2013}')
}

/// Whether `c` is a line or paragraph break that terminates German coordination scans.
fn is_line_or_para_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Whether `s` has at least `min` Unicode scalar values, inspecting at most `min` characters.
fn has_at_least_chars(s: &str, min: usize) -> bool {
    s.chars().take(min).count() >= min
}

/// Whether `s` ends with an alphabetic letter (ignoring trailing uncomposable combining marks).
fn ends_with_letter(s: &str) -> bool {
    s.trim_end_matches(unicode_normalization::char::is_combining_mark).chars().next_back().is_some_and(char::is_alphabetic)
}

/// Whether `s` is written in ALL-CAPS (at least one uppercase letter and no lowercase letters
/// other than `ß`, which may remain in older all-caps German).
fn is_all_caps(s: &str) -> bool {
    s.chars().any(char::is_uppercase) && s.chars().all(|c| !c.is_lowercase() || c == 'ß')
}

/// Lowercase the first character of `s`, preserving the rest.
fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            let mut res = c.to_lowercase().to_string();
            res.push_str(chars.as_str());
            res
        }
        None => String::new(),
    }
}

/// Uppercase the first character of `s`, preserving the rest.
fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            let mut res = c.to_uppercase().to_string();
            res.push_str(chars.as_str());
            res
        }
        None => String::new(),
    }
}

/// Whether `token` is a known German abbreviation ending with a period that does not terminate a sentence.
fn is_german_abbreviation(token: &str) -> bool {
    let lower = token.to_lowercase();
    matches!(
        lower.as_str(),
        "bzw." | "z.b." | "u.a." | "d.h." | "usw." | "etc." | "evtl." | "ggf." | "v.a." | "insb." | "ca." | "sog." | "inkl." | "exkl." | "bzgl."
    )
}

/// Whether `c` is an opening quotation mark or bracket.
fn is_opening_delimiter(c: char) -> bool {
    matches!(c, '"' | '\'' | '“' | '”' | '„' | '‘' | '’' | '‚' | '»' | '«' | '›' | '‹' | '(' | '[' | '{')
}

/// Whether `c` is a closing quotation mark or bracket that can follow terminal sentence punctuation.
fn is_closing_delimiter(c: char) -> bool {
    matches!(c, '"' | '\'' | '“' | '”' | '„' | '‘' | '’' | '‚' | '»' | '«' | '›' | '‹' | ')' | ']' | '}')
}

/// Whether `raw_token` ends with terminal sentence punctuation (optionally followed by closing
/// quotes or brackets), excluding known German abbreviations.
fn has_sentence_punctuation(raw_token: &str) -> bool {
    let trimmed = raw_token.trim_end_matches(is_closing_delimiter);
    trimmed.ends_with(['.', '!', '?', ';', ':']) && !is_german_abbreviation(trimmed.trim_start_matches(is_opening_delimiter))
}

/// Whether `raw_token` begins with terminal sentence punctuation (optionally preceded by opening
/// quotes or brackets), excluding known German abbreviations.
fn starts_with_sentence_punctuation(raw_token: &str) -> bool {
    let trimmed = raw_token.trim_start_matches(is_opening_delimiter);
    trimmed.starts_with(['.', '!', '?', ';', ':']) && !is_german_abbreviation(trimmed.trim_end_matches(is_closing_delimiter))
}

/// Whether `token` is a German coordination conjunction that establishes coordination between
/// constituents of a coordinated compound.
fn is_german_conjunction(token: &str) -> bool {
    matches!(
        token,
        "und" | "oder" | "sowie" | "bzw" | "bzw." | "wie" | "als" | "auch" | "noch" | "weder" | "entweder" | "aber" | "sondern" | "und/oder"
    )
}

/// Whether `token` is a German article that may appear inside a coordination chain without
/// establishing coordination on its own.
fn is_german_article(token: &str) -> bool {
    matches!(token, "der" | "die" | "das" | "dem" | "den" | "des" | "ein" | "eine" | "einer" | "eines" | "einem" | "einen")
}

/// Whether `lang` is German (standard German [`ProofLang::De`], or a German pack locale such as `de-CH`).
fn is_german_lang(lang: ProofLang) -> bool {
    lang == ProofLang::De
        || lang.pack_locale().is_some_and(|l| {
            let s = l.as_str();
            s == "de" || s.starts_with("de-") || s.starts_with("de_")
        })
}

/// Whether `word` is directly accepted by `lang`'s dictionary (or user dictionary), without option
/// exemptions (`ignore_numbers`, `ignore_uppercase`) or hyphen-part fallback splitting.
fn is_known_word(word: &str, lang: ProofLang) -> bool {
    let composed = crate::composed(word);
    lang.spelling_locale().is_some_and(|locale| crate::pack_lang::is_known_word(&composed, locale))
}

/// Whether `word` is accepted as a complete partner or inherently hyphenated compound (e.g.
/// `E-Mail-Adresse`, `Politik-Abteilung`): direct dictionary match, or when `word` contains an
/// internal hyphen, every non-single-letter hyphen-separated part is directly known.
fn is_known_word_or_hyphenated(word: &str, lang: ProofLang) -> bool {
    if is_known_word(word, lang) {
        return true;
    }
    if !word.contains('-') {
        return false;
    }
    let parts: Vec<&str> = word.split('-').collect();
    !parts.iter().any(|p| p.is_empty())
        && parts.iter().any(|p| has_at_least_chars(p, 2))
        && parts.iter().all(|p| !has_at_least_chars(p, 2) || is_known_word(p, lang))
}

/// Whether `word` (or its initial-case variants) is directly accepted by `lang`'s dictionary.
fn is_word_in_dict_any_case(word: &str, lang: ProofLang) -> bool {
    if !has_at_least_chars(word, 2) || !word.chars().any(char::is_alphabetic) {
        return false;
    }
    if is_known_word_or_hyphenated(word, lang) {
        return true;
    }
    let upper = uppercase_first(word);
    if upper != word && is_known_word_or_hyphenated(&upper, lang) {
        return true;
    }
    let lower = lowercase_first(word);
    lower != word && is_known_word_or_hyphenated(&lower, lang)
}

/// Whether `left` is a German word-formation prefix that can precede a suspended head constituent
/// (e.g. `be- und entladen`, `Hin- und Rückfahrt`).
fn is_german_prefix(left_lower: &str) -> bool {
    matches!(
        left_lower,
        "ab" | "an"
            | "auf"
            | "aus"
            | "be"
            | "bei"
            | "dar"
            | "durch"
            | "ein"
            | "emp"
            | "ent"
            | "er"
            | "fort"
            | "ge"
            | "gegen"
            | "her"
            | "hin"
            | "hinter"
            | "los"
            | "miss"
            | "miß"
            | "mit"
            | "nach"
            | "neben"
            | "nieder"
            | "ober"
            | "rück"
            | "über"
            | "um"
            | "un"
            | "unter"
            | "ur"
            | "ver"
            | "vor"
            | "weg"
            | "wider"
            | "wieder"
            | "zer"
            | "zu"
            | "zurück"
            | "zusammen"
            | "zwischen"
    )
}

/// Whether `right_lower` is a German derivational suffix that can be shared across coordinated
/// stems (e.g. `freund- und feindlich`, `fahr- und gehbar`).
fn is_german_derivational_suffix(right_lower: &str) -> bool {
    matches!(
        right_lower,
        "bar"
            | "chen"
            | "fach"
            | "fähig"
            | "fähigkeit"
            | "frei"
            | "haft"
            | "heit"
            | "ig"
            | "igkeit"
            | "isch"
            | "keit"
            | "lein"
            | "lich"
            | "lichkeit"
            | "ling"
            | "los"
            | "losigkeit"
            | "mal"
            | "malig"
            | "mäßig"
            | "nis"
            | "reich"
            | "sam"
            | "schaft"
            | "tum"
            | "ung"
            | "voll"
            | "wärts"
            | "weise"
            | "wert"
            | "wesen"
            | "würdig"
    )
}

/// Whether `left` has lexical evidence of being a German compound's leading constituent:
/// a known word, a known prefix, a stem with a German linking morpheme (*Fugenelement*), or a
/// truncated `-e`/`-en` combining stem (e.g. `Schul` from `Schule`, `Miet` from `Miete`).
fn is_left_constituent(left: &str, lang: ProofLang) -> bool {
    let clean = left.trim_end_matches(is_hyphen);
    if !has_at_least_chars(clean, 2) || !clean.chars().any(char::is_alphabetic) {
        return false;
    }
    if is_word_in_dict_any_case(clean, lang) {
        return true;
    }
    let lower = clean.to_lowercase();
    if is_german_prefix(&lower) {
        return true;
    }
    for fuge in ["ens", "ns", "es", "s", "n"] {
        if let Some(stem) = clean.strip_suffix(fuge)
            && has_at_least_chars(stem, 3)
            && (fuge != "s" || !stem.ends_with(['s', 'ß', 'z', 'x']))
            && (fuge != "n" || stem.ends_with('e'))
            && is_word_in_dict_any_case(stem, lang)
        {
            return true;
        }
    }
    if has_at_least_chars(clean, 3) {
        for ext in ["e", "en"] {
            let restored = format!("{clean}{ext}");
            if is_word_in_dict_any_case(&restored, lang) {
                return true;
            }
        }
    }
    false
}

/// Whether `right` has lexical evidence of being a German compound's trailing constituent:
/// a known word or a German derivational suffix.
fn is_right_constituent(right: &str, lang: ProofLang) -> bool {
    let clean = right.trim_start_matches(is_hyphen);
    if !has_at_least_chars(clean, 2) || !clean.chars().any(char::is_alphabetic) {
        return false;
    }
    if is_word_in_dict_any_case(clean, lang) {
        return true;
    }
    is_german_derivational_suffix(&clean.to_lowercase())
}

/// Whether splitting `partner` at `split_byte` aligns with a compound constituent boundary,
/// evidenced by the loaded Hunspell dictionary's compound analysis (authoritative when the
/// dictionary decomposes `partner`) or by dictionary entries for both constituents (with linking
/// morphemes / combining stems).
fn is_constituent_split(partner: &str, split_byte: usize, lang: ProofLang) -> bool {
    let (Some(left), Some(right)) = (partner.get(..split_byte), partner.get(split_byte..)) else {
        return false;
    };
    if !has_at_least_chars(left, 2) || !has_at_least_chars(right, 2) {
        return false;
    }
    if let Some(locale) = lang.spelling_locale() {
        match crate::pack_lang::compound_boundary(partner, split_byte, locale) {
            crate::affix::CompoundBoundary::Matched => return true,
            crate::affix::CompoundBoundary::DifferentSplit => return false,
            crate::affix::CompoundBoundary::NotCompound => {}
        }
    }
    is_left_constituent(left, lang) && is_right_constituent(right, lang)
}

/// Classification of a paragraph token for German coordination scanning.
#[derive(Debug)]
enum TokenKind {
    /// Line/paragraph break, standalone dash, or other hard barrier.
    Break,
    /// Coordination punctuation separator (`,` or `/`).
    Separator,
    /// German coordination conjunction (`und`, `oder`, `sowie`, `bzw.`, ...).
    Conjunction,
    /// German article allowed inside a coordination chain without establishing coordination.
    Article,
    /// Suspended fragment with a trailing hyphen attached to a letter (e.g. `Handlungs-`, `Textilgroß-`).
    TrailingFragment(String),
    /// Suspended fragment with a leading hyphen attached to a letter (e.g. `-qualität`, `-einzelhandel`).
    LeadingFragment(String),
    /// Suspended fragment with both leading and trailing hyphens attached to letters (e.g. `-zwischen-`).
    BothFragment(String),
    /// Complete word without leading or trailing hyphen (digits preserved, composed NFC).
    Word(String),
}

/// One scanned token in a paragraph with its byte span in the original text.
#[derive(Debug)]
struct Token {
    start: usize,
    end: usize,
    kind: TokenKind,
    ends_sentence: bool,
}

/// Paragraph token table with precomputed O(1) chain jump indices for both directions.
///
/// Any run of chain-transparent tokens (separators, conjunctions, articles, and same-direction
/// suspended fragments without terminal sentence punctuation) is skipped in one step, guaranteeing
/// O(1) scan work per issue while remaining exactly equivalent to a fresh scan from each fragment.
struct ParagraphTokens {
    tokens: Vec<Token>,
    /// For forward scans: first index `>= i` that is not forward-chain-transparent.
    fwd_next: Vec<usize>,
    /// Whether `i..fwd_next[i]` contains a coordination connector (`Separator` or `Conjunction`).
    fwd_has_conn: Vec<bool>,
    /// For backward scans: last index `<= i` that is not backward-chain-transparent (or `None`).
    bwd_prev: Vec<Option<usize>>,
    /// Whether `(bwd_prev[i] + 1)..=i` contains a coordination connector (`Separator` or `Conjunction`).
    bwd_has_conn: Vec<bool>,
}

/// Classify a single whitespace/separator-delimited raw token into a [`TokenKind`].
fn classify_trimmed_token(trimmed: &str) -> TokenKind {
    let starts_hyphen = trimmed.chars().next().is_some_and(is_hyphen);
    let ends_hyphen = trimmed.chars().next_back().is_some_and(is_hyphen);

    match (starts_hyphen, ends_hyphen) {
        (true, true) => {
            let lead = trimmed.chars().next().map_or(0, char::len_utf8);
            let trail = trimmed.chars().next_back().map_or(0, char::len_utf8);
            let inner =
                if trimmed.len() > lead.saturating_add(trail) { trimmed.get(lead..trimmed.len().saturating_sub(trail)).unwrap_or("") } else { "" };
            if inner.chars().next().is_some_and(char::is_alphabetic) && ends_with_letter(inner) {
                TokenKind::BothFragment(inner.to_string())
            } else {
                TokenKind::Break
            }
        }
        (false, true) => {
            let trail = trimmed.chars().next_back().map_or(0, char::len_utf8);
            let stem = trimmed.get(..trimmed.len().saturating_sub(trail)).unwrap_or("");
            if ends_with_letter(stem) { TokenKind::TrailingFragment(stem.to_string()) } else { TokenKind::Break }
        }
        (true, false) => {
            let lead = trimmed.chars().next().map_or(0, char::len_utf8);
            let stem = trimmed.get(lead..).unwrap_or("");
            if stem.chars().next().is_some_and(char::is_alphabetic) { TokenKind::LeadingFragment(stem.to_string()) } else { TokenKind::Break }
        }
        (false, false) => {
            let lower = trimmed.to_lowercase();
            if is_german_conjunction(&lower) {
                TokenKind::Conjunction
            } else if is_german_article(&lower) {
                TokenKind::Article
            } else {
                TokenKind::Word(trimmed.to_string())
            }
        }
    }
}

/// Precompute forward chain-jump indices and connector flags in O(N).
fn build_forward_jumps(tokens: &[Token]) -> (Vec<usize>, Vec<bool>) {
    let n = tokens.len();
    let mut fwd_next = vec![n; n];
    let mut fwd_has_conn = vec![false; n];
    let mut next_stop = n;
    let mut conn_to_stop = false;

    for i in (0..n).rev() {
        let Some(tok) = tokens.get(i) else { continue };
        let transparent = !tok.ends_sentence
            && matches!(
                tok.kind,
                TokenKind::Separator | TokenKind::Conjunction | TokenKind::Article | TokenKind::TrailingFragment(_) | TokenKind::BothFragment(_)
            );
        if transparent {
            if matches!(tok.kind, TokenKind::Separator | TokenKind::Conjunction) {
                conn_to_stop = true;
            }
            if let Some(slot) = fwd_next.get_mut(i) {
                *slot = next_stop;
            }
            if let Some(slot) = fwd_has_conn.get_mut(i) {
                *slot = conn_to_stop;
            }
        } else {
            next_stop = i;
            conn_to_stop = false;
            if let Some(slot) = fwd_next.get_mut(i) {
                *slot = i;
            }
            if let Some(slot) = fwd_has_conn.get_mut(i) {
                *slot = false;
            }
        }
    }

    (fwd_next, fwd_has_conn)
}

/// Precompute backward chain-jump indices and connector flags in O(N).
fn build_backward_jumps(tokens: &[Token]) -> (Vec<Option<usize>>, Vec<bool>) {
    let n = tokens.len();
    let mut bwd_prev = vec![None; n];
    let mut bwd_has_conn = vec![false; n];
    let mut prev_stop: Option<usize> = None;
    let mut conn_from_stop = false;

    for i in 0..n {
        let Some(tok) = tokens.get(i) else { continue };
        let transparent = !tok.ends_sentence
            && matches!(
                tok.kind,
                TokenKind::Separator | TokenKind::Conjunction | TokenKind::Article | TokenKind::LeadingFragment(_) | TokenKind::BothFragment(_)
            );
        if transparent {
            if matches!(tok.kind, TokenKind::Separator | TokenKind::Conjunction) {
                conn_from_stop = true;
            }
            if let Some(slot) = bwd_prev.get_mut(i) {
                *slot = prev_stop;
            }
            if let Some(slot) = bwd_has_conn.get_mut(i) {
                *slot = conn_from_stop;
            }
        } else {
            prev_stop = Some(i);
            conn_from_stop = false;
            if let Some(slot) = bwd_prev.get_mut(i) {
                *slot = Some(i);
            }
            if let Some(slot) = bwd_has_conn.get_mut(i) {
                *slot = false;
            }
        }
    }

    (bwd_prev, bwd_has_conn)
}

impl ParagraphTokens {
    fn tokenize(text: &str) -> Self {
        let mut tokens: Vec<Token> = Vec::new();
        let mut pos: usize = 0;

        while let Some(rem) = text.get(pos..) {
            let Some(first) = rem.chars().next() else {
                break;
            };
            if is_line_or_para_break(first) {
                let next_pos = pos.saturating_add(first.len_utf8());
                if let Some(last) = tokens.last_mut()
                    && matches!(last.kind, TokenKind::Break)
                {
                    last.end = next_pos;
                } else {
                    count_step();
                    tokens.push(Token { start: pos, end: next_pos, kind: TokenKind::Break, ends_sentence: true });
                }
                pos = next_pos;
                continue;
            }
            if first.is_whitespace() {
                let ws_len = rem.find(|c: char| !c.is_whitespace() || is_line_or_para_break(c)).unwrap_or(rem.len());
                pos = pos.saturating_add(ws_len.max(first.len_utf8()));
                continue;
            }
            if first == ',' || first == '/' {
                count_step();
                let next_pos = pos.saturating_add(1);
                tokens.push(Token { start: pos, end: next_pos, kind: TokenKind::Separator, ends_sentence: false });
                pos = next_pos;
                continue;
            }

            count_step();
            let token_len = rem.find(|c: char| c.is_whitespace() || c == ',' || c == '/').unwrap_or(rem.len());
            let token_end = pos.saturating_add(token_len);
            let raw_token = rem.get(..token_len).unwrap_or("");
            let token_start = pos;
            pos = token_end;

            let ends_sentence = has_sentence_punctuation(raw_token);
            let composed = crate::composed(raw_token);
            let is_trim_punct = |c: char| !c.is_alphanumeric() && !is_hyphen(c);
            let trimmed = composed
                .trim_start_matches(is_trim_punct)
                .trim_end_matches(|c: char| is_trim_punct(c) && !unicode_normalization::char::is_combining_mark(c));

            if trimmed.is_empty() {
                tokens.push(Token { start: token_start, end: token_end, kind: TokenKind::Break, ends_sentence: true });
                continue;
            }
            if starts_with_sentence_punctuation(raw_token) {
                tokens.push(Token { start: token_start, end: token_start, kind: TokenKind::Break, ends_sentence: true });
            }

            let kind = classify_trimmed_token(trimmed);
            tokens.push(Token { start: token_start, end: token_end, kind, ends_sentence });
        }

        let (fwd_next, fwd_has_conn) = build_forward_jumps(&tokens);
        let (bwd_prev, bwd_has_conn) = build_backward_jumps(&tokens);
        Self { tokens, fwd_next, fwd_has_conn, bwd_prev, bwd_has_conn }
    }

    /// Locate the token containing the byte span `start..end`.
    fn token_index_for(&self, start: usize, end: usize) -> Option<usize> {
        let idx = self.tokens.partition_point(|t| t.end <= start);
        let tok = self.tokens.get(idx)?;
        (tok.start <= start && end <= tok.end).then_some(idx)
    }

    /// Whether token `idx` begins a sentence (at paragraph start or immediately after a break or
    /// sentence-ending token).
    fn is_at_sentence_start(&self, idx: usize) -> bool {
        let Some(prev_idx) = idx.checked_sub(1) else {
            return true;
        };
        self.tokens.get(prev_idx).is_none_or(|t| t.ends_sentence || matches!(t.kind, TokenKind::Break))
    }
}

/// A structural partner found by scanning forward from a suspended fragment.
enum ForwardPartner<'a> {
    /// Complete partner word (e.g. `Zustandssatz` in `Handlungs- und Zustandssatz`).
    Complete(&'a str),
    /// Suspended leading-hyphen partner (e.g. `-einzelhandel` in `Textilgroß- und -einzelhandel`).
    LeadingFragment(&'a str),
}

/// A structural partner found by scanning backward from a suspended fragment.
#[derive(Clone, Copy)]
enum BackwardPartner<'a> {
    /// Complete partner word (e.g. `Wasserverbrauch` in `Wasserverbrauch und -qualität`).
    Complete(&'a str),
    /// Suspended trailing-hyphen partner (e.g. `Textilgroß-` in `Textilgroß- und -einzelhandel`).
    TrailingFragment(&'a str),
}

/// Whether `issue` has an adjacent hyphen in `text` that could make it a German ellipsis fragment.
fn has_adjacent_hyphen(text: &str, start: usize, end: usize) -> bool {
    let trailing = text.get(end..).and_then(|s| s.chars().next()).is_some_and(is_hyphen);
    let leading = text.get(..start).and_then(|s| s.chars().next_back()).is_some_and(is_hyphen);
    trailing || leading
}

/// Whether `start..end` is immediately followed by a hyphen and a valid forward boundary
/// (horizontal whitespace, `,`, or `/`, optionally after closing quotes/brackets).
fn has_forward_hyphen_boundary(text: &str, end: usize) -> bool {
    let Some(after_word) = text.get(end..) else {
        return false;
    };
    let Some(hyphen_char) = after_word.chars().next().filter(|c| is_hyphen(*c)) else {
        return false;
    };
    let Some(after_hyphen) = text.get(end.saturating_add(hyphen_char.len_utf8())..) else {
        return false;
    };
    let after_delims = after_hyphen.trim_start_matches(is_closing_delimiter);
    after_delims.chars().next().is_some_and(|c| !is_line_or_para_break(c) && (c.is_whitespace() || c == ',' || c == '/'))
}

/// Whether `start..end` is immediately preceded by a hyphen and a valid backward boundary
/// (horizontal whitespace or `/`, optionally before opening quotes/brackets).
fn has_backward_hyphen_boundary(text: &str, start: usize) -> bool {
    let Some(before_word) = text.get(..start) else {
        return false;
    };
    let Some(hyphen_char) = before_word.chars().next_back().filter(|c| is_hyphen(*c)) else {
        return false;
    };
    let Some(before_hyphen) = text.get(..start.saturating_sub(hyphen_char.len_utf8())) else {
        return false;
    };
    let before_delims = before_hyphen.trim_end_matches(is_opening_delimiter);
    before_delims.chars().next_back().is_some_and(|c| !is_line_or_para_break(c) && (c.is_whitespace() || c == '/'))
}

/// Filter out valid German coordinated compounds with ellipsis (*Ergänzungsstrich*).
///
/// In German orthography (§ 98 Amtliche Rechtschreibung), a suspended hyphen marks an omitted
/// compound constituent in coordinated compounds (e.g. `Handlungs- und Zustandssatz`,
/// `Schul- und Universitätsbetrieb`, `Wasserverbrauch und -qualität`, or two-sided forms such as
/// `Textilgroß- und -einzelhandel`).
///
/// The paragraph is tokenized once with O(1) jump links across coordination chains so repeated or
/// long lists scale linearly while evaluating connector and modifier rules fresh for each fragment.
pub(crate) fn filter_coordinated_compounds(text: &str, issues: &mut Vec<Issue>) {
    let mut parsed: Option<ParagraphTokens> = None;

    issues.retain(|issue| {
        if issue.kind != IssueKind::Spelling || !is_german_lang(issue.lang) {
            return true;
        }
        let Some(word) = text.get(issue.start..issue.end) else {
            return true;
        };
        if word.is_empty() || !has_adjacent_hyphen(text, issue.start, issue.end) {
            return true;
        }

        let para = parsed.get_or_insert_with(|| ParagraphTokens::tokenize(text));
        let Some(idx) = para.token_index_for(issue.start, issue.end) else {
            return true;
        };
        let Some(tok) = para.tokens.get(idx) else {
            return true;
        };
        let composed_word = crate::composed(word);

        match &tok.kind {
            TokenKind::TrailingFragment(frag) if frag == composed_word.as_ref() && has_forward_hyphen_boundary(text, issue.end) => {
                !check_forward_at(para, idx, frag, issue.lang)
            }
            TokenKind::LeadingFragment(frag) if frag == composed_word.as_ref() && has_backward_hyphen_boundary(text, issue.start) => {
                !check_backward_at(para, idx, frag, issue.lang)
            }
            TokenKind::BothFragment(frag)
                if frag == composed_word.as_ref()
                    && has_backward_hyphen_boundary(text, issue.start)
                    && has_forward_hyphen_boundary(text, issue.end) =>
            {
                !check_both_sided_middle_at(para, idx, frag, issue.lang)
            }
            _ => true,
        }
    });
}

/// Collect candidate forward partners reachable from token `idx` (strictly O(1) iterations).
fn scan_forward_partners(para: &ParagraphTokens, idx: usize) -> Vec<ForwardPartner<'_>> {
    let mut candidates = Vec::new();
    let mut cur = idx.saturating_add(1);
    let mut saw_connector = false;
    let mut modifier_count: usize = 0;

    while cur < para.tokens.len() {
        count_step();
        let jump = para.fwd_next.get(cur).copied().unwrap_or(para.tokens.len());
        if jump > cur {
            if para.fwd_has_conn.get(cur).copied().unwrap_or(false) {
                saw_connector = true;
            }
            cur = jump;
            continue;
        }

        let Some(tok) = para.tokens.get(cur) else {
            break;
        };

        match &tok.kind {
            TokenKind::Break => break,
            TokenKind::Separator | TokenKind::Conjunction | TokenKind::TrailingFragment(_) | TokenKind::BothFragment(_) => {
                if matches!(tok.kind, TokenKind::Separator | TokenKind::Conjunction) {
                    saw_connector = true;
                }
                if tok.ends_sentence {
                    break;
                }
                cur = cur.saturating_add(1);
            }
            TokenKind::Article => {
                if tok.ends_sentence {
                    break;
                }
                cur = cur.saturating_add(1);
            }
            TokenKind::LeadingFragment(partner) => {
                if saw_connector && has_at_least_chars(partner, 2) {
                    candidates.push(ForwardPartner::LeadingFragment(partner));
                }
                break;
            }
            TokenKind::Word(partner) => {
                let first_upper = partner.chars().next().is_some_and(char::is_uppercase);
                let first_lower = partner.chars().next().is_some_and(char::is_lowercase);

                if saw_connector {
                    if first_upper {
                        if has_at_least_chars(partner, 2) {
                            candidates.push(ForwardPartner::Complete(partner));
                        }
                        break;
                    }
                    if first_lower {
                        if has_at_least_chars(partner, 2) {
                            candidates.push(ForwardPartner::Complete(partner));
                        }
                        if !tok.ends_sentence && modifier_count < MAX_MODIFIERS {
                            modifier_count = modifier_count.saturating_add(1);
                            cur = cur.saturating_add(1);
                            continue;
                        }
                    }
                    break;
                }

                if !tok.ends_sentence && first_lower && modifier_count < MAX_MODIFIERS {
                    modifier_count = modifier_count.saturating_add(1);
                    cur = cur.saturating_add(1);
                    continue;
                }
                break;
            }
        }
    }

    candidates
}

/// Find the backward partner reachable from token `idx` (strictly O(1) iterations).
fn scan_backward_partner(para: &ParagraphTokens, idx: usize) -> Option<BackwardPartner<'_>> {
    let mut cur = idx.checked_sub(1)?;
    let mut saw_connector = false;
    let mut modifier_count: usize = 0;

    loop {
        count_step();
        let prev = para.bwd_prev.get(cur).copied().flatten();
        if prev != Some(cur) {
            if para.bwd_has_conn.get(cur).copied().unwrap_or(false) {
                saw_connector = true;
            }
            cur = prev?;
            continue;
        }

        let tok = para.tokens.get(cur)?;
        if tok.ends_sentence {
            return None;
        }

        match &tok.kind {
            TokenKind::Break => return None,
            TokenKind::Separator | TokenKind::Conjunction | TokenKind::LeadingFragment(_) | TokenKind::BothFragment(_) => {
                if matches!(tok.kind, TokenKind::Separator | TokenKind::Conjunction) {
                    saw_connector = true;
                }
                cur = cur.checked_sub(1)?;
            }
            TokenKind::Article => {
                cur = cur.checked_sub(1)?;
            }
            TokenKind::TrailingFragment(partner) => {
                return (saw_connector && has_at_least_chars(partner, 2)).then_some(BackwardPartner::TrailingFragment(partner));
            }
            TokenKind::Word(partner) => {
                if saw_connector {
                    if has_at_least_chars(partner, 2) {
                        return Some(BackwardPartner::Complete(partner));
                    }
                    let first_lower = partner.chars().next().is_some_and(char::is_lowercase);
                    if first_lower && modifier_count < MAX_MODIFIERS {
                        modifier_count = modifier_count.saturating_add(1);
                        cur = cur.checked_sub(1)?;
                        continue;
                    }
                    return None;
                }

                let first_lower = partner.chars().next().is_some_and(char::is_lowercase);
                if first_lower && modifier_count < MAX_MODIFIERS {
                    modifier_count = modifier_count.saturating_add(1);
                    cur = cur.checked_sub(1)?;
                    continue;
                }
                return None;
            }
        }
    }
}

/// Check if a trailing-hyphen fragment at token `idx` forms a valid forward or two-sided compound.
fn check_forward_at(para: &ParagraphTokens, idx: usize, word: &str, lang: ProofLang) -> bool {
    let is_upper = word.chars().next().is_some_and(char::is_uppercase);
    let at_sentence_start = para.is_at_sentence_start(idx);

    for partner in scan_forward_partners(para, idx) {
        match partner {
            ForwardPartner::Complete(p) => {
                let partner_is_upper = p.chars().next().is_some_and(char::is_uppercase);
                if is_upper && !partner_is_upper && !at_sentence_start {
                    continue;
                }
                if test_compound_forward(word, p, lang) {
                    return true;
                }
            }
            ForwardPartner::LeadingFragment(right_frag) => {
                if test_compound_two_sided(word, right_frag, lang) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a leading-hyphen fragment at token `idx` forms a valid backward or two-sided compound.
fn check_backward_at(para: &ParagraphTokens, idx: usize, word: &str, lang: ProofLang) -> bool {
    match scan_backward_partner(para, idx) {
        Some(BackwardPartner::Complete(partner)) => test_compound_backward(partner, word, lang),
        Some(BackwardPartner::TrailingFragment(left_frag)) => test_compound_two_sided(left_frag, word, lang),
        None => false,
    }
}

/// Match `prefix` case-insensitively at the start of `haystack`, returning the exact UTF-8 byte
/// offset in `haystack` immediately after the matched prefix.
fn case_insensitive_prefix_end(haystack: &str, prefix: &str) -> Option<usize> {
    let mut h_chars = haystack.char_indices();
    let mut end_byte = 0;
    for p in prefix.chars() {
        let (idx, h) = h_chars.next()?;
        if !h.to_lowercase().eq(p.to_lowercase()) {
            return None;
        }
        end_byte = idx.saturating_add(h.len_utf8());
    }
    (end_byte > 0 && end_byte < haystack.len()).then_some(end_byte)
}

/// Match `suffix` case-insensitively at the end of `haystack`, returning the exact UTF-8 byte
/// offset in `haystack` where the matched suffix starts.
fn case_insensitive_suffix_start(haystack: &str, suffix: &str) -> Option<usize> {
    let mut h_chars = haystack.char_indices().rev();
    let mut start_byte = haystack.len();
    for s in suffix.chars().rev() {
        let (idx, h) = h_chars.next()?;
        if !h.to_lowercase().eq(s.to_lowercase()) {
            return None;
        }
        start_byte = idx;
    }
    (start_byte > 0 && start_byte < haystack.len()).then_some(start_byte)
}

/// Check whether `prefix + middle + suffix` forms a valid dictionary word (preserving `middle`'s
/// written capitalization and allowing ALL-CAPS headings).
fn matches_middle_candidate(prefix: &str, middle: &str, suffix: &str, lang: ProofLang) -> bool {
    let suffix_lower = lowercase_first(suffix);
    let cand = format!("{prefix}{middle}{suffix_lower}");
    is_known_word(&cand, lang)
        || (is_all_caps(prefix) && is_all_caps(middle) && is_all_caps(suffix) && is_known_word(&format!("{prefix}{middle}{suffix}"), lang))
}

/// Collect all valid `(split_byte, suffix_clean)` candidates of `right` within the 40-byte window
/// that also align with a constituent boundary in `complete_left`.
fn collect_shared_suffixes_with_complete_left<'a>(complete_left: &str, right: &'a str, lang: ProofLang) -> Vec<(usize, usize, &'a str)> {
    let mut out = Vec::new();
    if complete_left.len() > crate::affix::MAX_WORD_BYTES || right.len() > crate::affix::MAX_WORD_BYTES {
        return out;
    }
    let mut window_start = right.len().saturating_sub(MAX_PARTNER_WINDOW_BYTES);
    while !right.is_char_boundary(window_start) {
        window_start = window_start.saturating_add(1);
    }
    let Some(window) = right.get(window_start..) else {
        return out;
    };

    for (k, _) in window.char_indices() {
        let split_byte = window_start.saturating_add(k);
        if split_byte == 0 {
            continue;
        }
        let Some(suffix) = window.get(k..) else { continue };
        if !has_at_least_chars(suffix, 2) {
            break;
        }
        let suffix_clean = suffix.trim_start_matches(is_hyphen);
        if !has_at_least_chars(suffix_clean, 2) {
            continue;
        }
        if let Some(left_split) = case_insensitive_suffix_start(complete_left, suffix_clean)
            && is_constituent_split(right, split_byte, lang)
            && is_constituent_split(complete_left, left_split, lang)
        {
            out.push((left_split, split_byte, suffix_clean));
        }
    }
    out
}

/// Collect all valid `(split_byte_in_left, split_byte_in_right, prefix_clean)` candidates of `left`
/// within the 40-byte window that also align with a constituent boundary in `complete_right`.
fn collect_shared_prefixes_with_complete_right<'a>(left: &'a str, complete_right: &str, lang: ProofLang) -> Vec<(usize, usize, &'a str)> {
    let mut out = Vec::new();
    if left.len() > crate::affix::MAX_WORD_BYTES || complete_right.len() > crate::affix::MAX_WORD_BYTES {
        return out;
    }
    for (k, _) in left.char_indices().take_while(|&(k, _)| k <= MAX_PARTNER_WINDOW_BYTES) {
        if k < 2 {
            continue;
        }
        let (Some(prefix), Some(rest)) = (left.get(..k), left.get(k..)) else {
            continue;
        };
        if !has_at_least_chars(rest, 2) {
            break;
        }
        let prefix_clean = prefix.trim_end_matches(is_hyphen);
        if !has_at_least_chars(prefix_clean, 2) {
            continue;
        }
        if let Some(right_split) = case_insensitive_prefix_end(complete_right, prefix_clean)
            && is_constituent_split(left, k, lang)
            && is_constituent_split(complete_right, right_split, lang)
        {
            out.push((k, right_split, prefix_clean));
        }
    }
    out
}

/// Visit valid `(split_byte, suffix_clean)` candidates of `partner` within the 40-byte window that
/// combine with `word` into a dictionary-accepted compound, stopping early when `on_match` returns `true`.
fn for_each_valid_borrowed_suffix<'a>(
    word: &str,
    partner: &'a str,
    lang: ProofLang,
    require_partner_known: bool,
    mut on_match: impl FnMut(usize, &'a str) -> bool,
) {
    if word.len() > crate::affix::MAX_WORD_BYTES || partner.len() > crate::affix::MAX_WORD_BYTES {
        return;
    }
    if require_partner_known && !is_known_word_or_hyphenated(partner, lang) {
        return;
    }

    let mut window_start = partner.len().saturating_sub(MAX_PARTNER_WINDOW_BYTES);
    while !partner.is_char_boundary(window_start) {
        window_start = window_start.saturating_add(1);
    }
    let Some(window) = partner.get(window_start..) else {
        return;
    };

    for (k, _) in window.char_indices() {
        let split_byte = window_start.saturating_add(k);
        if split_byte == 0 {
            continue;
        }
        let Some(suffix) = window.get(k..) else { continue };
        if !has_at_least_chars(suffix, 2) {
            break;
        }
        count_step();
        let suffix_clean = suffix.trim_start_matches(is_hyphen);
        if !has_at_least_chars(suffix_clean, 2) {
            continue;
        }
        if matches_forward_candidate(word, suffix_clean, lang)
            && is_constituent_split(partner, split_byte, lang)
            && on_match(split_byte, suffix_clean)
        {
            return;
        }
    }
}

/// Visit valid `(split_byte, prefix_clean)` candidates of `partner` within the 40-byte window that
/// combine with `word` into a dictionary-accepted compound, stopping early when `on_match` returns `true`.
fn for_each_valid_borrowed_prefix<'a>(
    partner: &'a str,
    word: &str,
    lang: ProofLang,
    require_partner_known: bool,
    mut on_match: impl FnMut(usize, &'a str) -> bool,
) {
    if word.len() > crate::affix::MAX_WORD_BYTES || partner.len() > crate::affix::MAX_WORD_BYTES {
        return;
    }
    if require_partner_known && !is_known_word_or_hyphenated(partner, lang) {
        return;
    }

    for (k, _) in partner.char_indices().take_while(|&(k, _)| k <= MAX_PARTNER_WINDOW_BYTES) {
        if k < 2 {
            continue;
        }
        let (Some(prefix), Some(rest)) = (partner.get(..k), partner.get(k..)) else {
            continue;
        };
        if !has_at_least_chars(rest, 2) {
            break;
        }
        count_step();
        let prefix_clean = prefix.trim_end_matches(is_hyphen);
        if !has_at_least_chars(prefix_clean, 2) {
            continue;
        }
        if matches_backward_candidate(prefix_clean, word, lang) && is_constituent_split(partner, k, lang) && on_match(k, prefix_clean) {
            return;
        }
    }
}

/// Collect all valid `(split_byte, suffix_clean)` candidates of `partner` (within the 40-byte window).
fn collect_valid_borrowed_suffixes<'a>(word: &str, partner: &'a str, lang: ProofLang, require_partner_known: bool) -> Vec<(usize, &'a str)> {
    let mut out = Vec::new();
    for_each_valid_borrowed_suffix(word, partner, lang, require_partner_known, |split_byte, suffix| {
        out.push((split_byte, suffix));
        false
    });
    out
}

/// Collect all valid `(split_byte, prefix_clean)` candidates of `partner` (within the 40-byte window).
fn collect_valid_borrowed_prefixes<'a>(partner: &'a str, word: &str, lang: ProofLang, require_partner_known: bool) -> Vec<(usize, &'a str)> {
    let mut out = Vec::new();
    for_each_valid_borrowed_prefix(partner, word, lang, require_partner_known, |split_byte, prefix| {
        out.push((split_byte, prefix));
        false
    });
    out
}

/// Check if a middle fragment with hyphens on both sides (e.g. `-zwischen-` in
/// `Textilgroß-, -zwischen- und -einzelhandel`, `Textilgroßhandel, -zwischen- und -einzelhandel`,
/// `Textilgroß-, -zwischen- und Textileinzelhandel`, or
/// `Textilgroßhandel, -zwischen- und Textileinzelhandel`) forms a valid compound using eligible
/// shared prefix and suffix constituents from the left and right anchors.
fn check_both_sided_middle_at(para: &ParagraphTokens, idx: usize, middle: &str, lang: ProofLang) -> bool {
    if middle.len() > crate::affix::MAX_WORD_BYTES {
        return false;
    }
    let Some(left_partner) = scan_backward_partner(para, idx) else {
        return false;
    };
    for right_partner in scan_forward_partners(para, idx) {
        if match_middle_with_anchors(left_partner, middle, right_partner, lang) {
            return true;
        }
    }
    false
}

/// Match `middle` against one `(left_partner, right_partner)` anchor pair, trying all eligible
/// constituent splits within the 40-byte windows and returning immediately on the first valid
/// reconstruction.
fn match_middle_with_anchors(left_partner: BackwardPartner<'_>, middle: &str, right_partner: ForwardPartner<'_>, lang: ProofLang) -> bool {
    match (left_partner, right_partner) {
        (BackwardPartner::TrailingFragment(l), ForwardPartner::LeadingFragment(r)) => {
            let prefixes = collect_valid_borrowed_prefixes(l, r, lang, false);
            if prefixes.is_empty() {
                return false;
            }
            let suffixes = collect_valid_borrowed_suffixes(l, r, lang, false);
            prefixes.iter().any(|&(_, p)| suffixes.iter().any(|&(_, s)| matches_middle_candidate(p, middle, s, lang)))
        }
        (BackwardPartner::Complete(l), ForwardPartner::LeadingFragment(r)) => {
            if l.len() > crate::affix::MAX_WORD_BYTES || r.len() > crate::affix::MAX_WORD_BYTES {
                return false;
            }
            let prefixes = collect_valid_borrowed_prefixes(l, r, lang, true);
            if prefixes.is_empty() {
                return false;
            }
            let suffixes = collect_shared_suffixes_with_complete_left(l, r, lang);
            prefixes.iter().any(|&(k_l, p)| suffixes.iter().any(|&(l_split, _, s)| k_l <= l_split && matches_middle_candidate(p, middle, s, lang)))
        }
        (BackwardPartner::TrailingFragment(l), ForwardPartner::Complete(r)) => {
            if l.len() > crate::affix::MAX_WORD_BYTES || r.len() > crate::affix::MAX_WORD_BYTES {
                return false;
            }
            let suffixes = collect_valid_borrowed_suffixes(l, r, lang, true);
            if suffixes.is_empty() {
                return false;
            }
            let prefixes = collect_shared_prefixes_with_complete_right(l, r, lang);
            prefixes.iter().any(|&(_, r_split, p)| suffixes.iter().any(|&(k_r, s)| r_split <= k_r && matches_middle_candidate(p, middle, s, lang)))
        }
        (BackwardPartner::Complete(l), ForwardPartner::Complete(r)) => {
            if l.len() > crate::affix::MAX_WORD_BYTES
                || r.len() > crate::affix::MAX_WORD_BYTES
                || !is_known_word_or_hyphenated(l, lang)
                || !is_known_word_or_hyphenated(r, lang)
            {
                return false;
            }
            let prefixes = collect_shared_prefixes_with_complete_right(l, r, lang);
            if prefixes.is_empty() {
                return false;
            }
            let suffixes = collect_shared_suffixes_with_complete_left(l, r, lang);
            prefixes.iter().any(|&(k_l, r_split, p)| {
                suffixes.iter().any(|&(l_split, k_r, s)| k_l <= l_split && r_split <= k_r && matches_middle_candidate(p, middle, s, lang))
            })
        }
    }
}

/// Check a reconstructed candidate that may legitimately carry a hyphen when `allow_hyphen_split`
/// is true (i.e. one of the constituent parts already contains an internal hyphen such as `E-Mail`).
fn is_known_candidate(cand: &str, lang: ProofLang, allow_hyphen_split: bool) -> bool {
    if allow_hyphen_split { is_known_word_or_hyphenated(cand, lang) } else { is_known_word(cand, lang) }
}

/// Build dictionary candidates for `word` joined with `suffix` and check whether any is a known word.
fn matches_forward_candidate(word: &str, suffix: &str, lang: ProofLang) -> bool {
    let word_is_upper = word.chars().next().is_some_and(char::is_uppercase);
    let suffix_lower = lowercase_first(suffix);
    let suffix_upper = uppercase_first(suffix);
    let has_internal_hyphen = word.contains('-') || suffix.contains('-');

    if word_is_upper {
        if is_all_caps(word) && is_all_caps(suffix) && is_known_candidate(&format!("{word}{suffix}"), lang, has_internal_hyphen) {
            return true;
        }
        if is_known_candidate(&format!("{word}{suffix_lower}"), lang, has_internal_hyphen) {
            return true;
        }
        if is_known_candidate(&format!("{word}-{suffix_upper}"), lang, has_internal_hyphen) {
            return true;
        }
        if suffix != suffix_lower && suffix != suffix_upper && is_known_candidate(&format!("{word}{suffix}"), lang, has_internal_hyphen) {
            return true;
        }
        false
    } else {
        is_known_candidate(&format!("{word}{suffix_lower}"), lang, has_internal_hyphen)
            || is_known_candidate(&format!("{word}-{suffix_lower}"), lang, has_internal_hyphen)
    }
}

/// Build dictionary candidates for `prefix` joined with `word` and check whether any is a known word.
fn matches_backward_candidate(prefix: &str, word: &str, lang: ProofLang) -> bool {
    let prefix_is_upper = prefix.chars().next().is_some_and(char::is_uppercase);
    let word_is_upper = word.chars().next().is_some_and(char::is_uppercase);
    let word_upper = uppercase_first(word);
    let has_internal_hyphen = prefix.contains('-') || word.contains('-');

    if prefix_is_upper {
        if is_all_caps(prefix) && is_all_caps(word) && is_known_candidate(&format!("{prefix}{word}"), lang, has_internal_hyphen) {
            return true;
        }
        if word_is_upper {
            is_known_candidate(&format!("{prefix}-{word}"), lang, has_internal_hyphen)
                || is_known_candidate(&format!("{prefix}{word}"), lang, has_internal_hyphen)
        } else {
            is_known_candidate(&format!("{prefix}{word}"), lang, has_internal_hyphen)
                || is_known_candidate(&format!("{prefix}-{word_upper}"), lang, has_internal_hyphen)
        }
    } else if word_is_upper {
        false
    } else {
        is_known_candidate(&format!("{prefix}{word}"), lang, has_internal_hyphen)
            || is_known_candidate(&format!("{prefix}-{word}"), lang, has_internal_hyphen)
    }
}

/// Find a valid constituent suffix of `partner` (within the bounded 40-byte window) that combines
/// with `word` into a dictionary-accepted compound.
fn find_valid_borrowed_suffix<'a>(word: &str, partner: &'a str, lang: ProofLang, require_partner_known: bool) -> Option<&'a str> {
    let mut found = None;
    for_each_valid_borrowed_suffix(word, partner, lang, require_partner_known, |_, suffix| {
        found = Some(suffix);
        true
    });
    found
}

/// Find a valid constituent prefix of `partner` (within the bounded 40-byte window) that combines
/// with `word` into a dictionary-accepted compound.
fn find_valid_borrowed_prefix<'a>(partner: &'a str, word: &str, lang: ProofLang, require_partner_known: bool) -> Option<&'a str> {
    let mut found = None;
    for_each_valid_borrowed_prefix(partner, word, lang, require_partner_known, |_, prefix| {
        found = Some(prefix);
        true
    });
    found
}

/// Test whether `word` plus a valid constituent suffix of `partner` forms a valid German compound.
fn test_compound_forward(word: &str, partner: &str, lang: ProofLang) -> bool {
    find_valid_borrowed_suffix(word, partner, lang, true).is_some()
}

/// Test whether a valid constituent prefix of `partner` plus `word` forms a valid German compound.
fn test_compound_backward(partner: &str, word: &str, lang: ProofLang) -> bool {
    find_valid_borrowed_prefix(partner, word, lang, true).is_some()
}

/// Test a two-sided suspended compound (e.g. `Textilgroß- und -einzelhandel`):
/// both reconstructed words must be accepted by the dictionary and both splits must fall on
/// constituent boundaries (`Textil|groß` + `handel` → `Textilgroßhandel`, `Textil` + `einzel|handel`
/// → `Textileinzelhandel`).
fn test_compound_two_sided(left_frag: &str, right_frag: &str, lang: ProofLang) -> bool {
    find_valid_borrowed_suffix(left_frag, right_frag, lang, false).is_some()
        && find_valid_borrowed_prefix(left_frag, right_frag, lang, false).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;

    struct TestDictGuard {
        temp_dir: std::path::PathBuf,
    }

    impl Drop for TestDictGuard {
        fn drop(&mut self) {
            crate::set_dictionaries_folder(None);
            let _ = std::fs::remove_dir_all(&self.temp_dir);
        }
    }

    fn write_dic(path: &std::path::Path, entries: &[&str]) {
        let mut content = format!("{}\n", entries.len());
        for entry in entries {
            content.push_str(entry);
            content.push('\n');
        }
        let _ = std::fs::write(path, content);
    }

    /// German coordinated compounds with ellipsis (e.g. `Handlungs- und Zustandssatz`,
    /// `Schul- und Universitätsbetrieb`, `Wasserverbrauch und -qualität`, and two-sided
    /// `Textilgroß- und -einzelhandel`) are valid compound forms (§ 98) and must not be flagged
    /// as spelling errors (storytold/wordcraft#5903).
    #[test]
    fn german_coordinated_compounds_with_ellipsis_are_accepted() {
        crate::hold_dictionary_tests();
        let temp_dir = std::env::temp_dir().join(format!("craft-proof-test-de-{}", std::process::id()));
        let _guard = TestDictGuard { temp_dir: temp_dir.clone() };

        let _ = std::fs::create_dir_all(&temp_dir);
        let aff = "SET UTF-8\nCOMPOUNDFLAG X\nCOMPOUNDMIDDLE m\nONLYINCOMPOUND o\nFORCEUCASE u\n";
        let base_entries = [
            // Full compound entries
            "Handlung/X",
            "Handlungssatz/X",
            "Zustandssatz/X",
            "Schulbetrieb/X",
            "Universitätsbetrieb/X",
            "Arbeitsrecht/X",
            "Sozialrecht/X",
            "Familienrecht/X",
            "Steuerrecht/X",
            "Vertragsrecht/X",
            "Versicherungsrecht/X",
            "Verkehrsrecht/X",
            "Mietrecht/X",
            "Forschungsabteilung/X",
            "Entwicklungsabteilung/X",
            "Wirtschaftspolitik/X",
            "Finanzpolitik/X",
            "Bildungsauftrag/X",
            "Erziehungsauftrag/X",
            "Bewegungsverb/X",
            "Wahrnehmungsverb/X",
            "Ordnungsdienst/X",
            "Sicherheitsdienst/X",
            "Gestaltungssatz/X",
            "Wasserverbrauch/X",
            "Wasserqualität/X",
            "Wassereinzelbrauch/X",
            "Wasserzwischenbrauch/X",
            "Friedensforschung/X",
            "Konfliktforschung/X",
            "Liebesbeziehung/X",
            "Ehebeziehung/X",
            "Lebensbedingungen/X",
            "Arbeitsbedingungen/X",
            "Abendmenü/X",
            "Mittagsmenü/X",
            "Abendkarte/X",
            "Textilgroßhandel/X",
            "Textileinzelhandel/X",
            "Textilzwischenhandel/X",
            "Texteinzelhandel/X",
            "Großhandel",
            "Preisersatz/X",
            "Futterneid/X",
            "Amtseid/X",
            "Hinfahrt/X",
            "Silberfeier/X",
            "beladen",
            "entladen",
            "freundlich",
            "feindlich",
            // Constituent stems / standalone words (as in a real German dictionary)
            "Zustand",
            "Satz",
            "Preis",
            "Ersatz",
            "Futter",
            "Neid",
            "Eid",
            "Universität",
            "Betrieb",
            "sozial",
            "Recht",
            "Miete",
            "Entwicklung",
            "Abteilung",
            "Finanz",
            "Politik",
            "Vertrag",
            "Erziehung",
            "Auftrag",
            "Wahrnehmung",
            "Verb",
            "Sicherheit",
            "Dienst",
            "Konflikt",
            "Forschung",
            "Ehe",
            "Beziehung",
            "Arbeit",
            "Bedingungen",
            "Wasser/X",
            "Verbrauch",
            "Brauch",
            "Qualität",
            "Abend",
            "Mittag",
            "Menü",
            "Karte",
            "Sport",
            "Platz",
            "Text",
            "ilgroß",
            "Textil/X",
            "groß/m",
            "Einzel",
            "Handel",
            "Mail",
            "Adresse",
            "Anhang",
            "Geschäftsadresse",
            "laden",
            "Feind",
            "was",
            // ONLYINCOMPOUND stems to test Hunspell compound-boundary evidence directly
            "Rück/oX",
            "fahrt/oX",
            "Hochzeits/oX",
            "feier/oX",
            "einzel/oX",
            "handel/oXu",
            "verbrauch/oXu",
            // Function words and misc
            "WA/X",
            "und",
            "oder",
            "bzw",
            "der",
            "neue",
            "sehr",
            "gut",
        ];
        let mut ch_entries = base_entries.to_vec();
        ch_entries.extend_from_slice(&["Fussballplatz/X", "Sportplatz/X"]);

        let _ = std::fs::write(temp_dir.join("de_DE.aff"), aff);
        write_dic(&temp_dir.join("de_DE.dic"), &base_entries);
        let _ = std::fs::write(temp_dir.join("de_CH.aff"), aff);
        write_dic(&temp_dir.join("de_CH.dic"), &ch_entries);
        crate::set_dictionaries_folder(Some(temp_dir));

        let de_de = ProofLang::from_tag(Some("de-DE"));
        assert!(de_de.is_proofed(), "German dictionary should be available");

        let cases = [
            "Handlungs- und Zustandssatz",
            "Schul- und Universitätsbetrieb",
            "Arbeits- und Sozialrecht",
            "Forschungs- und Entwicklungsabteilung",
            "Forschungs- und Politik-Abteilung",
            "Wirtschafts- und Finanzpolitik",
            "Versicherungs- und Vertragsrecht",
            "Bildungs- und Erziehungsauftrag",
            "Bewegungs- und Wahrnehmungsverb",
            "Ordnungs- und Sicherheitsdienst",
            "Friedens- und Konfliktforschung",
            "Liebes- und Ehebeziehung",
            "Lebens- und Arbeitsbedingungen",
            "Handlungs-, Gestaltungs- und Zustandssatz",
            "Schul-/Universitätsbetrieb",
            "Wasserverbrauch und -qualität",
            "Arbeits-, Sozial-, Familien-, Steuer-, Vertrags-, Versicherungs-, Verkehrs- und Mietrecht",
            "der Schul- und der neue Universitätsbetrieb",
            "Handlungs- und „Zustandssatz“",
            "„Handlungs-“ und „Zustandssatz“",
            "„Wasserverbrauch“ und -qualität",
            "„Wasserverbrauch“ und „-qualität“",
            "Abend- und Mittagsmenü",
            "Abend- und Mittagsmenu\u{0308}",
            "Abendmenu\u{0308} und -karte",
            "Abend-Menü und -karte",
            "Geschäfts- und E-Mail-Adresse",
            "E-Mail-Adresse und -anhang",
            "Textilgroß- und -einzelhandel",
            "Textilgroß- und -einzelhandel.",
            "Textilgroß-/-einzelhandel",
            "Textilgroß-, -zwischen- und -einzelhandel",
            "Textilgroßhandel, -zwischen- und -einzelhandel",
            "Textilgroß-, -zwischen- und Textileinzelhandel",
            "Textilgroßhandel, -zwischen- und Textileinzelhandel",
            "Hin- und Rückfahrt",
            "Silber- und Hochzeitsfeier",
            "be- und entladen",
            "Be- und entladen.",
            "freund- und feindlich",
            "Handlungs- und bzw. Zustandssatz",
            "Handlungs- und (bzw.) Zustandssatz",
        ];

        for text in cases {
            let issues = crate::check_text_with(text, &[(0..text.len(), de_de)], &Options::default(), true, false);
            let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).collect();
            assert!(spelling_issues.is_empty(), "expected no spelling issues for {text:?}, got: {spelling_issues:?}");
        }

        // German regional variants such as Swiss and Liechtenstein German (de-CH, de-LI)
        // must use their own dictionary for reconstruction and clear Swiss-only forms.
        let ch_phrase = "Fussball- und Sportplatz";
        let de_ch_issues = crate::check_text_with(ch_phrase, &[(0..ch_phrase.len(), de_de)], &Options::default(), true, false);
        assert!(
            de_ch_issues.iter().any(|i| i.kind == IssueKind::Spelling && ch_phrase.get(i.start..i.end) == Some("Fussball")),
            "expected de-DE not to accept Swiss-only Fussballplatz, got: {de_ch_issues:?}"
        );
        for lang_tag in ["de-CH", "de-LI"] {
            let lang = ProofLang::from_tag(Some(lang_tag));
            let isolated = "Fussball";
            let isolated_issues = crate::check_text_with(isolated, &[(0..isolated.len(), lang)], &Options::default(), true, false);
            assert!(
                isolated_issues.iter().any(|i| i.kind == IssueKind::Spelling && isolated.get(i.start..i.end) == Some("Fussball")),
                "expected {lang_tag} to flag isolated Fussball, got: {isolated_issues:?}"
            );
            let issues = crate::check_text_with(ch_phrase, &[(0..ch_phrase.len(), lang)], &Options::default(), true, false);
            let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).collect();
            assert!(spelling_issues.is_empty(), "expected no spelling issues for {lang_tag} on {ch_phrase:?}, got: {spelling_issues:?}");
        }

        // Coordinated lists beyond the previous 64-token scan limit are supported.
        let long_list = format!("{}und Sozialrecht", "Arbeits-, ".repeat(70));
        let issues = crate::check_text_with(&long_list, &[(0..long_list.len(), de_de)], &Options::default(), true, false);
        let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).collect();
        assert!(spelling_issues.is_empty(), "expected long coordinated list beyond 64 tokens to be accepted, got: {spelling_issues:?}");

        // Backward coordinated lists beyond 64 tokens are also supported without unbounded work.
        let long_backward_list = format!("Wasserverbrauch, {}und -qualität", "-qualität, ".repeat(70));
        let issues = crate::check_text_with(&long_backward_list, &[(0..long_backward_list.len(), de_de)], &Options::default(), true, false);
        let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).collect();
        assert!(spelling_issues.is_empty(), "expected long backward coordinated list to be accepted, got: {spelling_issues:?}");

        // Sentence boundaries on modifiers must not allow matching across sentences.
        let invalid_sentence = "Handlungs- und sehr gut. Der Zustandssatz.";
        let issues = crate::check_text_with(invalid_sentence, &[(0..invalid_sentence.len(), de_de)], &Options::default(), true, false);
        assert!(
            issues.iter().any(|i| i.kind == IssueKind::Spelling && invalid_sentence.get(i.start..i.end) == Some("Handlungs")),
            "expected Handlungs- not to match across sentence boundaries, got issues: {issues:?}"
        );

        // Invalid compounds must still be flagged as a spelling issue on the invalid fragment span.
        let invalid = "Xyzqwe- und Zustandssatz";
        let issues = crate::check_text_with(invalid, &[(0..invalid.len(), de_de)], &Options::default(), true, false);
        assert!(
            issues.iter().any(|i| i.kind == IssueKind::Spelling && invalid.get(i.start..i.end) == Some("Xyzqwe")),
            "expected invalid compound Xyzqwe to be flagged, got: {issues:?}"
        );

        // A rejected fragment must not poison valid fragments later in the same coordination chain.
        let mixed = "xyzq-, Arbeits- und Sozialrecht";
        let issues = crate::check_text_with(mixed, &[(0..mixed.len(), de_de)], &Options::default(), true, false);
        let flagged: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).filter_map(|i| mixed.get(i.start..i.end)).collect();
        assert_eq!(flagged, vec!["xyzq"], "expected only xyzq to stay flagged in {mixed:?}, got: {issues:?}");

        // Line breaks and Unicode line/paragraph separators must stop both forward and backward scans.
        for newline_case in [
            ("Handlungs- und\nZustandssatz", "Handlungs"),
            ("Handlungs- und\n\nZustandssatz", "Handlungs"),
            ("Handlungs- und\u{2028}Zustandssatz", "Handlungs"),
            ("Handlungs- und\u{2029}Zustandssatz", "Handlungs"),
            ("Wasserverbrauch\nund -qualität", "qualität"),
            ("Wasserverbrauch\n\nund -qualität", "qualität"),
            ("Wasserverbrauch\u{2028}und -qualität", "qualität"),
            ("Wasserverbrauch\u{2029}und -qualität", "qualität"),
        ] {
            let (sample, expected_flagged) = newline_case;
            let issues = crate::check_text_with(sample, &[(0..sample.len(), de_de)], &Options::default(), true, false);
            assert!(
                issues.iter().any(|i| i.kind == IssueKind::Spelling && sample.get(i.start..i.end) == Some(expected_flagged)),
                "expected {expected_flagged:?} to remain flagged across newline/paragraph break in {sample:?}, got: {issues:?}"
            );
        }

        // Modifier budget from an earlier fragment must not truncate a later fragment's scan,
        // and backward scans must enforce the modifier budget from the current fragment.
        let budget_text = "xyzq- sehr gut Arbeits- sehr und Sozialrecht";
        let budget_issues = crate::check_text_with(budget_text, &[(0..budget_text.len(), de_de)], &Options::default(), true, false);
        let budget_flagged: Vec<_> =
            budget_issues.iter().filter(|i| i.kind == IssueKind::Spelling).filter_map(|i| budget_text.get(i.start..i.end)).collect();
        assert_eq!(budget_flagged, vec!["xyzq"], "expected only xyzq flagged in {budget_text:?}, got: {budget_issues:?}");

        let bwd_budget_text = "Wasserverbrauch sehr und -xyzq sehr gut -qualität";
        let bwd_budget_issues = crate::check_text_with(bwd_budget_text, &[(0..bwd_budget_text.len(), de_de)], &Options::default(), true, false);
        let bwd_budget_flagged: Vec<_> =
            bwd_budget_issues.iter().filter(|i| i.kind == IssueKind::Spelling).filter_map(|i| bwd_budget_text.get(i.start..i.end)).collect();
        assert_eq!(
            bwd_budget_flagged,
            vec!["xyzq", "qualität"],
            "expected backward modifier budget to stay enforced for -qualität in {bwd_budget_text:?}, got: {bwd_budget_issues:?}"
        );

        // Forward candidates preceding a later fragment must not be reused for it.
        let earlier_cand_text = "xyzq- und qualität wasser- und Sozialrecht";
        let earlier_cand_issues = crate::check_text_with(earlier_cand_text, &[(0..earlier_cand_text.len(), de_de)], &Options::default(), true, false);
        let earlier_cand_flagged: Vec<_> =
            earlier_cand_issues.iter().filter(|i| i.kind == IssueKind::Spelling).filter_map(|i| earlier_cand_text.get(i.start..i.end)).collect();
        assert_eq!(
            earlier_cand_flagged,
            vec!["xyzq", "qualität", "wasser"],
            "expected wasser not to match preceding qualität in {earlier_cand_text:?}, got: {earlier_cand_issues:?}"
        );

        // Cache / jump-table lookups must require connector evidence from the current fragment and
        // must not inherit an earlier fragment's connector state across short lowercase modifiers.
        let no_conn = "xyzq-, Arbeits- Sozialrecht";
        let no_conn_issues = crate::check_text_with(no_conn, &[(0..no_conn.len(), de_de)], &Options::default(), true, false);
        let no_conn_flagged: Vec<_> =
            no_conn_issues.iter().filter(|i| i.kind == IssueKind::Spelling).filter_map(|i| no_conn.get(i.start..i.end)).collect();
        assert_eq!(no_conn_flagged, vec!["xyzq", "Arbeits"], "expected Arbeits without connector to stay flagged in {no_conn:?}");

        for short_mod in ["xyzq-, Arbeits- a und Sozialrecht", "xyzq-, Arbeits- sehr a und Sozialrecht"] {
            let short_mod_issues = crate::check_text_with(short_mod, &[(0..short_mod.len(), de_de)], &Options::default(), true, false);
            let short_mod_flagged: Vec<_> =
                short_mod_issues.iter().filter(|i| i.kind == IssueKind::Spelling).filter_map(|i| short_mod.get(i.start..i.end)).collect();
            assert_eq!(short_mod_flagged, vec!["xyzq"], "expected only xyzq flagged in {short_mod:?}, got: {short_mod_issues:?}");
        }

        // Negative controls:
        // - sentence punctuation before closing quotes must stop scans,
        // - articles alone must not establish coordination,
        // - standalone dashes (en-dash, em-dash, hyphen) must not act as connectors,
        // - digits in partners must be preserved and prevent false clearing,
        // - splits must fall on real constituent boundaries (not mid-constituent character cuts),
        // - non-compound hyphen pairs (e.g. Wasser-Satz) must not bypass dictionary lookup,
        // - two-sided forms require both reconstructed compounds to be valid.
        for boundary_case in [
            ("„Handlungs- und gut.“ Zustandssatz", "Handlungs"),
            ("„Wasserverbrauch.“ Und -qualität", "qualität"),
            ("Handlungs- der Zustandssatz", "Handlungs"),
            ("Wasserverbrauch der -qualität", "qualität"),
            ("Handlungs- Arbeits- Zustandssatz", "Handlungs"),
            ("Wasserverbrauch -xyzq -qualität", "qualität"),
            ("Handlungs- – Zustandssatz", "Handlungs"),
            ("Wasserverbrauch – -qualität", "qualität"),
            ("Handlungs- — Zustandssatz", "Handlungs"),
            ("Handlungs- - Zustandssatz", "Handlungs"),
            ("Handlungs- und – Zustandssatz", "Handlungs"),
            ("Handlungs- und Zustandssatz2", "Handlungs"),
            ("Handlungs- und 2Zustandssatz", "Handlungs"),
            ("Wasserverbrauch2 und -qualität", "qualität"),
            ("Xyzq- und ISO9001Norm", "Xyzq"),
            ("WA und -xYZQ", "xYZQ"),
            ("Abendm- und Mittagsmenü", "Abendm"),
            ("Abendme- und Mittagsmenü", "Abendme"),
            ("Silberf- und Hochzeitsfeier", "Silberf"),
            ("Handlungs- und Preisersatz", "Handlungs"),
            ("Amts- und Futterneid", "Amts"),
            ("Wasserverbrauch und -erqualität", "erqualität"),
            ("Abendmenü und -dkarte", "dkarte"),
            ("Wasserverbrauch und -satz", "satz"),
            ("Wasserverbrauch, -zwischen- und -einzelbrauch", "zwischen"),
            ("arbeits- und Sozialrecht", "arbeits"),
            ("Textilgroß-, -Zwischen- und -einzelhandel", "Zwischen"),
            ("Textilgroßhandel, -Zwischen- und Textileinzelhandel", "Zwischen"),
            ("Textilgroßhandel, -xyzq- und Textileinzelhandel", "xyzq"),
            ("Textilgroß- und -xyzqhandel", "Textilgroß"),
            ("Textilgroß- und -xyzqhandel", "xyzqhandel"),
            ("Textilxyz- und -einzelhandel", "Textilxyz"),
            ("Textilxyz- und -einzelhandel", "einzelhandel"),
            ("Textilgro- und -ßhandel", "Textilgro"),
            ("Textilgro- und -ßhandel", "ßhandel"),
        ] {
            let (sample, expected_flagged) = boundary_case;
            let issues = crate::check_text_with(sample, &[(0..sample.len(), de_de)], &Options::default(), true, false);
            assert!(
                issues.iter().any(|i| i.kind == IssueKind::Spelling && sample.get(i.start..i.end) == Some(expected_flagged)),
                "expected {expected_flagged:?} to remain flagged in {sample:?}, got: {issues:?}"
            );
        }

        // All-caps headings (when `ignore_uppercase` is disabled) accept valid compounds and reject invalid ones.
        let strict_caps = Options { ignore_uppercase: false, ..Options::default() };
        for valid_caps in ["ARBEITS- UND SOZIALRECHT", "WASSERVERBRAUCH UND -QUALITÄT"] {
            let issues = crate::check_text_with(valid_caps, &[(0..valid_caps.len(), de_de)], &strict_caps, true, false);
            let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).collect();
            assert!(spelling_issues.is_empty(), "expected no spelling issues for all-caps {valid_caps:?}, got: {spelling_issues:?}");
        }
        for invalid_caps in [("XYZQWER- UND SOZIALRECHT", "XYZQWER"), ("WASSERVERBRAUCH UND -XYZQWER", "XYZQWER")] {
            let (sample, expected_flagged) = invalid_caps;
            let issues = crate::check_text_with(sample, &[(0..sample.len(), de_de)], &strict_caps, true, false);
            assert!(
                issues.iter().any(|i| i.kind == IssueKind::Spelling && sample.get(i.start..i.end) == Some(expected_flagged)),
                "expected {expected_flagged:?} to remain flagged in all-caps {sample:?}, got: {issues:?}"
            );
        }

        // Pre-connector ordinary modifiers are bounded to two lowercase words in forward scans.
        let bounded_pre = "Handlungs- sehr gut und Zustandssatz";
        let issues = crate::check_text_with(bounded_pre, &[(0..bounded_pre.len(), de_de)], &Options::default(), true, false);
        let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).collect();
        assert!(spelling_issues.is_empty(), "expected two pre-connector modifiers to stay within bound, got: {spelling_issues:?}");

        for over_bound in ["Handlungs- sehr gut sehr und Zustandssatz", "Handlungs- Handlung und Zustandssatz"] {
            let issues = crate::check_text_with(over_bound, &[(0..over_bound.len(), de_de)], &Options::default(), true, false);
            assert!(
                issues.iter().any(|i| i.kind == IssueKind::Spelling && over_bound.get(i.start..i.end) == Some("Handlungs")),
                "expected pre-connector bound to reject {over_bound:?}, got: {issues:?}"
            );
        }
    }

    /// Words ending with abbreviation letters (e.g. `Mallorca.` ending in `ca.`) must not be treated as abbreviations.
    #[test]
    fn words_ending_with_abbreviation_letters_are_not_abbreviations() {
        assert!(has_sentence_punctuation("Mallorca."));
        assert!(has_sentence_punctuation("„Mallorca.“"));
        assert!(!has_sentence_punctuation("ca."));
        assert!(!has_sentence_punctuation("bzw."));
        assert!(!has_sentence_punctuation("(bzw.)"));
        assert!(!has_sentence_punctuation("„bzw.“"));
    }

    /// Repeated rejected hyphenated tokens must take linear work in [`crate::check_spelling_in`].
    #[test]
    fn repeated_rejected_hyphenated_tokens_stay_linear() {
        crate::hold_dictionary_tests();
        let temp_dir = std::env::temp_dir().join(format!("craft-proof-test-linear-{}", std::process::id()));
        let _guard = TestDictGuard { temp_dir: temp_dir.clone() };

        let _ = std::fs::create_dir_all(&temp_dir);
        let aff = "SET UTF-8\n";
        let long_partner = "Donaudampfschiffahrtsgesellschaftskapitaenswitwenrentenversicherungsgrundgesetzabwicklungsabteilungsleiter";
        let dic = format!("2\nund\n{long_partner}\n");
        let _ = std::fs::write(temp_dir.join("de_DE.aff"), aff);
        let _ = std::fs::write(temp_dir.join("de_DE.dic"), dic);
        crate::set_dictionaries_folder(Some(temp_dir));

        let de_de = ProofLang::from_tag(Some("de-DE"));

        // Forward coordinated compounds stay linear.
        let small_f = format!("{}und {long_partner}", "Xyzq- ".repeat(50));
        let big_f = format!("{}und {long_partner}", "Xyzq- ".repeat(400));
        let (sf, issues_sf) = count_steps(|| crate::check_spelling_in(&small_f, de_de));
        assert_eq!(issues_sf.len(), 50, "expected 50 rejected forward issues for small text");
        let (bf, issues_bf) = count_steps(|| crate::check_spelling_in(&big_f, de_de));
        assert_eq!(issues_bf.len(), 400, "expected 400 rejected forward issues for big text");
        let ratio_f = bf as f64 / sf.max(1) as f64;
        assert!(sf > 0, "no steps counted, so work is not being measured");
        assert!(ratio_f <= 12.0, "repeated forward rejected hyphenated tokens took {ratio_f:.1}x steps (8x input: {sf} -> {bf}), not linear");

        // Backward coordinated compounds with preceding partner stay linear.
        let small_b = format!("{long_partner} und {}", "-Xyzq ".repeat(50));
        let big_b = format!("{long_partner} und {}", "-Xyzq ".repeat(400));
        let (sb, issues_sb) = count_steps(|| crate::check_spelling_in(&small_b, de_de));
        assert_eq!(issues_sb.len(), 50, "expected 50 rejected backward issues for small text");
        let (bb, issues_bb) = count_steps(|| crate::check_spelling_in(&big_b, de_de));
        assert_eq!(issues_bb.len(), 400, "expected 400 rejected backward issues for big text");
        let ratio_b = bb as f64 / sb.max(1) as f64;
        assert!(sb > 0, "no steps counted, so work is not being measured");
        assert!(ratio_b <= 12.0, "repeated backward rejected hyphenated tokens took {ratio_b:.1}x steps (8x input: {sb} -> {bb}), not linear");

        // Growing partner length with a fixed fragment count must not scale quadratically
        // (O(1) partner length checks during scan + O(1) over-limit rejection).
        let huge_short = format!("A{}", "a".repeat(500));
        let huge_long = format!("A{}", "a".repeat(4000));
        let fwd_short = format!("{}und {huge_short}", "Xyzq- ".repeat(50));
        let fwd_long = format!("{}und {huge_long}", "Xyzq- ".repeat(50));
        let (steps_fwd_short, issues_fwd_short) = count_steps(|| crate::check_spelling_in(&fwd_short, de_de));
        let (steps_fwd_long, issues_fwd_long) = count_steps(|| crate::check_spelling_in(&fwd_long, de_de));
        assert!(issues_fwd_short.len() >= 50 && issues_fwd_long.len() >= 50);
        assert_eq!(steps_fwd_short, steps_fwd_long, "forward scan work must not grow with partner length");

        let bwd_short = format!("{huge_short} und {}", "-Xyzq ".repeat(50));
        let bwd_long = format!("{huge_long} und {}", "-Xyzq ".repeat(50));
        let (steps_bwd_short, issues_bwd_short) = count_steps(|| crate::check_spelling_in(&bwd_short, de_de));
        let (steps_bwd_long, issues_bwd_long) = count_steps(|| crate::check_spelling_in(&bwd_long, de_de));
        assert!(issues_bwd_short.len() >= 50 && issues_bwd_long.len() >= 50);
        assert_eq!(steps_bwd_short, steps_bwd_long, "backward scan work must not grow with partner length");
    }

    /// Candidate loops in [`test_compound_forward`] and [`test_compound_backward`] must remain bounded
    /// to the 40-byte window rather than scanning the full length of a long partner.
    #[test]
    fn partner_candidate_loops_stay_bounded_on_long_partners() {
        crate::hold_dictionary_tests();
        let temp_dir = std::env::temp_dir().join(format!("craft-proof-test-bounded-{}", std::process::id()));
        let _guard = TestDictGuard { temp_dir: temp_dir.clone() };

        let _ = std::fs::create_dir_all(&temp_dir);
        let aff = "SET UTF-8\n";
        let short_partner = "Rechtsanwalt";
        let long_prefix = "Donaudampfschiffahrtsgesellschaftskapitaenswitwenrentenversicherungsgrundgesetzabwicklungsabteilung";
        let long_partner = "Donaudampfschiffahrtsgesellschaftskapitaenswitwenrentenversicherungsgrundgesetzabwicklungsabteilungsleiter";
        let dic = format!("6\nund\n{short_partner}\n{long_prefix}\n{long_partner}\nLeiter\nHandlungsleiter\n");
        let _ = std::fs::write(temp_dir.join("de_DE.aff"), aff);
        let _ = std::fs::write(temp_dir.join("de_DE.dic"), dic);
        crate::set_dictionaries_folder(Some(temp_dir));

        let lang = ProofLang::De;

        let (steps_short_b, res_short_b) = count_steps(|| test_compound_backward(short_partner, "Xyzq", lang));
        assert!(!res_short_b);
        let (steps_long_b, res_long_b) = count_steps(|| test_compound_backward(long_partner, "Xyzq", lang));
        assert!(!res_long_b);
        assert!(steps_short_b > 0, "work must be counted in test_compound_backward");
        assert!(steps_long_b <= 45, "test_compound_backward scanned beyond the 40-byte candidate window: {steps_long_b} steps");

        let (steps_short_f, res_short_f) = count_steps(|| test_compound_forward("Xyzq", short_partner, lang));
        assert!(!res_short_f);
        let (steps_long_f, res_long_f) = count_steps(|| test_compound_forward("Xyzq", long_partner, lang));
        assert!(!res_long_f);
        assert!(steps_short_f > 0, "work must be counted in test_compound_forward");
        assert!(steps_long_f <= 45, "test_compound_forward scanned beyond the 40-byte candidate window: {steps_long_f} steps");

        // Forward check tokenizes once, scans in O(1), and tests the partner once within the window.
        let accepted_text = format!("Handlungs- und {long_partner}");
        let (steps_accepted, res_accepted) = count_steps(|| {
            let para = ParagraphTokens::tokenize(&accepted_text);
            check_forward_at(&para, 0, "Handlungs", lang)
        });
        assert!(res_accepted);
        assert!(steps_accepted <= 45, "check_forward_at tested the accepted partner twice: {steps_accepted} steps");

        let rejected_text = format!("Xyzq- und {long_partner}");
        let (steps_rejected, res_rejected) = count_steps(|| {
            let para = ParagraphTokens::tokenize(&rejected_text);
            check_forward_at(&para, 0, "Xyzq", lang)
        });
        assert!(!res_rejected);
        assert!(steps_rejected <= 45, "check_forward_at tested the rejected partner twice: {steps_rejected} steps");
    }
}
