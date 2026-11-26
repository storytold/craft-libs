//! [`LibraryPrompt`]: the question before Clear All Actions, Reset Actions… and Replace Actions….

use craft_ui::dialog::{PromptChoice, StandardPrompt};
use craft_ui::runtime::words::CommonWords;

use super::words::ActionsWords;

/// Which whole-list command a [`StandardPrompt`] from [`prompt`](LibraryPrompt::prompt) asks
/// about. The prompt carries it as its tag, so the answer says which command it confirms
/// (`PromptAnswer::tag::<LibraryPrompt>()`).
///
/// | Command | Question | Answers |
/// |---|---|---|
/// | Clear All | "Clear all actions?" | OK (`Proceed`), Cancel |
/// | Reset | "Replace the current actions?" | OK (`Proceed`: in place of the list), Append (`Alternate`: added to it), Cancel |
/// | Replace | "Replace all actions with {count} actions from {file}?" | OK (`Proceed`), Cancel |
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LibraryPrompt {
    /// Clear All Actions.
    ClearAll,
    /// Reset Actions….
    Reset,
    /// Replace Actions… with the file the app read.
    #[non_exhaustive]
    Replace {
        /// How many actions the file holds.
        count: usize,
        /// The file's display name.
        file: String,
    },
}

impl LibraryPrompt {
    /// Replace Actions… with `count` actions from the file named `file`.
    pub fn replace(count: usize, file: impl Into<String>) -> Self {
        LibraryPrompt::Replace { count, file: file.into() }
    }

    /// The question for the app's dialog host, tagged with `self`.
    pub fn prompt(&self, words: &ActionsWords, common: &CommonWords) -> StandardPrompt {
        let (title, message) = match self {
            LibraryPrompt::ClearAll => (words.clear_all_title.clone(), words.clear_all_question.clone()),
            LibraryPrompt::Reset => (words.reset_title.clone(), words.reset_question.clone()),
            LibraryPrompt::Replace { count, file } => {
                (words.replace_title.clone(), words.replace_question.fill(&[("count", count.to_string().as_str()), ("file", file.as_str())]).into())
            }
        };
        let mut prompt = StandardPrompt::custom(message).title(title).button(PromptChoice::Proceed, common.ok.clone());
        if *self == LibraryPrompt::Reset {
            prompt = prompt.button(PromptChoice::Alternate, words.append.clone());
        }
        prompt.button(PromptChoice::Cancel, common.cancel.clone()).tag(self.clone())
    }
}
