//! A press on the canvas background focuses the canvas, also while a control in its body holds the framework's focus:
//! a tool's claimed chords then reach the tool.

use craft_behavior::CancelReason;
use craft_keymap::{Chord, Key as ChordKey, Mods};
use craft_ui::buttons::Button;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::popover::Popover;
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Key, Modifiers, vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A tool that claims plain Enter and takes every key it is offered.
#[derive(Default)]
struct EnterClaimer {
    seen: Vec<ToolKey>,
}

impl CanvasTool for EnterClaimer {
    fn claims(&self) -> ToolClaims {
        static KEYS: std::sync::OnceLock<[Chord; 1]> = std::sync::OnceLock::new();
        ToolClaims::NONE.keys(KEYS.get_or_init(|| [Chord::new(Mods::NONE, ChordKey::Enter)]))
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        self.seen.push(key.clone());
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the claimer's form")
    }
}

#[trial]
fn a_press_on_the_background_focuses_the_canvas_from_a_body_control() {
    let open = Rc::new(Cell::new(false));
    let shown = Rc::clone(&open);
    let mut canvas = ToolFixture::with_body("Document", EnterClaimer::default(), move |ui, _| {
        let opener = Button::new("Options").show(ui);
        let mut is_open = shown.get();
        Popover::new("Options panel", Bind::bound(&mut is_open)).opened_by(&opener).show(ui, |ui, _| {
            let _ = Button::new("Item").show(ui);
        });
        shown.set(is_open);
    });
    canvas.fixture().settle();
    open.set(true);
    canvas.fixture().idle(4);
    canvas.fixture().key(Key::Escape, Modifiers::NONE).settle();
    assert!(!open.get(), "Escape closed the layer");
    canvas.fixture().idle(10);
    let canvas_id = canvas.canvas_id();
    let opener = canvas.fixture().focused_stop();
    assert!(opener.is_some_and(|(id, _)| id != canvas_id), "the opener in the canvas body has the focus: {opener:?}");
    // The background: the canvas's far corner, away from the opener at its start.
    let background = canvas.canvas().rect.max - vec2(20.0, 20.0);
    canvas.fixture().click_at(background).settle();
    assert_eq!(canvas.fixture().focused_stop().map(|(id, _)| id), Some(canvas_id), "the press on the background focuses the canvas");
    canvas.fixture().key(Key::Enter, Modifiers::NONE).settle();
    assert!(matches!(canvas.tool().seen.as_slice(), [ToolKey::Kept(_)]), "the tool's claimed Enter arrives: {:?}", canvas.tool().seen);
}
