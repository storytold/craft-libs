//! Generic repeated-word and spacing checks, shared with the other languages proofed by their
//! script, with Hindi messages. No Hindi-specific grammar or agreement rules are inferred.

use crate::script_checks::Rules;
use crate::{Issue, ProofLang};

/// Checks `text` for repeated words and spacing problems, with Hindi messages.
pub fn check(text: &str) -> Vec<Issue> {
    crate::script_checks::check(text, &RULES)
}

const RULES: Rules = Rules {
    lang: ProofLang::Hi,
    script: &crate::devanagari::SCRIPT,
    repeated_word: "दोहराया गया शब्द",
    space_before_punctuation: "विराम चिह्न से पहले रिक्त स्थान",
    double_space: "दोहरा रिक्त स्थान",
    flag_repeat,
};

/// Hindi doubles words on purpose all the time (`धीरे धीरे`, `बार बार`, `घर घर`, `छोटे छोटे`),
/// so a doubled Hindi word is flagged only when it is one of these postpositions, auxiliaries
/// and conjunctions, which are never doubled on purpose (`के के`, `है है`). A doubled word in
/// another script is flagged as in the other languages.
const NEVER_DOUBLED: &[&str] = &[
    "का",
    "की",
    "के",
    "को",
    "में",
    "से",
    "ने",
    "पर",
    "तक",
    "है",
    "हैं",
    "था",
    "थी",
    "थे",
    "हूँ",
    "हूं",
    "और",
    "या",
    "कि",
    "एवं",
    "तथा",
    "लेकिन",
    "परंतु",
    "किंतु",
    "क्योंकि",
    "भी",
];

fn flag_repeat(w: &str) -> bool {
    !w.chars().any(|c| ('\u{0900}'..='\u{097F}').contains(&c)) || NEVER_DOUBLED.contains(&w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IssueKind;

    #[test]
    fn intentional_repeats_are_fine() {
        for text in ["धीरे धीरे चलो।", "बार बार कहा।", "घर घर में दीये जले।", "कभी कभी", "अपने अपने घर जाओ।", "छोटे छोटे बच्चे"]
        {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        assert_eq!(check("मैं घर के के पास हूँ।").len(), 1);
        assert_eq!(check("यह सच है है।").len(), 1);
        assert!(check("1 1").is_empty());
        assert!(check("१ १").is_empty());
    }

    #[test]
    fn repeated_words_and_spaces_have_hindi_messages() {
        let text = "राम ने ने कहा ,  ठीक है।";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "दोहराया गया शब्द: “ने”");
        assert_eq!(issues[0].suggestions, ["ने"]);
        assert_eq!(&text[issues[0].start..issues[0].end], "ने ने");
        assert!(issues.iter().any(|i| i.message == "विराम चिह्न से पहले रिक्त स्थान" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "दोहरा रिक्त स्थान"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::Hi && i.kind == IssueKind::Grammar));
        // Latin words doubled inside Hindi text are still flagged.
        assert_eq!(check("यह the the है").len(), 1);
    }

    #[test]
    fn a_space_before_the_danda_is_left_alone() {
        for text in ["वह घर गया।", "वह घर गया ।", "श्लोक ॥"] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
    }

    #[test]
    fn ratios_and_scores_keep_their_spaces() {
        for text in ["3 : 1", "स्कोर 12 : 10।", "३ : १", "१ : २", "स्कोर १२ : १०।"] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        for text in ["शब्द : दूसरा", "3 : शब्द", "3 :", "३ : शब्द", "३ :"] {
            assert!(check(text).iter().any(|i| i.message == "विराम चिह्न से पहले रिक्त स्थान"), "{text}");
        }
    }

    proptest::proptest! {
        #[test]
        fn never_panics_and_ranges_are_valid(s in "\\PC{0,80}") {
            for issue in check(&s) {
                proptest::prop_assert!(issue.start <= issue.end && issue.end <= s.len());
                proptest::prop_assert!(s.is_char_boundary(issue.start) && s.is_char_boundary(issue.end));
            }
        }
    }
}
