//! Stories of the pins, the thread popover and the composer.

use craft_behavior::CanvasView;
use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, clicked};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use kurbo::Point;

use super::pins::{Face, PinBubble};
use super::{CommentComposer, CommentPin, CommentPins, ComposerClose, ComposerDraft, NoteDrafts, NotePopup, NotePopupMessage, ThreadPopover};
use crate::comments::{CommentAnchor, CommentMessage, CommentThread, ThreadId};

const NOW: i64 = 90_000;

fn thread() -> CommentThread {
    let message = |author: &str, text: &str, created: i64| CommentMessage {
        author: author.into(),
        created,
        modified: None,
        text: text.into(),
        reactions: Vec::new(),
        read: true,
        deletable: true,
    };
    CommentThread {
        id: ThreadId(7),
        anchor: CommentAnchor::Canvas { at: [0.0, 0.0], node: None },
        kind: None,
        color: None,
        messages: vec![message("Ana", "Check @Bo before we ship.", NOW - 7_200), message("Bo", "On it.", NOW - 600)],
        statuses: Vec::new(),
        reply_statuses: Vec::new(),
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
    let pin = |id: u64, x: f32, y: f32, author: &'static str| {
        CommentPin::new(ThreadId(id), pos2(x, y), author, "Check the margin", [200, 80, 40]).unread(id == 1).resolved(id == 3)
    };
    let pins = [pin(1, 60.0, 80.0, "Ana"), pin(2, 220.0, 80.0, "Me"), pin(3, 230.0, 84.0, "Bo")];
    CommentPins::new("Pins", &pins, "Me").open(Some(ThreadId(1))).show(ui).into()
}

#[story(CommentPins<'static>, covers = [], drive = Drive::Never)]
fn point_and_area_threads_on_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut point = thread();
    point.anchor = CommentAnchor::Canvas { at: [52.0, 68.0], node: None };
    point.messages[0].text = "Align this corner.".into();
    let mut area = thread();
    area.id = ThreadId(8);
    area.anchor = CommentAnchor::CanvasArea { bounds: kurbo::Rect::new(120.0, 58.0, 206.0, 125.0) };
    area.messages[0].text = "Check the spacing in this area.".into();
    let threads = [point, area];

    let zoomed = data.value("zoomed", false);
    let zoom_changed = clicked(&Button::new(if *zoomed { "Zoom out" } else { "Zoom in" }).show(ui));
    *zoomed ^= zoom_changed;
    let zoom = if *zoomed { 1.35 } else { 1.0 };
    let panned = data.value("panned", false);
    let pan_changed = clicked(&Button::new(if *panned { "Reset pan" } else { "Pan right" }).show(ui));
    *panned ^= pan_changed;
    let center = Point::new(if *panned { 125.0 } else { 145.0 }, 85.0);
    let (room, _) = ui.allocate_exact_size(vec2(335.0, 205.0), Sense::hover());
    let view = CanvasView::new(room, center, zoom);
    let mut canvas = ui.new_child(UiBuilder::new().id_salt("comment_canvas").max_rect(room));
    canvas.set_clip_rect(room.intersect(ui.clip_rect()));
    // Original, procedurally drawn demo content. Edges inside the rectangle remain visible.
    let content = canvas.painter();
    for row in 0..5 {
        for column in 0..8 {
            let at = Point::new(f64::from(column) * 32.0, f64::from(row) * 32.0);
            let end = Point::new(at.x + 32.0, at.y + 32.0);
            let shade = if (row + column) % 2 == 0 { 86 } else { 148 };
            content.rect_filled(Rect::from_two_pos(view.to_screen(at), view.to_screen(end)), 0.0, Color32::from_gray(shade));
        }
    }

    let pins: Vec<_> = threads
        .iter()
        .filter_map(|thread| {
            let first = thread.messages.first()?;
            CommentPin::from_canvas_anchor(thread.id, &thread.anchor, &view, &first.author, &first.text, [200, 80, 40])
        })
        .collect();
    let open = data.value("open", Some(threads[1].id));
    let shown = CommentPins::new("Comments", &pins, "Me").open(*open).show(&mut canvas);
    *open = shown.opened.or(open.filter(|id| Some(*id) != shown.closed));
    let selected = *open;

    if let Some((thread, pin)) = threads.iter().zip(&pins).find(|(thread, _)| Some(thread.id) == selected) {
        let draft = data.value("reply", String::new());
        let reply = ThreadPopover::new(thread, &shown.response, "Me", &[], draft).anchor(Rect::from_min_max(pin.point, pin.point)).now(NOW).show(ui);
        if reply.closed && !zoom_changed && !pan_changed {
            *data.value("open", None::<ThreadId>) = None;
        }
    }
    shown.into()
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

#[story(CommentComposer<'static>, covers = [], drive = Drive::Never)]
fn comment_composer_edit_draft(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let opener = Button::new("Edit").show(ui);
    let draft = data.value("draft", "Move this heading up.\n\n".to_owned());
    let _ = CommentComposer::new(Some(pos2(40.0, 60.0)), draft)
        .header("Typewriter", "Ana")
        .draft_kind(ComposerDraft::EditVerbatim)
        .close(ComposerClose::OnPostOrCancel)
        .show(ui);
    opener.into()
}

#[story(CommentComposer<'static>, covers = [], drive = Drive::Never)]
fn comment_composer_hint(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let opener = Button::new("Insert").show(ui);
    let draft = data.value("draft", String::new());
    let _ = CommentComposer::new(Some(pos2(40.0, 60.0)), draft).header("Inserted Text", "Ana").hint("Text to insert").show(ui);
    opener.into()
}

#[story(CommentComposer<'static>, covers = [], drive = Drive::Never)]
fn comment_composer_long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let opener = Button::new("Add").show(ui);
    let text = "The caption under the second figure runs into the margin on the printed proof, and the figure number \
                does not match the one the text cites. Check both against the list of figures before the next round.";
    let draft = data.value("draft", text.to_owned());
    let _ = CommentComposer::new(Some(pos2(40.0, 60.0)), draft).header("Sticky Note", "Bo").draft_kind(ComposerDraft::Edit).show(ui);
    opener.into()
}

/// A note's message: the note itself at depth 0, a reply deeper.
fn note_message(id: u64, author: &'static str, text: &'static str, depth: usize) -> NotePopupMessage<'static> {
    NotePopupMessage { id, title: "Sticky Note", author, text, modified: Some(NOW - 600 * id as i64), depth, editable: depth == 0 }
}

/// Where the story's note popup first opens, from the end corner of its opener.
const POPUP_OFFSET: Vec2 = vec2(80.0, 40.0);
/// The size the story's note popup first opens at.
const POPUP_SIZE: Vec2 = vec2(260.0, 200.0);

/// A note pop-up over `messages`, opened from a button: the button's rect stands for the note's mark.
/// The drafts and the window's rect live in the story's data, as a host keeps them, so a reply box, typed text, a move
/// and a resize last across frames.
fn note_popup(ui: &mut Ui, data: &mut StoryData, messages: &[NotePopupMessage<'_>], allowed: bool, drafts: NoteDrafts) -> StoryOutput {
    let opener = Button::new("Open note").show(ui);
    let open = data.value("open", false);
    *open |= clicked(&opener);
    if *open {
        let mark = opener.rect;
        let at = *data.value("rect", None::<Rect>);
        let rect = at.unwrap_or_else(|| Rect::from_min_size(mark.right_bottom() + POPUP_OFFSET, POPUP_SIZE));
        let drafts = data.value("drafts", drafts);
        let shown = NotePopup::new(1, "Note", rect, messages, drafts)
            .color(Some([224, 48, 48]))
            .anchor(mark, opener.hovered())
            .now(NOW)
            .allowed(allowed)
            .show(ui);
        if let Some((placed, _)) = shown.placed {
            *data.value("rect", None::<Rect>) = Some(placed);
        }
        *data.value("open", false) = !shown.closed;
    }
    opener.into()
}

#[story(NotePopup<'static>, covers = [Default], drive = Drive::Never)]
fn note_popup_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let messages = [note_message(1, "Ana", "Tighten the leading in this paragraph.", 0)];
    note_popup(ui, data, &messages, true, NoteDrafts::default())
}

#[story(NotePopup<'static>, covers = [], drive = Drive::Never)]
fn note_popup_replies(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let messages = [
        note_message(1, "Ana", "Tighten the leading in this paragraph.", 0),
        note_message(2, "Bo", "How tight? 12 on 14?", 1),
        note_message(3, "Ana", "12 on 13.5.", 2),
    ];
    note_popup(ui, data, &messages, true, NoteDrafts::default())
}

#[story(NotePopup<'static>, covers = [], drive = Drive::Never)]
fn note_popup_reply_box(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let messages = [note_message(1, "Ana", "Tighten the leading in this paragraph.", 0)];
    note_popup(ui, data, &messages, true, NoteDrafts::default().with_reply(1, 1, "Done in the next proof."))
}

#[story(NotePopup<'static>, covers = [], drive = Drive::Never)]
fn note_popup_locked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let messages = [note_message(1, "Ana", "Signed off; no more changes.", 0), note_message(2, "Bo", "Agreed.", 1)];
    note_popup(ui, data, &messages, false, NoteDrafts::default())
}

#[story(NotePopup<'static>, covers = [], drive = Drive::Never)]
fn note_popup_long_thread(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let text = "The running head on the left pages still shows the old chapter title, and the folio drops below the \
                trim on pages 40 to 52. Both need fixing before the files go to the printer.";
    let mut messages = vec![note_message(1, "Ana", text, 0)];
    // Deeper than the indent goes, so the last replies stop nesting.
    for depth in 1..=10 {
        let author = if depth % 2 == 0 { "Ana" } else { "Bo" };
        messages.push(note_message(depth as u64 + 1, author, "Checked; see the next reply.", depth));
    }
    note_popup(ui, data, &messages, true, NoteDrafts::default())
}
