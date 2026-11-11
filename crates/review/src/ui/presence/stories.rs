//! Stories of the people bar.

use super::{FollowFrame, PeopleBar, RemoteCursor, RemoteCursors};
use crate::collab::{CollabTransport, CursorChat, JoinState, PEOPLE_COLORS, Participant, ParticipantId, VoteSession};
use craft_behavior::CanvasView;
use craft_ui::canvas::Canvas;
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use kurbo::Point;

/// A session of `count` people, you the first.
pub(crate) struct Room(pub Vec<Participant>);

impl Room {
    pub(crate) fn of(names: &[&str]) -> Self {
        let person = |(at, name): (usize, &&str)| Participant {
            id: ParticipantId(at as u64 + 1),
            name: (*name).into(),
            color: PEOPLE_COLORS[at % PEOPLE_COLORS.len()],
            cursor: None,
            selection: Vec::new(),
            chat: None,
            viewer: false,
            spotlight: false,
        };
        Room(names.iter().enumerate().map(person).collect())
    }
}

impl CollabTransport for Room {
    fn me(&self) -> ParticipantId {
        ParticipantId(1)
    }

    fn participants(&self) -> Vec<Participant> {
        self.0.clone()
    }

    fn vote(&self) -> Option<VoteSession> {
        None
    }

    fn join_state(&self) -> JoinState {
        JoinState::Joined
    }
}

#[story(PeopleBar<'static>, covers = [Default, Many], drive = Drive::Never)]
fn people_bar_seven(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = Room::of(&["Ana Reyes", "Bo Chen", "Cy Dahl", "Di Evans", "Ed Fox", "Flo Gray", "Gus Hall"]);
    PeopleBar::new(&room).following(ParticipantId(3)).show(ui).into()
}

#[story(PeopleBar<'static>, covers = [One], drive = Drive::Never)]
fn people_bar_alone(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    PeopleBar::new(&Room::of(&["Ana Reyes"])).show(ui).into()
}

/// Shows `control` in the body of a canvas that shows the document's top left at the canvas's top left.
fn on_canvas(ui: &mut Ui, mut control: impl FnMut(&mut Ui, egui::Rect)) -> StoryOutput {
    let room = ui.available_rect_before_wrap();
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    Canvas::new("Document").view(view).show(ui, |ui, cx| control(ui, cx.rect));
    StoryOutput::default()
}

/// Two people with a cursor on the page; the second has said something.
fn cursors_on_canvas(ui: &mut Ui) -> StoryOutput {
    let mut room = Room::of(&["Ana Reyes", "Bo Chen", "Cy Dahl"]);
    room.0[1].cursor = Some([120.0, 80.0]);
    room.0[1].chat = Some(CursorChat { text: "Move this up?".into(), changed: 0.0 });
    room.0[2].cursor = Some([220.0, 160.0]);
    on_canvas(ui, |ui, at| {
        RemoteCursors::new("Cursors", &room).show_at(ui, at);
    })
}

#[story(RemoteCursor<'static>, covers = [Default], drive = Drive::Never)]
fn remote_cursor(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    cursors_on_canvas(ui)
}

#[story(RemoteCursors<'static>, covers = [Default], drive = Drive::Never)]
fn remote_cursors(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    cursors_on_canvas(ui)
}

#[story(FollowFrame, covers = [Default], drive = Drive::Never)]
fn follow_frame(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = ui.available_rect_before_wrap();
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    let _ = FollowFrame::new("Following", PEOPLE_COLORS[1], "Following Bo Chen \u{b7} Esc to stop").show(ui, |ui, _| {
        Canvas::new("Document").view(view).show(ui, |_, _| {});
    });
    StoryOutput::default()
}
