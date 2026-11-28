//! Hyphenation of Indic scripts by rule: a word breaks only between its orthographic syllables
//! (aksharas), never inside one.
//!
//! An akshara is a consonant with what is written on it — its vowel sign, nukta, anusvara,
//! candrabindu or visarga — and the consonants joined to it by a virama (a conjunct: `દ્ય`, `ર્થ`,
//! `न्द`), or an independent vowel with its marks. So a break goes only before a consonant or an
//! independent vowel that does not follow a virama: `ગુ-જ-રા-તી`, `વિ-દ્યા-ર્થી`, `સં-સ્કૃત`,
//! `हि-न्दी`. A vowel sign, a mark or the second consonant of a conjunct never starts a line.
//! Nor does a dead consonant — one ending in a virama with no consonant after it (`વાક્`) — since
//! it has no vowel of its own and is the end of the syllable before. A zero-width joiner or
//! non-joiner after a virama (`સ્` U+200D `ત`) only chooses how the conjunct is drawn: it stays
//! inside the akshara, and no break goes next to a joiner.
//!
//! The rules come from the scripts' Unicode encoding alone; no pattern file or word list is
//! read (and none is borrowed). The Brahmic blocks share one layout (the ISCII order), so a
//! script is its block's start and a few script-specific code points (`Script::kind`).
//! Gujarati ([`ProofLang::Gu`]) and Hindi ([`ProofLang::Hi`], [`Script::Devanagari`]) use them;
//! another language written in one of these scripts is one more line in [`Script::of`].
//!
//! A single akshara left at the end or start of a line is ruled out by the paragraph's
//! [`Limits`](crate::hyphen::Limits), which count letters (vowel signs and anusvara count,
//! viramas and nuktas don't), so a bare consonant (`…-ત`) is not carried over with the default
//! limits but a consonant with its vowel sign (`…-તી`) is.

use crate::ProofLang;

/// An Indic script hyphenated by [`points`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    /// Devanagari (U+0900..=U+097F): Hindi, Marathi, Nepali…
    Devanagari,
    /// Gujarati (U+0A80..=U+0AFF).
    Gujarati,
}

/// What a code point is in an akshara.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// An independent vowel: starts an akshara.
    Vowel,
    /// A consonant: starts an akshara, unless a virama joins it to the consonant before.
    Consonant,
    /// The virama: joins the consonant before it to the next one, or kills its vowel.
    Virama,
    /// A vowel sign, nukta, anusvara, candrabindu, visarga or other mark (and the avagraha):
    /// belongs to the akshara before it.
    Mark,
    /// A zero-width joiner or non-joiner: shapes the conjunct around it, in the same akshara.
    Joiner,
}

/// Is `c` a zero-width joiner or non-joiner (U+200D, U+200C)? Inside an Indic word it shapes a
/// conjunct, so it is part of the word (as in Unicode's word boundaries, UAX #29).
pub fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

impl Script {
    /// The script whose rules hyphenate `lang`'s words, if any.
    pub fn of(lang: ProofLang) -> Option<Script> {
        match lang {
            ProofLang::Gu => Some(Script::Gujarati),
            ProofLang::Hi => Some(Script::Devanagari),
            _ => None,
        }
    }

    /// First code point of the script's block.
    fn base(self) -> u32 {
        match self {
            Script::Devanagari => 0x0900,
            Script::Gujarati => 0x0A80,
        }
    }

    /// What `c` is in this script, or `None` for a character outside the script's letters and
    /// marks (another script, a digit, a danda, `ॐ`, the abbreviation sign…).
    fn kind(self, c: char) -> Option<Kind> {
        if is_joiner(c) {
            return Some(Kind::Joiner);
        }
        let o = u32::from(c).checked_sub(self.base()).filter(|&o| o < 0x80)?;
        let dev = self == Script::Devanagari;
        Some(match o {
            // Candrabindu, anusvara, visarga (and Devanagari's inverted candrabindu).
            0x00..=0x03 => Kind::Mark,
            0x04..=0x14 => Kind::Vowel,
            0x15..=0x39 => Kind::Consonant,
            // Devanagari's vowel signs OE and OOE, the nukta, the avagraha, the vowel signs.
            0x3A..=0x4C => Kind::Mark,
            0x4D => Kind::Virama,
            // Devanagari's vowel signs PRISHTHAMATRA E and AW, stress signs and vowel signs.
            0x4E..=0x4F | 0x51..=0x57 if dev => Kind::Mark,
            // Devanagari's consonants with nukta (`क़`, `ज़`, `फ़`…).
            0x58..=0x5F if dev => Kind::Consonant,
            // Vocalic RR and LL.
            0x60..=0x61 => Kind::Vowel,
            0x62..=0x63 => Kind::Mark,
            // Devanagari's vowels for Marathi, Kashmiri and Bihari languages, and consonants for
            // Sindhi, Marwari and others.
            0x72..=0x77 if dev => Kind::Vowel,
            0x78..=0x7F if dev => Kind::Consonant,
            // Gujarati ZHA, and the nuktas and signs for transliterating Arabic.
            0x79 => Kind::Consonant,
            0x7A..=0x7F => Kind::Mark,
            _ => return None,
        })
    }

    /// Is `c` one of this script's letters or marks, or a joiner (the characters [`points`]
    /// splits)?
    pub fn is_letter(self, c: char) -> bool {
        self.kind(c).is_some()
    }
}

/// Break points of one `word` in `script` (char indices; a hyphen goes before that char): before
/// each akshara but the first, except a dead consonant. Empty for anything that is not entirely
/// the script's letters and marks.
pub fn points(script: Script, word: &[char]) -> Vec<usize> {
    let Some(kinds) = word.iter().map(|&c| script.kind(c)).collect::<Option<Vec<Kind>>>() else {
        return Vec::new();
    };
    let at = |i: usize| kinds.get(i).copied();
    // Has the akshara starting at `i` a vowel (independent, a sign, or the inherent one)? Only a
    // consonant cluster ending in a virama (`ક્`, `ન્ત્`, with or without a joiner) has none.
    let live = |i: usize| {
        let mut j = i;
        loop {
            if at(j) != Some(Kind::Consonant) {
                return true;
            }
            j += 1;
            while matches!(at(j), Some(Kind::Mark | Kind::Joiner)) {
                j += 1;
            }
            if at(j) != Some(Kind::Virama) {
                return true;
            }
            j += 1;
            while at(j) == Some(Kind::Joiner) {
                j += 1;
            }
            if at(j) != Some(Kind::Consonant) {
                return false;
            }
        }
    };
    (1..word.len())
        .filter(|&i| matches!(at(i), Some(Kind::Vowel | Kind::Consonant)) && !matches!(at(i - 1), Some(Kind::Virama | Kind::Joiner)) && live(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(script: Script, w: &str) -> String {
        let c: Vec<char> = w.chars().collect();
        let pts = points(script, &c);
        let mut s = String::new();
        for (i, ch) in c.iter().enumerate() {
            if pts.contains(&i) {
                s.push('-');
            }
            s.push(*ch);
        }
        s
    }

    fn gu(w: &str) -> String {
        split(Script::Gujarati, w)
    }

    fn hi(w: &str) -> String {
        split(Script::Devanagari, w)
    }

    /// Every akshara boundary is a break, and a vowel sign stays on its consonant.
    #[test]
    fn gujarati_breaks_between_aksharas() {
        assert_eq!(gu("ગુજરાતી"), "ગુ-જ-રા-તી");
        assert_eq!(gu("અમદાવાદ"), "અ-મ-દા-વા-દ");
        assert_eq!(gu("આવક"), "આ-વ-ક", "independent vowels start aksharas");
        assert_eq!(gu("કઈ"), "ક-ઈ", "an independent vowel after a consonant");
    }

    /// A conjunct is never split, and the virama never ends a line.
    #[test]
    fn gujarati_conjuncts_stay_whole() {
        assert_eq!(gu("વિદ્યાર્થી"), "વિ-દ્યા-ર્થી");
        assert_eq!(gu("પુસ્તક"), "પુ-સ્ત-ક");
        assert_eq!(gu("સ્ત્રી"), "સ્ત્રી", "one akshara of three consonants");
        assert_eq!(gu("ક્ષત્રિય"), "ક્ષ-ત્રિ-ય");
    }

    /// Anusvara, candrabindu and visarga stay with the akshara before them.
    #[test]
    fn gujarati_marks_stay_with_their_akshara() {
        assert_eq!(gu("સંસ્કૃત"), "સં-સ્કૃ-ત");
        assert_eq!(gu("હાઁસલ"), "હાઁ-સ-લ");
        assert_eq!(gu("દુઃખી"), "દુઃ-ખી");
    }

    /// A consonant killed by a final virama has no vowel: it ends the syllable before.
    #[test]
    fn a_dead_final_consonant_is_not_carried_over() {
        assert_eq!(gu("વાક્"), "વાક્");
        assert_eq!(gu("સમ્યક્"), "સ-મ્યક્");
        assert_eq!(hi("जगत्"), "ज-गत्");
    }

    /// Anything but the script's letters and marks: no breaks.
    #[test]
    fn other_characters_get_no_breaks() {
        assert_eq!(gu("ૐકાર"), "ૐકાર", "OM is not a letter here");
        assert_eq!(gu("ગુજ૧૨"), "ગુજ૧૨", "digits");
        assert_eq!(gu("hyphen"), "hyphen");
        assert_eq!(gu("हिन्दी"), "हिन्दी", "Devanagari is not Gujarati");
        assert!(points(Script::Gujarati, &[]).is_empty());
    }

    /// The same rules on Devanagari, ready for Hindi: conjuncts, nukta (combining and in a
    /// precomposed consonant) and the anusvara.
    #[test]
    fn devanagari_uses_the_same_rules() {
        assert_eq!(hi("हिन्दी"), "हि-न्दी");
        assert_eq!(hi("विद्यार्थी"), "वि-द्या-र्थी");
        assert_eq!(hi("संस्कृत"), "सं-स्कृ-त");
        assert_eq!(hi("ज\u{093C}रूरी"), "ज\u{093C}-रू-री");
        assert_eq!(hi("\u{095B}रूरी"), "\u{095B}-रू-री");
        assert_eq!(hi("ગુજરાતી"), "ગુજરાતી", "Gujarati is not Devanagari");
    }

    /// CodeRabbit on craft-libs#3901: a joiner after a virama stays inside its conjunct, the break
    /// before the conjunct remains, and no break goes next to a joiner.
    #[test]
    fn joiners_stay_inside_the_akshara() {
        assert_eq!(gu("પુસ્\u{200D}તક"), "પુ-સ્\u{200D}ત-ક");
        assert_eq!(gu("પુસ્\u{200C}તક"), "પુ-સ્\u{200C}ત-ક");
        assert_eq!(gu("વાક્\u{200D}"), "વાક્\u{200D}", "still a dead consonant");
        assert_eq!(gu("ક\u{200D}ખ"), "ક\u{200D}ખ", "a joiner outside a conjunct");
        assert_eq!(hi("क्\u{200D}षमा"), "क्\u{200D}ष-मा");
    }

    /// Gujarati and Hindi are mapped to their scripts, other languages to none.
    #[test]
    fn languages_map_to_their_script() {
        assert_eq!(Script::of(ProofLang::Gu), Some(Script::Gujarati));
        assert_eq!(Script::of(ProofLang::Hi), Some(Script::Devanagari));
        assert_eq!(Script::of(ProofLang::En), None);
        assert_eq!(Script::of(ProofLang::Bg), None);
    }
}
