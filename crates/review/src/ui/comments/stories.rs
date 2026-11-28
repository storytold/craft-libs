//! Stories of the Comments panel and its card list.

use craft_behavior::Selection;
use craft_ui::contract::{Entry, SelectMode, ShowCollection as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use egui::Ui;

use super::list::{CardKey, CommentList, RowEntry, Rows};
use super::row::{CardActions, CommentRow};
use super::{CommentsHeader, CommentsPanel, CommentsPanelState};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentAnchor, CommentMessage, CommentThread, ReviewStatus, ReviewerStatus, ThreadId};

const NOW: i64 = 90_000;

fn message(author: &str, text: &str, created: i64) -> CommentMessage {
    CommentMessage { author: author.into(), created, modified: None, text: text.into(), reactions: Vec::new(), read: true, deletable: true }
}

fn reviewer(name: &str, status: ReviewStatus) -> ReviewerStatus {
    ReviewerStatus { reviewer: name.into(), status, text: String::new() }
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
        statuses: if id == 1 { vec![reviewer("Bo", ReviewStatus::Accepted)] } else { Vec::new() },
        reply_statuses: Vec::new(),
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

/// Statuses from three reviewers, one of them set back to None and one without a name.
fn reviewed() -> Vec<ReviewerStatus> {
    vec![
        reviewer("Ana", ReviewStatus::Rejected),
        reviewer("Bo", ReviewStatus::Completed),
        reviewer("Cy", ReviewStatus::None),
        reviewer("", ReviewStatus::Cancelled),
    ]
}

#[story(CommentsPanel<'static>, covers = [Default], drive = Drive::Never)]
fn comments_panel_two_pages_one_selected(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = threads();
    let state = data.value("state", CommentsPanelState { selected: Selection::single(ThreadId(2)), ..CommentsPanelState::default() });
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

/// The host draws the header: the panel starts with the add box, and shows the search box the host opened.
#[story(CommentsPanel<'static>, covers = [], drive = Drive::Never)]
fn comments_panel_hosted_header(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = threads();
    let state = data.value("state", CommentsPanelState { search_open: true, ..CommentsPanelState::default() });
    CommentsPanel::new("Comments", &threads, state).now(NOW).header(CommentsHeader::Hosted).show(ui).into()
}

/// Two cards selected across a page heading; the lead, the first, shows the reply box.
#[story(CommentsPanel<'static>, covers = [], drive = Drive::Never)]
fn comments_panel_multi_selection(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = threads();
    let selected = Selection::new(vec![ThreadId(2), ThreadId(3)], Some(ThreadId(2)), Some(ThreadId(2)));
    let state = data.value("state", CommentsPanelState { selected, ..CommentsPanelState::default() });
    CommentsPanel::new("Comments", &threads, state).now(NOW).select_mode(SelectMode::Multi).show(ui).into()
}

/// A card with a badge per reviewer's status.
#[story(CommentsPanel<'static>, covers = [], drive = Drive::Never)]
fn comments_panel_per_reviewer_statuses(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut threads = threads();
    if let Some(thread) = threads.get_mut(1) {
        thread.statuses = reviewed();
    }
    let state = data.value("state", CommentsPanelState { selected: Selection::single(ThreadId(2)), ..CommentsPanelState::default() });
    CommentsPanel::new("Comments", &threads, state).now(NOW).show(ui).into()
}

/// The host's rows follow each message menu's own rows behind a separator.
#[story(CommentsPanel<'static>, covers = [], drive = Drive::Never)]
fn comments_panel_host_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let threads = threads();
    let state = data.value("state", CommentsPanelState { selected: Selection::single(ThreadId(1)), ..CommentsPanelState::default() });
    CommentsPanel::new("Comments", &threads, state)
        .now(NOW)
        .card_rows(|_, _| vec![Entry::new(1_u32, "Show on page"), Entry::new(2_u32, "Properties")])
        .show(ui)
        .into()
}

fn list(ui: &mut Ui, threads: &[CommentThread]) -> StoryOutput {
    let entries = threads.iter().map(|thread| (CardKey::Thread(thread.id), RowEntry::Card { thread, depth: 0 })).collect();
    let source = Rows { entries };
    CommentList::new("Comments", None, false, SelectMode::Single)
        .show_items(ui, &source, &mut |_, item| match &source.entries[item.at].1 {
            RowEntry::Card { thread, .. } => CommentRow::card(thread, thread.messages.first().map_or_else(String::new, |first| first.text.clone()))
                .state(item.row.selected, true, NOW)
                .lead(item.row.selected)
                .drafts(None, ""),
            RowEntry::Group { title, count, open, page } => CommentRow::heading(title.clone(), count.clone(), *open, page.is_none()),
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
    show_as_only_item(
        ui,
        CommentRow::card(&thread, "Ana: The margin is too narrow on this page.".into()).state(true, true, NOW).lead(true).drafts(None, ""),
    )
    .into()
}

/// One badge per reviewer under the comment, wrapping to the card's width.
#[story(CommentRow<'static>, covers = [], drive = Drive::Never)]
fn comment_row_reviewer_statuses(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut thread = thread(2, 1, "Use the brand colour here.", 0);
    thread.statuses = reviewed();
    show_as_only_item(ui, CommentRow::card(&thread, "Ana: Use the brand colour here.".into()).state(false, true, NOW).drafts(None, "")).into()
}

#[story(CommentRow<'static>, covers = [], drive = Drive::Never)]
fn comment_row_selected_card_with_buttons(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let thread = thread(1, 1, "The margin is too narrow on this page.", 1);
    let row = CommentRow::card(&thread, "Ana: The margin is too narrow on this page.".into())
        .state(true, true, NOW)
        .actions(CardActions::Buttons)
        .drafts(None, "");
    show_as_only_item(ui, row).into()
}
