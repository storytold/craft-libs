//! Generic repeated-word and spacing checks, shared with the other languages proofed by their
//! script, with Gujarati messages. No Gujarati-specific grammar or case rules are inferred.

use crate::script_checks::Rules;
use crate::{Issue, ProofLang};

/// Checks `text` for repeated words and spacing problems, with Gujarati messages.
pub fn check(text: &str) -> Vec<Issue> {
    crate::script_checks::check(text, &RULES)
}

const RULES: Rules = Rules {
    lang: ProofLang::Gu,
    script: &super::SCRIPT,
    repeated_word: "પુનરાવર્તિત શબ્દ",
    space_before_punctuation: "વિરામચિહ્ન પહેલાં જગ્યા",
    double_space: "બેવડી જગ્યા",
    flag_repeat: |w| !ALLOWED_REPEATS.contains(&w),
};

/// Words Gujarati repeats on purpose (`હા હા`, `ના ના`, `ધીમે ધીમે`, `સાચે સાચે`).
const ALLOWED_REPEATS: &[&str] = &["હા", "ના", "ધીમે", "જલ્દી", "વારંવાર", "સાચે", "રોજે", "ખરેખર", "જરા", "થોડું", "સાથે", "પાસે"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IssueKind;

    #[test]
    fn intentional_repeats_are_fine() {
        for text in ["હા હા, હું આવીશ.", "ના ના, આ નથી.", "ધીમે ધીમે ચાલ.", "સાચે સાચે બોલ."]
        {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        assert_eq!(check("પુસ્તક પુસ્તક છે.").len(), 1);
        assert!(check("1 1").is_empty());
        assert!(check("૧ ૧").is_empty());
    }

    #[test]
    fn repeated_words_and_spaces_have_gujarati_messages() {
        let text = "ઘર ઘર ,  શાળા.";
        let issues = check(text);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues[0].message, "પુનરાવર્તિત શબ્દ: “ઘર”");
        assert_eq!(issues[0].suggestions, ["ઘર"]);
        assert!(issues.iter().any(|i| i.message == "વિરામચિહ્ન પહેલાં જગ્યા" && &text[i.start..i.end] == " ,"));
        assert!(issues.iter().any(|i| i.message == "બેવડી જગ્યા"));
        assert!(issues.iter().all(|i| i.lang == ProofLang::Gu && i.kind == IssueKind::Grammar));
    }

    #[test]
    fn ratios_and_scores_keep_their_spaces() {
        for text in ["3 : 1", "1 : 2", "સ્કોર 12 : 10.", "૩ : ૧", "૧ : ૨", "સ્કોર ૧૨ : ૧૦."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        for text in ["શબ્દ : બીજો", "3 : શબ્દ", "3 :", "૩ : શબ્દ", "૩ :"] {
            assert!(check(text).iter().any(|i| i.message == "વિરામચિહ્ન પહેલાં જગ્યા"), "{text}");
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
