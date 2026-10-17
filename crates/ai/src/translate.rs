//! Document translation as *segments*, never raw text: the model sees
//! only translatable runs, and protected spans (URLs, placeholders,
//! markup, numbers-only runs) come back byte-identical because we never
//! sent them.
//!
//! Each app declares its grammar via [`TranslateGrammar`]; the default
//! [`PdfGrammar`] suits extracted PDF text.

use crate::error::AiError;
use crate::provider::{GenOpts, Provider};

/// A unit of text the translator decided on.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    /// Sent to the model; replace with the translation.
    Text(String),
    /// Never sent; re-inserted verbatim.
    Protected(String),
}

impl Segment {
    /// The source text, translated or not.
    pub fn source(&self) -> &str {
        match self {
            Self::Text(s) | Self::Protected(s) => s,
        }
    }
}

/// Which runs of a document's text must never be translated.
pub trait TranslateGrammar {
    /// True when `token` (already delimited — a word, symbol, or
    /// whitespace-free run) is structural and must survive verbatim.
    fn is_protected(&self, token: &str) -> bool;
}

/// Grammar for text extracted from a PDF: protects URLs, e-mail
/// addresses, `{placeholders}`, `<markup>`, and runs with no letters
/// (page furniture, digits, punctuation).
pub struct PdfGrammar;

impl TranslateGrammar for PdfGrammar {
    fn is_protected(&self, token: &str) -> bool {
        is_protected_token(token)
    }
}

/// Shared protected-token test, useful to other apps' grammars.
pub fn is_protected_token(token: &str) -> bool {
    let t = token.trim();
    if t.is_empty() {
        return true;
    }
    if !t.chars().any(|c| c.is_alphabetic()) {
        return true; // numbers, punctuation, symbols
    }
    let lower = t.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("www.")
        || lower.contains('@') && lower.contains('.') && !lower.contains(' ')
        || (t.starts_with('{') && t.ends_with('}'))
        || (t.starts_with('<') && t.ends_with('>'))
}

/// Split `text` into segments on whitespace boundaries, grouping
/// consecutive protected or translatable tokens.
pub fn segment(text: &str, grammar: &dyn TranslateGrammar) -> Vec<Segment> {
    fn flush(out: &mut Vec<Segment>, buf: &mut String, protected: bool) {
        if buf.is_empty() {
            return;
        }
        let seg = if protected { Segment::Protected(std::mem::take(buf)) } else { Segment::Text(std::mem::take(buf)) };
        out.push(seg);
    }

    let mut out: Vec<Segment> = Vec::new();
    let mut buf = String::new();
    let mut buf_protected = false;
    let mut started = false;

    for token in split_words_ws(text) {
        let ws = token.chars().all(char::is_whitespace);
        // Whitespace glues to whatever the current run is.
        let this_protected = if ws { buf_protected } else { grammar.is_protected(token) };
        if started && this_protected != buf_protected {
            flush(&mut out, &mut buf, buf_protected);
        }
        buf_protected = this_protected;
        started = true;
        buf.push_str(token);
    }
    flush(&mut out, &mut buf, buf_protected);
    out
}

/// Split into alternating word/whitespace runs (like `split_inclusive`
/// but grouping every maximal run of whitespace or non-whitespace).
fn split_words_ws(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut prev_ws: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let ws = c.is_whitespace();
        if let Some(p) = prev_ws
            && p != ws
        {
            out.push(&text[start..i]);
            start = i;
        }
        prev_ws = Some(ws);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// How many segments one prompt carries — keeps responses snappy and
/// makes a numbering slip cheap to retry.
const BATCH: usize = 20;

/// Translate every `Text` segment to `target_lang`, leaving `Protected`
/// untouched. Batched with numbered items; a count mismatch retries the
/// offending half rather than misaligning the whole document.
pub fn translate_segments(provider: &mut dyn Provider, segs: &[Segment], target_lang: &str) -> Result<Vec<String>, AiError> {
    // Collect (index → source) for translatable segments only.
    let items: Vec<(usize, &str)> = segs
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            Segment::Text(t) => Some((i, t.as_str())),
            Segment::Protected(_) => None,
        })
        .collect();

    let mut out: Vec<String> = segs.iter().map(|s| s.source().to_string()).collect();
    for chunk in items.chunks(BATCH) {
        let sources: Vec<&str> = chunk.iter().map(|(_, s)| *s).collect();
        let translated = translate_batch(provider, &sources, target_lang)?;
        for ((i, _), t) in chunk.iter().zip(translated) {
            out[*i] = t;
        }
    }
    Ok(out)
}

/// One numbered batch; splits recursively on count mismatch so a single
/// malformed reply can't corrupt neighbors.
fn translate_batch(provider: &mut dyn Provider, sources: &[&str], lang: &str) -> Result<Vec<String>, AiError> {
    if sources.len() == 1 {
        return Ok(vec![translate_one(provider, sources[0], lang)?]);
    }
    let prompt = numbered_prompt(&sources.iter().map(|s| s.to_string()).collect::<Vec<_>>(), lang);
    let reply = provider.complete(&prompt, &GenOpts::default())?;
    match parse_numbered(&reply, sources.len()) {
        Some(v) => Ok(v),
        None => {
            let mid = sources.len() / 2;
            let mut a = translate_batch(provider, &sources[..mid], lang)?;
            let b = translate_batch(provider, &sources[mid..], lang)?;
            a.extend(b);
            Ok(a)
        }
    }
}

fn translate_one(provider: &mut dyn Provider, source: &str, lang: &str) -> Result<String, AiError> {
    let prompt = format!("Translate the following text to {lang}. Reply with ONLY the translation, no quotes, no notes.\n\n{source}");
    Ok(provider.complete(&prompt, &GenOpts::default())?.trim().to_string())
}

/// `"1. …"`-style numbered prompt — models follow it far more reliably
/// than delimiter schemes.
fn numbered_prompt(sources: &[String], lang: &str) -> String {
    let mut p = format!(
        "Translate each numbered item to {lang}. Keep the numbering. \
         Preserve all formatting, placeholders like {{name}}, URLs and markup verbatim. \
         Reply with ONLY the numbered translations.\n\n"
    );
    for (i, s) in sources.iter().enumerate() {
        p.push_str(&format!("{}. {}\n", i + 1, s.replace('\n', " ")));
    }
    p
}

/// Parse `"1. …\n2. …"` back into items; `None` when the count or
/// numbering is wrong (caller retries).
fn parse_numbered(reply: &str, want: usize) -> Option<Vec<String>> {
    let mut out = vec![String::new(); want];
    let mut seen = 0usize;
    for line in reply.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((num, rest)) = line.split_once('.').or_else(|| line.split_once(')')) else {
            continue; // chatter without a number — not ours
        };
        let Ok(idx) = num.trim().trim_matches(|c: char| !c.is_ascii_digit()).parse::<usize>() else {
            continue;
        };
        if idx == 0 || idx > want {
            continue; // stray numbered line — ignore
        }
        out[idx - 1] = rest.trim().to_string();
        seen += 1;
    }
    (seen >= want && out.iter().all(|s| !s.is_empty())).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_words_ws_groups_runs() {
        assert_eq!(split_words_ws("a  b\tc"), vec!["a", "  ", "b", "\t", "c"]);
    }

    #[test]
    fn pdf_grammar_protects() {
        let g = PdfGrammar;
        assert!(g.is_protected("https://x.io/a"));
        assert!(g.is_protected("{count}"));
        assert!(g.is_protected("42.5"));
        assert!(!g.is_protected("Bonjour"));
    }

    #[test]
    fn segmentation_glues_whitespace_and_protects_spans() {
        let segs = segment("Visit https://a.io now {n} times", &PdfGrammar);
        // "Visit " text, protected url, " now " text, protected {n}, " times" text
        let texts: Vec<&Segment> = segs.iter().filter(|s| matches!(s, Segment::Text(_))).collect();
        assert!(segs.iter().any(|s| matches!(s, Segment::Protected(p) if p.trim() == "https://a.io")));
        assert!(segs.iter().any(|s| matches!(s, Segment::Protected(p) if p.trim() == "{n}")));
        assert!(texts.iter().any(|s| s.source().contains("Visit")));
    }

    #[test]
    fn numbered_parse_roundtrip() {
        let r = "1. un\n2. deux\n3. trois";
        assert_eq!(parse_numbered(r, 3), Some(vec!["un".into(), "deux".into(), "trois".into()]));
        assert_eq!(parse_numbered("1. un\n3. trois", 3), None);
    }
}
