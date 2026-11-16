//! The input cap: a text, paste or IME event longer than `INPUT_TEXT_LIMIT` characters is cut before any component
//! sees it, so no component lays out or stores an unbounded paste.

use craft_behavior::{EditorInput, PartKey, Sense, TextPos};
use craft_ui::contract::{Common, Component, Cx, Input, PaintUi, PartsCx, Props as _, Role, Show as _};
use craft_ui::primitives::{EditorLine, EditorPurpose, EditorRun, EditorState};
use craft_ui::runtime::commands::Commands as _;
use craft_ui::runtime::egui::input::INPUT_TEXT_LIMIT;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::runtime::pending::Pending;
use craft_ui::style::StyleCx;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::click;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Event, Id, ImeEvent, Key, Modifiers, Pos2, Rect, Response};
use std::cell::RefCell;

const AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(210.0, 90.0) };
const SHOWN: &str = "abc";
const EDITOR: PartKey = PartKey::new("editor");

/// An editor that logs the inputs it receives and the length of each paste it takes. With `held` it keeps and shows
/// a document: the typed text joins it. A prose editor (`purpose`) lets Tab move the focus; a code editor keeps it.
#[derive(craft_ui::contract::Component)]
struct Probe<'a> {
    #[common]
    common: Common,
    log: &'a RefCell<Vec<EditorInput>>,
    purpose: EditorPurpose,
    held: Option<&'a RefCell<String>>,
    pastes: Option<&'a RefCell<Vec<usize>>>,
}

impl<'a> Probe<'a> {
    /// A code editor (its editor part keeps Tab) that keeps no text.
    fn new(log: &'a RefCell<Vec<EditorInput>>) -> Self {
        Probe { common: Common::new(), log, purpose: EditorPurpose::Code, held: None, pastes: None }
    }
}

impl Component for Probe<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::of(Input::Editor);
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ProbeStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, self.purpose);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let inputs: Vec<EditorInput> = cx.editor_input().collect();
        if let Some(held) = self.held {
            for input in &inputs {
                if let EditorInput::Text(text) = input {
                    held.borrow_mut().push_str(text);
                }
            }
        }
        self.log.borrow_mut().extend(inputs);
        if let (Some(pastes), Pending::Ready(content)) = (self.pastes, cx.clipboard().paste())
            && let Some(text) = &content.text
        {
            pastes.borrow_mut().push(text.chars().count());
        }
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        // The held document is ASCII, so its bytes are its characters.
        let held = self.held.map(RefCell::borrow);
        let shown = held.as_ref().map_or(SHOWN, |text| text.as_str());
        let positions: Vec<f32> = (0..shown.len()).map(|at| at as f32 * 8.0).collect();
        let widths = vec![8.0; shown.len()];
        let runs = [EditorRun::new(0..shown.len(), false, &positions, &widths)];
        let line = Rect::from_min_size(cx.content_rect.min, egui::vec2(cx.content_rect.width(), 16.0));
        let lines = [EditorLine::new(shown, line, line.bottom(), &runs)];
        let caret = Rect::from_min_size(line.min + egui::vec2(shown.len() as f32 * 8.0, 0.0), egui::vec2(0.0, 16.0));
        cx.editor_state(EditorState::new(caret, &lines, TextPos::new(0, shown.len())));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// `count` two-byte characters: a cut that splits one would leave invalid text.
fn long(count: usize) -> String {
    "é".repeat(count)
}

/// The texts and preedits the focused editor received in one frame with `events`.
fn received(frames: &mut Frames, log: &RefCell<Vec<EditorInput>>, events: Vec<Event>) -> Vec<EditorInput> {
    frames.frame(events, |ui| {
        Probe::new(log).label("Source").id_salt("capped").show_at(ui, AT);
    });
    log.take().into_iter().filter(|input| matches!(input, EditorInput::Text(_) | EditorInput::Preedit { .. })).collect()
}

#[trial]
fn oversized_text_events_are_cut_before_components_see_them() {
    let log = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        Probe::new(&log).label("Source").id_salt("capped").show_at(ui, AT);
    };
    frames.idle(3, &mut show);
    click(&mut frames, AT.center(), &mut show);
    log.take();

    let cut = |inputs: &[EditorInput]| match inputs {
        [EditorInput::Text(text)] => Some((text.chars().count(), text.len())),
        _ => None,
    };
    let typed = received(&mut frames, &log, vec![Event::Text(long(INPUT_TEXT_LIMIT + 10))]);
    assert_eq!(cut(&typed), Some((INPUT_TEXT_LIMIT, INPUT_TEXT_LIMIT * 2)), "typed text is cut at a character boundary");
    let committed = received(&mut frames, &log, vec![Event::Ime(ImeEvent::Commit(long(INPUT_TEXT_LIMIT + 1)))]);
    assert_eq!(cut(&committed), Some((INPUT_TEXT_LIMIT, INPUT_TEXT_LIMIT * 2)), "an IME commit is cut");

    // The cap is per event: text that fits arrives whole, however much the field holds already.
    for _ in 0..2 {
        let whole = received(&mut frames, &log, vec![Event::Text(long(INPUT_TEXT_LIMIT))]);
        assert_eq!(cut(&whole), Some((INPUT_TEXT_LIMIT, INPUT_TEXT_LIMIT * 2)), "an event at the limit is not cut");
    }

    let preedit = |range: std::ops::Range<usize>| Event::Ime(ImeEvent::Preedit { text: long(INPUT_TEXT_LIMIT + 5), active_range_chars: Some(range) });
    let composing = |inputs: &[EditorInput]| match inputs {
        [EditorInput::Preedit { text, cursor, .. }] => Some((text.chars().count(), cursor.clone())),
        _ => None,
    };
    let past = received(&mut frames, &log, vec![preedit(INPUT_TEXT_LIMIT + 1..INPUT_TEXT_LIMIT + 3)]);
    assert_eq!(composing(&past), Some((INPUT_TEXT_LIMIT, None)), "a preedit is cut and its range past the cut dropped");
    let inside = received(&mut frames, &log, vec![preedit(1..2)]);
    assert_eq!(composing(&inside), Some((INPUT_TEXT_LIMIT, Some(2..4))), "a range inside the cut stays");

    // A paste reaches an app's own focused widget cut too.
    let field = Id::unique("app_field");
    let pasted = RefCell::new(None);
    let mut frames = Frames::new();
    let mut host = |ui: &mut egui::Ui| {
        ui.interact(AT, field, egui::Sense::click());
        let paste = ui.input(|input| input.events.iter().find_map(|event| if let Event::Paste(text) = event { Some(text.clone()) } else { None }));
        if let Some(text) = paste {
            pasted.replace(Some(text));
        }
    };
    frames.idle(2, &mut host);
    craft_ui::testing::wheel::focus(&frames.ctx, field);
    frames.idle(2, &mut host);
    frames.frame(vec![Event::Paste(long(INPUT_TEXT_LIMIT * 2))], &mut host);
    let pasted = pasted.take().map(|text| (text.chars().count(), text.len()));
    assert_eq!(pasted, Some((INPUT_TEXT_LIMIT, INPUT_TEXT_LIMIT * 2)), "a paste is cut before the field reads it");

    // Tab and a paste in one frame: the paste reaches only the field Tab moves to, cut as any paste is.
    let (first, second) = (RefCell::new(Vec::new()), RefCell::new(Vec::new()));
    let (first_pastes, second_pastes) = (RefCell::new(Vec::new()), RefCell::new(Vec::new()));
    let (first_at, second_at) = (AT, AT.translate(egui::vec2(0.0, 100.0)));
    let mut fields = |ui: &mut egui::Ui| {
        Probe { purpose: EditorPurpose::Text, pastes: Some(&first_pastes), ..Probe::new(&first) }
            .label("First")
            .id_salt("first")
            .show_at(ui, first_at);
        Probe { purpose: EditorPurpose::Text, pastes: Some(&second_pastes), ..Probe::new(&second) }
            .label("Second")
            .id_salt("second")
            .show_at(ui, second_at);
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut fields);
    click(&mut frames, first_at.center(), &mut fields);
    frames.idle(1, &mut fields);
    first_pastes.take();
    second_pastes.take();
    frames.frame(vec![key_press(Key::Tab, Modifiers::NONE), Event::Paste(long(INPUT_TEXT_LIMIT + 10))], &mut fields);
    assert!(first_pastes.take().is_empty(), "the field Tab leaves takes none of the paste");
    assert_eq!(second_pastes.take(), [INPUT_TEXT_LIMIT], "the field Tab moves to takes the paste, cut");

    // A cut never truncates document text: a field that holds more than the limit keeps it whole, and typing joins it.
    let held = RefCell::new("x".repeat(INPUT_TEXT_LIMIT + 10));
    let mut frames = Frames::new();
    let mut document = |ui: &mut egui::Ui| {
        Probe { held: Some(&held), ..Probe::new(&log) }.label("Document").id_salt("document").show_at(ui, AT);
    };
    frames.idle(3, &mut document);
    click(&mut frames, AT.center(), &mut document);
    frames.frame(vec![Event::Text("y".repeat(INPUT_TEXT_LIMIT + 1))], &mut document);
    frames.frame(vec![Event::Text("z".into())], &mut document);
    let held = held.take();
    assert_eq!(held.len(), INPUT_TEXT_LIMIT * 2 + 11, "the held text stays whole and each event joins it, cut to the limit");
    assert!(held.starts_with(&"x".repeat(INPUT_TEXT_LIMIT + 10)) && held.ends_with('z'), "nothing of the document was cut");
}
