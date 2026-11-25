//! The Comments panel: its keys, selection, menus, filter and sorts, folding, reveal, host rows, and cards that change
//! out of view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::Selection;
use craft_review::AnnotationKind;
use craft_review::comments::{
    CommentAnchor, CommentFilter, CommentMessage, CommentSort, CommentThread, PageOrder, ReviewStatus, ReviewerStatus, ThreadId,
};
use craft_review::ui::{CommentEdit, CommentsHeader, CommentsPanel, CommentsPanelState, comments_more_menu};
use craft_ui::buttons::Button;
use craft_ui::contract::{Entry, Role, SelectMode, Show as _};
use craft_ui::testing::{Fixture, Room, trial};
use egui::{Key, Modifiers, Rect};

fn message(author: &str, text: &str) -> CommentMessage {
    CommentMessage { author: author.into(), created: 0, modified: None, text: text.into(), reactions: Vec::new(), read: true, deletable: true }
}

#[trial]
fn root_delete_respects_its_deletable_flag() {
    let mut item = thread(1, 1, "Comment");
    item.messages[0].deletable = false;
    item.messages.push(message("Bo", "Reply"));
    let mut state = CommentsPanelState::default();
    state.selected = Selection::single(ThreadId(1));
    let mut fixture = Fixture::new(move |ui| {
        let _ = CommentsPanel::new("Comments", std::slice::from_ref(&item), &mut state).show(ui);
    });
    fixture.settle();
    let more = fixture.by_label("More").expect("the root menu");
    fixture.click(&more).settle();
    let delete = fixture.by_label("Delete").expect("deletion is shown");
    assert!(fixture.tree().is_disabled_key(delete.node), "the root and its descendants cannot be deleted");
}

/// A locked comment does not prevent editing or deleting its separate reply.
#[trial]
fn a_locked_comment_still_offers_reply_actions() {
    let mut item = thread(1, 1, "Locked comment");
    item.locked = true;
    item.messages.push(message("Bo", "Reply"));
    let mut state = CommentsPanelState::default();
    state.selected = Selection::single(ThreadId(1));
    let mut fixture = Fixture::new(move |ui| {
        let _ = CommentsPanel::new("Comments", std::slice::from_ref(&item), &mut state).show(ui);
    });
    fixture.settle();
    let more = fixture.by_label("More").expect("the reply menu");
    fixture.click(&more).settle();
    assert!(fixture.by_label("Edit").is_some(), "the reply can be edited");
    assert!(fixture.by_label("Delete").is_some(), "the reply can be deleted");

    let mut item = thread(2, 1, "Comment");
    let mut reply = message("Bo", "Locked reply");
    reply.deletable = false;
    item.messages.push(reply);
    let mut state = CommentsPanelState::default();
    state.selected = Selection::single(ThreadId(2));
    let mut fixture = Fixture::new(move |ui| {
        let _ = CommentsPanel::new("Comments", std::slice::from_ref(&item), &mut state).show(ui);
    });
    fixture.settle();
    let menus = fixture.by_role(Role::BUTTON);
    let reply_menu = menus.into_iter().rfind(|target| fixture.tree().name_of(target.node) == Some("More")).expect("the reply menu");
    fixture.click(&reply_menu).settle();
    let edit = fixture.by_label("Edit").expect("editing the reply is offered");
    let delete = fixture.by_label("Delete").expect("deletion is shown");
    assert!(!fixture.tree().is_disabled_key(edit.node), "locked reply text is editable");
    assert!(fixture.tree().is_disabled_key(delete.node), "a reply that cannot be deleted is disabled");
}

fn thread(id: u64, page: usize, text: &str) -> CommentThread {
    CommentThread {
        id: ThreadId(id),
        anchor: CommentAnchor::Page { page, at: [0.0, id as f64] },
        kind: None,
        color: None,
        messages: vec![message("Ana", text)],
        statuses: Vec::new(),
        reply_statuses: Vec::new(),
        marked: false,
        locked: false,
        resolved: false,
    }
}

fn reviewer(name: &str, status: ReviewStatus) -> ReviewerStatus {
    ReviewerStatus { reviewer: name.into(), status, text: String::new() }
}

/// The names of the collection's rows, headings and cards, in tree order.
fn row_names(fixture: &Fixture) -> Vec<String> {
    fixture.by_role(Role::TREE_ITEM).iter().filter_map(|row| fixture.tree().name_of(row.node).map(str::to_owned)).collect()
}

/// The rect of the built card whose name holds `text`.
fn card(fixture: &Fixture, text: &str) -> Option<Rect> {
    fixture
        .by_role(Role::TREE_ITEM)
        .into_iter()
        .find(|row| fixture.tree().name_of(row.node).is_some_and(|name| name.contains(text)))
        .map(|row| row.rect)
}

/// The rect of the built heading named `title`.
fn heading(fixture: &Fixture, title: &str) -> Option<Rect> {
    fixture.by_role(Role::TREE_ITEM).into_iter().find(|row| fixture.tree().name_of(row.node) == Some(title)).map(|row| row.rect)
}

/// The threads of the last selection the panel reported.
fn last_selection(edits: &RefCell<Vec<CommentEdit>>) -> Option<Vec<ThreadId>> {
    edits.borrow().iter().rev().find_map(|edit| match edit {
        CommentEdit::Select(ids) => Some(ids.clone()),
        _ => None,
    })
}

/// Enter in "Add a comment" posts it; Tab to the cards and Enter puts the focus in the selected card's reply box,
/// where Cmd+Enter posts the reply to that card's thread.
#[trial]
fn enter_posts_a_comment_and_enter_on_a_card_opens_its_reply_box() {
    let mut initial = CommentsPanelState::default();
    initial.selected = Selection::single(ThreadId(2));
    let state = Rc::new(RefCell::new(initial));
    let edits = Rc::new(RefCell::new(Vec::new()));
    let (shown_state, shown_edits) = (state.clone(), edits.clone());
    let threads = vec![thread(1, 1, "The margin is too narrow."), thread(2, 1, "Use the brand colour.")];
    let mut fixture = Fixture::new(move |ui| {
        let mut state = shown_state.borrow_mut();
        let out = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
        shown_edits.borrow_mut().extend(out.edits);
    });
    fixture.settle();
    let add = fixture.by_label("Add a comment").expect("the add box");
    fixture.click(&add).settle();
    fixture.type_text("Hello").key(Key::Enter, Modifiers::NONE).settle();
    assert!(
        edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Add { text, .. } if text == "Hello")),
        "Enter in the add box posts the comment: {:?}",
        edits.borrow()
    );
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    fixture.type_text("Thanks").key(Key::Enter, Modifiers::COMMAND).settle();
    let selected = state.borrow().selected.lead.expect("a card is selected");
    assert!(
        edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Reply { thread, text, .. } if *thread == selected && text == "Thanks")),
        "Enter on the card put the focus in its reply box, where Cmd+Enter posts: {:?}",
        edits.borrow()
    );
}

/// Under multi-selection a Shift range over a page heading and a Cmd/Ctrl toggle report thread ids only, the lead
/// (the card clicked) first.
#[trial]
fn multi_selection_reports_only_threads_with_the_lead_first() {
    let state = Rc::new(RefCell::new(CommentsPanelState::default()));
    let edits = Rc::new(RefCell::new(Vec::new()));
    let (shown_state, shown_edits) = (state.clone(), edits.clone());
    // Page 2's heading sits between the cards of page 1 and those of page 2.
    let threads = vec![thread(1, 1, "Note 1."), thread(2, 1, "Note 2."), thread(3, 2, "Note 3."), thread(4, 2, "Note 4.")];
    let mut fixture = Fixture::new(move |ui| {
        let mut state = shown_state.borrow_mut();
        let out = CommentsPanel::new("Comments", &threads, &mut state).select_mode(SelectMode::Multi).show(ui);
        shown_edits.borrow_mut().extend(out.edits);
    });
    // The panel's room: both pages' headings and cards are in view, so each is built and can be clicked.
    fixture.in_room(Room::Panel).settle();
    let first = card(&fixture, "Note 1.").expect("card 1");
    fixture.click_at(first.center()).settle();
    assert_eq!(last_selection(&edits), Some(vec![ThreadId(1)]), "a click selects its card");
    let third = card(&fixture, "Note 3.").expect("card 3");
    fixture.hold(Modifiers::SHIFT).click_at(third.center()).hold(Modifiers::NONE).settle();
    let range = last_selection(&edits).expect("the Shift click reports the selection");
    assert_eq!(range.first(), Some(&ThreadId(3)), "the card clicked leads: {range:?}");
    assert!(range.contains(&ThreadId(1)), "the range keeps its anchor: {range:?}");
    let mut distinct = range.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), range.len(), "each thread once, and the heading in the range is no thread: {range:?}");
    let fourth = card(&fixture, "Note 4.").expect("card 4");
    fixture.hold(Modifiers::COMMAND).click_at(fourth.center()).hold(Modifiers::NONE).settle();
    let toggled = last_selection(&edits).expect("the Cmd click reports the selection");
    assert_eq!(toggled.first(), Some(&ThreadId(4)), "the card toggled on leads: {toggled:?}");
    assert!(toggled.contains(&ThreadId(1)) && toggled.contains(&ThreadId(3)), "a toggle keeps the others: {toggled:?}");
    let mut kept = state.borrow().selected.items.clone();
    kept.sort();
    let mut reported = toggled.clone();
    reported.sort();
    assert_eq!(kept, reported, "the panel keeps the selection it reported");
    assert_eq!(state.borrow().selected.lead, Some(ThreadId(4)), "the card toggled on is the lead");
}

/// A thread hides by status only when every reviewer's status is hidden; the Checkmark filter hides marked threads;
/// the Page sort keeps document order under `PageOrder::Document`; the Type sort heads plain comments first.
#[trial]
fn the_filter_and_sorts_hide_and_order_threads() {
    let mut reviewed = thread(9, 1, "Reviewed.");
    reviewed.statuses = vec![reviewer("Ana", ReviewStatus::Accepted), reviewer("Bo", ReviewStatus::Rejected)];
    let unreviewed = thread(10, 1, "Unreviewed.");
    let mut filter = CommentFilter { hidden_statuses: vec![ReviewStatus::Accepted], ..CommentFilter::default() };
    assert!(filter.shows(&reviewed), "Bo's Rejected still shows the thread");
    assert!(filter.shows(&unreviewed), "a thread without statuses counts as None, which shows");
    filter.hidden_statuses.push(ReviewStatus::Rejected);
    assert!(!filter.shows(&reviewed), "every reviewer's status is hidden");
    filter.hidden_statuses = vec![ReviewStatus::None];
    assert!(!filter.shows(&unreviewed), "a thread without statuses hides with None");
    assert!(filter.shows(&reviewed));

    // Note 1 lists first in the document but sits lower on page 1 than Note 2.
    let mut first = thread(1, 1, "Note 1.");
    first.anchor = CommentAnchor::Page { page: 1, at: [0.0, 100.0] };
    let mut second = thread(2, 1, "Note 2.");
    second.kind = Some(AnnotationKind::Highlight);
    second.marked = true;
    let mut third = thread(3, 2, "Note 3.");
    third.kind = Some(AnnotationKind::Highlight);
    let threads = vec![first, second, third];
    let state = Rc::new(RefCell::new(CommentsPanelState::default()));
    let shown_state = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut state = shown_state.borrow_mut();
        let _ = CommentsPanel::new("Comments", &threads, &mut state).page_order(PageOrder::Document).show(ui);
    });
    fixture.settle();
    let (one, two) = (card(&fixture, "Note 1.").expect("card 1"), card(&fixture, "Note 2.").expect("card 2"));
    assert!(one.top() < two.top(), "document order puts Note 1 above Note 2 whatever their positions");

    let funnel = fixture.by_label("Filter comments").expect("the filter button");
    fixture.click(&funnel).settle();
    let checkmark = fixture.by_label("Checkmark").expect("the Checkmark submenu");
    fixture.click(&checkmark).settle();
    let checked = fixture.by_label("Checked").expect("the Checked row");
    fixture.click(&checked).settle();
    assert_eq!(state.borrow().filter.hidden_checks, vec![true], "unchecking Checked hides the marked threads");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(card(&fixture, "Note 2.").is_none(), "the marked thread is hidden");
    assert!(card(&fixture, "Note 1.").is_some(), "the unmarked thread shows");

    let sort = fixture.by_label("Sort by: Page").expect("the Sort by button");
    fixture.click(&sort).settle();
    let by_type = fixture.by_label("Type").expect("the Type row");
    fixture.click(&by_type).settle();
    assert_eq!(state.borrow().sort, CommentSort::Type);
    let names = row_names(&fixture);
    assert!(names.first().is_some_and(|name| !name.contains("Note")), "a heading comes first: {names:?}");
    assert!(names.get(1).is_some_and(|name| name.contains("Note 1.")), "the plain comment under the first heading: {names:?}");
    assert!(names.get(2).is_some_and(|name| !name.contains("Note") && Some(name) != names.first()), "another heading next: {names:?}");
    assert!(names.get(3).is_some_and(|name| name.contains("Note 3.")), "the unmarked highlight under it: {names:?}");
}

/// A heading's click folds its page; a reveal unfolds it and selects only its thread, once; a double click on a card
/// edits its comment in place.
#[trial]
fn a_heading_folds_its_page_and_a_double_click_edits_a_card() {
    let state = Rc::new(RefCell::new(CommentsPanelState::default()));
    let shown_state = state.clone();
    let threads = vec![thread(1, 1, "Note 1."), thread(2, 2, "Note 2.")];
    let mut fixture = Fixture::new(move |ui| {
        let mut state = shown_state.borrow_mut();
        let _ = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
    });
    fixture.settle();
    let page_one = heading(&fixture, "Page 1, 1").expect("page 1's heading");
    fixture.click_at(page_one.center()).settle();
    assert_eq!(state.borrow().collapsed_pages, vec![1], "the click folds page 1");
    assert!(card(&fixture, "Note 1.").is_none(), "a folded page hides its cards");

    state.borrow_mut().reveal(ThreadId(1));
    fixture.settle();
    assert!(state.borrow().collapsed_pages.is_empty(), "the reveal unfolds the thread's page");
    assert_eq!(state.borrow().selected.items, vec![ThreadId(1)], "the reveal selects only its thread");
    assert!(card(&fixture, "Note 1.").is_some(), "the revealed card is drawn");

    let page_one = heading(&fixture, "Page 1, 1").expect("page 1's heading");
    fixture.click_at(page_one.center()).settle();
    assert_eq!(state.borrow().collapsed_pages, vec![1], "the reveal was taken once: later passes do not unfold the page again");

    let note = card(&fixture, "Note 2.").expect("card 2").center();
    fixture.click_at(note).click_at(note).settle();
    assert_eq!(state.borrow().editing, Some((ThreadId(2), 0, "Note 2.".to_owned())), "the double click opens the comment's editor");
    assert!(fixture.by_label("Save").is_some(), "the editor's button is Save, as production's");
}

/// The Own header draws its Sort, filter and Pages buttons and the Hosted one none; the host's rows follow a
/// message menu's rows and report their value; the More menu returns the host row that ran.
#[trial]
fn host_rows_follow_the_cards_rows_and_report_their_value() {
    let mut initial = CommentsPanelState::default();
    initial.selected = Selection::single(ThreadId(1));
    let state = Rc::new(RefCell::new(initial));
    let header = Rc::new(Cell::new(CommentsHeader::Own));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let ran = Rc::new(RefCell::new(Vec::new()));
    let (shown_state, shown_header, shown_picked, shown_ran) = (state.clone(), header.clone(), picked.clone(), ran.clone());
    let threads = vec![thread(1, 1, "Note 1."), thread(2, 1, "Note 2.")];
    let mut fixture = Fixture::new(move |ui| {
        let mut state = shown_state.borrow_mut();
        let header = shown_header.get();
        if header == CommentsHeader::Hosted {
            let opener = Button::new("Panel options").show(ui);
            if let Some(value) = comments_more_menu(ui, &opener, &threads, &mut state, &[Entry::new(9_u32, "Export comments")]) {
                shown_ran.borrow_mut().push(value);
            }
        }
        let out =
            CommentsPanel::new("Comments", &threads, &mut state).header(header).card_rows(|_, _| vec![Entry::new(7_u32, "Flag message")]).show(ui);
        shown_picked.borrow_mut().extend(out.host);
    });
    fixture.settle();
    for name in ["Sort by: Page", "Filter comments", "Pages"] {
        assert!(fixture.by_label(name).is_some(), "the Own header draws {name}");
    }
    header.set(CommentsHeader::Hosted);
    fixture.settle();
    for name in ["Sort by: Page", "Filter comments", "Pages"] {
        assert!(fixture.by_label(name).is_none(), "the Hosted header draws no {name}");
    }

    let more = fixture.by_label("More").expect("the lead card's More button");
    fixture.click(&more).settle();
    assert!(fixture.by_label("Delete").is_some(), "the card's own rows come first");
    let flag = fixture.by_label("Flag message").expect("the host row behind the card's rows");
    fixture.click(&flag).settle();
    assert_eq!(picked.borrow().as_slice(), &[(ThreadId(1), 0, 7_u32)], "the host row reports its thread, message and value");

    let options = fixture.by_label("Panel options").expect("the host's button");
    fixture.click(&options).settle();
    assert!(fixture.by_label("Date").is_some(), "the More menu holds the Sort by rows");
    let export = fixture.by_label("Export comments").expect("the host row in the More menu");
    fixture.click(&export).settle();
    assert_eq!(ran.borrow().as_slice(), &[9_u32], "the More menu returns the host row that ran");
}

/// Each reply of a card is a node of its own, named by its author and text, inside the card and in message order;
/// the card is named by its comment.
#[trial]
fn a_cards_replies_are_nodes_inside_it() {
    let mut first = thread(1, 1, "Note 1.");
    first.messages.extend([message("Bo", "A first reply."), message("Cy", "A second reply.")]);
    let threads = vec![first];
    let mut state = CommentsPanelState::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
    });
    fixture.in_room(Room::Panel).settle();
    let card = card(&fixture, "Note 1.").expect("the card");
    let names = row_names(&fixture);
    assert!(names.iter().all(|name| !name.contains("reply.")), "the card is named by its comment: {names:?}");
    let reply = |author: &str, text: &str| {
        fixture
            .by_role(Role::GROUP)
            .into_iter()
            .find(|node| fixture.tree().name_of(node.node).is_some_and(|name| name.contains(author) && name.contains(text)))
            .map(|node| node.rect)
    };
    let one = reply("Bo", "A first reply.").expect("the first reply's node");
    let two = reply("Cy", "A second reply.").expect("the second reply's node");
    assert!(card.contains_rect(one) && card.contains_rect(two), "the replies sit inside the card: {card:?} holds {one:?} and {two:?}");
    assert!(one.bottom() <= two.top(), "the replies follow message order: {one:?} then {two:?}");
}

/// The search finds a comment by the text of a status set on one of its replies, which its card does not show.
#[trial]
fn the_search_finds_a_status_set_on_a_reply() {
    let mut first = thread(1, 1, "Note 1.");
    first.messages.push(message("Bo", "A reply."));
    first.reply_statuses = vec![ReviewerStatus { reviewer: "Cy".into(), status: ReviewStatus::Accepted, text: "Reply-status token".into() }];
    let threads = vec![first, thread(2, 1, "Note 2.")];
    let mut state = CommentsPanelState::default();
    state.search = "reply-status".into();
    let mut fixture = Fixture::new(move |ui| {
        let _ = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
    });
    fixture.in_room(Room::Panel).settle();
    assert!(card(&fixture, "Note 1.").is_some(), "the comment whose reply has the status matches");
    assert!(card(&fixture, "Note 2.").is_none(), "the other does not");
}

/// An editor the app opens takes the keyboard as it shows, once: after the focus goes elsewhere the editor does not
/// take it back.
#[trial]
fn an_editor_the_app_opens_takes_the_focus_once() {
    let state = Rc::new(RefCell::new(CommentsPanelState::default()));
    let shown_state = state.clone();
    let threads = vec![thread(1, 1, "Note 1.")];
    let mut fixture = Fixture::new(move |ui| {
        let mut state = shown_state.borrow_mut();
        let _ = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
    });
    fixture.in_room(Room::Panel).settle();
    state.borrow_mut().edit(ThreadId(1), 0, String::new());
    fixture.settle();
    fixture.type_text("Typed").settle();
    let draft = || state.borrow().editing.as_ref().map(|(_, _, text)| text.clone());
    assert_eq!(draft().as_deref(), Some("Typed"), "the opened editor has the keyboard");
    let add = fixture.by_label("Add a comment").expect("the add box");
    fixture.click(&add).settle();
    fixture.idle(3);
    fixture.type_text("Elsewhere").settle();
    assert_eq!(draft().as_deref(), Some("Typed"), "the editor does not take the keyboard back");
    assert_eq!(state.borrow().new_comment, "Elsewhere", "the add box keeps it");
}

/// A value the rig and its trial share.
type Shared<T> = Rc<RefCell<T>>;

/// A panel over 120 threads on page 1, scrolled by a reveal to thread 100; the threads are shared so a trial can
/// change them between frames.
fn long_list() -> (Shared<Vec<CommentThread>>, Shared<CommentsPanelState>, Fixture) {
    let threads = Rc::new(RefCell::new((1..=120).map(|id| thread(id, 1, &format!("Note {id}."))).collect::<Vec<_>>()));
    let state = Rc::new(RefCell::new(CommentsPanelState::default()));
    let (shown_threads, shown_state) = (threads.clone(), state.clone());
    let mut fixture = Fixture::new(move |ui| {
        let threads = shown_threads.borrow();
        let mut state = shown_state.borrow_mut();
        let _ = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
    });
    // A panel room is tall enough for the grown first card and the card below it to be in view together.
    fixture.in_room(Room::Panel).settle();
    state.borrow_mut().reveal(ThreadId(100));
    fixture.settle();
    (threads, state, fixture)
}

/// Card 1, far above the cards in view, grows by two replies: the cards in view do not jump, and scrolled back to the
/// top the card is rebuilt at its grown size with the card below it starting under it.
#[trial]
fn a_card_out_of_view_that_grows_is_rebuilt_at_its_size_when_scrolled_back() {
    let (threads, state, mut fixture) = long_list();
    assert!(card(&fixture, "Note 1.").is_none(), "card 1 is out of view");
    let before = card(&fixture, "Note 100.").expect("the revealed card is in view").top();
    if let Some(first) = threads.borrow_mut().first_mut() {
        first.messages.extend([message("Bo", "A first reply."), message("Bo", "A second reply.")]);
    }
    fixture.settle();
    let after = card(&fixture, "Note 100.").expect("the revealed card stays in view").top();
    assert_eq!(before, after, "the cards in view stay where they are while an out-of-view card grows");
    state.borrow_mut().reveal(ThreadId(1));
    fixture.settle();
    let first = card(&fixture, "Note 1.").expect("card 1 is back in view");
    let second = card(&fixture, "Note 2.").expect("card 2 is in view below it");
    assert!(first.height() > second.height(), "card 1 is rebuilt at its grown size: {first:?} then {second:?}");
    assert!(second.top() >= first.bottom(), "the card below starts under the grown card: {first:?} then {second:?}");
}

/// Each group heading puts its count in its access name: under Page sort, the unfiltered count of that page's threads
/// even when search hides some of them; under another sort, the shown count of that group.
#[trial]
fn a_heading_puts_its_count_in_its_access_name() {
    let threads = vec![
        thread(1, 1, "Needle on page one."),
        thread(2, 1, "Other on page one."),
        thread(3, 1, "Third on page one."),
        thread(4, 2, "Needle on page two."),
    ];
    let mut initial = CommentsPanelState::default();
    initial.search = "needle".into();
    let state = Rc::new(RefCell::new(initial));
    let shown_state = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut state = shown_state.borrow_mut();
        let _ = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
    });
    fixture.in_room(Room::Panel).settle();
    let names = row_names(&fixture);
    assert!(names.iter().any(|name| name.contains("Page 1") && name.contains('3')), "Page 1's heading carries its unfiltered count 3: {names:?}");
    assert!(names.iter().any(|name| name.contains("Page 2") && name.contains('1')), "Page 2's heading carries its unfiltered count 1: {names:?}");

    state.borrow_mut().sort = CommentSort::Type;
    fixture.settle();
    let names = row_names(&fixture);
    assert!(
        names.iter().any(|name| !name.contains("Needle") && name.contains('2')),
        "under Type sort, the group heading carries its shown count 2: {names:?}"
    );
}

/// An editor opened on card 1, far above the cards in view: the cards in view do not jump, and scrolled back to the
/// top the card is rebuilt with its editor and the card below it starts under it.
#[trial]
fn an_editor_opened_on_an_out_of_view_card_makes_room_when_scrolled_back() {
    let (_threads, state, mut fixture) = long_list();
    assert!(card(&fixture, "Note 1.").is_none(), "card 1 is out of view");
    let before = card(&fixture, "Note 100.").expect("the revealed card is in view").top();
    state.borrow_mut().editing = Some((ThreadId(1), 0, "Note 1.".to_owned()));
    fixture.settle();
    let after = card(&fixture, "Note 100.").expect("the revealed card stays in view").top();
    assert_eq!(before, after, "the cards in view stay where they are while an out-of-view card opens an editor");
    state.borrow_mut().reveal(ThreadId(1));
    fixture.settle();
    let first = card(&fixture, "Note 1.").expect("card 1 is back in view");
    let second = card(&fixture, "Note 2.").expect("card 2 is in view below it");
    assert!(first.height() > second.height(), "card 1 is rebuilt with its editor: {first:?} then {second:?}");
    assert!(second.top() >= first.bottom(), "the card below starts under the card with the editor: {first:?} then {second:?}");
}

/// A panel over `threads` with `state`, settled in a panel-sized room.
fn panel_of(threads: Vec<CommentThread>, mut state: CommentsPanelState) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let _ = CommentsPanel::new("Comments", &threads, &mut state).show(ui);
    });
    fixture.in_room(Room::Panel).settle();
    fixture
}

/// The rect of the first painted text run equal to `text`.
fn text_rect(fixture: &Fixture, text: &str) -> Option<Rect> {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect)
}

/// An empty panel says what to do next, under its "No comments yet." line.
#[trial]
fn an_empty_panel_says_what_to_do_next() {
    let fixture = panel_of(Vec::new(), CommentsPanelState::default());
    assert!(text_rect(&fixture, "No comments yet.").is_some(), "the empty line shows");
    assert!(fixture.texts().iter().any(|run| run.text.contains("type above")), "a hint says to pick a tool or type above");
}

/// The More menu names its sort rows with a "Sort by" heading.
#[trial]
fn the_more_menu_heads_its_sort_rows() {
    let threads = vec![thread(1, 1, "Note 1.")];
    let mut state = CommentsPanelState::default();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Panel options").show(ui);
        let _ = comments_more_menu::<u32>(ui, &opener, &threads, &mut state, &[]);
    });
    fixture.settle();
    let options = fixture.by_label("Panel options").expect("the host's button");
    fixture.click(&options).settle();
    assert!(fixture.texts().iter().any(|run| run.text == "Sort by"), "the sort rows have a Sort by heading");
}

/// A selected card is a raised surface with an accent rule at its edge: its fill is not the strong selection fill a
/// selected row of a list has (in the dark theme, a mid grey), and no card-sized fill is blue.
#[trial]
fn a_selected_card_is_a_raised_surface_not_a_selection_fill() {
    let mut state = CommentsPanelState::default();
    state.selected = Selection::single(ThreadId(1));
    let mut fixture = panel_of(vec![thread(1, 1, "Note 1.")], state);
    fixture.theme(craft_theme::Mode::Dark, craft_theme::Contrast::Standard).settle();
    let card = card(&fixture, "Note 1.").expect("the card");
    let shapes = fixture.frame_shapes();
    let fills: Vec<egui::Color32> = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.rect.width() > 60.0 && rect.rect.height() > 20.0 && card.contains_rect(rect.rect.shrink(0.5)) => {
                Some(rect.fill)
            }
            _ => None,
        })
        .collect();
    assert!(
        fills.iter().all(|fill| fill.r() < 0x60 && i32::from(fill.b()) - i32::from(fill.r()) < 40),
        "the card fill is a quiet surface: {fills:?}"
    );
}

/// The avatars of a card's messages are muted discs, not accent-coloured ones.
#[trial]
fn card_avatars_are_muted() {
    let mut item = thread(1, 1, "Note 1.");
    item.messages.push(message("Bo", "A reply."));
    let mut state = CommentsPanelState::default();
    state.selected = Selection::single(ThreadId(1));
    let mut fixture = panel_of(vec![item], state);
    let shapes = fixture.frame_shapes();
    let discs: Vec<egui::Color32> = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Circle(circle) if (8.0..20.0).contains(&circle.radius) => Some(circle.fill),
            _ => None,
        })
        .collect();
    assert!(!discs.is_empty(), "the reply's avatar disc paints");
    assert!(discs.iter().all(|fill| i32::from(fill.b()) - i32::from(fill.r()) < 40), "no disc is blue: {discs:?}");
}

/// A card's text starts under its author's name, and a selected card's reply field is indented with it.
#[trial]
fn card_text_and_the_reply_field_sit_under_the_name() {
    let mut state = CommentsPanelState::default();
    state.selected = Selection::single(ThreadId(1));
    let fixture = panel_of(vec![thread(1, 1, "Note 1.")], state);
    let name = text_rect(&fixture, "Ana").expect("the author");
    let body = text_rect(&fixture, "Note 1.").expect("the text");
    assert!((name.left() - body.left()).abs() < 0.5, "the text starts under the name: {name:?} {body:?}");
    let field = fixture.by_label("Add a reply").expect("the reply field");
    assert!(field.rect.left() >= name.left() - 0.5, "the reply field is indented to the name: {:?} {name:?}", field.rect);
}

/// A heading of a sort other than Page is the group's name alone: no count and no disclosure.
#[trial]
fn a_group_heading_of_the_author_sort_is_plain() {
    let mut state = CommentsPanelState::default();
    state.sort = CommentSort::Author;
    let fixture = panel_of(vec![thread(1, 1, "Note 1.")], state);
    assert!(text_rect(&fixture, "Ana").is_some(), "the group heading names the author");
    assert!(text_rect(&fixture, "1").is_none(), "the heading paints no count");
}
