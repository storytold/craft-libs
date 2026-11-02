//! Turkish grammar and punctuation checks: cheap, rule-based and tuned to stay quiet when unsure.
//! Messages are in Turkish.
//!
//! - **Repeated word** (`çok çok`), except idiomatic reduplications and doublings (`ALLOWED_REPEATS`:
//!   `yavaş yavaş`, `tıkır tıkır`, `adım adım`).
//! - **Space before punctuation** (`ev ,`) and **double spaces**.
//! - **Capital letter at the start of a sentence**, knowing Turkish abbreviations (`vb.`, `vs.`,
//!   `prof.`, `dr.`, `mad.`, `yy.`, `s.`…) after which lowercase or numbers may continue.
//! - **No comma before/after conjunctions**: `ve`, `veya`, `yahut`, `ile`, `de`, `da`, `hem ... hem`,
//!   `ne ... ne` conjunctions should not have commas immediately before or after them.
//! - **Doubled punctuation** (`..`, `,,`, `;;`).

use super::suggest::turkish_to_lower;
use crate::{Issue, IssueKind, ProofLang, words};

/// Turkish reduplications and emphatic word pairings that are intentionally repeated.
const ALLOWED_REPEATS: &[&str] = &[
    "yavaş",
    "hızlı",
    "koşa",
    "güle",
    "adım",
    "kara",
    "tıkır",
    "şakır",
    "akın",
    "ağır",
    "açık",
    "ayrı",
    "bire",
    "bir",
    "boş",
    "bölük",
    "buram",
    "cana",
    "can",
    "cılk",
    "çifter",
    "çit",
    "dalga",
    "damla",
    "deste",
    "didik",
    "dizi",
    "dolu",
    "deste",
    "döne",
    "eğri",
    "enine",
    "epey",
    "fıkır",
    "fırıl",
    "gece",
    "gıcır",
    "gide",
    "günü",
    "güzel",
    "halk",
    "harıl",
    "hık",
    "hım",
    "horul",
    "ılık",
    "ışıl",
    "için",
    "içten",
    "iğne",
    "iri",
    "iyiden",
    "iyi",
    "kaba",
    "kaç",
    "kat",
    "kaynar",
    "kelli",
    "kese",
    "kıtır",
    "kıvrım",
    "koca",
    "kol",
    "kucak",
    "kütür",
    "mırıl",
    "öbek",
    "parça",
    "patır",
    "pıtır",
    "sabah",
    "salına",
    "salkım",
    "sap",
    "sara",
    "sarı",
    "serin",
    "sıra",
    "sırım",
    "sıkı",
    "sırılsıklam",
    "tane",
    "teker",
    "tek",
    "tıpış",
    "top",
    "tutam",
    "uzun",
    "üçer",
    "üst",
    "yalan",
    "yana",
    "yan",
    "yarım",
    "yer",
    "yudum",
    "yığın",
    "yürekten",
    "yüz",
    "zaman",
    "zor",
];

/// Turkish abbreviations that end with a period and after which lower case may continue.
const ABBREVIATIONS: &[&str] = &[
    "vb", "vs", "vd", "bkz", "bknz", "ör", "örnek", "mad", "md", "s", "sf", "sn", "dk", "sa", "yy", "m", "km", "cm", "mm", "gr", "kg", "mg", "dr",
    "prof", "doç", "yrd", "av", "ist", "ank", "izm", "cad", "sok", "mah", "apt", "no", "tel", "faks", "tl", "kr", "krş", "ltd", "şti", "tbmm", "meb",
    "möt", "mö", "ms",
];

/// Conjunctions that must not be preceded by a comma.
const CONJUNCTIONS_NO_PRE_COMMA: &[&str] = &["ve", "veya", "yahut", "velev", "veyahut", "de", "da"];

/// Conjunctions that must not be followed by a comma.
const CONJUNCTIONS_NO_POST_COMMA: &[&str] = &["ve", "veya", "yahut", "veyahut", "hem", "gerek", "ister"];

/// Check grammar and punctuation in a Turkish text block.
pub fn check(text: &str) -> Vec<Issue> {
    check_with(text, true)
}

/// [`check`] where `capital_first` says whether the text's first word must start a sentence with a capital
/// (false for a list item, which may be a fragment).
pub fn check_with(text: &str, capital_first: bool) -> Vec<Issue> {
    let mut v = Vec::new();
    let ws = words(text);

    // 1. Repeated words (`kelime kelime`)
    for pair in ws.windows(2) {
        let (w1, w2) = match (text.get(pair[0].0..pair[0].1), text.get(pair[1].0..pair[1].1)) {
            (Some(a), Some(b)) => (a, b),
            _ => continue,
        };
        let gap = match text.get(pair[0].1..pair[1].0) {
            Some(g) => g,
            None => continue,
        };
        let only_spaces = gap.chars().all(|c| c == ' ' || c == '\t');
        let lower1 = turkish_to_lower(w1);
        let lower2 = turkish_to_lower(w2);
        // Words, not numbers or codes (`1 1`, `A1 A1`); `Ankara'da Ankara'da` is one.
        let word = w1.chars().any(char::is_alphabetic) && !w1.chars().any(|c| c.is_numeric());
        if only_spaces && word && lower1 == lower2 && !ALLOWED_REPEATS.contains(&lower1.as_str()) {
            v.push(Issue {
                start: pair[1].0,
                end: pair[1].1,
                kind: IssueKind::Grammar,
                message: "Yinelenen sözcük".into(),
                suggestions: Vec::new(),
                lang: ProofLang::Tr,
            });
        }
    }

    // 2. Spaces before punctuation and double spaces
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for i in 0..chars.len() {
        let (idx, c) = chars[i];
        if matches!(c, ',' | '.' | '?' | '!' | ';' | ':') && i > 0 && chars[i - 1].1 == ' ' {
            let prev_sp = chars[i - 1].0;
            // Leave ellipses (` ...`), numbers (` .5`), emoticons (` :)`, ` ;-)`, ` :D`) and
            // scores (`3 : 1`) alone.
            let after = chars.get(i + 1).map(|&(_, a)| a);
            let ratio = c == ':'
                && i >= 2
                && chars.get(i - 2).is_some_and(|&(_, b)| b.is_ascii_digit())
                && after == Some(' ')
                && chars.get(i + 2).is_some_and(|&(_, d)| d.is_ascii_digit());
            // `:)`, `;(`, `:-)` are emoticons; `:D` and `;P` only when a space or the end follows
            // (`;Dün` is a stray space before a word).
            let ends = |k: usize| chars.get(k).is_none_or(|&(_, e)| e.is_whitespace());
            let emoticon = match after {
                Some(')' | '(') => true,
                Some('-') => matches!(chars.get(i + 2), Some(&(_, ')' | '('))),
                Some('D' | 'P') => ends(i + 2),
                _ => false,
            };
            let flag = !ratio
                && match c {
                    ',' | '!' | '?' => true,
                    ':' | ';' => !emoticon,
                    _ => !matches!(after, Some('.')) && !after.is_some_and(|a| a.is_ascii_digit()),
                };
            if flag {
                v.push(Issue {
                    start: prev_sp,
                    end: idx + c.len_utf8(),
                    kind: IssueKind::Grammar,
                    message: "Noktalama işaretinden önce boşluk bırakılmaz".into(),
                    suggestions: vec![c.to_string()],
                    lang: ProofLang::Tr,
                });
            }
        }
        // Double space
        if c == ' ' && i + 1 < chars.len() && chars[i + 1].1 == ' ' && (i == 0 || chars[i - 1].1 != ' ') {
            let mut end = idx + 1;
            while end < text.len() && text.as_bytes().get(end) == Some(&b' ') {
                end += 1;
            }
            v.push(Issue {
                start: idx,
                end,
                kind: IssueKind::Grammar,
                message: "Fazladan boşluk".into(),
                suggestions: vec![" ".into()],
                lang: ProofLang::Tr,
            });
        }
    }

    // 3. Comma before or after conjunctions (`ali, ve veli` or `ve, veli`)
    for (i, &(w_start, w_end)) in ws.iter().enumerate() {
        let word = match text.get(w_start..w_end) {
            Some(w) => turkish_to_lower(w),
            None => continue,
        };
        // Check comma before conjunction
        if CONJUNCTIONS_NO_PRE_COMMA.contains(&word.as_str())
            && i > 0
            && let Some(between) = text.get(ws[i - 1].1..w_start)
            && let Some(comma_pos) = between.find(',')
        {
            let actual_pos = ws[i - 1].1 + comma_pos;
            v.push(Issue {
                start: actual_pos,
                end: actual_pos + 1,
                kind: IssueKind::Grammar,
                message: format!("'{word}' bağlacından önce virgül konulmaz"),
                suggestions: vec![String::new()],
                lang: ProofLang::Tr,
            });
        }
        // Check comma after conjunction
        if CONJUNCTIONS_NO_POST_COMMA.contains(&word.as_str())
            && i + 1 < ws.len()
            && let Some(between) = text.get(w_end..ws[i + 1].0)
            && let Some(comma_pos) = between.find(',')
        {
            let actual_pos = w_end + comma_pos;
            v.push(Issue {
                start: actual_pos,
                end: actual_pos + 1,
                kind: IssueKind::Grammar,
                message: format!("'{word}' bağlacından sonra virgül konulmaz"),
                suggestions: vec![String::new()],
                lang: ProofLang::Tr,
            });
        }
    }

    // 4. Sentence capitalization knowing Turkish abbreviations
    for (i, &(w_start, w_end)) in ws.iter().enumerate() {
        if i == 0 {
            // First word in paragraph
            if capital_first
                && let Some(w) = text.get(w_start..w_end)
                && w.chars().next().is_some_and(char::is_lowercase)
            {
                let cap = turkish_capitalize(w);
                v.push(Issue {
                    start: w_start,
                    end: w_end,
                    kind: IssueKind::Grammar,
                    message: "Cümle büyük harfle başlar".into(),
                    suggestions: vec![cap],
                    lang: ProofLang::Tr,
                });
            }
            continue;
        }

        // Check if previous character sequence ended a sentence
        let prev_end = ws[i - 1].1;
        let prev_word = text.get(ws[i - 1].0..prev_end).unwrap_or("");
        if let Some(gap) = text.get(prev_end..w_start) {
            let trimmed = gap.trim();
            // A space must follow the mark (`rapor.txt`, `www.example.com` are one token), and a
            // number before a period is an ordinal (`5. sınıf`, `21. yüzyıl`).
            let spaced = gap.rfind(['.', '?', '!']).and_then(|m| gap.get(m + 1..)).is_some_and(|rest| rest.contains(char::is_whitespace));
            let ordinal = trimmed.ends_with('.') && prev_word.chars().any(|c| c.is_ascii_digit());
            // An initial (`A.Ş.`, `T.C.`, `M. Kemal`) ends no sentence.
            let initial = trimmed.starts_with('.') && prev_word.chars().count() == 1;
            if (trimmed.ends_with('.') || trimmed.ends_with('?') || trimmed.ends_with('!'))
                && spaced
                && !ordinal
                && !initial
                && !trimmed.contains("...")
                && !ABBREVIATIONS.contains(&turkish_to_lower(prev_word).as_str())
                && let Some(w) = text.get(w_start..w_end)
                && w.chars().next().is_some_and(char::is_lowercase)
            {
                let cap = turkish_capitalize(w);
                v.push(Issue {
                    start: w_start,
                    end: w_end,
                    kind: IssueKind::Grammar,
                    message: "Cümle büyük harfle başlar".into(),
                    suggestions: vec![cap],
                    lang: ProofLang::Tr,
                });
            }
        }
    }

    v.sort_by_key(|issue| (issue.start, issue.end));
    v
}

/// Capitalize according to Turkish rules (`i` -> `İ`, `ı` -> `I`).
pub fn turkish_capitalize(s: &str) -> String {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return String::new() };
    let first_cap = match first {
        'i' => 'İ',
        'ı' => 'I',
        other => other.to_uppercase().next().unwrap_or(other),
    };
    let mut out = String::with_capacity(s.len() + 1);
    out.push(first_cap);
    out.push_str(chars.as_str());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CodeRabbit on craft-libs#484/#489: emoticons, decimals and scores aren't stray spaces;
    /// ordinals and file or web names don't end a sentence.
    #[test]
    fn emoticons_numbers_ordinals_and_names_are_left_alone() {
        for ok in [
            "Harika :)",
            "Tamam ;)",
            "Süper :D",
            "Oran .5 oldu.",
            "Maç 3 : 1 bitti.",
            "Ali 5. sınıfta okuyor.",
            "Bina 21. yüzyıldan kalma.",
            "Dosyayı rapor.txt olarak kaydet.",
            "Adres www.example.com olarak yazıldı.",
        ] {
            assert!(check(ok).is_empty(), "{ok}: {:?}", check(ok));
        }
        assert_eq!(check("Gel , bak.").len(), 1, "a real stray space is still flagged");
        for bad in ["Not :bu", "Ne ?Bu", "Peki ;Dün", "Evet !"] {
            assert!(check(bad).iter().any(|i| i.message.starts_with("Noktalama")), "{bad}: {:?}", check(bad));
        }
        assert!(check("Harika :-) dedi.").is_empty() && check("Peki ;P").is_empty());
        assert_eq!(check("Geldi. bu iyi.").len(), 1, "a real sentence start is still flagged");
    }

    #[test]
    fn clean_turkish_text_produces_no_issues() {
        let text = "Bu bir test cümlesidir. Kitap ve defter aldım. Çocuk yavaş yavaş yürüdü.";
        assert!(check(text).is_empty());
    }

    #[test]
    fn repeated_words_flagged_except_allowed() {
        let text = "Gitti gitti ama bulamadı.";
        let issues = check(text);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Yinelenen sözcük");

        // Allowed repeat
        let text2 = "Çocuk yavaş yavaş ilerliyordu.";
        assert!(check(text2).is_empty());
    }

    #[test]
    fn space_before_punctuation_flagged() {
        let text = "Merhaba , dünya!";
        let issues = check(text);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Noktalama işaretinden önce boşluk bırakılmaz");
    }

    #[test]
    fn comma_around_conjunctions_flagged() {
        let text = "Elma, ve armut.";
        let issues = check(text);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("virgül konulmaz"));

        let text2 = "Elma ve, armut.";
        let issues2 = check(text2);
        assert_eq!(issues2.len(), 1);
        assert!(issues2[0].message.contains("'ve' bağlacından sonra virgül konulmaz"));
    }

    /// `de`/`da`, `ne` and `ya` often take a comma after them in correct Turkish, so no mark
    /// (craft-libs#488 review).
    #[test]
    fn a_comma_after_de_ne_or_ya_is_fine() {
        for text in ["Ali de, Veli de geldi.", "Ne, sen mi geldin?", "Ya, bilmiyorum.", "Bu mesele de, haklı olan bizdik."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
    }

    /// craft-libs#495 review: initials (`A.Ş.`, `T.C.`) and `İst.` end no sentence, and Turkish
    /// case folding finds `Işık ışık`; a word with an apostrophe is still a repeat.
    #[test]
    fn turkish_case_and_initials() {
        for text in ["Şirket A.Ş. bugün açıldı.", "O T.C. vatandaşı.", "Adres İst. caddesi."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
        for text in ["Işık ışık yandı.", "Ankara'da Ankara'da kaldık."] {
            assert_eq!(check(text).iter().filter(|i| i.message == "Yinelenen sözcük").count(), 1, "{text}");
        }
    }

    /// Repeated numbers are not repeated words (craft-libs#488 review).
    #[test]
    fn repeated_numbers_are_not_flagged() {
        for text in ["Skor 1 1 bitti.", "Toplam 100 100 değil.", "Oda A1 A1 değil."] {
            assert!(check(text).is_empty(), "{text}: {:?}", check(text));
        }
    }

    #[test]
    fn sentence_capitalization_respects_abbreviations() {
        let text = "Dr. ahmet geldi. Fakat geç kaldı.";
        let issues = check(text);
        // "Dr. ahmet" does not flag "ahmet" because "Dr." is in ABBREVIATIONS.
        assert!(issues.is_empty());
    }
}
