//! Generic repeated-word and spacing checks with Gujarati messages.
//! No Gujarati-specific grammar or case rules are inferred.

use crate::{Issue, IssueKind, ProofLang, words};

fn issue(start: usize, end: usize, message: String, suggestions: Vec<String>) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message, suggestions, lang: ProofLang::Gu }
}

/// Checks `text` for repeated words and spacing problems, with Gujarati messages.
pub fn check(text: &str) -> Vec<Issue> {
    let mut issues = Vec::new();
    repeated_words(text, &words(text), &mut issues);
    spaces(text, &mut issues);
    issues.sort_by_key(|i| (i.start, i.end));
    issues
}

/// Words Gujarati repeats on purpose (`હા હા`, `ના ના`, `ધીમે ધીમે`, `સાચે સાચે`).
const ALLOWED_REPEATS: &[&str] = &["હા", "ના", "ધીમે", "જલ્દી", "વારંવાર", "સાચે", "રોજે", "ખરેખર", "જરા", "થોડું", "સાથે", "પાસે"];

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('\u{0AE6}'..='\u{0AEF}').contains(&c)
}

fn repeated_words(text: &str, ws: &[(usize, usize)], v: &mut Vec<Issue>) {
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if !gap.is_empty()
            && gap.chars().all(char::is_whitespace)
            && w0.chars().all(|c| c.is_alphabetic() || (super::is_gujarati_char(c) && !c.is_numeric()))
            && w0 == w1
            && !ALLOWED_REPEATS.contains(&w0)
        {
            v.push(issue(*a0, *b1, format!("પુનરાવર્તિત શબ્દ: “{w1}”"), vec![w0.to_string()]));
        }
    }
}

fn spaces(text: &str, v: &mut Vec<Issue>) {
    for (i, c) in text.char_indices() {
        if c != ' ' {
            continue;
        }
        let rest = text.get(i + 1..).unwrap_or("");
        let mut it = rest.chars();
        let Some(p) = it.next() else { continue };
        let after = it.next();
        // A ratio/score keeps the spaces in digit-space-colon-space-digit (ASCII or Gujarati digits).
        let ratio = p == ':'
            && text.get(..i).and_then(|before| before.chars().next_back()).is_some_and(is_digit)
            && after == Some(' ')
            && it.next().is_some_and(is_digit);
        let flag = !ratio
            && match p {
                ',' => true,
                ';' => !matches!(after, Some(')' | '(' | '-' | 'P' | 'D')),
                '.' => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
                ':' | '!' | '?' => after.is_none_or(|a| a.is_whitespace() || matches!(a, '"' | '”' | '’' | '!' | '?')),
                _ => false,
            };
        if flag {
            v.push(issue(i, i + 1 + p.len_utf8(), "વિરામચિહ્ન પહેલાં જગ્યા".into(), vec![p.to_string()]));
        }
    }
    for run in crate::space_runs(text) {
        v.push(issue(run.start, run.end, "બેવડી જગ્યા".into(), vec![" ".into()]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
