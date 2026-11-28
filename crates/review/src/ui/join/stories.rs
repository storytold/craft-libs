//! Stories of the join field.

use super::JoinField;
use crate::collab::{CollabTransport, JoinState, Participant, ParticipantId, VoteSession};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// A session that is only joining: the field reads nothing else from it.
struct Joining(JoinState);

impl CollabTransport for Joining {
    fn me(&self) -> ParticipantId {
        ParticipantId(1)
    }

    fn participants(&self) -> Vec<Participant> {
        Vec::new()
    }

    fn vote(&self) -> Option<VoteSession> {
        None
    }

    fn join_state(&self) -> JoinState {
        self.0.clone()
    }
}

fn field(ui: &mut Ui, data: &mut StoryData, state: JoinState) -> StoryOutput {
    let typed = data.value("typed", String::new());
    JoinField::new(&Joining(state), typed).show(ui).into()
}

#[story(JoinField<'static>, covers = [Default, Empty], drive = Drive::Never)]
fn join_field_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    field(ui, data, JoinState::Idle)
}

#[story(JoinField<'static>, covers = [], drive = Drive::Never)]
fn join_field_pending(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    field(ui, data, JoinState::Joining)
}

#[story(JoinField<'static>, covers = [], drive = Drive::Never)]
fn join_field_failed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    field(ui, data, JoinState::Failed("No session with that code".into()))
}
