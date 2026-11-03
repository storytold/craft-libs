//! Serbian, written in Cyrillic and in Latin script, letter for letter: what hyphenation needs to
//! treat both alike ([`crate::hyphen`]).
//!
//! - **Latin through Cyrillic.** The TeX Serbian patterns (by Dejan Muhamedagić) are Cyrillic.
//!   A Latin word is hyphenated through its Cyrillic spelling ([`cyrillic`]) and the breaks are
//!   mapped back, so `uni-ver-zi-tet` and `уни-вер-зи-тет` break at the same letters and `lj`,
//!   `nj` and `dž` are never split where they are one letter.
//! - **Two letters.** Where `nj` or `dž` is two letters, the Latin spelling doesn't show it, the
//!   Cyrillic does (конјугација, not коњугација; надживети, not наџивети): a short list of roots
//!   ([`two_letters`]) says so, so `konjugacija` breaks as `kon-ju-ga-ci-ja` while `kanjon` and
//!   `odžak` keep one letter.
//! - **Word limits.** `lj`, `nj` and `dž` count as one letter (Pravopis srpskoga jezika, §186г),
//!   so `njive` stays whole as `њиве` does ([`second_letters`]).
//! - **Compounds** break at the join (§186е): `пред-услов`, never `преду-слов`
//!   ([`compound_breaks`]).

use crate::lang::ProofLang;

/// Is `lang` Serbian, in either script (`sr`, `sr-latn`, `sr-me`; `sh`, the code Hunspell's
/// Serbian Latin files use; ISO 639-2 `srp`)?
pub(crate) fn is_serbian(lang: ProofLang) -> bool {
    lang.pack_locale().is_some_and(|l| matches!(l.primary(), "sr" | "sh" | "srp"))
}

/// The Cyrillic Serbian language, whose pack holds the Cyrillic patterns.
pub(crate) fn cyrillic_lang() -> ProofLang {
    ProofLang::from_tag(Some("sr"))
}

/// Roots of Serbian Latin words where `nj` or `dž` is two letters, not one. `|` marks the pair.
/// These only start a word: `an|jon` is not in `kanjon`, nor `od|žal` in `odžak`. Checked against
/// the ~190,000 stems of the Serbian Hunspell dictionaries: they touch no word with one letter but
/// the variants коњугација and коњунктив, which break as the words they vary. A list, not a
/// dictionary lookup, so that a text breaks the same on every machine. The headwords of the Rečnik
/// srpskoga jezika (Matica srpska, 2011) raise no other case. A root stays long enough to leave
/// foreign names alone (Поџи).
const TWO_LETTERS_AT_START: &[&str] = &[
    "an|jon",
    "and|žej",
    "feld|žandarm",
    "man|jogan",
    "nad|žet",
    "nad|živ",
    "nad|žnj",
    "nad|žup",
    "od|žali",
    "od|žalj",
    "od|žar",
    "od|živ",
    "pan|jug",
    "pod|žanr",
    "pod|žar",
    "pod|ždrel",
    "pod|žel",
    "pod|žup",
    "pred|žalb",
    "pred|žel",
    "pred|žet",
    "pred|živ",
    "šen|jan",
    "šen|jeng",
    "tan|jug",
    "van|jed",
    "van|jez",
    "van|jugo",
];

/// The same, anywhere in a word (`mikroin|jekcija`, `hiperkon|jugacija`).
const TWO_LETTERS_ANYWHERE: &[&str] = &["in|jek", "in|junk", "kon|jekt", "kon|jug", "kon|juk", "kon|junk", "kon|junt"];

/// The index of the second letter of each such pair in a lowercase Serbian Latin word.
pub(crate) fn two_letters(word: &[char]) -> Vec<usize> {
    let is_at = |root: &str, at: usize| root.chars().filter(|&c| c != '|').enumerate().all(|(i, c)| word.get(at + i) == Some(&c));
    let mark = |root: &str| root.chars().position(|c| c == '|').unwrap_or(0);
    let mut out: Vec<usize> = TWO_LETTERS_AT_START.iter().filter(|r| is_at(r, 0)).map(|r| mark(r)).collect();
    for at in 0..word.len() {
        out.extend(TWO_LETTERS_ANYWHERE.iter().filter(|r| is_at(r, at)).map(|r| at + mark(r)));
    }
    out
}

/// Is `core` written in Serbian Latin (any Latin letter)?
pub(crate) fn is_latin(core: &[char]) -> bool {
    core.iter().any(|c| c.is_ascii_alphabetic() || "čćđšž".contains(*c))
}

/// A lowercase Serbian Latin word in Cyrillic, letter for letter (`lj`, `nj`, `dž` are one letter
/// each: љ, њ, џ, unless [`two_letters`] says otherwise), with the Latin index each Cyrillic letter
/// starts at. None when the word has a letter outside the Serbian Latin alphabet (`q`, `w`, `x`,
/// `y`, an apostrophe).
pub(crate) fn cyrillic(word: &[char]) -> Option<(Vec<char>, Vec<usize>)> {
    let two = two_letters(word);
    let mut out = Vec::with_capacity(word.len());
    let mut at = Vec::with_capacity(word.len());
    let mut i = 0;
    while let Some(&c) = word.get(i) {
        let next = word.get(i + 1).copied();
        let one_letter = !two.contains(&(i + 1));
        let (cyr, len) = match (c, next) {
            ('l', Some('j')) if one_letter => ('љ', 2),
            ('n', Some('j')) if one_letter => ('њ', 2),
            ('d', Some('ž')) if one_letter => ('џ', 2),
            _ => (
                match c {
                    'a' => 'а',
                    'b' => 'б',
                    'c' => 'ц',
                    'č' => 'ч',
                    'ć' => 'ћ',
                    'd' => 'д',
                    'đ' => 'ђ',
                    'e' => 'е',
                    'f' => 'ф',
                    'g' => 'г',
                    'h' => 'х',
                    'i' => 'и',
                    'j' => 'ј',
                    'k' => 'к',
                    'l' => 'л',
                    'm' => 'м',
                    'n' => 'н',
                    'o' => 'о',
                    'p' => 'п',
                    'r' => 'р',
                    's' => 'с',
                    'š' => 'ш',
                    't' => 'т',
                    'u' => 'у',
                    'v' => 'в',
                    'z' => 'з',
                    'ž' => 'ж',
                    _ => return None,
                },
                1,
            ),
        };
        out.push(cyr);
        at.push(i);
        i += len;
    }
    Some((out, at))
}

/// For each char of a lowercase Serbian `core`: is it the second char of a one-letter `lj`, `nj`
/// or `dž`? Those aren't counted against the word limits. All false for a Cyrillic word.
pub(crate) fn second_letters(core: &[char]) -> Vec<bool> {
    let mut out = vec![false; core.len()];
    if let Some((_, at)) = cyrillic(core) {
        let mut starts = at.iter().peekable();
        for (i, second) in out.iter_mut().enumerate() {
            *second = starts.next_if(|&&s| s == i).is_none();
        }
    }
    out
}

/// Compounds the patterns don't see, in Cyrillic: `|` is where the prefix ends. Pravopis srpskoga
/// jezika, §186е: a transparent compound breaks at the join.
const COMPOUNDS: &[&str] = &["над|индивид", "над|истор", "под|одбор", "под|одељ", "под|одјељ", "под|оцењ", "под|оцен", "пред|инфаркт"];

/// Prefixes that keep their join before a vowel.
const PREFIXES: &[&str] = &["без", "из", "над", "об", "обез", "од", "под", "пред", "раз"];

/// `pts` of a lowercase Serbian `core`, with the join of a compound as §186е has it: `пред-услов`,
/// never `преду-слов`, though the patterns allow both. A vowel alone between two breaks after a
/// prefix belongs to the second word, so the break after it goes. In a Latin word, a break inside
/// a one-letter `lj`, `nj` or `dž` goes too.
pub(crate) fn compound_breaks(core: &[char], pts: Vec<usize>) -> Vec<usize> {
    let (word, at) = if is_latin(core) {
        match cyrillic(core) {
            Some(x) => x,
            None => return pts,
        }
    } else {
        (core.to_vec(), (0..core.len()).collect())
    };
    // The letter starting at each char of `core`, so each break is looked up once.
    let mut letter = vec![None; core.len()];
    for (k, &a) in at.iter().enumerate() {
        if let Some(l) = letter.get_mut(a) {
            *l = Some(k);
        }
    }
    let mut marks: Vec<usize> = pts.iter().filter_map(|&p| letter.get(p).copied().flatten()).collect();
    let starts = |s: &str, from: usize| s.chars().enumerate().all(|(i, c)| word.get(from + i) == Some(&c));
    for root in COMPOUNDS {
        let (prefix, rest) = root.split_once('|').unwrap_or((root, ""));
        let n = prefix.chars().count();
        if starts(prefix, 0) && starts(rest, n) {
            marks.retain(|&p| p > n);
            marks.push(n);
        }
    }
    for prefix in PREFIXES {
        let n = prefix.chars().count();
        if starts(prefix, 0) && word.get(n).is_some_and(|c| "аеиоу".contains(*c)) && marks.contains(&n) {
            marks.retain(|&p| p != n + 1);
        }
    }
    marks.sort_unstable();
    marks.dedup();
    marks.into_iter().filter_map(|p| at.get(p).copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn latin_spells_in_cyrillic() {
        let (cyr, at) = cyrillic(&chars("džunglanja")).unwrap();
        assert_eq!(cyr.iter().collect::<String>(), "џунглања");
        assert_eq!(at, vec![0, 2, 3, 4, 5, 6, 7, 9]);
        assert!(cyrillic(&['w']).is_none());
        assert!(cyrillic(&chars("don't")).is_none());
    }

    /// Pairs the Latin spelling hides: Cyrillic has нј and дж, not њ and џ.
    #[test]
    fn a_pair_of_two_letters_spells_as_two() {
        for (latin, cyr) in [
            ("konjugacija", "конјугација"),
            ("nadživeti", "надживети"),
            ("injekcija", "инјекција"),
            ("mikroinjekcija", "микроинјекција"),
            ("vanjezički", "ванјезички"),
            ("panjugoslovenski", "панјугословенски"),
            ("podžanr", "поджанр"),
            ("tanjug", "танјуг"),
            ("podželudačni", "поджелудачни"),
            // A real digraph later in the word stays one letter.
            ("nadživljen", "надживљен"),
            ("odžaljen", "оджаљен"),
        ] {
            assert_eq!(cyrillic(&chars(latin)).unwrap().0.iter().collect::<String>(), cyr, "{latin}");
        }
    }

    /// Words that look like those roots, where the pair is one letter.
    #[test]
    fn a_pair_of_one_letter_spells_as_one() {
        for (latin, cyr) in [
            ("kanjon", "кањон"),
            ("konj", "коњ"),
            ("tanjur", "тањур"),
            ("odžak", "оџак"),
            ("nadžak", "наџак"),
            ("budžet", "буџет"),
            ("podži", "поџи"),
        ] {
            assert_eq!(cyrillic(&chars(latin)).unwrap().0.iter().collect::<String>(), cyr, "{latin}");
        }
    }

    #[test]
    fn second_letters_of_digraphs() {
        assert_eq!(second_letters(&chars("njive")), [false, true, false, false, false]);
        assert_eq!(second_letters(&chars("konjugacija")), vec![false; 11]);
        assert_eq!(second_letters(&chars("њиве")), vec![false; 4]);
    }

    /// Pravopis, §186е: a transparent compound breaks at the join, in either script.
    #[test]
    fn compounds_break_at_the_join() {
        // Breaks after every letter but the last, as patterns that allow everything give them.
        let all = |w: &str| (1..w.chars().count()).collect::<Vec<_>>();
        assert!(!compound_breaks(&chars("предуслов"), all("предуслов")).contains(&5), "пред-у-слов loses у-");
        assert!(!compound_breaks(&chars("preduslov"), all("preduslov")).contains(&5));
        assert_eq!(
            compound_breaks(&chars("пододбор"), vec![2, 4, 5]),
            [3, 5],
            "под-од-бор: the join replaces по-, and о alone goes with the second word"
        );
        // Not a prefix before a vowel: nothing changes.
        assert_eq!(compound_breaks(&chars("разорити"), vec![2, 4, 6]), [2, 4, 6]);
        // A break inside a one-letter digraph goes.
        assert_eq!(compound_breaks(&chars("konj"), vec![2, 3]), [2]);
    }
}
