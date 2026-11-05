//! Insertions are returned separately from source offsets, never written to the text.
use super::{Limits, MAX_HYPHENATION_RUN_BYTES, Opportunity, ProofLang, composed_chars, is_apostrophe, points_with_caps, word_runs};

/// Allowed source-character breaks plus display-only letters before the hyphen.
/// Point-only clients keep using `hyphen_points_in`, which omits insertions it cannot render.
/// Installed nonstandard patterns preserve source normalization, casing, punctuation and limits.
pub fn hyphen_opportunities_in(word: &str, lim: &Limits, lang: ProofLang) -> Vec<Opportunity> {
    opportunities_in(word, lim, lang, false)
}

/// Nonstandard layout breaks may also hyphenate all-capital words. Capitalized
/// words still require `lim.capitalized`; the opt-in also applies to point-only fallbacks.
pub fn hyphen_opportunities_including_all_caps(word: &str, lim: &Limits, lang: ProofLang) -> Vec<Opportunity> {
    opportunities_in(word, lim, lang, true)
}

fn opportunities_in(word: &str, lim: &Limits, lang: ProofLang, all_caps: bool) -> Vec<Opportunity> {
    let Some(patterns) = crate::packs::hyphenation(lang).filter(|p| p.has_nonstandard()) else {
        return points_with_caps(word, lim, lang, all_caps).into_iter().map(|at| Opportunity { at, suffix: "".into() }).collect();
    };
    opportunities_with(word, lim, lang, &patterns, all_caps)
}

fn opportunities_with(word: &str, lim: &Limits, lang: ProofLang, patterns: &super::Patterns, all_caps: bool) -> Vec<Opportunity> {
    let mut out = Vec::new();
    for (offset, text) in word_runs(word) {
        if text.len() > MAX_HYPHENATION_RUN_BYTES {
            continue;
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut a, mut b) = (0, chars.len());
        while a < b && is_apostrophe(chars[a]) {
            a += 1;
        }
        while b > a && is_apostrophe(chars[b - 1]) {
            b -= 1;
        }
        let run = &chars[a..b];
        let letters = run.iter().filter(|c| c.is_alphabetic()).count();
        if letters >= lim.min_word.max(2)
            && (all_caps || letters < 2 || !run.iter().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()))
            && (lim.capitalized || run.first().is_none_or(|c| !c.is_uppercase()))
        {
            // Bound replacement processing, while keeping the legacy safe point
            // fallback available for longer runs (including source offsets).
            if run.iter().map(|c| c.len_utf8()).sum::<usize>() > 1024 {
                let text: String = run.iter().collect();
                out.extend(points_with_caps(&text, lim, lang, all_caps).into_iter().map(|at| Opportunity { at: offset + a + at, suffix: "".into() }));
                continue;
            }
            let (nfc, starts) = composed_chars(run);
            let lower: Vec<char> = nfc.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
            if let Some(points) = super::pack_patterns_fit(lang, patterns, &lower).then(|| patterns.opportunities(&lower)).flatten() {
                for mut point in points {
                    let Some(at) = starts.get(point.at).copied().flatten() else { continue };
                    let (Some(left), Some(right)) = (run.get(..at), run.get(at..)) else { continue };
                    if left.iter().filter(|c| c.is_alphabetic()).count() < lim.after_first.max(1)
                        || right.iter().filter(|c| c.is_alphabetic()).count() < lim.before_last.max(1)
                    {
                        continue;
                    }
                    if left.iter().rev().find(|c| c.is_alphabetic()).is_some_and(|c| c.is_uppercase()) {
                        point.suffix = point.suffix.to_uppercase().into_boxed_str();
                    }
                    point.at = offset + a + at;
                    out.push(point);
                }
            } else {
                let text: String = run.iter().collect();
                out.extend(points_with_caps(&text, lim, lang, all_caps).into_iter().map(|at| Opportunity { at: offset + a + at, suffix: "".into() }));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::Patterns;
    use super::super::hyphen_points_in;
    use super::*;
    #[test]
    fn huge_runs_are_skipped_without_losing_neighbours() {
        let patterns = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let giant = "á".repeat(MAX_HYPHENATION_RUN_BYTES * 4);
        let text = format!("asszony {giant} asszony");
        let lang = ProofLang::from_tag(Some("hu"));
        let actual = opportunities_with(&text, &Limits::default(), lang, &patterns, false);
        assert_eq!(actual.iter().map(|p| p.at).collect::<Vec<_>>(), vec![2, 8 + giant.chars().count() + 1 + 2]);
        assert!(hyphen_points_in(&giant, &Limits::default(), lang).is_empty());
        let normal = "asszony ".repeat(1000);
        assert_eq!(opportunities_with(&normal, &Limits::default(), lang, &patterns, false).len(), 1000);
    }
    #[test]
    fn point_only_fallback_honours_explicit_all_caps_and_limits() {
        let lim = Limits::default();
        let expected = hyphen_points_in("typography", &lim, ProofLang::En);
        assert!(!expected.is_empty());
        assert!(hyphen_opportunities_in("TYPOGRAPHY", &lim, ProofLang::En).is_empty());
        assert_eq!(hyphen_opportunities_including_all_caps("TYPOGRAPHY", &lim, ProofLang::En).iter().map(|p| p.at).collect::<Vec<_>>(), expected);
        assert!(hyphen_opportunities_including_all_caps("TYPOGRAPHY", &Limits { capitalized: false, ..lim }, ProofLang::En).is_empty());
    }
    #[test]
    fn over_limit_runs_keep_point_fallback_with_source_offsets() {
        let patterns = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let word = "kitaplar".repeat(150);
        let lim = Limits::default();
        let expected = hyphen_points_in(&word, &lim, ProofLang::Tr);
        assert!(!expected.is_empty());
        let text = format!("..{word}");
        let actual = opportunities_with(&text, &lim, ProofLang::Tr, &patterns, false);
        assert_eq!(actual.iter().map(|p| p.at).collect::<Vec<_>>(), expected.iter().map(|at| at + 2).collect::<Vec<_>>());
        assert!(actual.iter().all(|p| p.suffix.is_empty()));
        assert!(opportunities_with(&word.to_uppercase(), &lim, ProofLang::Tr, &patterns, false).is_empty());
    }
    #[test]
    fn all_caps_require_the_explicit_layout_opt_in() {
        let patterns = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let lim = Limits::default();
        let lang = ProofLang::from_tag(Some("hu"));
        assert!(opportunities_with("ASSZONY", &lim, lang, &patterns, false).is_empty());
        assert_eq!(opportunities_with("ASSZONY", &lim, lang, &patterns, true), vec![Opportunity { at: 2, suffix: "Z".into() }]);
        assert!(opportunities_with("ASSZONY", &Limits { capitalized: false, ..lim }, lang, &patterns, true).is_empty());
        assert!(!opportunities_with("Asszony", &lim, lang, &patterns, false).is_empty());
    }
    #[test]
    fn unknown_pack_letters_retain_builtin_language_breaks() {
        let patterns = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let lim = Limits { min_word: 2, after_first: 1, before_last: 1, capitalized: true };
        for (word, lang) in [("молоко", ProofLang::Ru), ("автомобил", ProofLang::Bg), ("kitaplar", ProofLang::Tr), ("Ankara'nın", ProofLang::Tr)]
        {
            let expected = hyphen_points_in(word, &lim, lang);
            assert!(!expected.is_empty(), "positive fallback control: {word}");
            let actual = opportunities_with(word, &lim, lang, &patterns, false);
            assert_eq!(actual.iter().map(|p| p.at).collect::<Vec<_>>(), expected, "{word}");
            assert!(actual.iter().all(|p| p.suffix.is_empty()));
        }
    }
    #[test]
    fn soft_hyphens_separate_automatic_pattern_runs() {
        let patterns = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let lim = Limits { min_word: 2, ..Default::default() };
        let actual = opportunities_with("asszony\u{ad}asszony", &lim, ProofLang::from_tag(Some("hu")), &patterns, false);
        assert_eq!(actual, vec![Opportunity { at: 2, suffix: "z".into() }, Opportunity { at: 10, suffix: "z".into() }]);
    }
}
