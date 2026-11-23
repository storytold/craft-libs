//! Stories of the pins, the thread popover and the composer.

use craft_ui::buttons::Button;
use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Ui, pos2};

use super::pins::{Face, PinBubble};
use super::{CommentComposer, CommentPin, CommentPins, ThreadPopover};
use crate::comments::{CommentAnchor, CommentMessage, CommentThread, ReviewStatus, ThreadId};

const NOW: i64 = 90_000;

fn thread() -> CommentThread {
    let message = |author: &str, text: &str, created: i64| CommentMessage {
        author: author.into(),
        created,
        modified: None,
        text: text.into(),
        reactions: Vec::new(),
        read: true,
    };
    CommentThread {
        id: ThreadId(7),
        anchor: CommentAnchor::Canvas { at: [0.0, 0.0], node: None },
        kind: None,
        color: None,
        messages: vec![message("Ana", "Check @Bo before we ship.", NOW - 7_200), message("Bo", "On it.", NOW - 600)],
        status: ReviewStatus::None,
        marked: false,
        locked: false,
        resolved: false,
    }
}

#[story(PinBubble, covers = [Default], drive = Drive::Never)]
fn pin_bubble_unread(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let face = Face::Pin { author: "Ana".into(), color: [200, 80, 40], unread: true, resolved: false, open: false, mine: false };
    PinBubble::new(face, "Comment by Ana".into()).show(ui).into()
}

#[story(CommentPins<'static>, covers = [Default], drive = Drive::Never)]
fn comment_pins_with_a_cluster(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let pin = |id: u64, x: f32, y: f32, author: &'static str| CommentPin {
        id: ThreadId(id),
        point: pos2(x, y),
        author,
        text: "Check the margin",
        color: [200, 80, 40],
        unread: id == 1,
        resolved: id == 3,
    };
    let pins = [pin(1, 60.0, 80.0, "Ana"), pin(2, 220.0, 80.0, "Me"), pin(3, 230.0, 84.0, "Bo")];
    CommentPins::new("Pins", &pins, "Me").open(Some(ThreadId(1))).show(ui).into()
}

#[story(ThreadPopover<'static>, covers = [Default], drive = Drive::Never)]
fn thread_popover_with_replies_and_a_mention(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let thread = thread();
    let people = vec!["Ana".to_owned(), "Bo".to_owned(), "Me".to_owned()];
    let opener = Button::new("Pin").show(ui);
    let draft = data.value("draft", "Thanks @B".to_owned());
    let _ = ThreadPopover::new(&thread, &opener, "Me", &people, draft).now(NOW).show(ui);
    opener.into()
}

#[story(CommentComposer<'static>, covers = [Default], drive = Drive::Never)]
fn comment_composer_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let opener = Button::new("Add").show(ui);
    let draft = data.value("draft", String::new());
    let _ = CommentComposer::new(Some(pos2(40.0, 60.0)), draft).show(ui);
    opener.into()
}
