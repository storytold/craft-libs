//! Insertions are returned separately from source offsets, never written to the text.
use super::{Limits, Opportunity, ProofLang, composed_chars, hyphen_points_in, is_apostrophe, is_word_char};

/// Allowed source-character breaks plus display-only letters before the hyphen.
/// Point-only clients keep using `hyphen_points_in`, which omits insertions it cannot render.
/// Installed nonstandard patterns preserve source normalization, casing, punctuation and limits.
pub fn hyphen_opportunities_in(word: &str, lim: &Limits, lang: ProofLang) -> Vec<Opportunity> {
    let Some(patterns) = crate::packs::hyphenation(lang).filter(|p| p.has_nonstandard()) else {
        return hyphen_points_in(word, lim, lang).into_iter().map(|at| Opportunity { at, suffix: "".into() }).collect();
    };
    opportunities_with(word, lim, lang, &patterns)
}

fn opportunities_with(word: &str, lim: &Limits, lang: ProofLang, patterns: &super::Patterns) -> Vec<Opportunity> {
    let chars: Vec<char> = word.chars().collect();
    let belongs = |c: char| is_word_char(c) || unicode_normalization::char::is_combining_mark(c);
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !belongs(chars[i]) {
            i += 1;
            continue;
        }
        let mut end = i;
        while chars.get(end).is_some_and(|&c| belongs(c)) {
            end += 1;
        }
        let (mut a, mut b) = (i, end);
        while a < b && is_apostrophe(chars[a]) {
            a += 1;
        }
        while b > a && is_apostrophe(chars[b - 1]) {
            b -= 1;
        }
        let run = &chars[a..b];
        let letters = run.iter().filter(|c| c.is_alphabetic()).count();
        if run.iter().map(|c| c.len_utf8()).sum::<usize>() <= 1024
            && letters >= lim.min_word.max(2)
            && (lim.capitalized || run.first().is_none_or(|c| !c.is_uppercase()))
        {
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
                    point.at = a + at;
                    out.push(point);
                }
            } else {
                let text: String = run.iter().collect();
                out.extend(hyphen_points_in(&text, lim, lang).into_iter().map(|at| Opportunity { at: a + at, suffix: "".into() }));
            }
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::Patterns;
    use super::*;
    #[test]
    fn unknown_pack_letters_retain_builtin_language_breaks() {
        let patterns = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let lim = Limits { min_word: 2, after_first: 1, before_last: 1, capitalized: true };
        for (word, lang) in [("молоко", ProofLang::Ru), ("автомобил", ProofLang::Bg), ("kitaplar", ProofLang::Tr), ("Ankara'nın", ProofLang::Tr)]
        {
            let expected = hyphen_points_in(word, &lim, lang);
            assert!(!expected.is_empty(), "positive fallback control: {word}");
            let actual = opportunities_with(word, &lim, lang, &patterns);
            assert_eq!(actual.iter().map(|p| p.at).collect::<Vec<_>>(), expected, "{word}");
            assert!(actual.iter().all(|p| p.suffix.is_empty()));
        }
    }
    #[test]
    fn soft_hyphens_separate_automatic_pattern_runs() {
        let patterns = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let lim = Limits { min_word: 2, ..Default::default() };
        let actual = opportunities_with("asszony\u{ad}asszony", &lim, ProofLang::from_tag(Some("hu")), &patterns);
        assert_eq!(actual, vec![Opportunity { at: 2, suffix: "z".into() }, Opportunity { at: 10, suffix: "z".into() }]);
    }
}
