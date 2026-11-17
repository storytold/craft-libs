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
/// consecutive protected or translatable tokens. `<markup>` spans are
/// carved out first — a tag with attributes contains whitespace, and a
/// whole `<em>Hello</em>` run must keep "Hello" translatable while the
/// tags themselves stay byte-identical.
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

    for (token, is_tag) in split_tokens(text) {
        let ws = token.chars().all(char::is_whitespace);
        // Whitespace glues to whatever the current run is; markup tags
        // are always protected.
        let this_protected = if ws { buf_protected } else { is_tag || grammar.is_protected(token) };
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

/// Tokenize into `(slice, is_tag)`: markup tags are whole `<…>` spans
/// (attributes included), everything else alternates between maximal
/// whitespace and non-whitespace runs.
fn split_tokens(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let bytes = text.as_bytes();
    while pos < bytes.len() {
        if bytes[pos] == b'<'
            && let Some(tag_len) = tag_len(&text[pos..])
        {
            out.push((&text[pos..pos + tag_len], true));
            pos += tag_len;
            continue;
        }
        // Advance a non-tag run up to the next '<' that starts a tag;
        // bare '<' characters just stay inside the run.
        let mut scan = pos;
        let mut run_end = bytes.len();
        while let Some(lt) = bytes[scan..].iter().position(|&b| b == b'<') {
            let at = scan + lt;
            if tag_len(&text[at..]).is_some() {
                run_end = at;
                break;
            }
            scan = at + 1;
        }
        for piece in split_words_ws(&text[pos..run_end]) {
            out.push((piece, false));
        }
        pos = run_end;
    }
    out
}

/// Length of a `<…>` tag starting at `s[0]`, or `None` when `<` does not
/// begin markup (bare `<`, `<3`, `< hello`). Tags start `</`, `<?`, `<!`
/// or `<`+letter and close at the first `>` before any `<` or newline;
/// the scan is capped so pathological input can't stall it.
fn tag_len(s: &str) -> Option<usize> {
    const MAX_TAG: usize = 512;
    let second = s.as_bytes().get(1)?;
    let opener_ok = second.is_ascii_alphabetic() || matches!(second, b'/' | b'!' | b'?');
    if !opener_ok {
        return None;
    }
    s.char_indices().skip(2).take(MAX_TAG).find(|(_, c)| matches!(c, '>' | '<' | '\n')).and_then(|(i, c)| (c == '>').then_some(i + 1))
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
/// …and how many bytes one prompt carries. A document with no protected
/// tokens is one `Text` segment of arbitrary length — without a size
/// bound the request can exceed `max_tokens` and come back truncated
/// (which providers must reject, not return as success).
const BATCH_BYTES: usize = 4 * 1024;
/// A single `Text` run larger than this is chopped into pieces before
/// batching, so no one request is unbounded.
const RUN_BYTES: usize = 3 * 1024;

/// A translatable piece of one segment: the whitespace-stripped `core`
/// is sent to the model, `lead`/`trail` are restored verbatim around the
/// translation so `Visitez` never glues to `https://a.io`.
struct Run<'a> {
    seg: usize,
    lead: &'a str,
    core: &'a str,
    trail: &'a str,
    /// False for whitespace-only pieces: never sent to the model, but
    /// still reassembled into the segment.
    send: bool,
}

/// `(leading ws, core, trailing ws)` of `s`, all on char boundaries.
fn ws_edges(s: &str) -> (&str, &str, &str) {
    let lead = s.len() - s.trim_start().len();
    let tail = s.len() - s.trim_end().len();
    if lead + tail >= s.len() {
        return (s, "", ""); // all-whitespace run: nothing to translate
    }
    (&s[..lead], &s[lead..s.len() - tail], &s[s.len() - tail..])
}

/// Chop `s` into ≤ `max`-byte pieces on whitespace boundaries (the last
/// ws before the limit; a run with no ws is hard-cut at the limit). The
/// boundary whitespace stays inside the pieces — nothing is lost.
fn split_run(s: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while rest.len() > max {
        let mut ws_cut = None;
        let mut boundary = 0usize;
        for (i, c) in rest.char_indices() {
            if i >= max {
                break;
            }
            boundary = i + c.len_utf8();
            if c.is_whitespace() {
                ws_cut = Some(i); // cut *before* the ws — it leads the next piece
            }
        }
        let cut = match ws_cut.filter(|w| *w > max / 4) {
            Some(w) => w,
            None => boundary.max(1),
        };
        out.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

/// Translate every `Text` segment to `target_lang`, leaving `Protected`
/// untouched. Batched with numbered items; a count mismatch retries the
/// offending half rather than misaligning the whole document.
pub fn translate_segments(provider: &mut dyn Provider, segs: &[Segment], target_lang: &str) -> Result<Vec<String>, AiError> {
    // Source whitespace never reaches the model: each Text segment is
    // split edge-to-edge, and oversized runs are chopped further.
    let mut runs: Vec<Run<'_>> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        let Segment::Text(src) = s else { continue };
        for piece in split_run(src, RUN_BYTES) {
            let (lead, core, trail) = ws_edges(piece);
            // Pure-whitespace pieces (possible after a long run is chopped)
            // keep their bytes in the output but skip the provider.
            runs.push(Run { seg: i, lead, core, trail, send: !core.is_empty() });
        }
    }
    let run_segs: std::collections::HashSet<usize> = runs.iter().map(|r| r.seg).collect();

    let mut out: Vec<String> = segs.iter().map(|s| s.source().to_string()).collect();
    for &i in &run_segs {
        out[i].clear(); // rebuilt from translated runs below
    }
    let mut i = 0;
    while i < runs.len() {
        // Whitespace-only pieces restore verbatim, in order, and never
        // reach the model.
        while i < runs.len() && !runs[i].send {
            let r = &runs[i];
            out[r.seg].push_str(r.lead);
            out[r.seg].push_str(r.trail);
            i += 1;
        }
        // A batch is bounded by item count *and* total prompt bytes.
        let mut bytes = 0usize;
        let start = i;
        while i < runs.len() && runs[i].send && i - start < BATCH && bytes + runs[i].core.len() <= BATCH_BYTES {
            bytes += runs[i].core.len();
            i += 1;
        }
        if i == start {
            if i < runs.len() {
                i += 1; // never spin on an oversized single item
            }
            continue;
        }
        let chunk = &runs[start..i];
        let translated = translate_batch(provider, chunk, target_lang)?;
        for (run, t) in chunk.iter().zip(translated) {
            out[run.seg].push_str(run.lead);
            out[run.seg].push_str(&t);
            out[run.seg].push_str(run.trail);
        }
    }
    Ok(out)
}

/// One numbered batch; splits recursively on count mismatch so a single
/// malformed reply can't corrupt neighbors.
fn translate_batch(provider: &mut dyn Provider, runs: &[Run<'_>], lang: &str) -> Result<Vec<String>, AiError> {
    if runs.len() == 1 {
        return Ok(vec![translate_one(provider, runs[0].core, lang)?]);
    }
    let prompt = numbered_prompt(&runs.iter().map(|r| r.core.to_string()).collect::<Vec<_>>(), lang);
    let reply = provider.complete(&prompt, &GenOpts::default())?;
    match parse_numbered(&reply, runs.len()) {
        Some(v) => Ok(v),
        None => {
            let mid = runs.len() / 2;
            let mut a = translate_batch(provider, &runs[..mid], lang)?;
            let b = translate_batch(provider, &runs[mid..], lang)?;
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

/// Parse `"1. …\n2. …"` back into items; `None` when the reply is
/// malformed — duplicates, out-of-range numbers, or a missing item all
/// mean the numbering slipped, and the caller retries/splits rather
/// than accepting corrupted text. Non-numbered lines after an item are
/// continuations of it; non-numbered chatter before item 1 is rejected
/// the same way, so nothing is silently dropped.
fn parse_numbered(reply: &str, want: usize) -> Option<Vec<String>> {
    let mut out = vec![String::new(); want];
    let mut seen = vec![false; want];
    let mut current: Option<usize> = None;
    for line in reply.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match numbered_line(line) {
            Some(idx) if idx >= 1 && idx <= want => {
                if seen[idx - 1] {
                    return None; // "1. un\n2. deux\n1. trois" overwrites — reject
                }
                seen[idx - 1] = true;
                let rest = line.split_once(['.', ')']).map_or("", |(_, r)| r).trim().to_string();
                out[idx - 1] = rest;
                current = Some(idx - 1);
            }
            Some(_) => return None, // numbered but out of range — malformed reply
            // A wrapped/multi-line item: keep the continuation, not
            // "1. Première phrase.\nSuite" dropping "Suite". Unnumbered
            // preamble is rejected — not our format.
            None => {
                let i = current?;
                out[i].push('\n');
                out[i].push_str(line);
            }
        }
    }
    (seen.iter().all(|s| *s) && out.iter().all(|s| !s.is_empty())).then_some(out)
}

/// `Some(idx)` when `line` opens as `"12."` / `"12)"` — a 1-3 digit
/// number followed by `'.'` or `')'`. Anything else is `None`, so
/// "Section 2. notes" is a continuation, not item 2.
fn numbered_line(line: &str) -> Option<usize> {
    let line = line.trim_start();
    let digits = line.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 3 || !matches!(line.as_bytes().get(digits), Some(b'.' | b')')) {
        return None;
    }
    line.get(..digits)?.parse().ok()
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

    #[test]
    fn numbered_parse_rejects_duplicates_and_overflow() {
        assert_eq!(parse_numbered("1. un\n2. deux\n1. trois", 2), None);
        assert_eq!(parse_numbered("1. un\n2. deux\n3. trois", 2), None);
        assert_eq!(parse_numbered("0. zero\n1. un\n2. deux", 2), None);
    }

    #[test]
    fn numbered_parse_keeps_continuation_lines() {
        let r = "1. Première phrase.\nSuite du paragraphe\n2. Deuxième élément.";
        assert_eq!(parse_numbered(r, 2), Some(vec!["Première phrase.\nSuite du paragraphe".into(), "Deuxième élément.".into()]));
        // Preamble chatter that isn't a numbered item is rejected, not
        // silently dropped.
        assert_eq!(parse_numbered("Sure, here you go:\n1. un\n2. deux", 2), None);
        // "Section 2. notes" looks numbered but isn't → continuation.
        let r2 = "1. un\nSection 2. notes\n2. deux";
        assert_eq!(parse_numbered(r2, 2), Some(vec!["un\nSection 2. notes".into(), "deux".into()]));
    }

    #[test]
    fn markup_tags_split_from_their_text() {
        let segs = segment("<em>Hello</em> world", &PdfGrammar);
        assert!(segs.iter().any(|s| matches!(s, Segment::Protected(p) if p == "<em>")));
        assert!(segs.iter().any(|s| matches!(s, Segment::Protected(p) if p.trim() == "</em>")));
        assert!(segs.iter().any(|s| matches!(s, Segment::Text(t) if t.contains("Hello"))));
        assert!(segs.iter().any(|s| matches!(s, Segment::Text(t) if t.contains("world"))));
        // Tags containing spaces stay whole and protected.
        let segs = segment("<a href=\"x\">link</a>", &PdfGrammar);
        assert!(segs.iter().any(|s| matches!(s, Segment::Protected(p) if p == "<a href=\"x\">")));
        // A bare '<' is text, not a tag opener — words around it stay
        // translatable (the lone '<' itself is protected as punctuation).
        let segs = segment("a < b and <3 hearts", &PdfGrammar);
        assert!(segs.iter().any(|s| matches!(s, Segment::Text(t) if t.contains("hearts"))));
        assert!(!segs.iter().any(|s| matches!(s, Segment::Protected(p) if p.contains('<') && p.contains('>'))));
    }

    #[test]
    fn ws_edges_and_split_run_preserve_every_byte() {
        assert_eq!(ws_edges("  héllo \n"), ("  ", "héllo", " \n"));
        assert_eq!(ws_edges("   "), ("   ", "", ""));
        let s = "alpha beta gamma delta epsilon";
        let pieces = split_run(s, 12);
        assert!(pieces.iter().all(|p| p.len() <= 12));
        assert_eq!(pieces.concat(), s); // boundary whitespace survives
        // A run with no whitespace is hard-cut at a char boundary.
        let long = "x".repeat(50);
        let pieces = split_run(&long, 20);
        assert!(pieces.iter().all(|p| p.len() <= 20));
        assert_eq!(pieces.concat(), long);
    }

    /// A provider that echoes numbered replies — exercises the batching,
    /// whitespace restore and reassembly end to end without a network.
    struct Echo;

    impl Provider for Echo {
        fn name(&self) -> String {
            "echo".into()
        }

        fn complete(&mut self, prompt: &str, _opts: &GenOpts) -> Result<String, AiError> {
            if prompt.starts_with("Translate each numbered item") {
                // Parse our own prompt's numbering and echo each item.
                let body = prompt.split("\n\n").nth(1).unwrap_or("");
                let mut out = String::new();
                for line in body.lines() {
                    out.push_str(line);
                    out.push('\n');
                }
                return Ok(out);
            }
            Ok(prompt.rsplit("\n\n").next().unwrap_or(prompt).to_string())
        }

        fn test(&mut self) -> Result<crate::provider::ProviderInfo, AiError> {
            Ok(crate::provider::ProviderInfo { label: "echo".into(), models: vec![] })
        }

        fn is_local(&self) -> bool {
            true
        }
    }

    #[test]
    fn numbered_parse_accepts_paren_markers() {
        // "1) …" replies contain sentence-ending periods inside items —
        // the leading delimiter must be picked before inspecting text.
        let r = "1) Première phrase.\nSuite.\n2) Deuxième élément.";
        assert_eq!(parse_numbered(r, 2), Some(vec!["Première phrase.\nSuite.".into(), "Deuxième élément.".into()]));
    }

    #[test]
    fn whitespace_only_pieces_survive_reconstruction() {
        let segs = vec![Segment::Text(format!("{}Hello", " ".repeat(RUN_BYTES)))];
        let mut echo = Echo;
        let out = translate_segments(&mut echo, &segs, "fr").unwrap_or_default();
        assert_eq!(out.concat(), format!("{}Hello", " ".repeat(RUN_BYTES)));
    }

    #[test]
    fn translate_restores_whitespace_around_protected_spans() {
        let segs = segment("Visit https://a.io now", &PdfGrammar);
        let mut echo = Echo;
        let out = translate_segments(&mut echo, &segs, "fr").unwrap_or_default();
        assert_eq!(out.concat(), "Visit https://a.io now");
    }
}
