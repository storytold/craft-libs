//! The Comments panel's keys: Enter posts from the add box, Enter on a card goes to its reply box.

use std::cell::RefCell;
use std::rc::Rc;

use craft_review::comments::{CommentAnchor, CommentMessage, CommentThread, ReviewStatus, ThreadId};
use craft_review::ui::{CommentEdit, CommentsPanel, CommentsPanelState};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

fn thread(id: u64, text: &str) -> CommentThread {
    CommentThread {
        id: ThreadId(id),
        anchor: CommentAnchor::Page { page: 1, at: [0.0, id as f64] },
        kind: None,
        color: None,
        messages: vec![CommentMessage { author: "Ana".into(), created: 0, modified: None, text: text.into(), reactions: Vec::new(), read: true }],
        status: ReviewStatus::None,
        marked: false,
        locked: false,
        resolved: false,
    }
}

/// Enter in "Add a comment" posts it; Tab to the cards and Enter puts the focus in the selected card's reply box,
/// where Cmd+Enter posts the reply to that card's thread.
#[trial]
fn enter_posts_a_comment_and_enter_on_a_card_opens_its_reply_box() {
    let state = Rc::new(RefCell::new(CommentsPanelState { selected: Some(ThreadId(2)), ..CommentsPanelState::default() }));
    let edits = Rc::new(RefCell::new(Vec::new()));
    let (shown_state, shown_edits) = (state.clone(), edits.clone());
    let threads = vec![thread(1, "The margin is too narrow."), thread(2, "Use the brand colour.")];
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
    let selected = state.borrow().selected.expect("a card is selected");
    assert!(
        edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Reply { thread, text, .. } if *thread == selected && text == "Thanks")),
        "Enter on the card put the focus in its reply box, where Cmd+Enter posts: {:?}",
        edits.borrow()
    );
}
