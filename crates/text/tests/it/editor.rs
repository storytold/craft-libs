//! The text editor over an in-memory host: lines of four chars, one unit per char.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use craft_behavior::kurbo::Point;
use craft_text::quotes::QuoteStyle;
use craft_text::ui::editor::{CaretLine, EditKind, HostLine, StyleKey, TextEditor, TextEditorHost, TextSelection};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rect, pos2, vec2};

#[derive(Default)]
struct Host {
    text: String,
    revision: u64,
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

    fn revision(&self) -> u64 {
        self.revision
    }

    fn vertical(&self) -> bool {
        self.vertical
    }

    fn replace(&mut self, range: Range<usize>, text: &str, _: EditKind) -> bool {
        let byte = |chars: usize| self.text.char_indices().nth(chars).map_or(self.text.len(), |(at, _)| at);
        let (from, to) = (byte(range.start), byte(range.end));
        self.text.replace_range(from..to, text);
        self.revision += 1;
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
    session_on(Host { text: text.to_owned(), ..Host::default() }, quotes)
}

fn session_on(host: Host, quotes: Option<QuoteStyle>) -> Session {
    let text = host.text.clone();
    let host = Rc::new(RefCell::new(host));
    let selection = Rc::new(RefCell::new(TextSelection::caret_at(text.chars().count())));
    let commits = Rc::new(Cell::new(0));
    let (shown_host, shown_selection, shown_commits) = (Rc::clone(&host), Rc::clone(&selection), Rc::clone(&commits));
    let mut fixture = Fixture::new(move |ui| {
        let (mut host, mut selection) = (shown_host.borrow_mut(), shown_selection.borrow_mut());
        Canvas::new("Document").show(ui, |ui, body| {
            let bounds = Rect::from_min_size(body.rect.min, vec2(200.0, 20.0));
            let editor = TextEditor::new("Text", &mut *host, Bind::bound(&mut *selection));
            let output = if let Some(style) = quotes { editor.smart_quotes(style) } else { editor }.show_at(ui, bounds);
            let commits = output.edit.events.iter().filter(|event| matches!(event, craft_behavior::EditEvent::Commit { .. })).count() as u32;
            shown_commits.set(shown_commits.get() + commits);
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

/// In vertical text Alt+Down follows the flow and loosens the tracking; a host that applies no formatting leaves
/// Alt+Up to move the caret a line up.
#[trial]
fn formatting_keys_follow_the_flow_and_fall_back_to_caret_moves() {
    let mut vertical = session_on(Host { text: "abcdefgh".to_owned(), vertical: true, formats: true, ..Host::default() }, None);
    vertical.focus();
    vertical.fixture.key(Key::ArrowDown, Modifiers::ALT).settle();
    let asked = vertical.host.borrow().styles.last().copied();
    assert!(matches!(asked, Some(StyleKey::Kern(by) | StyleKey::Tracking(by)) if by > 0.0), "Alt+Down tracks in vertical text: {asked:?}");

    let mut plain = session_on(Host { text: "abcdefgh".to_owned(), ..Host::default() }, None);
    plain.focus();
    let before = plain.selection.borrow().caret;
    assert!(before >= LINE, "the click left the caret past the first line");
    plain.fixture.key(Key::ArrowUp, Modifiers::ALT).settle();
    assert!(!plain.host.borrow().styles.is_empty(), "the host was asked for the leading step");
    assert_eq!(plain.selection.borrow().caret, before.saturating_sub(LINE), "the step the host did not take moved the caret a line up");
}
