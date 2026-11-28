//! The comment balloons: stacking, floating, selecting and replying, the buttons of the selected card, and long bodies.

use std::cell::RefCell;
use std::rc::Rc;

use craft_pages::ui::PlacedPage;
use craft_review::comments::{CommentAnchor, CommentMessage, CommentThread, ThreadId};
use craft_review::ui::{BalloonItem, BalloonPlace, CommentBalloons, CommentEdit, CommentsPanelState, CommentsWords};
use craft_ui::runtime::env::Direction;
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Vec2, pos2, vec2};

fn thread(id: u64, text: &str) -> CommentThread {
    CommentThread {
        id: ThreadId(id),
        anchor: CommentAnchor::Page { page: 0, at: [0.0, 0.0] },
        kind: None,
        color: None,
        messages: vec![CommentMessage {
            author: "Ana".into(),
            created: 0,
            modified: None,
            text: text.into(),
            reactions: Vec::new(),
            read: true,
            deletable: true,
        }],
        statuses: Vec::new(),
        reply_statuses: Vec::new(),
        marked: false,
        locked: false,
        resolved: false,
    }
}

/// A panel state with thread `id` selected.
fn selecting(id: u64) -> CommentsPanelState {
    let mut state = CommentsPanelState::default();
    state.selected = craft_behavior::Selection::single(ThreadId(id));
    state
}

/// The page at (20, 20), 200 wide, at one screen point per page point.
fn page() -> PlacedPage {
    PlacedPage::new(0, Rect::from_min_size(pos2(20.0, 20.0), vec2(200.0, 1200.0)), 1.0, (0.0, 1200.0))
}

/// Balloons over `threads`, each anchored at the page height beside it, with what a pass reported kept.
struct Shown {
    fixture: Fixture,
    state: Rc<RefCell<CommentsPanelState>>,
    edits: Rc<RefCell<Vec<CommentEdit>>>,
    cards: Rc<RefCell<Vec<(ThreadId, Rect)>>>,
}

impl Shown {
    fn new(threads: Vec<(CommentThread, f32)>, state: CommentsPanelState, place: BalloonPlace, room: Vec2, direction: Direction) -> Shown {
        Shown::with(threads, state, place, room, direction, false)
    }

    /// [`Shown::new`] with the balloons `read_only` or not.
    fn with(
        threads: Vec<(CommentThread, f32)>,
        state: CommentsPanelState,
        place: BalloonPlace,
        room: Vec2,
        direction: Direction,
        read_only: bool,
    ) -> Shown {
        let state = Rc::new(RefCell::new(state));
        let edits = Rc::new(RefCell::new(Vec::new()));
        let cards = Rc::new(RefCell::new(Vec::new()));
        let (shown_state, shown_edits, shown_cards) = (state.clone(), edits.clone(), cards.clone());
        let pages = [page()];
        let mut fixture = Fixture::new(move |ui| {
            let items: Vec<BalloonItem<'_>> = threads.iter().map(|(thread, y)| BalloonItem::thread(thread, 0, Pos2::new(30.0, *y))).collect();
            let mut state = shown_state.borrow_mut();
            let out = CommentBalloons::new("Comments", &pages, &items, &mut state).place(place).now(Some(3_600)).read_only(read_only).show(ui);
            shown_edits.borrow_mut().extend(out.edits);
            *shown_cards.borrow_mut() = out.cards;
        });
        fixture.room(room).direction(direction).settle();
        Shown { fixture, state, edits, cards }
    }

    fn card(&self, id: u64) -> Rect {
        let cards = self.cards.borrow();
        cards.iter().find(|(thread, _)| *thread == ThreadId(id)).map(|(_, rect)| *rect).expect("the thread's card")
    }
}

fn room() -> Vec2 {
    vec2(900.0, 900.0)
}

/// The cards of three threads whose anchors are 100, 110 and 400 down the page, given in the order 400, 100, 110:
/// they stack in anchor order, the second is pushed below the first, and the late one stays at its anchor.
#[trial]
fn cards_stack_down_a_page_in_anchor_order_without_overlap_and_a_late_anchor_keeps_its_place() {
    let threads = vec![(thread(3, "Late"), 400.0), (thread(1, "First"), 100.0), (thread(2, "Second"), 110.0)];
    let shown = Shown::new(threads, CommentsPanelState::default(), BalloonPlace::Column, room(), Direction::Ltr);
    let (first, second, late) = (shown.card(1), shown.card(2), shown.card(3));
    assert!((first.top() - 120.0).abs() < 0.5, "the first card sits at its anchor: {first:?}");
    assert!(second.top() >= first.bottom(), "the second card does not overlap the first: {first:?} {second:?}");
    assert!(second.top() > 130.0, "the second card was pushed below its own anchor: {second:?}");
    assert!(late.top() >= second.bottom(), "the late card does not overlap the second: {second:?} {late:?}");
    assert!((late.top() - 420.0).abs() < 0.5, "a card whose anchor lies below the card before keeps its anchor: {late:?}");
}

/// Floating cards sit beside the page when the room allows and over its edge when it does not.
#[trial]
fn a_floating_card_sits_beside_the_page_when_there_is_room_and_over_its_edge_when_not() {
    let page = page().rect;
    let at = |width: f32| {
        let shown =
            Shown::new(vec![(thread(1, "Hello"), 100.0)], CommentsPanelState::default(), BalloonPlace::Floating, vec2(width, 600.0), Direction::Ltr);
        shown.card(1)
    };
    let wide = at(900.0);
    assert!(wide.left() >= page.right(), "with room the card is beside the page: {wide:?} {page:?}");
    let narrow = at(300.0);
    assert!(narrow.left() < page.right() && narrow.right() > page.right(), "without room the card overlaps the page's edge: {narrow:?} {page:?}");
}

/// A click on a card selects it; a reply typed in the selected card goes into the state's reply, and Post reports it.
#[trial]
fn selecting_a_card_and_posting_a_reply_report_the_matching_edits() {
    let threads = vec![(thread(1, "First"), 100.0), (thread(2, "Second"), 400.0)];
    let mut shown = Shown::new(threads, CommentsPanelState::default(), BalloonPlace::Column, room(), Direction::Ltr);
    let words = CommentsWords::default();
    let card = shown.card(1);
    shown.fixture.click_at(card.center()).settle();
    assert!(
        shown.edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Select(threads) if threads == &[ThreadId(1)])),
        "a click on a card reports its selection: {:?}",
        shown.edits.borrow()
    );
    assert_eq!(shown.state.borrow().selected.lead, Some(ThreadId(1)));
    let reply = shown.fixture.by_label(words.reply.as_str()).expect("the selected card's reply field");
    shown.fixture.click(&reply).settle();
    shown.fixture.type_text("Thanks").settle();
    assert_eq!(shown.state.borrow().reply, "Thanks", "the typed reply goes into the bound reply");
    let post = shown.fixture.by_label(words.post.as_str()).expect("the reply's Post");
    shown.fixture.click(&post).settle();
    assert!(
        shown.edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Reply { thread, text, .. } if *thread == ThreadId(1) && text == "Thanks")),
        "Post reports the reply: {:?}",
        shown.edits.borrow()
    );
    assert!(shown.state.borrow().reply.is_empty(), "a posted reply clears the binding");
}

/// An edit draft the app could not write back stays when its card is selected again, as the old WordCraft kept a
/// refused draft for when its balloon was selected again; selecting another card drops it.
#[trial]
fn reselecting_a_card_keeps_its_edit_draft() {
    let threads = vec![(thread(1, "First"), 100.0), (thread(2, "Second"), 400.0)];
    // The app left the balloon (no card selected) and kept the draft the document refused.
    let mut left = CommentsPanelState::default();
    left.editing = Some((ThreadId(1), 0, "Edited".into()));
    let mut shown = Shown::new(threads, left, BalloonPlace::Column, room(), Direction::Ltr);
    // A click on the card's heading, above the open edit's field.
    let card = shown.card(1);
    shown.fixture.click_at(card.left_top() + vec2(30.0, 8.0)).settle();
    assert_eq!(shown.state.borrow().selected.lead, Some(ThreadId(1)), "the click selects the card");
    assert_eq!(shown.state.borrow().editing, Some((ThreadId(1), 0, "Edited".into())), "the card's own draft survives its selection");

    let other = shown.card(2);
    shown.fixture.click_at(other.center()).settle();
    assert_eq!(shown.state.borrow().selected.lead, Some(ThreadId(2)));
    assert_eq!(shown.state.borrow().editing, None, "selecting another card drops the draft, as before");
}

/// The inline editor takes the keyboard in the pass it opens, so typing needs no second click on its field, as the
/// old WordCraft's balloon took typing once selected. An editor that stays open does not take the keyboard back.
#[trial]
fn an_edit_that_opens_takes_the_keyboard() {
    let threads = vec![(thread(1, "First"), 100.0)];
    let selected = selecting(1);
    let mut shown = Shown::new(threads, selected, BalloonPlace::Column, room(), Direction::Ltr);
    let words = CommentsWords::default();
    // The app opens the edit of the selected card's comment, as on a click on its balloon.
    shown.state.borrow_mut().editing = Some((ThreadId(1), 0, "First".into()));
    shown.fixture.settle();
    shown.fixture.type_text("!").settle();
    let draft = shown.state.borrow().editing.as_ref().map(|(.., text)| text.clone());
    assert!(
        draft.as_deref().is_some_and(|text| text.contains('!') && text.len() == "First!".len()),
        "typing reaches the edit that opened: {draft:?}"
    );

    // The reply field takes the keyboard; the open edit does not take it back.
    let reply = shown.fixture.by_label(words.reply.as_str()).expect("the selected card's reply field");
    shown.fixture.click(&reply).settle();
    shown.fixture.type_text("x").settle();
    assert_eq!(shown.state.borrow().reply, "x", "the reply field keeps the keyboard");
}

/// Guards `CommentBalloons::read_only`: while comments cannot change, a selected card takes no typing, as the panel's
/// rows do: it shows no reply field, and text typed after a click on it reaches no reply.
#[trial]
fn a_read_only_card_takes_no_typing() {
    let threads = vec![(thread(1, "First"), 100.0)];
    let selected = selecting(1);
    let mut shown = Shown::with(threads, selected, BalloonPlace::Column, room(), Direction::Ltr, true);
    let words = CommentsWords::default();
    assert!(shown.fixture.by_label(words.reply.as_str()).is_none(), "a read-only card shows no reply field");
    let card = shown.card(1);
    shown.fixture.click_at(card.center()).settle();
    shown.fixture.type_text("Thanks").settle();
    assert!(shown.state.borrow().reply.is_empty(), "typing reaches no reply: {:?}", shown.state.borrow().reply);
    assert!(!shown.edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Reply { .. })), "no reply is reported: {:?}", shown.edits.borrow());
}

/// The selected card's buttons: Reply puts the focus in the reply field, Resolve and Reopen report `Resolve`, Delete reports
/// `Delete` of the whole thread.
#[trial]
fn a_selected_card_in_buttons_form_reports_reply_resolve_reopen_and_delete() {
    let words = CommentsWords::default();
    let selected = || selecting(1);
    let open = vec![(thread(1, "Open"), 100.0)];
    let mut shown = Shown::new(open.clone(), selected(), BalloonPlace::Column, room(), Direction::Ltr);

    let reply = shown.fixture.by_label(words.reply_button.as_str()).expect("the Reply button");
    shown.fixture.click(&reply).settle();
    shown.fixture.type_text("x").settle();
    assert_eq!(shown.state.borrow().reply, "x", "Reply puts the focus in the reply field");

    let resolve = shown.fixture.by_label(words.resolve.as_str()).expect("the Resolve button");
    shown.fixture.click(&resolve).settle();
    assert!(
        shown.edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Resolve { thread, resolved: true, .. } if *thread == ThreadId(1))),
        "Resolve reports resolved: {:?}",
        shown.edits.borrow()
    );

    let delete = shown.fixture.by_label(words.delete.as_str()).expect("the Delete button");
    shown.fixture.click(&delete).settle();
    assert!(
        shown.edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Delete { thread, index: 0, .. } if *thread == ThreadId(1))),
        "Delete reports the thread: {:?}",
        shown.edits.borrow()
    );

    let mut done = thread(1, "Done");
    done.resolved = true;
    let mut shown = Shown::new(vec![(done, 100.0)], selected(), BalloonPlace::Column, room(), Direction::Ltr);
    assert!(shown.fixture.by_label(words.resolve.as_str()).is_none(), "a resolved thread offers Reopen, not Resolve");
    let reopen = shown.fixture.by_label(words.reopen.as_str()).expect("the Reopen button");
    shown.fixture.click(&reopen).settle();
    assert!(
        shown.edits.borrow().iter().any(|edit| matches!(edit, CommentEdit::Resolve { thread, resolved: false, .. } if *thread == ThreadId(1))),
        "Reopen reports not resolved: {:?}",
        shown.edits.borrow()
    );
}

/// Long bodies are cut to ten rows by the text layout (`Wrap::Clamp`), selected or not, so the next card stays in view; a locked card's text stays in its
/// card; under right-to-left a card's text starts at its right edge.
#[trial]
fn balloons_stack_without_overlap_clip_long_bodies_and_keep_the_next_card_in_view() {
    let long = "lorem ipsum dolor ".repeat(120);
    let mut locked = thread(4, "Locked body");
    locked.locked = true;
    let threads = vec![(thread(1, &long), 100.0), (thread(2, "Short."), 110.0), (thread(3, &long), 120.0), (locked, 130.0)];
    let state = selecting(3);
    let shown = Shown::new(threads, state, BalloonPlace::Column, vec2(900.0, 1100.0), Direction::Ltr);
    let texts = shown.fixture.texts().to_vec();
    let line = texts.iter().find(|text| text.text == "Short.").map(|text| text.rect.height()).expect("the short card's body");
    assert!(line > 0.0, "a row of text has a height");

    let cards: Vec<Rect> = (1..=4).map(|id| shown.card(id)).collect();
    for pair in cards.windows(2) {
        assert!(pair[1].top() >= pair[0].bottom(), "the cards do not overlap: {cards:?}");
    }
    for id in [1, 3] {
        let card = shown.card(id);
        let body = texts
            .iter()
            .find(|text| text.text.starts_with("lorem") && text.rect.min.y >= card.top() - 1.0 && text.rect.min.y <= card.bottom())
            .expect("the long body of the card");
        assert!(body.rect.height() <= 10.0 * line + 1.0, "card {id}: the body is cut to ten rows: {:?} for a row of {line}", body.rect);
        assert!(body.rect.height() >= 9.0 * line, "card {id}: ten rows of the body show: {:?}", body.rect);
        assert!(body.elided, "card {id}: the cut body says it is cut");
    }
    let last = cards[3];
    assert!(last.bottom() <= 1100.0, "the card after the long ones stays in view: {last:?}");

    let own = texts.iter().find(|text| text.text == "Locked body").expect("the locked card's body");
    assert!(cards[3].contains_rect(own.rect), "a read-only card's text stays inside its rect: {:?} in {:?}", own.rect, cards[3]);

    // The card's own padding, read from a left-to-right twin: the text starts that far in from the edge it starts at.
    let ltr = Shown::new(vec![(thread(1, "الهامش ضيق"), 100.0)], CommentsPanelState::default(), BalloonPlace::Column, room(), Direction::Ltr);
    let ltr_body = ltr.fixture.texts().iter().find(|text| text.text == "الهامش ضيق").expect("the left-to-right body").clone();
    let pad = ltr_body.rect.min.x - ltr.card(1).min.x;
    let rtl = Shown::new(vec![(thread(1, "الهامش ضيق"), 100.0)], CommentsPanelState::default(), BalloonPlace::Column, room(), Direction::Rtl);
    let card = rtl.card(1);
    let body = rtl.fixture.texts().iter().find(|text| text.text == "الهامش ضيق").expect("the right-to-left body").clone();
    assert!(body.rect.max.x >= card.max.x - pad - 0.5, "a right-to-left card's text starts at its right edge: {:?} in {card:?}", body.rect);
}
