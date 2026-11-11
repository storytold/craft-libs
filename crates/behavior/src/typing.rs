//! `TypingRun<C>`: the draft, rejected text, commit, cancel, escape rule and limits of a text part (18.2).

use crate::codec::{Codec, NotAValue};
use crate::value::GestureId;

/// The state a [`TypingRun`] keeps per text part.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EditRun<V> {
    /// The value when the run began; relative input and a cancel use it.
    pub start: V,
    /// The text being typed; `None` while no run is open.
    pub draft: Option<String>,
    /// The draft is not a value of the codec.
    pub error: Option<NotAValue>,
    /// The last text a commit refused and the value it left in place.
    pub rejected: Option<(String, V)>,
    /// An IME composition is under way in the part.
    pub composing: bool,
    /// The gesture the run is, for the undo step it names.
    pub gesture: GestureId,
}

impl<V: Clone> EditRun<V> {
    /// No run is open; `start` is the value shown.
    pub fn new(start: V) -> Self {
        EditRun { start, draft: None, error: None, rejected: None, composing: false, gesture: GestureId::default() }
    }

    /// A run is open.
    pub fn is_open(&self) -> bool {
        self.draft.is_some()
    }
}

/// What Escape does to a draft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum EscapeRule {
    /// Drop the draft and show the value from before the run.
    #[default]
    Restore,
    /// Escape clears the text and commits the empty text; it never restores. A host passes Escape on only while
    /// the text is already empty (a search field).
    Clear,
}

/// What typing did to the run.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Typed<V> {
    /// The draft changed; no value follows from it yet.
    Draft,
    /// The draft parsed and applies as typed.
    Apply(V),
}

/// How a commit ended.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Ended<V> {
    /// The draft parsed to this value and the run closed.
    Commit(V),
    /// The draft is not a value; the run stays open.
    Rejected,
    /// No run was open.
    Idle,
}

/// What Escape did.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Escaped<V> {
    /// The draft was cleared and the run closed on the empty text's value.
    Cleared(V),
    /// The run closed; the value from before it comes back.
    Restore(V),
    /// No run was open.
    Idle,
}

/// The behaviour of a text part over a codec.
#[derive(Clone, Debug)]
pub struct TypingRun<C: Codec> {
    codec: C,
    live: bool,
    limit: Option<usize>,
    escape: EscapeRule,
}

impl<C: Codec> TypingRun<C> {
    /// A run that applies a draft as typed when the codec says so, with no length limit.
    pub fn new(codec: C) -> Self {
        TypingRun { codec, live: false, limit: None, escape: EscapeRule::Restore }
    }

    /// Whether every parsable draft applies as typed, whatever the codec says.
    #[must_use]
    pub fn live(mut self, live: bool) -> Self {
        self.live = live;
        self
    }

    /// The most characters a draft holds.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// What Escape does.
    #[must_use]
    pub fn escape_rule(mut self, rule: EscapeRule) -> Self {
        self.escape = rule;
        self
    }

    /// The codec.
    pub fn codec(&self) -> &C {
        &self.codec
    }

    /// The text to show: the draft, else the value's text.
    pub fn shown(&self, run: &EditRun<C::Value>, value: &C::Value) -> String {
        run.draft.clone().unwrap_or_else(|| self.codec.format(value))
    }

    /// Opens the run on `start` when none is open.
    pub fn open(&self, run: &mut EditRun<C::Value>, start: &C::Value) {
        if !run.is_open() {
            run.start = start.clone();
            run.draft = Some(self.codec.format(start));
            run.error = None;
            run.rejected = None;
        }
    }

    /// The draft is now `text`: characters the codec refuses and characters past the limit drop out.
    pub fn edit(&self, run: &mut EditRun<C::Value>, start: &C::Value, text: &str) -> Typed<C::Value> {
        if !run.is_open() {
            run.start = start.clone();
        }
        let mut draft: String = text.chars().filter(|c| self.codec.filter(*c)).collect();
        if let Some(limit) = self.limit
            && let Some((at, _)) = draft.char_indices().nth(limit)
        {
            draft.truncate(at);
        }
        run.rejected = None;
        let parsed = self.codec.parse(&draft, &run.start);
        run.error = parsed.as_ref().err().copied();
        let applies = C::DRAFTS_ARE_VALUES || self.live || self.codec.applies_as_typed(&draft);
        let typed = match parsed {
            Ok(value) if applies => Typed::Apply(value),
            _ => Typed::Draft,
        };
        run.draft = Some(draft);
        typed
    }

    /// Commits the draft: a value closes the run, a draft that is no value keeps it open.
    pub fn commit(&self, run: &mut EditRun<C::Value>) -> Ended<C::Value> {
        self.commit_tagged(run).0
    }

    /// [`commit`](Self::commit), with the tag of the unit the committed text named (`Codec::parse_tagged`).
    pub fn commit_tagged(&self, run: &mut EditRun<C::Value>) -> (Ended<C::Value>, Option<u32>) {
        let Some(draft) = run.draft.as_deref() else { return (Ended::Idle, None) };
        match self.codec.parse_tagged(draft, &run.start) {
            Ok((value, tag)) => {
                run.draft = None;
                run.error = None;
                (Ended::Commit(value), tag)
            }
            Err(error) => {
                run.error = Some(error);
                run.rejected = Some((draft.to_owned(), run.start.clone()));
                (Ended::Rejected, None)
            }
        }
    }

    /// Closes the run without a commit and returns the value from before it; `None` when no run was open.
    pub fn cancel(&self, run: &mut EditRun<C::Value>) -> Option<C::Value> {
        run.draft.take()?;
        run.error = None;
        Some(run.start.clone())
    }

    /// Applies the escape rule. A codec that refuses the empty text keeps the cleared draft open under
    /// [`EscapeRule::Clear`].
    pub fn escape(&self, run: &mut EditRun<C::Value>) -> Escaped<C::Value> {
        match (&run.draft, self.escape) {
            (None, _) => Escaped::Idle,
            (Some(_), EscapeRule::Clear) => {
                run.draft = Some(String::new());
                match self.commit(run) {
                    Ended::Commit(value) => Escaped::Cleared(value),
                    _ => Escaped::Idle,
                }
            }
            _ => self.cancel(run).map_or(Escaped::Idle, Escaped::Restore),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::NumberCodec;

    #[test]
    fn a_number_run_applies_on_commit_rejects_bad_text_and_restores_on_escape() {
        let typing = TypingRun::new(NumberCodec::default());
        let mut run = EditRun::new(Some(1.0));
        assert_eq!(typing.edit(&mut run, &Some(1.0), "2+x"), Typed::Draft);
        assert_eq!(run.error, Some(NotAValue));
        assert_eq!(typing.commit(&mut run), Ended::Rejected, "the run stays open on a refused commit");
        assert!(run.is_open() && run.rejected.is_some());
        assert_eq!(typing.edit(&mut run, &Some(1.0), "2+3"), Typed::Draft, "a number applies on commit");
        assert_eq!(typing.commit(&mut run), Ended::Commit(Some(5.0)));
        assert_eq!(typing.commit(&mut run), Ended::Idle);
        typing.edit(&mut run, &Some(5.0), "9");
        assert_eq!(typing.escape(&mut run), Escaped::Restore(Some(5.0)));
    }
}
