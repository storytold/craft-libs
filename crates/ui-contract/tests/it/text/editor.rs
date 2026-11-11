//! The editor contract: a component with an editor part receives one ordered input stream, a read-only editor
//! receives only inputs that change no text, and its visible lines become AccessKit run nodes.

use craft_behavior::{EditorInput, PartKey, Sense, TextPos};
use craft_ui::contract::{Common, Component, Cx, PaintUi, PartsCx, Props as _, Role, Show as _};
use craft_ui::primitives::{EditorLine, EditorPurpose, EditorRun, EditorState};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::StyleCx;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::click;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Response};
use std::cell::{Cell, RefCell};

const AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(210.0, 90.0) };
const TEXT: &str = "fn main";
const EDITOR: PartKey = PartKey::new("editor");

/// An editor that keeps no text: it logs the inputs it receives and reports `TEXT` as its one line.
#[derive(craft_ui::contract::Component)]
struct Probe<'a> {
    #[common]
    common: Common,
    log: &'a RefCell<Vec<EditorInput>>,
}

impl Component for Probe<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::CODE;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ProbeStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, EditorPurpose::Code);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.log.borrow_mut().extend(cx.editor_input());
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let positions: Vec<f32> = (0..TEXT.len()).map(|at| at as f32 * 8.0).collect();
        let widths = vec![8.0; TEXT.len()];
        let runs = [EditorRun::new(0..TEXT.len(), false, &positions, &widths)];
        let line = Rect::from_min_size(cx.content_rect.min, egui::vec2(cx.content_rect.width(), 16.0));
        let lines = [EditorLine::new(TEXT, line, line.bottom(), &runs)];
        let end = TextPos::new(0, TEXT.len());
        let caret = Rect::from_min_size(line.min + egui::vec2(TEXT.len() as f32 * 8.0, 0.0), egui::vec2(0.0, 16.0));
        cx.editor_state(EditorState::new(caret, &lines, end, 0));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

struct Rig {
    frames: Frames,
    log: RefCell<Vec<EditorInput>>,
    id: Cell<Id>,
    read_only: bool,
}

impl Rig {
    fn new() -> Rig {
        let mut rig = Rig { frames: Frames::new(), log: RefCell::new(Vec::new()), id: Cell::new(Id::NULL), read_only: false };
        for _ in 0..3 {
            rig.frame(Vec::new());
        }
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        let (log, id, read_only) = (&self.log, &self.id, self.read_only);
        self.frames.frame(events, |ui| show(ui, log, id, read_only));
    }

    fn focus(&mut self) {
        let (log, id, read_only) = (&self.log, &self.id, self.read_only);
        click(&mut self.frames, AT.center(), &mut |ui| show(ui, log, id, read_only));
        self.log.borrow_mut().clear();
    }

    fn take(&self) -> Vec<EditorInput> {
        self.log.take()
    }
}

fn show(ui: &mut egui::Ui, log: &RefCell<Vec<EditorInput>>, id: &Cell<Id>, read_only: bool) {
    let response = Probe { common: Common::new(), log }.label("Source").read_only(read_only).id_salt("code").show_at(ui, AT);
    id.set(response.id);
}

/// Text, Backspace, Tab and Enter reach an editor as one input stream in the order they were typed; a read-only
/// editor still receives moves but none of the inputs that change text.
#[trial]
fn editor_input_reaches_an_editor_part_and_read_only_filtering_drops_edits() {
    let mut rig = Rig::new();
    rig.focus();
    rig.frame(vec![Event::Text("x".into())]);
    rig.frame(vec![key_press(Key::Backspace, Modifiers::NONE)]);
    rig.frame(vec![key_press(Key::Tab, Modifiers::NONE)]);
    rig.frame(vec![key_press(Key::Enter, Modifiers::NONE)]);
    let inputs = rig.take();
    assert!(
        matches!(inputs.as_slice(), [EditorInput::Text(text), EditorInput::Delete { forward: false, .. }, EditorInput::Tab, EditorInput::Newline] if &**text == "x"),
        "the inputs arrive in the pass they are typed, in order: {inputs:?}"
    );

    rig.read_only = true;
    rig.frame(vec![Event::Text("y".into())]);
    rig.frame(vec![key_press(Key::Tab, Modifiers::NONE)]);
    rig.frame(vec![key_press(Key::Backspace, Modifiers::NONE)]);
    assert!(rig.take().is_empty(), "a read-only editor receives no edit");
    rig.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE)]);
    let moves = rig.take();
    assert!(matches!(moves.as_slice(), [EditorInput::Move(..)]), "moves still arrive: {moves:?}");
}
