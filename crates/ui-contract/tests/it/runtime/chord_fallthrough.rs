//! A key press passes over a binding whose command is disabled where the press lands, and an app's catalog replaces
//! or drops a linked set's default chords: in both cases the app's own binding of the chord runs. The case is a
//! word processor's Alt+F7 (next misspelling) against the framework's `window.move` default, pressed inside a
//! floating window.

use craft_behavior::{GesturePhase, Sense};
use craft_ui::contract::overlay::{PlacementChange, PlacementKind};
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Mount, Overlay, Props as _, Role, Show as _, ShowContainer as _, Sizing,
};
use craft_ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandSet, CommandState, Commands as _, Scope, WindowCommand};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::runtime::frame::take_output;
use craft_ui::style::StyleCx;
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
use std::cell::RefCell;

/// The app's command on Alt+F7.
const SPELLING: &str = "chordTrial.spelling";
const HEADER: Vec2 = Vec2::new(100.0, 24.0);
const BODY: Vec2 = Vec2::new(120.0, 60.0);
const START: Rect = Rect { min: Pos2::new(60.0, 70.0), max: Pos2::new(280.0, 210.0) };

/// A floating window that can say its Move command is disabled, and logs the placement changes it is given.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Floating<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    move_disabled: bool,
    log: &'a RefCell<Vec<PlacementChange>>,
}

impl Component for Floating<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FloatingStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Floating<'_> {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        allocate(ui, HEADER);
        if self.move_disabled {
            cx.publish(WindowCommand::Move, CommandState::new().enabled(false));
        }
    }

    fn placement_changed(&mut self, change: &PlacementChange) {
        self.log.borrow_mut().push(*change);
    }
}

/// What Alt+F7 does with the focus inside a floating window, whose Move is disabled with `move_disabled`, under an app
/// catalog binding it to `SPELLING` that drops `window.move`'s defaults with `dropped`: the commands it fired into
/// the output, and whether a keyboard move began.
fn alt_f7(move_disabled: bool, dropped: bool) -> (Vec<String>, bool) {
    let Ok(binding) = Binding::parse("Alt+F7") else { return (vec!["the trial's chord does not parse".to_owned()], false) };
    let Ok(catalog) = AppCatalog::new(vec![CommandDef::new(SPELLING, "Spelling", Scope::app()).with_default(binding)], 1) else {
        return (vec!["the trial's catalog is refused".to_owned()], false);
    };
    let catalog = if dropped { catalog.replace_defaults(WindowCommand::Move.id(), Vec::new()) } else { catalog };
    let (log, seen) = (RefCell::new(Vec::new()), RefCell::new(Seen::default()));
    let mut show = |ui: &mut Ui| {
        let props = ContainerProps::new()
            .mount(Mount::Layer(Overlay::floating().movable(true).placed(START)))
            .sizing(Sizing::Content)
            .header(HeaderRegion::new().band(DragBand::Move))
            .command_scope(<WindowCommand as CommandSet>::SCOPE);
        Floating { common: Common::new(), props, move_disabled, log: &log }.id_salt("floating").show(ui, |ui, _| {
            Probe::new(&seen).size(BODY).sense(Sense::CLICK | Sense::FOCUS).show(ui);
        });
    };
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog));
    frames.idle(4, &mut show);
    // The focus goes into the window, where its commands are.
    let Some(inside) = seen.borrow().id else { return (vec!["the window's body never showed".to_owned()], false) };
    request(&frames.ctx, FocusTarget::Id(inside));
    frames.idle(2, &mut show);
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(inside), "the window's body has the focus");
    take_output(&frames.ctx);
    log.borrow_mut().clear();
    frames.frame(vec![key_press(Key::F7, Modifiers::ALT)], &mut show);
    frames.idle(2, &mut show);
    let fired = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.id).collect();
    let began = log.borrow().iter().any(|change| change.kind == PlacementKind::Keyboard && change.phase == GesturePhase::Begin);
    (fired, began)
}

/// By default Alt+F7 inside a floating window is its Move (16.3), ahead of the app's binding.
#[trial]
fn alt_f7_in_a_floating_window_moves_it_by_default() {
    assert_eq!(alt_f7(false, false), (Vec::new(), true), "the window's Move takes Alt+F7");
}

/// A disabled Move leaves its chord to the next binding: the app's command runs, where the press used to be dropped.
#[trial]
fn a_disabled_binding_leaves_its_chord_to_the_next_one() {
    assert_eq!(alt_f7(true, false), (vec![SPELLING.to_owned()], false), "the disabled Move passes Alt+F7 to the app's command");
}

/// An app that drops `window.move`'s default chords through its catalog gets Alt+F7 for its own command, even where
/// Move is enabled.
#[trial]
fn an_app_catalog_drops_a_linked_set_s_default_chord() {
    assert_eq!(alt_f7(false, true), (vec![SPELLING.to_owned()], false), "with Move's defaults dropped, Alt+F7 is the app's");
}
