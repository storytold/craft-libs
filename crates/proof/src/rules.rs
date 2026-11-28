//! Stable identifiers of the grammar rules.
//!
//! Every grammar [`Issue`](crate::Issue) carries the id of the rule that found it in
//! [`Issue::rule`](crate::Issue::rule), so a host can keep the rules that suit its text (a label, a
//! caption) and leave out the others (the capital after a sentence's end) without comparing messages,
//! which are written in each language. An id is lower-case words joined by `-`, it is the same
//! wherever the same rule applies in more than one language (`repeated-word`,
//! `space-before-punctuation`), and **it is never renamed or reused**: a rule that changes its mind about
//! what it flags keeps its id, and a new rule takes a new one.
//!
//! [`grammar_rules`] lists the ids a language can report, and the README has the same table. Spelling
//! issues ([`IssueKind::Spelling`](crate::IssueKind::Spelling) and
//! [`IssueKind::Repeated`](crate::IssueKind::Repeated)) have no rule.

use crate::ProofLang;

/// A word written twice in a row (`the the`), except what a language repeats on purpose.
pub const REPEATED_WORD: &str = "repeated-word";
/// A space before `,` `.` `;` and the like (`word ,`), where the language puts none.
pub const SPACE_BEFORE_PUNCTUATION: &str = "space-before-punctuation";
/// Two or more spaces in a row.
pub const DOUBLE_SPACE: &str = "double-space";
/// A lower-case word that begins a sentence, or the paragraph.
pub const SENTENCE_CAPITAL: &str = "sentence-capital";
/// The same punctuation mark twice (`,,`, `..`).
pub const DOUBLED_PUNCTUATION: &str = "doubled-punctuation";
/// A comma missing before a word that opens a clause.
pub const MISSING_COMMA: &str = "missing-comma";
/// English: `a` before a vowel sound or `an` before a consonant sound.
pub const A_AN: &str = "a-an";
/// Polish: `w` and `z` that need their long forms `we` and `ze`.
pub const WE_ZE: &str = "we-ze";
/// Spanish: a `?` or `!` that closes a sentence with no `¿` or `¡` opening it.
pub const OPENING_MARKS: &str = "opening-marks";
/// Spanish: a space after `¿` or `¡`.
pub const SPACE_AFTER_OPENING_MARK: &str = "space-after-opening-mark";
/// French: the non-breaking space missing before `;` `:` `!` `?`.
pub const HIGH_PUNCTUATION: &str = "high-punctuation";
/// French: the non-breaking space missing inside `«` and `»`.
pub const GUILLEMETS: &str = "guillemets";
/// French: straight quotes (`"`) where `«` and `»` belong.
pub const STRAIGHT_QUOTES: &str = "straight-quotes";
/// French: the elision a word asks for (`le arbre` → `l’arbre`).
pub const ELISION: &str = "elision";
/// Italian: `po'` written with an accent or without its apostrophe.
pub const PO_APOSTROPHE: &str = "po-apostrophe";
/// Italian: `qual è` written with an apostrophe.
pub const QUAL_E: &str = "qual-e";
/// Italian: `è` written `e'`.
pub const E_APOSTROPHE: &str = "e-apostrophe";
/// Italian: `un'` before a masculine noun.
pub const UN_APOSTROPHE_MASCULINE: &str = "un-apostrophe-masculine";
/// Italian: `un` before a feminine noun that begins with a vowel, which wants `un'`.
pub const UN_APOSTROPHE_FEMININE: &str = "un-apostrophe-feminine";
/// Turkish: a comma before a conjunction that takes none (`ve`, `veya`…).
pub const COMMA_BEFORE_CONJUNCTION: &str = "comma-before-conjunction";
/// Turkish: a comma after a conjunction that takes none.
pub const COMMA_AFTER_CONJUNCTION: &str = "comma-after-conjunction";

const ENGLISH: &[&str] = &[REPEATED_WORD, A_AN, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE, SENTENCE_CAPITAL];
const POLISH: &[&str] = &[REPEATED_WORD, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE, SENTENCE_CAPITAL, MISSING_COMMA, WE_ZE, DOUBLED_PUNCTUATION];
const PORTUGUESE: &[&str] = &[REPEATED_WORD, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE, SENTENCE_CAPITAL, DOUBLED_PUNCTUATION];
const ITALIAN: &[&str] = &[
    REPEATED_WORD,
    SPACE_BEFORE_PUNCTUATION,
    DOUBLE_SPACE,
    SENTENCE_CAPITAL,
    DOUBLED_PUNCTUATION,
    PO_APOSTROPHE,
    QUAL_E,
    E_APOSTROPHE,
    UN_APOSTROPHE_MASCULINE,
    UN_APOSTROPHE_FEMININE,
];
const CROATIAN: &[&str] = &[REPEATED_WORD, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE, SENTENCE_CAPITAL, DOUBLED_PUNCTUATION];
const TURKISH: &[&str] =
    &[REPEATED_WORD, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE, COMMA_BEFORE_CONJUNCTION, COMMA_AFTER_CONJUNCTION, SENTENCE_CAPITAL];
const RUSSIAN: &[&str] = &[REPEATED_WORD, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE, SENTENCE_CAPITAL, MISSING_COMMA, DOUBLED_PUNCTUATION];
const CZECH: &[&str] = &[REPEATED_WORD, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE];
const INDONESIAN: &[&str] = &[REPEATED_WORD, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE];
const SPANISH: &[&str] = &[REPEATED_WORD, SPACE_AFTER_OPENING_MARK, SPACE_BEFORE_PUNCTUATION, DOUBLE_SPACE, OPENING_MARKS];
const FRENCH: &[&str] = &[HIGH_PUNCTUATION, GUILLEMETS, STRAIGHT_QUOTES, REPEATED_WORD, SENTENCE_CAPITAL, ELISION];

/// The ids of the grammar rules that can report an issue in text of `lang`, in the order the
/// README lists them. Empty for a language with no grammar rules (German, Bulgarian, any
/// [`ProofLang::Pack`] language but French and the regional Portuguese, and
/// [`ProofLang::Other`]). A region shares its language's rules: every English has the English ones,
/// every Spanish the Spanish ones, `pt-PT` the Portuguese ones and every French locale the French
/// ones.
pub fn grammar_rules(lang: ProofLang) -> &'static [&'static str] {
    if lang.is_english() {
        return ENGLISH;
    }
    if lang.pack_locale().is_some_and(|l| l.primary() == "fr") {
        return FRENCH;
    }
    let lang = lang.built_in();
    if lang.is_spanish() {
        return SPANISH;
    }
    match lang {
        ProofLang::Pl => POLISH,
        ProofLang::Pt => PORTUGUESE,
        ProofLang::It => ITALIAN,
        ProofLang::Hr => CROATIAN,
        ProofLang::Tr => TURKISH,
        ProofLang::Ru => RUSSIAN,
        ProofLang::Cs => CZECH,
        ProofLang::Id => INDONESIAN,
        _ => &[],
    }
}

#[cfg(test)]
mod tests;
