//! The text editor over an in-memory host: lines of four chars, one unit per char.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_text::quotes::QuoteStyle;
use craft_text::ui::editor::{CaretLine, EditKind, EditorEnd, HostLine, StyleKey, TextEditor, TextEditorHost, TextSelection};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Event, ImeEvent, Key, Modifiers, Rect, pos2, vec2};

#[derive(Default)]
struct Host {
    text: String,
    vertical: bool,
    /// The host applies formatting keys.
    formats: bool,
    /// The formatting keys the editor asked for.
    styles: Vec<StyleKey>,
    /// The editor is shown with Esc cancelling (`TextEditor::esc_commits(false)`).
    esc_cancels: bool,
}

/// The chars of one line.
const LINE: usize = 4;

impl TextEditorHost for Host {
    fn text(&self) -> &str {
        &self.text
    }

    fn vertical(&self) -> bool {
        self.vertical
    }

    fn replace(&mut self, range: Range<usize>, text: &str, _: EditKind) -> bool {
        let byte = |chars: usize| self.text.char_indices().nth(chars).map_or(self.text.len(), |(at, _)| at);
        let (from, to) = (byte(range.start), byte(range.end));
        self.text.replace_range(from..to, text);
        true
    }

    fn hit(&mut self, at: Point) -> (usize, usize) {
        ((at.x.round().max(0.0) as usize).min(self.text.chars().count()), 0)
    }

    fn caret(&mut self, offset: usize, _: Option<usize>) -> Option<CaretLine> {
        Some(CaretLine::new(Point::new(offset as f64, 0.0), Point::new(offset as f64, 1.0)))
    }

    fn selection_quads(&mut self, _: Range<usize>) -> Vec<[Point; 4]> {
        Vec::new()
    }

    fn underline_segments(&mut self, _: Range<usize>) -> Vec<[Point; 2]> {
        Vec::new()
    }

    fn lines(&mut self) -> Vec<HostLine> {
        let count = self.text.chars().count();
        vec![HostLine::new(self.text.clone(), 0..count, Rect::from_min_size(pos2(0.0, 0.0), vec2(count as f32, 1.0)), 1.0, Vec::new())]
    }

    fn line_range(&mut self, _: usize) -> Range<usize> {
        0..self.text.chars().count()
    }

    fn line_step(&mut self, offset: usize, lines: i32) -> usize {
        (offset as i64 + i64::from(lines) * LINE as i64).clamp(0, self.text.chars().count() as i64) as usize
    }

    fn line_edge(&mut self, offset: usize, end: bool) -> usize {
        let _ = offset;
        if end { self.text.chars().count() } else { 0 }
    }

    fn style_key(&mut self, key: StyleKey, _: TextSelection) -> bool {
        self.styles.push(key);
        self.formats
    }
}

struct Session {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
    selection: Rc<RefCell<TextSelection>>,
    commits: Rc<Cell<u32>>,
    cancels: Rc<Cell<u32>>,
    /// Every session end the editor reported (`TextEditorOutput::ended`).
    ends: Rc<RefCell<Vec<EditorEnd>>>,
    /// Every context click the editor reported (`TextEditorOutput::context`).
    contexts: Rc<RefCell<Vec<egui::Pos2>>>,
    /// The next pass asks egui for a second pass of its frame.
    discard: Rc<Cell<bool>>,
}

fn session(text: &str, quotes: Option<QuoteStyle>) -> Session {
    session_on(Host { text: text.to_owned(), ..Host::default() }, quotes, false)
}

/// A session on `host`; with `beside`, a second editor on its own text is shown after the session's.
fn session_on(host: Host, quotes: Option<QuoteStyle>, beside: bool) -> Session {
    let text = host.text.clone();
    let host = Rc::new(RefCell::new(host));
    let selection = Rc::new(RefCell::new(TextSelection::caret_at(text.chars().count())));
    let (commits, cancels, discard) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Rc::new(Cell::new(false)));
    let (shown_host, shown_selection, shown_commits) = (Rc::clone(&host), Rc::clone(&selection), Rc::clone(&commits));
    let (shown_cancels, shown_discard) = (Rc::clone(&cancels), Rc::clone(&discard));
    let ends = Rc::new(RefCell::new(Vec::new()));
    let shown_ends = Rc::clone(&ends);
    let contexts = Rc::new(RefCell::new(Vec::new()));
    let shown_contexts = Rc::clone(&contexts);
    let mut other = (Host { text: "xyz".to_owned(), ..Host::default() }, TextSelection::caret_at(0));
    let mut fixture = Fixture::new(move |ui| {
        if shown_discard.take() {
            ui.ctx().request_discard("the trial reruns the pass");
        }
        let (mut host, mut selection) = (shown_host.borrow_mut(), shown_selection.borrow_mut());
        let room = ui.available_rect_before_wrap();
        // Document (0, 0) at the room's top left, at 100 %.
        let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let bounds = Rect::from_min_size(body.rect.min, vec2(200.0, 20.0));
            let esc_commits = !host.esc_cancels;
            let editor = TextEditor::new("Text", &mut *host, Bind::bound(&mut *selection)).esc_commits(esc_commits);
            let output = if let Some(style) = quotes { editor.smart_quotes(style) } else { editor }.show_at(ui, bounds);
            let commits = output.edit.events.iter().filter(|event| matches!(event, craft_behavior::EditEvent::Commit { .. })).count() as u32;
            shown_commits.set(shown_commits.get() + commits);
            let cancels = output.edit.events.iter().filter(|event| matches!(event, craft_behavior::EditEvent::Cancel { .. })).count() as u32;
            shown_cancels.set(shown_cancels.get() + cancels);
            shown_contexts.borrow_mut().extend(output.context);
            shown_ends.borrow_mut().extend(output.ended);
            if beside {
                let (host, selection) = (&mut other.0, &mut other.1);
                drop(TextEditor::new("Other", host, Bind::bound(selection)).show_at(ui, bounds.translate(vec2(0.0, 40.0))));
            }
        });
    });
    fixture.settle();
    Session { fixture, host, selection, commits, cancels, ends, contexts, discard }
}

impl Session {
    fn focus(&mut self) {
        let at = self.fixture.root_id().and(None::<egui::Pos2>).unwrap_or(pos2(20.0, 10.0));
        self.fixture.click_at(at);
        self.fixture.settle();
    }

    fn text(&self) -> String {
        self.host.borrow().text.clone()
    }
}

#[trial]
fn command_enter_commits_one_session_and_a_session_without_edits_commits_nothing() {
    let mut editor = session("abc", None);
    editor.focus();
    editor.fixture.key(Key::Enter, Modifiers::COMMAND).settle();
    assert_eq!(editor.commits.get(), 0, "no text changed");
    editor.fixture.type_text("d").key(Key::Enter, Modifiers::COMMAND).settle();
    assert_eq!(editor.commits.get(), 1);
    assert_eq!(editor.text(), "abcd");
}

/// A primary press on the canvas background commits and keeps the marked text as typed; a press on the text and a
/// secondary press on the background (the canvas menu acts on the live edit) leave the session open.
#[trial]
fn a_primary_press_on_the_canvas_background_commits_the_session_with_its_marked_text() {
    let mut editor = session("abc", None);
    editor.focus();
    editor.fixture.type_text("d").settle();
    editor.fixture.click_at(pos2(40.0, 10.0)).settle();
    assert_eq!(editor.commits.get(), 0, "a press on the text is the editor's");
    let away = pos2(300.0, 300.0);
    editor
        .fixture
        .frame(vec![pointer::moved(away)])
        .frame(vec![pointer::secondary(away, true)])
        .frame(vec![pointer::secondary(away, false)])
        .settle();
    assert_eq!(editor.commits.get(), 0, "a secondary press on the background leaves the session open");

    editor.focus();
    editor.fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: "kzk".to_owned(), active_range_chars: None })]).settle();
    assert_eq!(editor.text(), "abcdkzk", "the marked text is written into the text");
    editor.fixture.click_at(away).settle();
    assert_eq!(editor.commits.get(), 1, "a primary press on the canvas background commits the session");
    assert_eq!(editor.text(), "abcdkzk", "the marked text stays as typed");
}

/// A secondary click on the text is reported once as a context click at its release, and neither moves the caret,
/// selects nor ends the session: the host's menu acts on the live edit.
#[trial]
fn a_secondary_click_on_the_text_reports_a_context_click_and_starts_no_drag() {
    let mut editor = session("abc def", None);
    editor.focus();
    let before = *editor.selection.borrow();
    let at = pos2(60.0, 10.0);
    editor.fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::secondary(at, true)]).settle();
    assert!(editor.contexts.borrow().is_empty(), "nothing is reported before the button is released");
    editor.fixture.frame(vec![pointer::secondary(at, false)]).settle();
    assert_eq!(editor.contexts.borrow().len(), 1, "one context click");
    assert!(editor.contexts.borrow()[0].distance(at) < 1.0);
    assert_eq!(*editor.selection.borrow(), before, "the caret and selection stay");
    assert_eq!(editor.commits.get() + editor.cancels.get(), 0, "the session stays open");
}

/// The I-beam is the editor's only over its text and while its own press is held: elsewhere the pointer is the
/// canvas's or the rest of the UI's.
#[trial]
fn the_i_beam_shows_over_the_text_and_during_its_drag_but_not_elsewhere() {
    let mut editor = session("abc def", None);
    editor.focus();
    let i_beam = Some(craft_behavior::Cursor::System(craft_behavior::CursorIcon::Text));
    let cursor = |editor: &Session| craft_ui::runtime::cursor::current(editor.fixture.ctx());
    let (on_text, away) = (pos2(40.0, 10.0), pos2(300.0, 300.0));
    editor.fixture.frame(vec![pointer::moved(on_text)]).settle();
    assert_eq!(cursor(&editor), i_beam, "over the text");
    editor.fixture.frame(vec![pointer::moved(away)]).settle();
    assert_ne!(cursor(&editor), i_beam, "away from the text, with the session open");
    editor.fixture.frame(vec![pointer::moved(on_text)]).frame(vec![pointer::press(on_text)]).frame(vec![pointer::moved(away)]);
    assert_eq!(cursor(&editor), i_beam, "a drag that leaves the text keeps it");
    editor.fixture.frame(vec![pointer::release(away)]).settle();
    assert_ne!(cursor(&editor), i_beam, "after the release");
}

fn preedit(text: &str) -> Event {
    Event::Ime(ImeEvent::Preedit { text: text.to_owned(), active_range_chars: None })
}

/// Composing on a canvas: the marked text is written at the caret, the IME opens at the editor's caret, and
/// Command+Enter while composing commits nothing. A click in the text that arrives with its release and a late commit
/// in one raw frame keeps the marked text as typed and interrupts the composition once: one press per pass holds the
/// release and the commit for the next pass, where the commit is stale. Later events of the interrupted composition
/// are dropped until the OS completes the reset; then a new composition starts at the clicked caret. A preedit and a
/// press in one raw frame apply in that order: the marked text changes once and the press interrupts. A second editor
/// shown after the composing one leaves the composing editor's report standing.
#[trial]
fn a_click_during_composition_keeps_the_marked_text_and_interrupts_the_composition_once() {
    let mut editor = session_on(Host { text: "abc".to_owned(), ..Host::default() }, None, true);
    editor.focus();
    editor.fixture.frame(vec![preedit("kzk")]).settle();
    assert_eq!(editor.text(), "abckzk", "the marked text is written at the caret");
    let ime = editor.fixture.ime().cloned().expect("the editor owns the IME");
    assert!(!ime.interrupt);
    assert!(
        Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 20.0)).expand(1.0).contains_rect(ime.caret),
        "the IME opens at the caret: {:?}",
        ime.caret
    );
    editor.fixture.key(Key::Enter, Modifiers::COMMAND).settle();
    assert_eq!(editor.commits.get(), 0, "Enter while composing belongs to the input method");

    let at = pos2(1.0, 10.0);
    editor.fixture.frame(vec![pointer::moved(at)]);
    editor.fixture.frame(vec![pointer::press(at), pointer::release(at), Event::Ime(ImeEvent::Commit("kzk".to_owned()))]);
    let pressed = editor.fixture.ime().cloned().expect("the editor keeps the IME");
    let reset = pressed.reset.filter(|_| pressed.interrupt).expect("the press pass interrupts the composition and asks for a reset");
    for _ in 0..3 {
        editor.fixture.frame(Vec::new());
        assert!(!editor.fixture.ime().is_some_and(|ime| ime.interrupt), "the composition is interrupted once");
    }
    assert_eq!(editor.text(), "abckzk", "the marked text stays as typed and the late commit is dropped");
    assert_eq!(*editor.selection.borrow(), TextSelection::caret_at(1), "the press placed the caret");
    editor.fixture.frame(vec![preedit("q")]).settle();
    assert_eq!(editor.text(), "abckzk", "a preedit before the reset completes belongs to the interrupted composition");
    editor.fixture.frame(vec![Event::ImeResetCompleted { request_id: reset }, preedit("q")]).settle();
    assert_eq!(editor.text(), "aqbckzk", "after the reset a new composition starts at the caret");

    let at = pos2(6.0, 10.0);
    editor.fixture.frame(vec![pointer::moved(at)]);
    editor.fixture.frame(vec![preedit("qr"), pointer::press(at), pointer::release(at)]);
    let pressed = editor.fixture.ime().cloned().expect("the editor keeps the IME");
    let second = pressed.reset.filter(|_| pressed.interrupt).expect("a press after a preedit in its frame interrupts the composition");
    assert_ne!(second, reset);
    editor.fixture.settle();
    assert_eq!(editor.text(), "aqrbckzk", "the preedit before the press changes the marked text once");
    assert_eq!(*editor.selection.borrow(), TextSelection::caret_at(6), "the press placed the caret after the preedit");
    assert_eq!(editor.commits.get(), 0, "a press in the text keeps the session open");
}

/// An input method that counts the text around the caret in bytes (winit on macOS and Wayland) deletes the characters
/// those bytes cover: "é" is two bytes, "€" three, and a character the range only partly covers stays.
#[trial]
fn the_input_methods_byte_counts_delete_the_characters_around_the_caret() {
    let mut editor = session("xé€y", None);
    editor.focus();
    *editor.selection.borrow_mut() = TextSelection::caret_at(2);
    editor.fixture.settle();
    editor.fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 2 })]).settle();
    assert_eq!(editor.text(), "xé€y", "a range that ends inside a character deletes nothing of it");
    editor.fixture.frame(vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 2, after_bytes: 3 })]).settle();
    assert_eq!(editor.text(), "xy", "two bytes before cover \"é\", three after cover \"€\"");
    assert_eq!(*editor.selection.borrow(), TextSelection::caret_at(1), "the caret stays between the neighbours");
}

/// Cmd+. cancels a session with edits, though Esc commits (the default); while a composition is open it does nothing,
/// and a session without edits gives no `Cancel`. A press frame that runs a second pass cancels once.
#[trial]
fn cmd_period_cancels_the_session_but_not_while_composing() {
    use craft_ui::testing::focus::key_release;
    use craft_ui::testing::input::key_press;

    let mut editor = session("abc", None);
    editor.focus();
    editor.fixture.key(Key::Period, Modifiers::COMMAND).settle();
    assert_eq!((editor.commits.get(), editor.cancels.get()), (0, 0), "a session without edits ends with nothing to cancel");

    editor.fixture.type_text("d").settle();
    editor.fixture.frame(vec![preedit("kzk")]).settle();
    assert_eq!(editor.text(), "abcdkzk", "the composition is open");
    editor.fixture.key(Key::Period, Modifiers::COMMAND).settle();
    assert_eq!((editor.commits.get(), editor.cancels.get()), (0, 0), "Cmd+. while composing belongs to the input method");

    editor.fixture.frame(vec![Event::Ime(ImeEvent::Commit("kzk".to_owned()))]).settle();
    editor.fixture.hold(Modifiers::COMMAND).frame(Vec::new());
    editor.discard.set(true);
    editor.fixture.frame(vec![key_press(Key::Period, Modifiers::COMMAND)]);
    assert_eq!(editor.fixture.passes(), 2, "the press frame ran a second pass");
    editor.fixture.frame(vec![key_release(Key::Period, Modifiers::COMMAND)]).hold(Modifiers::NONE).settle();
    assert_eq!(editor.cancels.get(), 1, "Cmd+. cancels the session once");
    assert_eq!(editor.commits.get(), 0, "Cmd+. cancels though Esc commits");
}

/// Escape ends a session with no text change (an untouched placeholder, an empty mask) on the editor itself: it
/// commits or cancels as Esc is set to, and does not first return focus to the canvas. With typed text the session's
/// gesture ends with it.
#[trial]
fn escape_ends_an_untouched_session_and_a_typed_one() {
    let mut editor = session("abc", None);
    editor.focus();
    editor.fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(*editor.ends.borrow(), [EditorEnd::Commit], "Esc commits an untouched session");
    assert_eq!((editor.commits.get(), editor.cancels.get()), (0, 0), "no text changed, so no gesture ends");

    let mut editor = session_on(Host { text: "abc".to_owned(), esc_cancels: true, ..Host::default() }, None, false);
    editor.focus();
    editor.fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(*editor.ends.borrow(), [EditorEnd::Cancel], "Esc cancels an untouched session where it does not commit");
    editor.fixture.type_text("d").key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(*editor.ends.borrow(), [EditorEnd::Cancel, EditorEnd::Cancel], "the focus stayed on the editor");
    assert_eq!(editor.cancels.get(), 1, "the typed session's gesture is cancelled");
}

/// In vertical text Alt+Down follows the flow and loosens the tracking; a host that applies no formatting leaves
/// Alt+Up to move the caret a line up.
#[trial]
fn formatting_keys_follow_the_flow_and_fall_back_to_caret_moves() {
    let mut vertical = session_on(Host { text: "abcdefgh".to_owned(), vertical: true, formats: true, ..Host::default() }, None, false);
    vertical.focus();
    vertical.fixture.key(Key::ArrowDown, Modifiers::ALT).settle();
    let asked = vertical.host.borrow().styles.last().copied();
    assert!(matches!(asked, Some(StyleKey::Kern(by) | StyleKey::Tracking(by)) if by > 0.0), "Alt+Down tracks in vertical text: {asked:?}");

    let mut plain = session_on(Host { text: "abcdefgh".to_owned(), ..Host::default() }, None, false);
    plain.focus();
    let before = plain.selection.borrow().caret;
    assert!(before >= LINE, "the click left the caret past the first line");
    plain.fixture.key(Key::ArrowUp, Modifiers::ALT).settle();
    assert!(!plain.host.borrow().styles.is_empty(), "the host was asked for the leading step");
    assert_eq!(plain.selection.borrow().caret, before.saturating_sub(LINE), "the step the host did not take moved the caret a line up");
}

/// #15555: a session the app ends (Cmd+Enter, then the editor is not shown) on a canvas with rulers hands the focus back
/// to the canvas it came from, not to the stop before the editor (the rulers' corner, whose arrows move the zero point),
/// and the view stays where it was. Without rulers the canvas is that stop anyway.
#[trial]
fn an_ended_session_returns_the_focus_to_the_canvas_and_the_view_stays() {
    const EDITOR: craft_behavior::DocKey = craft_behavior::DocKey::new("trial.type-editor");
    use craft_ui::canvas::Rulers;
    use craft_ui::contract::Props as _;
    use craft_ui::runtime::focus::{FocusTarget, request};
    use craft_ui::testing::access::{focused_key, name};

    for rulers in [true, false] {
        let editing = Rc::new(Cell::new(false));
        let view = Rc::new(Cell::new(None::<CanvasView>));
        let (shown_editing, shown_view) = (Rc::clone(&editing), Rc::clone(&view));
        let mut host = Host { text: "abc".to_owned(), ..Host::default() };
        let mut selection = TextSelection::caret_at(3);
        let mut zero = Point::ZERO;
        let mut fixture = Fixture::new(move |ui| {
            let room = ui.available_rect_before_wrap();
            let given = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(100.0, 100.0), 1.0));
            let strips = rulers.then(|| Rulers::new("Rulers", Bind::bound(&mut zero)));
            let canvas = Canvas::new("Document").view(given).with_rulers(strips).show(ui, |ui, body| {
                if shown_editing.get() {
                    let bounds = Rect::from_min_size(body.rect.min + vec2(40.0, 40.0), vec2(200.0, 20.0));
                    drop(TextEditor::new("Text", &mut host, Bind::bound(&mut selection)).doc_key(EDITOR).show_at(ui, bounds));
                }
            });
            shown_view.set(Some(canvas.output.view.unwrap_or(given)));
        });
        fixture.room(vec2(600.0, 400.0)).settle();
        let focused = |fixture: &Fixture| name(fixture.tree(), focused_key(fixture.tree())).map(str::to_owned);
        // A press on the canvas focuses it, and the session that press starts asks for the editor's focus.
        fixture.click_at(pos2(400.0, 300.0)).settle();
        assert_eq!(focused(&fixture).as_deref(), Some("Document"), "rulers {rulers}: the press focused the canvas");
        editing.set(true);
        fixture.settle();
        request(fixture.ctx(), FocusTarget::Doc(EDITOR));
        fixture.settle();
        assert_eq!(focused(&fixture).as_deref(), Some("Text"), "rulers {rulers}: the editor took the focus");
        let before = view.get();
        editing.set(false);
        fixture.settle();
        assert_eq!(focused(&fixture).as_deref(), Some("Document"), "rulers {rulers}: the focus went back to the canvas");
        assert_eq!(view.get(), before, "rulers {rulers}: the view stays");
    }
}
