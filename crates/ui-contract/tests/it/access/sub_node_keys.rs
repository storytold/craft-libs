//! A sub-node declares its own keys and numbers: the access tree lists its shortcut, and the owner serves the
//! Increment, Decrement and SetValue requests aimed at it.

use craft_behavior::{Input, PartKey, Sense};
use craft_ui::contract::{Access, Common, Component, Cx, Focus as _, MeasureCx, Numeric, PartsCx, Props as _, Role, Show as _};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::StyleCx;
use craft_ui::testing::access::Declared;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Rect, Vec2, pos2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const HEAD: PartKey = PartKey::new("head");
const KNOB: PartKey = PartKey::new("knob");
const SHORTCUT: &str = "Alt+Down";
const START: [f64; 2] = [3.0, 7.0];
const STEP: f64 = 2.0;
const TYPED: f64 = 9.0;

/// What the owner saw this run.
struct Seen {
    opened: Cell<usize>,
    knobs: RefCell<[f64; 2]>,
    root: Cell<Id>,
}

/// A header part that declares Alt+Down and two numeric knobs, each a focus stop with a sub-node.
#[derive(craft_ui::contract::Component)]
struct Pair {
    #[common]
    common: Common,
    seen: Rc<Seen>,
}

impl Component for Pair {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn keys(&self) -> KeyUse {
        KeyUse::of(Input::Pointer).with_moves()
    }

    fn style(_: &StyleCx<'_>, _: PairStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(150.0, 30.0)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let third = rect.width() / 3.0;
        let cell = |at: f32| Rect::from_min_max(pos2(rect.min.x + third * at, rect.min.y), pos2(rect.min.x + third * (at + 1.0), rect.max.y));
        cx.part(HEAD, cell(0.0)).sense(Sense::CLICK).focusable();
        cx.part(KNOB.at(0), cell(1.0)).focusable();
        cx.part(KNOB.at(1), cell(2.0)).focusable();
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.node(HEAD, Role::BUTTON).shortcut(SHORTCUT);
        let knobs = *self.seen.knobs.borrow();
        for (at, value) in knobs.into_iter().enumerate() {
            let node = access.node(KNOB.at(at as u64), Role::SLIDER);
            node.numeric = Some(Numeric::new(value, 0.0..=10.0).step(STEP));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.intents().take(craft_behavior::Intent::Open) {
            self.seen.opened.set(self.seen.opened.get() + 1);
        }
        let active = cx.active_part().filter(|part| part.key == KNOB).and_then(|part| part.index);
        while let Some((mv, _)) = cx.intents().take_move() {
            let Some(at) = active else { continue };
            let by = match mv {
                craft_behavior::NavMove::Forward => STEP,
                craft_behavior::NavMove::Back => -STEP,
                _ => continue,
            };
            self.seen.knobs.borrow_mut()[at as usize] += by;
        }
        for at in 0..2 {
            if let Some(value) = cx.intents().take_set_value_at(KNOB.at(at)) {
                self.seen.knobs.borrow_mut()[at as usize] = value;
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

fn pair() -> (Fixture, Rc<Seen>) {
    let seen = Rc::new(Seen { opened: Cell::new(0), knobs: RefCell::new(START), root: Cell::new(Id::NULL) });
    let shown = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let response = Pair { common: Common::new(), seen: shown.clone() }.label("Pair").show(ui);
        shown.root.set(response.id);
    });
    fixture.settle();
    (fixture, seen)
}

#[trial]
fn a_numeric_sub_node_serves_increment_decrement_and_set_value() {
    let (mut fixture, seen) = pair();
    let root = seen.root.get();
    let knob = fixture.part(root, KNOB.at(1)).expect("the second knob sub-node");
    let tree = fixture.tree();
    for action in [Declared::Increment, Declared::Decrement, Declared::SetValue] {
        assert!(tree.declares_key(knob.node, action), "the knob declares {action:?}");
    }
    fixture.access(AccessActionKind::Increment, &knob);
    fixture.settle();
    assert_eq!(*seen.knobs.borrow(), [START[0], START[1] + STEP], "Increment steps the knob it targets");
    let knob = fixture.part(root, KNOB.at(1)).expect("the second knob sub-node");
    fixture.access(AccessActionKind::Decrement, &knob);
    fixture.settle();
    assert_eq!(*seen.knobs.borrow(), START, "Decrement steps it back");
    let first = fixture.part(root, KNOB.at(0)).expect("the first knob sub-node");
    fixture.access(AccessActionKind::SetValue(TYPED), &first);
    fixture.settle();
    assert_eq!(*seen.knobs.borrow(), [TYPED, START[1]], "SetValue sets the knob it targets");
}
