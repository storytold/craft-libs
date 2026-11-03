//! Offline Khmer segmentation, spelling and preferred spellings.
//!
//! Adapted from SBBIC Khmer's MIT-licensed Aksara at
//! 31740e9d51d38171ae6119fc2d18a4a18fbfb2f1. See LICENSE-AKSARA.
//! All positions address the original UTF-8 text; text is never normalized.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod assets;
mod lexicon;
mod refine;
mod tagger;

/// Upstream notice for apps' language-data attribution views.
pub const LICENSE: &str = include_str!("../LICENSE-AKSARA");

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::OnceLock,
};

fn parse_counts(data: &'static str) -> HashMap<&'static str, u64> {
    data.lines()
        .filter_map(|line| {
            let (word, frequency) = line.rsplit_once('\t')?;
            Some((word, frequency.parse().ok()?))
        })
        .collect()
}
fn frequencies() -> &'static HashMap<&'static str, u64> {
    static DATA: OnceLock<HashMap<&'static str, u64>> = OnceLock::new();
    DATA.get_or_init(|| parse_counts(assets::frequencies()))
}
fn spellings() -> &'static lexicon::Lexicon {
    static DATA: OnceLock<lexicon::Lexicon> = OnceLock::new();
    DATA.get_or_init(|| lexicon::Lexicon::new(assets::spellings()))
}
fn variants() -> &'static HashMap<&'static str, &'static str> {
    static DATA: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut variants = HashMap::new();
        for line in assets::variants().lines() {
            if line.starts_with('#') {
                continue;
            }
            if let Some((alternatives, standard)) = line.split_once('=') {
                for alternative in alternatives.split('|') {
                    let (alternative, standard) = (alternative.trim(), standard.trim());
                    if !alternative.is_empty() && !standard.is_empty() {
                        variants.insert(alternative, standard);
                    }
                }
            }
        }
        variants
    })
}
// Some whole repeated expressions have a different preferred spelling from their
// bare stem. Their explicitly listed standard must take precedence over stem cleanup.
fn repeated_standards() -> &'static HashSet<&'static str> {
    static DATA: OnceLock<HashSet<&'static str>> = OnceLock::new();
    DATA.get_or_init(|| variants().values().copied().filter(|word| word.ends_with('ៗ')).collect())
}
fn base(c: char) -> bool {
    ('\u{1780}'..='\u{17b3}').contains(&c)
}
fn mark(c: char) -> bool {
    ('\u{17b4}'..='\u{17d3}').contains(&c) || c == '\u{17dd}' || matches!(c, '\u{200c}' | '\u{200d}')
}
fn word_char(c: char) -> bool {
    base(c) || mark(c) || c == 'ៗ'
}
fn connector(c: char) -> bool {
    matches!(c, '-' | '/' | '.' | ':')
}
fn safe(before: char, after: char) -> bool {
    !matches!(before, '\u{2060}' | '្' | '័')
        && !matches!(after, '\u{2060}' | '្' | 'ៗ')
        && !mark(after)
        && !(connector(before) && word_char(after) || connector(after) && word_char(before))
}
/// Cluster edges, including zero and the end. COENG + base and marks stay attached.
fn edges(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    let mut previous = None;
    for (offset, ch) in text.char_indices() {
        if offset > 0 && previous.is_some_and(|before| safe(before, ch)) {
            out.push(offset);
        }
        previous = Some(ch);
    }
    if !text.is_empty() {
        out.push(text.len());
    }
    out
}
fn frequency(word: &str) -> u64 {
    frequencies().get(word).copied().unwrap_or(0)
}
fn penalty(clusters: usize, freq: u64) -> f64 {
    let (threshold, multiplier): (u64, f64) = match clusters {
        0 | 1 => (8000, 8.0),
        2 => (500, 2.0),
        _ => return 0.0,
    };
    multiplier * (threshold.saturating_sub(freq) as f64 / threshold as f64)
}
// Aksara's scores are additive. Dynamic programming retains the best predecessor
// at each safe cluster edge without copying a growing token path per beam state.
fn beam_segment(run: &str, origin: usize, out: &mut Vec<Range<usize>>) {
    let boundaries = edges(run);
    let mut states = vec![(f64::NEG_INFINITY, 0usize); boundaries.len()];
    if let Some(first) = states.first_mut() {
        first.0 = 0.0;
    }
    for (i, &start) in boundaries.iter().enumerate() {
        let Some(&(score, _)) = states.get(i) else { continue };
        if !score.is_finite() {
            continue;
        }
        let mut candidates = Vec::new();
        for (j, &end) in boundaries.iter().enumerate().skip(i + 1) {
            if end.saturating_sub(start) > 240 {
                break;
            }
            let Some(word) = run.get(start..end) else { continue };
            if word.chars().count() > 20 {
                break;
            }
            let freq = frequency(word);
            if standard_form(word).is_some() {
                candidates.push((j, 30.0));
            }
            if freq > 0 {
                candidates.push((j, (freq as f64 + 1.0).ln() - 0.25 - penalty(j - i, freq)));
            }
            if let Some(stem_frequency) = fused_compound_frequency(word) {
                candidates.push((j, (stem_frequency as f64 + 1.0).ln() + 2.0 - 0.25));
            }
        }
        // Consume OOV clusters until a strong dictionary start, at most six.
        let mut unknown_end = i.saturating_add(1).min(boundaries.len().saturating_sub(1));
        for j in i.saturating_add(1)..boundaries.len().min(i.saturating_add(7)) {
            unknown_end = j;
            let Some(&at) = boundaries.get(j) else { break };
            let strong = boundaries.iter().skip(j + 1).take(20).any(|&end| run.get(at..end).is_some_and(|word| frequency(word) >= 500));
            if strong {
                break;
            }
        }
        if unknown_end > i {
            candidates.push((unknown_end, -10.0 - if unknown_end == i + 1 { 12.0 } else { 0.0 }));
        }
        for (j, contribution) in candidates {
            if let Some(state) = states.get_mut(j)
                && score + contribution > state.0
            {
                *state = (score + contribution, i);
            }
        }
    }
    let mut cursor = boundaries.len().saturating_sub(1);
    let mut reversed = Vec::new();
    while cursor > 0 {
        let Some(&(_, previous)) = states.get(cursor) else { break };
        let (Some(&start), Some(&end)) = (boundaries.get(previous), boundaries.get(cursor)) else { break };
        if previous >= cursor {
            break;
        }
        reversed.push(origin.saturating_add(start)..origin.saturating_add(end));
        cursor = previous;
    }
    out.extend(reversed.into_iter().rev());
}
// Keep the model's training units distinct from permitted line-break positions.
// Aksara extracts base + marks/COENG clusters first, then applies break safety.
fn kcc_edges(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    let mut chars = text.char_indices().peekable();
    while let Some((_, first)) = chars.next() {
        if base(first) {
            while let Some(&(_, next)) = chars.peek() {
                if next == '្' {
                    chars.next();
                    if chars.peek().is_some_and(|(_, following)| base(*following)) {
                        chars.next();
                    }
                } else if ('\u{17b4}'..='\u{17d1}').contains(&next) || matches!(next, '\u{17d3}' | '\u{17dd}') {
                    chars.next();
                } else {
                    break;
                }
            }
        }
        out.push(chars.peek().map_or(text.len(), |(offset, _)| *offset));
    }
    out
}

fn segment(run: &str, origin: usize, out: &mut Vec<Range<usize>>) {
    // Default builds have no model. Keep the run intact without scoring empty
    // features; callers still exclude Khmer from another language's dictionary.
    if !cfg!(craft_khmer_data) {
        out.push(origin..origin.saturating_add(run.len()));
        return;
    }
    let boundaries = kcc_edges(run);
    if boundaries.len() < 3 {
        beam_segment(run, origin, out);
        return;
    }
    let clusters: Vec<&str> = boundaries.windows(2).filter_map(|pair| if let [start, end] = pair { run.get(*start..*end) } else { None }).collect();
    let mut start = 0;
    let mut tagger = tagger::Tagger::new();
    for gap in 1..clusters.len() {
        if tagger.boundary(run, &boundaries, &clusters, gap)
            && let Some(&end) = boundaries.get(gap)
            && run
                .get(..end)
                .and_then(|s| s.chars().next_back())
                .zip(run.get(end..).and_then(|s| s.chars().next()))
                .is_some_and(|(before, after)| safe(before, after))
        {
            out.push(origin.saturating_add(start)..origin.saturating_add(end));
            start = end;
        }
    }
    out.push(origin.saturating_add(start)..origin.saturating_add(run.len()));
}
/// Khmer word spans only, with original UTF-8 byte offsets. Latin and digits are excluded.
pub fn word_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (at, ch) in text.char_indices().chain(std::iter::once((text.len(), '\0'))) {
        if word_char(ch) {
            if start.is_none() {
                start = Some(at);
            }
        } else if let Some(from) = start.take()
            && let Some(run) = text.get(from..at)
        {
            segment(run, from, &mut out);
        }
    }
    refine::words(text, out)
}
/// Safe Khmer inter-word breaks and explicit ZWSP opportunities; never inserts text.
pub fn line_breaks(text: &str) -> Vec<usize> {
    let words = word_ranges(text);
    let mut out = Vec::new();
    for pair in words.windows(2) {
        if let [left, right] = pair
            && left.end == right.start
        {
            out.push(left.end);
        }
    }
    for (at, c) in text.char_indices() {
        if c == '\u{200b}' {
            out.push(at + c.len_utf8());
        }
    }
    out.retain(|&at| {
        if at == 0 || at >= text.len() {
            return false;
        }
        match (text.get(..at).and_then(|s| s.chars().next_back()), text.get(at..).and_then(|s| s.chars().next())) {
            (Some(before), Some(after)) => safe(before, after),
            _ => false,
        }
    });
    out.sort_unstable();
    out.dedup();
    out
}
/// Khmer honorific recognized by Aksara's contextual proper-name handling.
pub fn is_title(word: &str) -> bool {
    assets::titles().lines().any(|title| title == word)
}
fn fused_compound_frequency(word: &str) -> Option<u64> {
    assets::affixes()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let affix = fields.next()?;
            let kind = fields.next()?;
            if fields.next()? != "false" {
                return None;
            }
            let stem = if kind == "prefix" { word.strip_prefix(affix)? } else { word.strip_suffix(affix)? };
            let freq = frequency(stem);
            let clusters = edges(stem).len().saturating_sub(1);
            (freq >= if clusters <= 1 { 8000 } else { 500 }).then_some(freq)
        })
        .max()
}
/// Whether all embedded language assets decoded successfully.
/// A damaged binary disables corresponding rules; spelling fails open rather
/// than underlining every word. No network or disk fallback is attempted.
pub fn data_available() -> bool {
    [assets::frequencies(), assets::spellings(), assets::variants(), assets::weights(), assets::titles(), assets::affixes()]
        .iter()
        .all(|text| !text.is_empty())
}
/// Whether the complete word occurs in the bundled Aksara spelling dictionary.
pub fn is_correct(word: &str) -> bool {
    spellings().is_empty() || spellings().contains_key(word) || fused_compound_frequency(word).is_some()
}
/// Preferred spelling for a nonstandard variant (including phrase variants).
pub fn standard_form(word: &str) -> Option<&'static str> {
    variants().get(word).copied().filter(|standard| *standard != word)
}
/// Variant matches, longest first at each start; matches preserve original offsets.
pub fn grammar_matches(text: &str) -> Vec<(Range<usize>, String)> {
    let mut matches = Vec::new();
    // Check only dictionary-segment boundaries to avoid matching inside words.
    let words = word_ranges(text);
    let max_bytes = variants().keys().map(|v| v.len()).max().unwrap_or(0);
    for (index, word) in words.iter().enumerate() {
        // Exact lookups at valid ends retain phrase rules and choose the longest.
        let candidate = words
            .iter()
            .skip(index)
            .take_while(|end| end.start.saturating_sub(word.start) <= max_bytes)
            .filter_map(|end| {
                let raw = text.get(word.start..end.end)?;
                // Layout keeps LEK TOO attached. Proofing can match the preceding
                // spelling, leaving the sign outside the replacement range. Prefer
                // an explicitly listed whole expression when one exists.
                let variant = if raw.ends_with('ៗ') && raw.len() <= max_bytes && (variants().contains_key(raw) || repeated_standards().contains(raw))
                {
                    raw
                } else {
                    raw.trim_end_matches('ៗ')
                };
                if variant.len() > max_bytes {
                    return None;
                }
                let standard = variants().get(variant)?;
                Some((variant, standard))
            })
            .max_by_key(|(variant, _)| variant.len());
        if let Some((variant, standard)) = candidate
            && variant != *standard
            && !matches.last().is_some_and(|(span, _): &(Range<usize>, String)| span.end > word.start)
        {
            matches.push((word.start..word.start + variant.len(), (*standard).to_owned()));
        }
    }
    matches
}
// Bounded Unicode Levenshtein; early-out prevents hostile long-word work.
fn distance(a: &[char], b: &[char]) -> Option<usize> {
    if a.len().abs_diff(b.len()) > 2 {
        return None;
    }
    // Public suggestion inputs have at most 32 scalars; candidates differ by
    // at most two. Keep both rows on the stack and evaluate only the band.
    if b.len() > 34 || a.len() > 32 {
        return None;
    }
    let mut previous = [100usize; 35];
    for (j, cell) in previous.iter_mut().enumerate().take(b.len() + 1) {
        *cell = j;
    }
    for (i, x) in a.iter().enumerate() {
        let mut current = [100usize; 35];
        current[0] = i + 1;
        let mut minimum = current[0];
        for j in i.saturating_sub(2)..b.len().min(i + 3) {
            let value =
                (previous.get(j).copied()? + usize::from(x != b.get(j)?)).min(previous.get(j + 1).copied()? + 1).min(current.get(j).copied()? + 1);
            *current.get_mut(j + 1)? = value;
            minimum = minimum.min(value);
        }
        if minimum > 2 {
            return None;
        }
        previous = current;
    }
    previous.get(b.len()).copied().filter(|&value| value <= 2)
}
/// At most 16 suggestions, ranked by Unicode edit distance then corpus frequency.
/// Inputs beyond 32 characters are rejected to bound interactive work.
pub fn suggest(word: &str, limit: usize) -> Vec<String> {
    let limit = limit.min(16);
    if limit == 0 || word.chars().count() > 32 || word.is_empty() {
        return Vec::new();
    }
    if let Some(standard) = standard_form(word) {
        return vec![standard.to_owned()];
    }
    let input: Vec<char> = word.chars().collect();
    let mut candidates = Vec::new();
    for (candidate, freq) in spellings().iter() {
        if candidate == word || candidate.chars().count().abs_diff(input.len()) > 2 {
            continue;
        }
        let mut chars = ['\0'; 34];
        let length = candidate.chars().count();
        for (cell, ch) in chars.iter_mut().zip(candidate.chars()) {
            *cell = ch;
        }
        if let Some(candidate_chars) = chars.get(..length)
            && let Some(distance) = distance(&input, candidate_chars)
        {
            candidates.push((distance, std::cmp::Reverse(freq), candidate));
        }
    }
    candidates.sort_unstable();
    candidates.into_iter().take(limit).map(|(_, _, candidate)| candidate.to_owned()).collect()
}

#[cfg(all(test, craft_khmer_data))]
mod tests {
    use super::*;
    #[test]
    fn model_clusters_match_aksara_before_filtering_break_safety() {
        for (text, expected) in [("ព័ណ៌នា", vec!["ព័", "ណ៌", "នា"]), ("អ្វីៗ", vec!["អ្វី", "ៗ"])] {
            let boundaries = kcc_edges(text);
            let actual: Vec<&str> = boundaries.windows(2).map(|pair| text.get(pair[0]..pair[1]).unwrap()).collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn complete_parsed_data_and_float_bits_are_preserved() {
        assert!(data_available());
        assert_eq!(frequencies().len(), 29_599);
        assert_eq!(spellings().len(), 76_847);
        assert_eq!(variants().len(), 1_824);
        for (raw, spelling) in [
            (include_str!(concat!(env!("OUT_DIR"), "/frequency.tsv")), false),
            (include_str!(concat!(env!("OUT_DIR"), "/km_symspell_dictionary.txt")), true),
        ] {
            for line in raw.lines() {
                let (word, frequency) = line.rsplit_once('\t').unwrap();
                let actual = if spelling { spellings().get(word) } else { frequencies().get(word) };
                assert_eq!(actual, Some(&frequency.parse::<u64>().unwrap()));
            }
        }
        let mut expected_variants = HashMap::new();
        for line in include_str!(concat!(env!("OUT_DIR"), "/khmer-multiple-spellings.txt")).lines().filter(|line| !line.starts_with('#')) {
            if let Some((alternatives, standard)) = line.split_once('=') {
                for alternative in alternatives.split('|') {
                    if !alternative.trim().is_empty() && !standard.trim().is_empty() {
                        expected_variants.insert(alternative.trim(), standard.trim());
                    }
                }
            }
        }
        assert_eq!(*variants(), expected_variants);
        assert_eq!(assets::titles().lines().count(), 53);
        assert_eq!(assets::affixes().lines().count(), 38);
    }
    #[test]
    fn known_and_variants() {
        assert!(is_correct("ខ្មែរ"));
        assert!(!is_correct("ខ្មែររររររ"));
        assert_eq!(standard_form("សំដី"), Some("សម្ដី"));
        assert_eq!(suggest("សំដី", 3), vec!["សម្ដី"]);
        assert!(suggest("ខ្មែ", 16).contains(&"ខ្មែរ".to_owned()));
    }
    #[test]
    fn original_offsets_and_mixed_scripts() {
        let text = "Latin 🦀 ខ្ញុំស្រឡាញ់ភាសាខ្មែរ ១២៣!";
        let ranges = word_ranges(text);
        assert!(ranges.len() >= 3);
        assert_eq!(ranges.iter().filter_map(|range| text.get(range.clone())).collect::<String>(), "ខ្ញុំស្រឡាញ់ភាសាខ្មែរ");
        assert!(ranges.iter().all(|range| text.is_char_boundary(range.start) && text.is_char_boundary(range.end)));
    }
    #[test]
    fn clusters_and_joiners() {
        for text in ["ខ្ញុំស្រឡាញ់ភាសាខ្មែរ", "ព័ណ៌នា", "អ្វីៗ", "បុត្រា/ព្រះ", "ខ្មែរ\u{2060}ភាសា", "ក្\u{1781}ិ"]
        {
            for at in line_breaks(text) {
                let before = text.get(..at).and_then(|s| s.chars().next_back());
                let after = text.get(at..).and_then(|s| s.chars().next());
                assert!(matches!((before, after), (Some(a), Some(b)) if safe(a,b)));
            }
        }
        assert!(line_breaks("ខ្មែរ\u{2060}ភាសា").is_empty());
        assert_eq!(line_breaks("ខ្មែរ\u{200b}ភាសា"), vec!["ខ្មែរ\u{200b}".len()]);
    }
    #[test]
    fn long_and_malformed_inputs() {
        let text = "ក្".repeat(20_000);
        assert_eq!(word_ranges(&text), vec![0..text.len()]);
        assert!(suggest(&text, usize::MAX).is_empty());
        assert!(word_ranges("").is_empty());
        assert!(word_ranges("abc\u{200b}123").is_empty());
    }
    // Expected segments obtained by executing the pinned Aksara TS breaker
    // with its full default dictionary and feature-set v3 tagger.
    #[test]
    fn aksara_runtime_differential_fixtures() {
        let fixtures: &[(&str, &[&str])] = &[
            ("ខ្ញុំស្រឡាញ់ភាសាខ្មែរ", &["ខ្ញុំ", "ស្រឡាញ់", "ភាសា", "ខ្មែរ"]),
            ("ព្រះជាម្ចាស់", &["ព្រះ", "ជាម្ចាស់"]),
            ("ជាមួយ", &["ជា", "មួយ"]),
            ("ខិតខំ", &["ខិតខំ"]),
            ("ព័ណ៌នា", &["ព័ណ៌នា"]),
            ("អ្វីៗ", &["អ្វីៗ"]),
            ("សំដី", &["សំដី"]),
            ("កំលាំង", &["កំលាំង"]),
            ("កំណត់សំគាល់", &["កំណត់", "សំគាល់"]),
            ("អ្នកចម្រៀង", &["អ្នក", "ចម្រៀង"]),
            ("មហាសមុទ្រ", &["មហា", "សមុទ្រ"]),
            ("ខាងវិញ្ញាណ", &["ខាង", "វិញ្ញាណ"]),
            ("ក្រុមហ៊ុន", &["ក្រុមហ៊ុន"]),
            ("ខ្មែរររររ", &["ខ្មែរររររ"]),
            ("សួស្តីពិភពលោក", &["សួស្តី", "ពិភព", "លោក"]),
            ("កម្មសិទ្ធ", &["កម្មសិទ្ធ"]),
        ];
        for (text, expected) in fixtures {
            let actual: Vec<&str> = word_ranges(text).into_iter().filter_map(|span| text.get(span)).collect();
            assert_eq!(&actual, expected, "{text}");
        }
    }
    #[test]
    fn phrase_rule_offsets() {
        let text = "ខាងវិញ្ញាណ";
        assert!(grammar_matches(text).iter().any(|(range, replacement)| text.get(range.clone()) == Some(text) && replacement == "ខាងឯវិញ្ញាណ"));
    }
}

#[cfg(test)]
fn baseline_distance(a: &[char], b: &[char]) -> Option<usize> {
    if a.len().abs_diff(b.len()) > 2 {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        let mut minimum = i + 1;
        for (j, y) in b.iter().enumerate() {
            let value = previous.get(j).copied().unwrap_or(100) + usize::from(x != y);
            let value = value.min(previous.get(j + 1).copied().unwrap_or(100) + 1).min(current.get(j).copied().unwrap_or(100) + 1);
            if let Some(cell) = current.get_mut(j + 1) {
                *cell = value;
            }
            minimum = minimum.min(value);
        }
        if minimum > 2 {
            return None;
        }
        previous = current;
    }
    previous.last().copied().filter(|&value| value <= 2)
}

#[cfg(test)]
#[test]
fn distance_differential_mutations() {
    let mut words: Vec<_> = spellings().keys().collect();
    words.sort_unstable();
    for word in words.into_iter().step_by(97) {
        let original: Vec<char> = word.chars().take(32).collect();
        for at in 0..=original.len() {
            let mut changed = original.clone();
            changed.insert(at, 'ក');
            if changed.len() <= 32 {
                assert_eq!(distance(&original, &changed), baseline_distance(&original, &changed));
            }
            if at < original.len() {
                let mut changed = original.clone();
                changed.remove(at);
                assert_eq!(distance(&original, &changed), baseline_distance(&original, &changed));
                changed.extend(['x', 'y', 'z']);
                if changed.len() <= 34 {
                    assert_eq!(distance(&original, &changed), baseline_distance(&original, &changed));
                }
            }
        }
    }
}

#[cfg(test)]
fn baseline_grammar_matches(text: &str) -> Vec<(Range<usize>, String)> {
    let mut matches = Vec::new();
    // Check only dictionary-segment boundaries to avoid matching inside words.
    let words = word_ranges(text);
    let ends: std::collections::HashSet<usize> = words.iter().map(|word| word.end).collect();
    for word in &words {
        let Some(tail) = text.get(word.start..) else { continue };
        let candidate = variants()
            .iter()
            .filter(|(variant, _)| tail.starts_with(**variant))
            .filter(|(variant, _)| ends.contains(&word.start.saturating_add(variant.len())))
            .max_by_key(|(variant, _)| variant.len());
        if let Some((variant, standard)) = candidate
            && variant != standard
            && !matches.last().is_some_and(|(span, _): &(Range<usize>, String)| span.end > word.start)
        {
            matches.push((word.start..word.start + variant.len(), (*standard).to_owned()));
        }
    }
    matches
}

#[cfg(test)]
#[test]
fn every_variant_grammar_differential() {
    for variant in variants().keys() {
        for text in [variant.to_string(), format!("Latin 🦀 {variant}!"), format!("ក{variant}ខ"), format!("{variant} {variant}")] {
            assert_eq!(grammar_matches(&text), baseline_grammar_matches(&text), "{text}");
        }
    }
}

#[cfg(all(test, not(craft_khmer_data)))]
#[test]
fn absent_pack_preserves_runs_and_manual_opportunities_without_model_work() {
    assert!(!data_available());
    let text = "ខ្ញុំស្រឡាញ់ភាសាខ្មែរ";
    assert_eq!(word_ranges(text), vec![0..text.len()]);
    assert!(line_breaks(text).is_empty());
    assert!(is_correct(text));
    assert!(suggest(text, 16).is_empty());
    let manual = "មូល\u{200b}ដ្ឋាន";
    assert_eq!(line_breaks(manual), vec!["មូល\u{200b}".len()]);
    assert!(line_breaks("មូល\u{2060}ដ្ឋាន").is_empty());
}
