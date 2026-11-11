//! Stories of the vote pill.

use super::VotePill;
use crate::collab::{CollabTransport, JoinState, Participant, ParticipantId, VoteScope, VoteSession};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// A session with at most one vote.
struct Board(Option<VoteSession>);

impl CollabTransport for Board {
    fn me(&self) -> ParticipantId {
        ParticipantId(1)
    }

    fn participants(&self) -> Vec<Participant> {
        Vec::new()
    }

    fn vote(&self) -> Option<VoteSession> {
        self.0.clone()
    }

    fn join_state(&self) -> JoinState {
        JoinState::Joined
    }
}

#[story(VotePill<'static>, covers = [Default], drive = Drive::Never)]
fn vote_pill_idle(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    VotePill::new(&Board(None)).show(ui).into()
}

#[story(VotePill<'static>, covers = [], drive = Drive::Never)]
fn vote_pill_running(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let running = VoteSession { per_person: 3, scope: VoteScope::All, ended: false, used: 1, tallies: Vec::new(), has_history: false };
    VotePill::new(&Board(Some(running))).show(ui).into()
}
