//! Bulgarian hyphenation by rule ([`ProofLang::Bg`](crate::ProofLang::Bg)).
//!
//! Bulgarian spelling splits words at syllables, and its rules are regular enough to apply
//! directly, so no pattern file or word list is needed (and none is borrowed):
//! - two vowels next to each other may be split (`на-у-ка`);
//! - one consonant between vowels goes with the next syllable (`ма-ма`, `ро-за`);
//! - of two or more consonants between vowels, the first stays with the syllable before and the
//!   rest go with the next (`сес-тра`, `въз-дух`, `ком-пю-тър`) — the spelling rules allow any
//!   split of a consonant group, and this is the usual one — except that a consonant followed by
//!   `р` or `л` starts the next syllable together with it (`до-бре`, `ко-пре`, `та-бли-ца`);
//! - `й` before a consonant stays with the vowel before it (`строй-ка`, `май-ка`);
//! - `ьо` is never split, and the consonant before `ь` goes with it (`шо-фьор`);
//! - `дж` and `дз`, one sound each, are never split (`хо-джа`).
//!
//! A single letter left at the end or start of a line is ruled out by the paragraph's
//! [`Limits`](crate::hyphen::Limits) (`Бъл-га-рия`, not `Бъл-га-ри-я`).

fn is_vowel(c: char) -> bool {
    matches!(c, 'а' | 'е' | 'и' | 'о' | 'у' | 'ъ' | 'ю' | 'я' | 'ѝ')
}

/// Consonants that may start a syllable before `р` or `л` (`бр`, `пл`, `тр`, `гл` …).
fn is_obstruent(c: char) -> bool {
    matches!(c, 'б' | 'в' | 'г' | 'д' | 'з' | 'к' | 'п' | 'с' | 'т' | 'ф' | 'х')
}

/// A lowercase Bulgarian letter (the words [`points`] splits).
pub(crate) fn is_letter(c: char) -> bool {
    matches!(c, 'а'..='я' | 'ѝ')
}

/// Break points of one lowercase Bulgarian word (char indices; a hyphen goes before that char).
/// Empty for anything that is not entirely Bulgarian letters.
pub fn points(word: &[char]) -> Vec<usize> {
    if word.len() < 2 || !word.iter().all(|&c| is_letter(c)) {
        return Vec::new();
    }
    let vowels: Vec<usize> = (0..word.len()).filter(|&i| is_vowel(word[i])).collect();
    let mut out = Vec::new();
    for pair in vowels.windows(2) {
        let (v1, v2) = (pair[0], pair[1]);
        let mut pt = match v2 - v1 - 1 {
            0 | 1 => v1 + 1,
            _ => v1 + 2,
        };
        // A consonant with `р` or `л` after it starts a syllable together with it.
        if v2 - v1 > 2 && matches!(word[v2 - 1], 'р' | 'л') && is_obstruent(word[v2 - 2]) {
            pt = v2 - 2;
        }
        // `дж`, `дз`: one sound, so both go to the next syllable.
        if pt > v1 + 1 && word[pt - 1] == 'д' && matches!(word[pt], 'ж' | 'з') {
            pt -= 1;
        }
        // `ь` keeps the consonant before it (`фьо`).
        if pt < v2 && word[pt] == 'ь' && pt > v1 + 1 {
            pt -= 1;
        }
        if pt > v1 && pt <= v2 && word[pt - 1] != 'ь' {
            out.push(pt);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(w: &str) -> String {
        let chars: Vec<char> = w.chars().collect();
        let pts = points(&chars);
        let mut s = String::new();
        for (i, c) in chars.iter().enumerate() {
            if pts.contains(&i) {
                s.push('-');
            }
            s.push(*c);
        }
        s
    }

    #[test]
    fn syllables_follow_the_bulgarian_rules() {
        for (word, expected) in [
            ("мама", "ма-ма"),
            ("наука", "на-у-ка"),
            ("сестра", "сес-тра"),
            ("въздух", "въз-дух"),
            ("компютър", "ком-пю-тър"),
            ("стройка", "строй-ка"),
            ("майка", "май-ка"),
            ("шофьор", "шо-фьор"),
            ("ходжа", "хо-джа"),
            ("здравей", "здра-вей"),
            ("плевен", "пле-вен"),
            ("българия", "бъл-га-ри-я"),
            ("сричкопренасяне", "срич-ко-пре-на-ся-не"),
            ("добре", "до-бре"),
            ("таблица", "та-бли-ца"),
            ("отговор", "от-го-вор"),
        ] {
            assert_eq!(split(word), expected, "{word}");
        }
    }

    #[test]
    fn non_bulgarian_letters_are_left_alone() {
        assert!(points(&"word".chars().collect::<Vec<_>>()).is_empty());
        assert!(points(&"съобщение1".chars().collect::<Vec<_>>()).is_empty());
        assert!(points(&"объём".chars().collect::<Vec<_>>()).is_empty(), "Russian ё is not a Bulgarian letter");
    }
}
