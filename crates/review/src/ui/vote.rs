//! The voting pill on a board.

use craft_theme::variant::Variant;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use crate::collab::{CollabRequest, CollabTransport, VoteScope};

#[cfg(feature = "screen-testing")]
mod stories;

/// Votes per person a new vote starts with.
const DEFAULT_VOTES: u32 = 3;
/// Most votes per person.
const MAX_VOTES: u32 = 99;

craft_ui::words! {
    /// The vote's words, English by default.
    #[non_exhaustive]
    pub struct VoteWords {
        voting_session: "vote.session" = "Voting session";
        votes_per_person: "vote.perPerson" = "Votes per person";
        vote_on: "vote.on" = "Vote on";
        all: "vote.all" = "All objects";
        stickies: "vote.stickies" = "Sticky notes";
        selection: "vote.selection" = "Selection";
        start: "vote.start" = "Start vote";
        votes_left: template "vote.left" = "{n} votes left";
        hint: "vote.hint" = "Click objects to vote; click your badge to take a vote back";
        end: "vote.end" = "End vote";
        results: "vote.results" = "Results";
    }
}

/// What the Voting session popover keeps between passes.
#[derive(Clone)]
struct Draft {
    open: bool,
    per_person: u32,
    scope: VoteScope,
}

impl Default for Draft {
    fn default() -> Self {
        Draft { open: false, per_person: DEFAULT_VOTES, scope: VoteScope::All }
    }
}

/// The vote part of a board's top-right pill: with no vote running, the Voting session button (a popover: votes per
/// person, default 3; Vote on All objects, Sticky notes or Selection; Start vote) and Results when earlier results
/// are kept; while running, "n votes left" (the hint as its description) and End vote. Editors only start and end
/// votes. Enter in the number field starts the vote.
#[derive(Pattern, Props)]
pub struct VotePill<'a> {
    label: Label,
    transport: &'a dyn CollabTransport,
    /// Whether the user may start and end votes.
    #[prop]
    editable: bool,
    /// Whether something is selected on the board: Vote on offers Selection only then.
    #[prop]
    has_selection: bool,
}

impl<'a> VotePill<'a> {
    /// A pill showing the vote in `transport`'s session.
    pub fn new(transport: &'a dyn CollabTransport) -> Self {
        VotePill { label: Label::Empty, transport, editable: true, has_selection: false }
    }

    /// Shows the pill. Start vote, End vote and Results come back as [`CollabRequest`]s.
    pub fn show(self, ui: &mut Ui) -> VotePillOutput {
        let VotePill { label, transport, editable, has_selection } = self;
        let env = Env::of(ui);
        let words = env.words::<VoteWords>();
        let vote = transport.vote();
        let mut requests = Vec::new();
        let shown = Stack::new().label(label).show(ui, |ui, cx| {
            Row::new().show(ui, |ui, _| match vote.as_ref().filter(|vote| !vote.ended) {
                Some(running) => {
                    let left = env.format().integer(u64::from(running.left()));
                    Text::new(words.votes_left.fill(&[("n", left.as_str())])).description(words.hint.as_str().to_owned()).show(ui);
                    if editable && clicked(&Button::new(words.end.clone()).show(ui)) {
                        requests.push(CollabRequest::EndVote);
                    }
                }
                None => {
                    if editable {
                        let opener = Button::new(words.voting_session.clone()).show(ui);
                        let draft = cx.keep::<Draft>();
                        if clicked(&opener) {
                            draft.open = !draft.open;
                        }
                        // The selection may have gone since the scope was picked: the vote then covers all.
                        if !has_selection && draft.scope == VoteScope::Selection {
                            draft.scope = VoteScope::All;
                        }
                        let mut open = draft.open;
                        let (mut per_person, mut scope) = (draft.per_person, draft.scope);
                        let mut start = false;
                        Popover::new(words.voting_session.clone(), Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
                            Text::new(words.votes_per_person.clone()).show(ui);
                            let mut votes = Some(f64::from(per_person));
                            let field = NumberField::new(words.votes_per_person.clone(), Bind::bound(&mut votes))
                                .range(1.0..=f64::from(MAX_VOTES))
                                .step(1.0)
                                .commit_decimals(0)
                                .show(ui);
                            per_person = votes.map_or(per_person, |v| v.round().clamp(1.0, f64::from(MAX_VOTES)) as u32);
                            let mut entries =
                                vec![Entry::new(VoteScope::All, words.all.clone()), Entry::new(VoteScope::Stickies, words.stickies.clone())];
                            if has_selection {
                                entries.push(Entry::new(VoteScope::Selection, words.selection.clone()));
                            }
                            Select::new(words.vote_on.clone(), Bind::bound(&mut scope), &entries).show(ui);
                            let go = clicked(&Button::new(words.start.clone()).variant(Variant::PRIMARY).show(ui));
                            start = go || field.submitted;
                        });
                        if start {
                            requests.push(CollabRequest::StartVote { per_person, scope });
                            open = false;
                        }
                        let draft = cx.keep::<Draft>();
                        (draft.open, draft.per_person, draft.scope) = (open, per_person, scope);
                    }
                    if vote.as_ref().is_some_and(|vote| vote.has_history || vote.ended) && clicked(&Button::new(words.results.clone()).show(ui)) {
                        requests.push(CollabRequest::ShowResults);
                    }
                }
            });
        });
        VotePillOutput { response: shown.output, requests }
    }
}

/// What the vote pill did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct VotePillOutput {
    /// The pill's response.
    pub response: Response,
    /// Start vote, End vote or Results, in order.
    pub requests: Vec<CollabRequest>,
}

impl Outcome for VotePillOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}
