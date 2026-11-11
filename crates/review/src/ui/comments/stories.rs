//! Stories of the Comments panel and its card list.

use craft_ui::contract::ShowCollection as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use egui::Ui;

use super::list::{CardKey, CommentList, RowEntry, Rows};
use super::row::CommentRow;
use super::{CommentsPanel, CommentsPanelState};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentAnchor, CommentMessage, CommentThread, ReviewStatus, ThreadId};

const NOW: i64 = 90_000;

fn message(author: &str, text: &str, created: i64) -> CommentMessage {
    CommentMessage { author: author.into(), created, modified: None, text: text.into(), reactions: Vec::new(), read: true }
}

fn thread(id: u64, page: usize, text: &str, replies: usize) -> CommentThread {
    let mut messages = vec![message("Ana", text, NOW - 3_600)];
    messages.extend((0..replies).map(|n| message("Bo", &format!("Reply {n}"), NOW - 600)));
    CommentThread {
        id: ThreadId(id),
        anchor: CommentAnchor::Page { page, at: [0.0, id as f64] },
        kind: Some(AnnotationKind::Note),
        color: Some([255, 200, 0]),
        messages,
        status: if id == 1 { ReviewStatus::Accepted } else { ReviewStatus::None },
        marked: id == 3,
        locked: false,
        resolved: false,
    }
}

fn threads() -> Vec<CommentThread> {
    vec![
        thread(1, 1, "The margin is too narrow on this page.", 2),
        thread(2, 1, "Use the brand colour here.", 0),
        thread(3, 2, "Typo in the second paragraph.", 1),
        thread(4, 2, "Move the figure up.", 0),
    ]
}

#[story(CommentsPanel<'static>, covers = [Default], drive = Drive::Never)]
fn comments_panel_two_pages_one_selected(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = threads();
    let state = data.value("state", CommentsPanelState { selected: Some(ThreadId(2)), ..CommentsPanelState::default() });
    CommentsPanel::new("Comments", &threads, state).now(NOW).show(ui).into()
}

#[story(CommentsPanel<'static>, covers = [], drive = Drive::Never)]
fn comments_panel_filtered_to_nothing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = threads();
    let state = data.value("state", CommentsPanelState { search: "zzz".into(), ..CommentsPanelState::default() });
    CommentsPanel::new("Comments", &threads, state).now(NOW).show(ui).into()
}

#[story(CommentsPanel<'static>, covers = [], drive = Drive::Never)]
fn comments_panel_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("state", CommentsPanelState::default());
    CommentsPanel::new("Comments", &[], state).now(NOW).show(ui).into()
}

fn list(ui: &mut Ui, threads: &[CommentThread]) -> StoryOutput {
    let entries = threads.iter().map(|thread| (CardKey::Thread(thread.id), RowEntry::Card { thread, depth: 0 })).collect();
    let source = Rows { entries };
    CommentList::new("Comments", None, false)
        .show_items(ui, &source, &mut |_, item| match &source.entries[item.at].1 {
            RowEntry::Card { thread, .. } => CommentRow::card(thread, thread.messages.first().map_or_else(String::new, |first| first.text.clone()))
                .state(item.row.selected, true, NOW)
                .drafts(None, ""),
            RowEntry::Group { title, count, open, .. } => CommentRow::heading(title.clone(), count.clone(), *open),
        })
        .into()
}

#[story(CommentList<'static>, covers = [Empty], drive = Drive::Never)]
fn comment_list_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, &[])
}

#[story(CommentList<'static>, covers = [One], drive = Drive::Never)]
fn comment_list_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, &threads()[1..2])
}

#[story(CommentList<'static>, covers = [Default, Many], drive = Drive::Never)]
fn comment_list_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, &threads())
}

#[story(CommentRow<'static>, covers = [Default], drive = Drive::Never)]
fn comment_row_selected_with_replies(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let thread = thread(1, 1, "The margin is too narrow on this page.", 2);
    show_as_only_item(ui, CommentRow::card(&thread, "Ana: The margin is too narrow on this page.".into()).state(true, true, NOW).drafts(None, ""))
        .into()
}
