//! Find (N3.03): [`FindBar`], Acrobat's Cmd+F bar over a document (query, "3 of 12", previous and next, an options
//! menu with Whole words only and Case-sensitive, close), and [`FindReplaceDialog`], Word's Find and Replace (Find,
//! Replace and Go To tabs, Search Options, Replace and Replace All) and PowerPoint's simpler Find / Replace, on the
//! dialog stack. Both search with [`crate::find`]; the document's text and the replacing are the app's.

#[cfg(feature = "screen-testing")]
mod stories;

mod bar;
mod dialog;

pub use bar::{FindBar, FindBarOutput, FindBarState, FindStep};
pub use dialog::{FindReplaceDialog, FindRequest, FindTab};

use crate::find::{FindError, MAX_COUNT, MAX_PATTERN, RegexError, WildcardError};

craft_ui::words! {
    /// The words of the find bar and the Find and Replace dialog.
    #[non_exhaustive]
    pub struct FindLabels {
        find_text: "find.findText" = "Find text";
        searching: template "find.searching" = "Searching… {s}/{p}";
        no_matches: "find.noMatches" = "No matches";
        count: template "find.count" = "{c} of {n}{more}";
        options: "find.options" = "Find options";
        whole_words_only: "find.wholeWordsOnly" = "Whole words only";
        case_sensitive: "find.caseSensitive" = "Case-sensitive";
        close: "find.close" = "Close (Esc)";
        title: "find.title" = "Find and Replace";
        find_tab: "find.findTab" = "Find";
        replace_tab: "find.replaceTab" = "Replace";
        go_to_tab: "find.goToTab" = "Go To";
        find_what: "find.findWhat" = "Find what:";
        replace_with: "find.replaceWith" = "Replace with:";
        match_case: "find.matchCase" = "Match case";
        whole_words: "find.wholeWords" = "Whole words";
        use_wildcards: "find.useWildcards" = "Use wildcards";
        search_options: "find.searchOptions" = "Search Options";
        match_prefix: "find.matchPrefix" = "Match prefix";
        match_suffix: "find.matchSuffix" = "Match suffix";
        ignore_punctuation: "find.ignorePunctuation" = "Ignore punctuation characters";
        ignore_space: "find.ignoreSpace" = "Ignore white-space characters";
        regular_expressions: "find.regex" = "Regular expressions";
        special: "find.special" = "Special";
        find_next: "find.findNext" = "Find Next";
        replace: "find.replace" = "Replace";
        replace_more: "find.replaceMore" = "Replace…";
        replace_all: "find.replaceAll" = "Replace All";
        replaced: template "find.replaced" = "All done. We made {n} replacements.";
        not_found: template "find.notFound" = "{app} can't find “{text}”.";
        go_to_what: "find.goToWhat" = "Go to what:";
        go_to: "find.goTo" = "Go To";
        clear: "find.clear" = "Clear";
        close_button: "find.closeButton" = "Close";
        highlight_all: "find.highlightAll" = "Highlight All";
        clear_highlight: "find.clearHighlight" = "Clear Highlighting";
        paragraph_mark: "find.paragraphMark" = "Paragraph Mark";
        tab_character: "find.tabCharacter" = "Tab Character";
        any_character: "find.anyCharacter" = "Any Character";
        any_digit: "find.anyDigit" = "Any Digit";
        any_letter: "find.anyLetter" = "Any Letter";
        caret_character: "find.caretCharacter" = "Caret Character";
        em_dash: "find.emDash" = "Em Dash";
        en_dash: "find.enDash" = "En Dash";
        nonbreaking_hyphen: "find.nonbreakingHyphen" = "Nonbreaking Hyphen";
        nonbreaking_space: "find.nonbreakingSpace" = "Nonbreaking Space";
        optional_hyphen: "find.optionalHyphen" = "Optional Hyphen";
        white_space: "find.whiteSpace" = "White Space";
        section_break: "find.sectionBreak" = "Section Break";
        line_break: "find.lineBreak" = "Manual Line Break";
        found_text: "find.foundText" = "Find What Text";
        clipboard_contents: "find.clipboardContents" = "Clipboard Contents";
        special_item: template "find.specialItem" = "{name}  {code}";
        error_too_long: template "find.errorTooLong" = "The Find what text is longer than {max} characters";
        wildcard_unclosed_brace: "find.wildcardUnclosedBrace" = "An opening brace needs a closing brace";
        wildcard_unclosed_bracket: "find.wildcardUnclosedBracket" = "An opening bracket needs a closing bracket";
        wildcard_back_reference: "find.wildcardBackReference" = "Back-references (\\1) work in Replace with, not in Find what";
        wildcard_bad_count: template "find.wildcardBadCount" = "{count} is not a valid count";
        wildcard_count_too_large: template "find.wildcardCountTooLarge" = "Counts go up to {max}";
        wildcard_empty_class: "find.wildcardEmptyClass" = "[ ] needs at least one character";
        wildcard_reversed_range: template "find.wildcardReversedRange" = "[{from}-{to}] is not in ascending order";
        wildcard_refused: "find.wildcardRefused" = "This wildcard pattern can't be searched for";
        regex_unclosed_group: "find.regexUnclosedGroup" = "A ( needs a closing )";
        regex_unopened_group: "find.regexUnopenedGroup" = "A ) has no ( before it";
        regex_unclosed_class: "find.regexUnclosedClass" = "A [ needs a closing ]";
        regex_nothing_to_repeat: "find.regexNothingToRepeat" = "Nothing before the repeat (*, +, ? or a count)";
        regex_bad_repetition: "find.regexBadRepetition" = "A repeat count is a number, a number and a comma, or two numbers with the first not above the second";
        regex_reversed_range: "find.regexReversedRange" = "A range of a [ ] class must go up, as in [a-z]";
        regex_bad_escape: "find.regexBadEscape" = "A \\ must be followed by a character it can escape";
        regex_bad_group_name: "find.regexBadGroupName" = "A group name must be a word, used once";
        regex_unsupported: "find.regexUnsupported" = "Back-references and look-around are not supported";
        regex_bad_flag: "find.regexBadFlag" = "This flag is not supported";
        regex_too_big: "find.regexTooBig" = "The expression is too big";
        regex_invalid: "find.regexInvalid" = "This is not a valid regular expression";
    }
}
impl FindLabels {
    /// What to tell the user of a Find what text the engine refused, in these words (the app shows it under the
    /// field or through [`FindReplaceDialog::set_message`]).
    pub fn error(&self, error: &FindError) -> String {
        match error {
            FindError::TooLong => self.error_too_long.fill(&[("max", &MAX_PATTERN.to_string())]),
            FindError::Wildcards(kind) => match kind {
                WildcardError::UnclosedBrace => self.wildcard_unclosed_brace.as_str().to_owned(),
                WildcardError::UnclosedBracket => self.wildcard_unclosed_bracket.as_str().to_owned(),
                WildcardError::BackReference => self.wildcard_back_reference.as_str().to_owned(),
                WildcardError::BadCount(body) => self.wildcard_bad_count.fill(&[("count", &["{", body, "}"].concat())]),
                WildcardError::CountTooLarge => self.wildcard_count_too_large.fill(&[("max", &MAX_COUNT.to_string())]),
                WildcardError::EmptyClass => self.wildcard_empty_class.as_str().to_owned(),
                WildcardError::ReversedRange { from, to } => {
                    self.wildcard_reversed_range.fill(&[("from", &from.to_string()), ("to", &to.to_string())])
                }
                _ => self.wildcard_refused.as_str().to_owned(),
            },
            FindError::Regex(kind) => match kind {
                RegexError::UnclosedGroup => self.regex_unclosed_group.as_str().to_owned(),
                RegexError::UnopenedGroup => self.regex_unopened_group.as_str().to_owned(),
                RegexError::UnclosedClass => self.regex_unclosed_class.as_str().to_owned(),
                RegexError::NothingToRepeat => self.regex_nothing_to_repeat.as_str().to_owned(),
                RegexError::BadRepetition => self.regex_bad_repetition.as_str().to_owned(),
                RegexError::ReversedRange => self.regex_reversed_range.as_str().to_owned(),
                RegexError::BadEscape => self.regex_bad_escape.as_str().to_owned(),
                RegexError::BadGroupName => self.regex_bad_group_name.as_str().to_owned(),
                RegexError::Unsupported => self.regex_unsupported.as_str().to_owned(),
                RegexError::BadFlag => self.regex_bad_flag.as_str().to_owned(),
                RegexError::TooBig => self.regex_too_big.as_str().to_owned(),
                _ => self.regex_invalid.as_str().to_owned(),
            },
        }
    }
}
