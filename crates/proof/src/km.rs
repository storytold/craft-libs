//! Khmer proofing from Aksara's bundled dictionary and standardized spelling rules.
//! These grammar marks recommend standardized spellings; they do not infer sentence grammar.
use crate::{Issue, IssueKind, Options, ProofLang};

pub(crate) fn khmer(c: char) -> bool {
    ('\u{1780}'..='\u{17ff}').contains(&c)
}

pub(crate) fn detected_language(word: &str, lang: ProofLang) -> ProofLang {
    if lang.is_english() && word.chars().any(khmer) { ProofLang::Km } else { lang }
}

/// Latin words and segmented Khmer words, retaining original UTF-8 byte offsets.
pub(crate) fn proof_words(text: &str) -> Vec<(usize, usize)> {
    if !text.chars().any(khmer) {
        return crate::words(text);
    }
    let mut out: Vec<_> = craft_khmer::word_ranges(text).into_iter().map(|r| (r.start, r.end)).collect();
    // Split the Latin scanner at Khmer script boundaries, including Khmer punctuation.
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if khmer(c) {
            if let Some(part) = text.get(start..i) {
                out.extend(crate::words(part).into_iter().map(|(a, b)| (start + a, start + b)));
            }
            start = i + c.len_utf8();
        }
    }
    if let Some(part) = text.get(start..) {
        out.extend(crate::words(part).into_iter().map(|(a, b)| (start + a, start + b)));
    }
    out.sort_unstable();
    out
}

/// Aksara checks the spelling before a trailing LEK TOO sign. Keep exact dictionary
/// entries too: some repeated expressions occur only with the sign in the lexicon.
pub(crate) fn is_correct(word: &str, opts: &Options) -> bool {
    let stem = word.trim_end_matches('ៗ');
    !craft_khmer::data_available()
        || !word.chars().any(khmer)
        || stem.is_empty()
        || crate::in_user_dictionary(word)
        || crate::in_user_dictionary(stem)
        || (opts.ignore_numbers && word.chars().any(char::is_numeric))
        || craft_khmer::is_correct(word)
        || (stem.len() != word.len() && craft_khmer::is_correct(stem))
}

/// Direct word suggestions preserve a supplied repetition sign. Paragraph issues
/// address just the misspelled stem, leaving the sign and its formatting outside edits.
pub(crate) fn suggest(word: &str, max: usize) -> Vec<String> {
    if !craft_khmer::data_available() || max == 0 || word.chars().take(33).count() > 32 {
        return Vec::new();
    }
    if let Some(standard) = craft_khmer::standard_form(word) {
        return vec![standard.to_owned()];
    }
    let stem = word.trim_end_matches('ៗ');
    let suffix = word.get(stem.len()..).unwrap_or("");
    craft_khmer::suggest(stem, max).into_iter().map(|suggestion| format!("{suggestion}{suffix}")).collect()
}

pub(crate) fn grammar(text: &str) -> Vec<Issue> {
    if !craft_khmer::data_available() {
        return Vec::new();
    }
    craft_khmer::grammar_matches(text)
        .into_iter()
        .filter_map(|(r, standard)| {
            let word = text.get(r.clone())?;
            if crate::in_user_dictionary(word) {
                return None;
            }
            Some(Issue {
                start: r.start,
                end: r.end,
                kind: IssueKind::Grammar,
                message: "អក្ខរាវិរុទ្ធស្តង់ដារ".into(),
                suggestions: vec![standard],
                lang: ProofLang::Km,
            })
        })
        .collect()
}

/// Aksara skips up to three whitespace-separated name parts after a Khmer honorific.
pub(crate) fn follows_title(text: &str, start: usize) -> bool {
    let Some(mut before) = text.get(..start) else {
        return false;
    };
    for _ in 0..3 {
        let trimmed = before.trim_end_matches(|c: char| c.is_whitespace() || c == '\u{200b}');
        if trimmed.len() == before.len() {
            return false;
        }
        let at = trimmed
            .rfind(|c: char| c.is_whitespace() || c == '\u{200b}')
            .map_or(0, |i| trimmed.get(i..).and_then(|s| s.chars().next()).map_or(i, |c| i + c.len_utf8()));
        let Some(word) = trimmed.get(at..) else {
            return false;
        };
        if craft_khmer::is_title(word) {
            return true;
        }
        if !word.chars().all(khmer) {
            return false;
        }
        before = trimmed.get(..at).unwrap_or("");
    }
    false
}

/// English rules must not mark Khmer or impose English spacing beside it on the same line.
/// A Latin word still gets its own grammar checks even when a Khmer word is adjacent.
pub(crate) fn touches_khmer(text: &str, start: usize, end: usize) -> bool {
    let Some(marked) = text.get(start..end) else { return false };
    let line_break = |c: char| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
    let before = text.get(..start).and_then(|s| s.rsplit(line_break).next());
    let after = text.get(end..).and_then(|s| s.split(line_break).next());
    marked.chars().any(khmer)
        || (!marked.chars().any(char::is_alphabetic)
            && (before.and_then(|s| s.chars().rev().find(|c| !c.is_whitespace())).is_some_and(khmer)
                || after.and_then(|s| s.chars().find(|c| !c.is_whitespace())).is_some_and(khmer)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn required_data_pack_is_available() {
        if option_env!("CRAFT_KHMER_REQUIRED") == Some("1") {
            assert!(craft_khmer::data_available(), "required Khmer data must activate proofing tests");
        }
        assert_eq!(crate::packs::embedded_locales().contains(&"km"), craft_khmer::data_available());
        assert_eq!(crate::packs::licence_texts().iter().any(|(_, text)| text == craft_khmer::LICENSE), craft_khmer::data_available());
    }

    #[test]
    fn missing_data_skips_all_khmer_proofing() {
        if craft_khmer::data_available() {
            return;
        }
        let text = "ឝឝឝឝ សំដី";
        assert!(crate::is_correct_in(text, ProofLang::Km));
        assert!(crate::suggest_in(text, 5, ProofLang::Km).is_empty());
        assert!(crate::check_text(text, &[]).is_empty());
        assert!(crate::check_grammar_in(text, ProofLang::Km).is_empty());
        assert!(crate::check_spelling_in(text, ProofLang::Km).is_empty());
        assert!(proof_words("hello world").iter().map(|&(a, b)| &"hello world"[a..b]).eq(["hello", "world"]));
        assert!(!crate::has_dictionary(Some("km")));
    }

    #[test]
    fn human_review_misspelling_stays_whole_and_can_be_corrected() {
        if !craft_khmer::data_available() {
            return;
        }
        // The reviewer rejected ក | សី | ករ. Keeping the typo whole makes
        // the intended correction available in continuous, unspaced text.
        let text = "ខ្មែរកសីករខ្មែរ";
        let issues = crate::check_spelling_in(text, ProofLang::Km);
        assert_eq!(issues.len(), 1);
        let issue = &issues[0];
        assert_eq!(text.get(issue.start..issue.end), Some("កសីករ"));
        let suggestions = crate::suggest_in("កសីករ", 5, ProofLang::Km);
        assert_eq!(suggestions.first().map(String::as_str), Some("កសិករ"));
        let mut corrected = text.to_owned();
        corrected.replace_range(issue.start..issue.end, &suggestions[0]);
        assert_eq!(corrected, "ខ្មែរកសិករខ្មែរ");
        assert!(crate::check_spelling_in(&corrected, ProofLang::Km).is_empty());
    }

    #[test]
    fn detects_khmer_in_mixed_default_text_with_original_ranges() {
        if !craft_khmer::data_available() {
            return;
        }
        let text = "Hello «សំដី»។ world";
        let issues = crate::check_text(text, &[]);
        let grammar = issues.iter().find(|i| i.lang == ProofLang::Km && i.kind == IssueKind::Grammar).unwrap();
        assert_eq!(text.get(grammar.start..grammar.end), Some("សំដី"));
        assert_eq!(grammar.suggestions, ["សម្ដី"]);
        assert_eq!(grammar.start, "Hello «".len());
        assert!(!issues.iter().any(|i| i.lang.is_english() && i.start >= grammar.start && i.end <= grammar.end));
        assert!(crate::check_text(text, &[(0..text.len(), ProofLang::Other)]).is_empty());
        assert!(!crate::check_text("សំដី", &[(0.."សំដី".len(), ProofLang::Pl)]).iter().any(|i| i.lang == ProofLang::Km));
        assert!(!crate::check_text_with(text, &[], &Options::default(), false, false).iter().any(|i| i.kind == IssueKind::Grammar));
        let mixed = "hello  ខ្មែរ";
        let issues = crate::check_text(mixed, &[]);
        assert!(issues.iter().any(|i| i.kind == IssueKind::Grammar && i.start == 0 && i.end == 5 && i.suggestions == ["Hello"]));
        assert!(!issues.iter().any(|i| i.message == "Extra space"), "Khmer-adjacent spacing stays untouched");
        for line_break in ["\n", "\r", "\r\n", "\u{85}", "\u{2028}", "\u{2029}"] {
            for separate_lines in [format!("ខ្មែរ{line_break}  Hello"), format!("Hello  {line_break}ខ្មែរ")] {
                let issues = crate::check_text(&separate_lines, &[]);
                assert!(issues.iter().any(|i| i.message == "Extra space"), "a line break ends Khmer adjacency: {separate_lines:?} {issues:?}");
            }
        }
        let mixed = "Tôi yêu tiếng Việt ពាក្យសំដី";
        assert!(crate::vi::looks_vietnamese(mixed));
        for spans in [vec![], vec![(0..mixed.len(), ProofLang::En)]] {
            let issues = crate::check_text(mixed, &spans);
            assert!(issues.iter().any(|i| i.lang == ProofLang::Km && i.suggestions == ["ពាក្យសម្ដី"]), "{issues:?}");
            assert!(!issues.iter().any(|i| i.lang.is_english()));
        }
        assert!(crate::check_text(mixed, &[(0..mixed.len(), ProofLang::Other)]).is_empty());
        let disabled = mixed.find("សំដី").unwrap()..mixed.len();
        assert!(
            !crate::check_text(mixed, &[(disabled.clone(), ProofLang::Other)]).iter().any(|i| i.start < disabled.start && i.end > disabled.start)
        );
    }

    #[test]
    fn phrase_rules_names_and_mixed_scripts_keep_ranges() {
        if !craft_khmer::data_available() {
            return;
        }
        let text = "Hello ពាក្យសំដី world";
        let issues = crate::check_text(text, &[]);
        let phrase = issues.iter().find(|i| i.suggestions.iter().any(|s| s == "ពាក្យសម្ដី")).unwrap();
        assert_eq!(text.get(phrase.start..phrase.end), Some("ពាក្យសំដី"));
        let mixed = "helloខ្មែរworld";
        let ranges = proof_words(mixed);
        assert_eq!(mixed.get(ranges[0].0..ranges[0].1), Some("hello"));
        assert_eq!(mixed.get(ranges.last().unwrap().0..ranges.last().unwrap().1), Some("world"));
        assert!(follows_title("លោក ឝឝឝឝ", "លោក ".len()));
        assert!(!follows_title("ខ្មែរ ឝឝឝឝ", "ខ្មែរ ".len()));
        let strict = Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
        assert!(!crate::is_correct_in_with("ឝឝឝឝ", ProofLang::Km, &strict));
        assert!(crate::check_text("លោក ឝឝឝឝ", &[]).is_empty());
    }

    #[test]
    fn correction_never_crosses_a_disabled_run() {
        if !craft_khmer::data_available() {
            return;
        }
        let text = "ខាងវិញ្ញាណ";
        assert!(crate::check_text(text, &[]).iter().any(|issue| issue.kind == IssueKind::Grammar));
        let disabled = "ខាង".len()..text.len();
        assert!(
            !crate::check_text(text, &[(disabled.clone(), ProofLang::Other)])
                .iter()
                .any(|issue| issue.start < disabled.start && issue.end > disabled.start)
        );
        let unknown = "ឝឝឝឝ";
        let split = "ឝ".len();
        assert!(
            !crate::check_text(unknown, &[(split..unknown.len(), ProofLang::Other)]).iter().any(|issue| issue.start < split && issue.end > split)
        );
        assert!(crate::check_text(text, &[(disabled, ProofLang::Km)]).iter().any(|issue| issue.kind == IssueKind::Grammar));
    }

    #[test]
    fn khmer_is_not_an_uppercase_acronym_and_honors_custom_words() {
        if !craft_khmer::data_available() {
            return;
        }
        let word = "ឝឝឝឝ";
        assert!(!crate::is_correct_in(word, ProofLang::Km));
        crate::add_word(word);
        assert!(crate::is_correct_in(word, ProofLang::Km));
        crate::remove_word(word);
        assert!(!crate::is_correct_in(word, ProofLang::Km));
        crate::ignore_word("ឞឞឞឞ");
        assert!(crate::is_correct_in("ឞឞឞឞ", ProofLang::Km));
        assert_eq!(ProofLang::from_tag(Some("km-KH")), ProofLang::Km);
        assert!(crate::has_dictionary(Some("km")));
        assert_eq!(crate::missing_dictionary(Some("km-KH")), None);
        assert!(crate::check_grammar_in("ខ្មែរ  ខ្មែរ ។", ProofLang::Km).is_empty());
    }

    #[test]
    fn repetition_accepts_valid_stems_without_whitelisting_misspellings() {
        if !craft_khmer::data_available() {
            return;
        }
        for word in ["គ្រប់ៗ", "ខុសៗ", "អ្វីៗ", "ផ្សេងៗ", "កូនៗ", "ថ្មីៗ", "ល្អៗ", "ស្អាតៗ", "រ៉ាមៗ"]
        {
            assert!(crate::is_correct_in(word, ProofLang::Km), "{word}");
            assert!(crate::check_spelling_in(word, ProofLang::Km).is_empty(), "{word}");
        }
        assert!(!crate::is_correct_in("ខ្មែៗ", ProofLang::Km));
        let text = "«ខ្មែៗ»។";
        let issues = crate::check_spelling_in(text, ProofLang::Km);
        assert_eq!(issues.len(), 1, "{issues:?}");
        let issue = &issues[0];
        assert_eq!(&text[issue.start..issue.end], "ខ្មែ");
        let mut corrected = text.to_string();
        corrected.replace_range(issue.start..issue.end, "ខ្មែរ");
        assert_eq!(corrected, "«ខ្មែរៗ»។");
        let suggestions = crate::suggest_in("ខ្មែៗ", 6, ProofLang::Km);
        assert!(suggestions.iter().any(|s| s == "ខ្មែរៗ"), "{suggestions:?}");
        assert!(suggestions.iter().all(|s| s.ends_with('ៗ')));
        assert!(crate::suggest_in(&format!("ខ្មែ{}", "ៗ".repeat(40)), 6, ProofLang::Km).is_empty());
        assert!(crate::check_spelling_in("ៗ", ProofLang::Km).is_empty());
    }

    #[test]
    fn repetition_in_reported_paragraphs_has_no_false_spelling_marks() {
        if !craft_khmer::data_available() {
            return;
        }
        let examples = [
            "ព្រះជាម្ចាស់ចាត់ចែងនិងរៀបចំឋានៈឲ្យយើងគ្រប់ៗគ្នាតាមដែលទ្រង់ឃើញថាល្អនៅក្នុងព្រះនេត្ររបស់ទ្រង់។",
            "ទ្រង់បានបង្កើតរបស់ខ្លះមកឲ្យអាចកម្រើកបាន និងខ្លះទៀតមិនអាចកម្រើកបានខ្លះស្អាត និងខ្លះទៀតមិនស្អាត ហើយរបស់ទាំងអស់នោះសុទ្ធតែមានគុណសម្បត្តិខុសៗគ្នា ប៉ុន្តែគ្មាននិចលភាព ឬក៏ស្មារតីសោះឡើយ។",
        ];
        for text in examples {
            assert!(crate::check_text(text, &[]).is_empty(), "default language: {}", text);
            assert!(crate::check_text(text, &[(0..text.len(), ProofLang::Km)]).is_empty(), "explicit Khmer: {}", text);
        }
        for text in ["គ្រប់ៗគ្នា", "ខុសៗគ្នា", "«គ្រប់ៗ»។", "ខុសៗ\u{200b}គ្នា", "ខុសៗ\u{2060}គ្នា"]
        {
            assert!(crate::check_spelling_in(text, ProofLang::Km).is_empty(), "{text}");
        }
    }

    #[test]
    fn explicit_repeated_spelling_rules_override_conflicting_stem_rules() {
        if !craft_khmer::data_available() {
            return;
        }
        assert!(crate::check_grammar_in("ព្រឹលៗ", ProofLang::Km).is_empty());
        let repeated = crate::check_grammar_in("ព្រិលៗ", ProofLang::Km);
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].suggestions, ["ព្រឹលៗ"]);
        assert_eq!(repeated[0].end, "ព្រិលៗ".len());
        assert_eq!(crate::suggest_in("ព្រិលៗ", 6, ProofLang::Km), ["ព្រឹលៗ"]);
        assert!(crate::suggest_in("ព្រិលៗ", 0, ProofLang::Km).is_empty());
        let bare = crate::check_grammar_in("ព្រឹល", ProofLang::Km);
        assert_eq!(bare[0].suggestions, ["ព្រិល"]);
    }

    #[test]
    fn repeated_preferred_forms_preserve_the_sign_and_original_byte_offsets() {
        if !craft_khmer::data_available() {
            return;
        }
        for (text, variant, replacement) in [("«សំដីៗ»។", "សំដី", "សម្ដី"), ("«ពាក្យសំដីៗ»។", "ពាក្យសំដី", "ពាក្យសម្ដី")]
        {
            let issues = crate::check_grammar_in(text, ProofLang::Km);
            let issue = issues.iter().find(|i| i.suggestions.iter().any(|s| s == replacement)).unwrap();
            assert_eq!(&text[issue.start..issue.end], variant);
            let mut corrected = text.to_string();
            corrected.replace_range(issue.start..issue.end, replacement);
            assert_eq!(corrected, format!("«{replacement}ៗ»។"));
        }
        assert!(crate::check_text("សំដីៗ", &[(0.."សំដីៗ".len(), ProofLang::Other)]).is_empty());
        assert!(!craft_khmer::line_breaks("គ្រប់ៗគ្នា").contains(&"គ្រប់".len()), "proofing must not permit a wrap before LEK TOO");
    }
}
