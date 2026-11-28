//! Stories of `CommentBalloons`: cards beside a page.

use craft_behavior::Selection;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_pages::ui::PlacedPage;
use craft_ui::testing::canvas_story::{OUTLINE, StoryPicture, StoryShape, accent, on_picture, paint_shapes};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, Pos2, Rect, Ui};

use super::{BalloonItem, BalloonPlace, CommentBalloons};
use crate::comments::{CommentAnchor, CommentMessage, CommentThread, ThreadId};
use crate::revisions::{Revision, RevisionKind};
use crate::ui::comments::CommentsPanelState;

const NOW: i64 = 90_000;

/// The page's size in its own points.
const PAGE: (f64, f64) = (300.0, 400.0);

fn thread(id: u64, author: &str, text: &str, locked: bool) -> CommentThread {
    let message = |author: &str, text: &str| CommentMessage {
        author: author.into(),
        created: NOW - 3_600,
        modified: None,
        text: text.into(),
        reactions: Vec::new(),
        read: true,
        deletable: true,
    };
    CommentThread {
        id: ThreadId(id),
        anchor: CommentAnchor::Page { page: 0, at: [0.0, 0.0] },
        kind: None,
        color: None,
        messages: vec![message(author, text), message("Bo", "Agreed.")],
        statuses: Vec::new(),
        reply_statuses: Vec::new(),
        marked: false,
        locked,
        resolved: false,
    }
}

/// Shows `threads` and `revisions` as balloons beside one page, drawn as a picture, the threads' anchors going down it.
fn beside_page(
    ui: &mut Ui,
    data: &mut StoryData,
    threads: &[CommentThread],
    revisions: &[Revision],
    selected: Option<u64>,
    place: BalloonPlace,
    read_only: bool,
) -> StoryOutput {
    let state = data.value("state", {
        let mut state = CommentsPanelState::default();
        state.selected = selected.map_or_else(Selection::default, |id| Selection::single(ThreadId(id)));
        state
    });
    // design: allow[raw-colour/const](the demo page is white paper, document data and not a theme colour)
    let (fill, line) = (Color32::WHITE, accent(ui, 1.0));
    let shapes = [StoryShape::filled(&DocRect::new(0.0, 0.0, PAGE.0, PAGE.1), fill).outlined(OUTLINE, line)];
    let picture = StoryPicture::new(PAGE.0 as u32, PAGE.1 as u32, move |p: Point| paint_shapes(&shapes, Color32::TRANSPARENT, p));
    on_picture(ui, &picture, |ui, _, view| {
        let rect = Rect::from_two_pos(view.to_screen(Point::new(0.0, 0.0)), view.to_screen(Point::new(PAGE.0, PAGE.1)));
        let pages = [PlacedPage::new(0, rect, f64::from(rect.width()) / PAGE.0, (0.0, rect.height()))];
        let anchor = |at: usize| Pos2::new(40.0, 30.0 + 80.0 * at as f32);
        let mut items: Vec<BalloonItem<'_>> =
            threads.iter().enumerate().map(|(at, thread)| BalloonItem::Thread { thread, page: 0, anchor: anchor(at) }).collect();
        items.extend(revisions.iter().enumerate().map(|(at, revision)| BalloonItem::Revision {
            revision,
            page: 0,
            anchor: anchor(threads.len() + at),
        }));
        CommentBalloons::new("Comments", &pages, &items, &mut *state).place(place).now(Some(NOW)).read_only(read_only).show(ui)
    })
}

#[story(CommentBalloons<'static>, covers = [Default], drive = Drive::Never)]
fn balloons_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = [thread(1, "Ana", "Locked: nobody edits this.", true), thread(2, "Cy", "So is this one.", true)];
    beside_page(ui, data, &threads, &[], None, BalloonPlace::Column, false)
}

/// Read-only balloons (`CommentBalloons::read_only`): comments cannot change, so the selected card, its thread not
/// locked, shows no reply field and no buttons.
#[story(CommentBalloons<'static>, covers = [], drive = Drive::Never)]
fn read_only_balloons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = [thread(1, "Ana", "The margin is too narrow.", false), thread(2, "Cy", "Use the brand colour here.", false)];
    beside_page(ui, data, &threads, &[], Some(1), BalloonPlace::Column, true)
}

#[story(CommentBalloons<'static>, covers = [], drive = Drive::Never)]
fn balloons_selected(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = [thread(1, "Ana", "The margin is too narrow.", false), thread(2, "Cy", "Use the brand colour here.", false)];
    beside_page(ui, data, &threads, &[], Some(2), BalloonPlace::Column, false)
}

#[story(CommentBalloons<'static>, covers = [], drive = Drive::Never)]
fn balloons_long(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let long = "A long comment that runs over many lines of the card. ".repeat(24);
    let threads = [thread(1, "Ana", &long, false), thread(2, "Cy", "The next card stays in view.", false)];
    beside_page(ui, data, &threads, &[], None, BalloonPlace::Column, false)
}

#[story(CommentBalloons<'static>, covers = [], drive = Drive::Never)]
fn balloons_right_to_left(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = [thread(1, "מיכל", "השוליים צרים מדי בעמוד הזה.", false)];
    beside_page(ui, data, &threads, &[], None, BalloonPlace::Column, false)
}

#[story(CommentBalloons<'static>, covers = [], drive = Drive::Never)]
fn balloons_revision(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = [thread(1, "Ana", "Please check this change.", false)];
    let revisions = [Revision {
        kind: RevisionKind::Changed,
        heading: "Slide 3 changed".into(),
        details: vec!["Title text changed".into(), "Picture moved".into()],
    }];
    beside_page(ui, data, &threads, &revisions, None, BalloonPlace::Column, false)
}

#[story(CommentBalloons<'static>, covers = [], drive = Drive::Never)]
fn balloons_floating(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = [thread(1, "Ana", "This card floats over the page.", false)];
    beside_page(ui, data, &threads, &[], None, BalloonPlace::Floating, false)
}
