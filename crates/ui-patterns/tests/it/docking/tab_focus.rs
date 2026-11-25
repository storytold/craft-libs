//! A pointer press on a dock's panel tab leaves key focus where it was, so a hosted app's own shortcuts keep working
//! after a tab click, a tab drag or the tab menu; a press on an app's own egui body is the app's, and the area names
//! the panel pressed into (`DockAreaOutput::pressed`).

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Panel 2's body is the app's own egui code; panels 0 and 1 are plain panels in the other group.
const HOSTED: usize = 2;

#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == HOSTED { spec.host_body() } else { spec }
    }
}

struct Panels;

impl DockPanels<usize> for Panels {
    /// The hosted body holds an app's own egui button.
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *panel == HOSTED {
            let _ = ui.button("App button");
        }
    }
}

/// The panels the area named as pressed into, one entry per pass that named one.
type Pressed = Rc<RefCell<Vec<usize>>>;

/// A dock of the plain group [0, 1] beside the hosted group [2], the workspace it edits and the panels it reported
/// pressed.
fn dock() -> (Fixture, Rc<RefCell<Arc<Workspace<usize>>>>, Pressed) {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0, 1], 0), Node::tabs(vec![HOSTED], 0));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let pressed: Pressed = Rc::default();
    let (held, seen, specs) = (Rc::clone(&workspace), Rc::clone(&pressed), Tracked::new(Specs));
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = held.borrow().clone();
        let out = DockArea::new("Dock", Bind::bound(&mut shown), &specs, &mut Panels, DockConfig::default()).show(ui);
        seen.borrow_mut().extend(out.pressed);
        *held.borrow_mut() = shown;
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    (fixture, workspace, pressed)
}

/// A secondary press and release at `at`, then an idle frame.
fn secondary_click(fixture: &mut Fixture, at: Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![moved(at)]).frame(vec![button(true)]).frame(vec![button(false)]).idle(1);
}

#[trial]
fn a_press_on_a_panel_tab_leaves_key_focus_where_it_was() {
    let (mut fixture, workspace, _) = dock();
    let tab = fixture.by_label("Panel 1").expect("the first group's second tab");
    fixture.click(&tab).idle(1);
    let shows_one =
        matches!(workspace.borrow().layout().root.as_ref(), Some(Node::Split { first, .. }) if matches!(&**first, Node::Tabs { active: 1, .. }));
    assert!(shows_one, "the click shows panel 1");
    assert!(fixture.focused_stop().is_none(), "a click on a tab takes no key focus");

    let tab = fixture.by_label("Panel 1").expect("the tab to drag");
    let hosted = fixture.by_label("Panel 2").expect("the hosted group's tab").rect;
    let by = Pos2::new(hosted.center().x, hosted.bottom() + 100.0) - tab.center();
    fixture.drag(&tab, by).idle(2);
    assert!(fixture.focused_stop().is_none(), "a tab drag takes no key focus");

    let tab = fixture.by_label("Panel 0").expect("a tab for the menu");
    secondary_click(&mut fixture, tab.center());
    assert!(fixture.by_label("Float").is_some(), "the tab menu opened");
    fixture.key(Key::Escape, Modifiers::NONE).idle(2);
    assert!(fixture.by_label("Float").is_none(), "Escape closed the menu");
    assert!(fixture.focused_stop().is_none(), "with the tab menu closed, no key focus is left behind");
}

#[trial]
fn a_press_on_an_app_body_is_the_apps_and_one_on_a_plain_body_focuses_its_panel() {
    let (mut fixture, ..) = dock();
    let hosted = fixture.by_label("Panel 2").expect("the hosted group's tab").rect;
    fixture.click_at(Pos2::new(hosted.center().x, hosted.bottom() + 200.0)).idle(1);
    assert!(fixture.focused_stop().is_none(), "a press on the app's own body takes no framework focus");
    let plain = fixture.by_label("Panel 0").expect("the plain group's tab").rect;
    fixture.click_at(Pos2::new(plain.center().x, plain.bottom() + 200.0)).idle(1);
    assert!(fixture.focused_stop().is_some(), "a press on a framework panel's empty body focuses its panel");
}

/// A press in a group's body names the group's shown panel, whatever takes it: the app's own button in a host body
/// (which keeps the press and takes no framework focus) or a plain panel's empty body. A press on a tab is no body
/// press.
#[trial]
fn a_press_in_a_groups_body_names_its_panel_whatever_takes_it() {
    let (mut fixture, _, pressed) = dock();
    let button = fixture.by_label("App button").expect("the app's own button in the hosted body");
    fixture.click(&button).idle(1);
    assert_eq!(pressed.borrow().as_slice(), [HOSTED], "a press on the app's own widget names the hosted panel");
    pressed.borrow_mut().clear();
    let plain = fixture.by_label("Panel 0").expect("the plain group's tab");
    fixture.click_at(Pos2::new(plain.rect.center().x, plain.rect.bottom() + 200.0)).idle(1);
    assert_eq!(pressed.borrow().as_slice(), [0], "a press on a plain body names its shown panel");
    pressed.borrow_mut().clear();
    fixture.click(&plain).idle(1);
    assert!(pressed.borrow().is_empty(), "a press on a tab is no body press: {:?}", pressed.borrow());
}
