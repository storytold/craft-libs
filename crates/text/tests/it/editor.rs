//! The text editor over an in-memory host: lines of four chars, one unit per char.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_text::quotes::QuoteStyle;
use craft_text::ui::editor::{CaretLine, EditKind, HostLine, StyleKey, TextEditor, TextEditorHost, TextSelection};
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
}

fn session(text: &str, quotes: Option<QuoteStyle>) -> Session {
    session_on(Host { text: text.to_owned(), ..Host::default() }, quotes, false)
}

/// A session on `host`; with `beside`, a second editor on its own text is shown after the session's.
fn session_on(host: Host, quotes: Option<QuoteStyle>, beside: bool) -> Session {
    let text = host.text.clone();
    let host = Rc::new(RefCell::new(host));
    let selection = Rc::new(RefCell::new(TextSelection::caret_at(text.chars().count())));
    let commits = Rc::new(Cell::new(0));
    let (shown_host, shown_selection, shown_commits) = (Rc::clone(&host), Rc::clone(&selection), Rc::clone(&commits));
    let mut other = (Host { text: "xyz".to_owned(), ..Host::default() }, TextSelection::caret_at(0));
    let mut fixture = Fixture::new(move |ui| {
        let (mut host, mut selection) = (shown_host.borrow_mut(), shown_selection.borrow_mut());
        let room = ui.available_rect_before_wrap();
        // Document (0, 0) at the room's top left, at 100 %.
        let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let bounds = Rect::from_min_size(body.rect.min, vec2(200.0, 20.0));
            let editor = TextEditor::new("Text", &mut *host, Bind::bound(&mut *selection));
            let output = if let Some(style) = quotes { editor.smart_quotes(style) } else { editor }.show_at(ui, bounds);
            let commits = output.edit.events.iter().filter(|event| matches!(event, craft_behavior::EditEvent::Commit { .. })).count() as u32;
            shown_commits.set(shown_commits.get() + commits);
            if beside {
                let (host, selection) = (&mut other.0, &mut other.1);
                drop(TextEditor::new("Other", host, Bind::bound(selection)).show_at(ui, bounds.translate(vec2(0.0, 40.0))));
            }
        });
    });
    fixture.settle();
    Session { fixture, host, selection, commits }
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
