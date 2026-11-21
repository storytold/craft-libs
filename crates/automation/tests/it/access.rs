//! Named controls through the protocol: a `control` target places a click on the control, `ui.find` describes it, and
//! a target that is ambiguous, unknown or asked for too early queues nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_automation::access::{AMBIGUOUS, NO_CONTROL, NO_TREE, access_method};
use craft_automation::synthetic::input_method;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, clicked};
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Pos2, ViewportId};
use serde_json::{Value, json};

/// Two buttons and a text field named "Name", with the clicks each button got and where the pointer was.
struct Rig {
    fx: Fixture,
    host: WindowHost,
    clicks: Rc<[Cell<u32>; 2]>,
    pointer: Rc<Cell<Option<Pos2>>>,
}

impl Rig {
    fn new(names: [&'static str; 2]) -> Self {
        let clicks: Rc<[Cell<u32>; 2]> = Rc::default();
        let pointer = Rc::new(Cell::new(None));
        let text = RefCell::new(String::new());
        let (seen, at) = (clicks.clone(), pointer.clone());
        let fx = Fixture::new(move |ui| {
            for (n, name) in names.iter().enumerate() {
                if clicked(&Button::new(*name).show(ui)) {
                    seen[n].set(seen[n].get() + 1);
                }
            }
            TextField::new("Name", Bind::bound(&mut *text.borrow_mut())).show(ui);
            at.set(ui.input(|input| input.pointer.latest_pos()));
        });
        let host = WindowHost::of(fx.ctx());
        Self { fx, host, clicks, pointer }
    }

    /// The same, after the host kept its first access tree.
    fn kept(names: [&'static str; 2]) -> Self {
        let mut rig = Self::new(names);
        rig.host.keep_access_trees();
        rig.fx.idle(2);
        rig
    }

    /// Sends `method`; when it is accepted, runs frames until the host has processed the input.
    fn send(&mut self, method: &str, params: Value) -> Result<(), String> {
        input_method(method, &params, &self.host, ViewportId::ROOT, &|_, _| None).expect("an input method")?;
        for _ in 0..32 {
            self.fx.frame(Vec::new());
            if self.host.injected_idle() {
                break;
            }
        }
        assert!(self.host.injected_idle(), "{method} was processed");
        self.fx.frame(Vec::new());
        Ok(())
    }

    fn find(&self, params: Value) -> Result<Value, String> {
        access_method("ui.find", &params, &self.host, ViewportId::ROOT).expect("an access method")
    }

    fn counts(&self) -> [u32; 2] {
        [self.clicks[0].get(), self.clicks[1].get()]
    }
}

/// A click `{control: {name}}` presses that button and no other, also for a text field named by its label, and
/// `ui.find` reports where the control is.
#[trial]
fn a_click_on_a_named_control_lands_on_it() {
    let mut rig = Rig::kept(["Save", "Cancel"]);
    rig.send("ui.click", json!({"control": {"name": "Save"}})).expect("valid");
    assert_eq!(rig.counts(), [1, 0]);
    rig.send("ui.click", json!({"control": {"name": "Cancel:"}})).expect("valid");
    assert_eq!(rig.counts(), [1, 1], "a trailing colon of the name is ignored");

    let field = rig.find(json!({"name": "Name"})).expect("the field is found");
    assert_eq!(field["enabled"], json!(true));
    assert_eq!(field["focused"], json!(false));
    let rect: Vec<f64> = field["rect"].as_array().expect("a rect").iter().map(|v| v.as_f64().expect("a number")).collect();
    rig.send("ui.click", json!({"control": {"name": "Name"}})).expect("valid");
    let at = rig.pointer.get().expect("the pointer arrived");
    let (x, y) = (f64::from(at.x), f64::from(at.y));
    assert!(x >= rect[0] && x <= rect[0] + rect[2] && y >= rect[1] && y <= rect[1] + rect[3], "{at:?} lies in {rect:?}");
    assert_eq!(rig.find(json!({"name": "Name"})).expect("found")["focused"], json!(true), "the click focused the field");
}

/// A name two controls share needs an `index`; an unknown name matches none; neither queues anything. Before the
/// first kept pass there is no tree to look in.
#[trial]
fn an_ambiguous_or_missing_name_queues_nothing() {
    let mut early = Rig::new(["Go", "Go"]);
    assert_eq!(early.send("ui.click", json!({"control": {"name": "Go"}})), Err(NO_TREE.to_owned()));
    assert_eq!(early.find(json!({"name": "Go"})), Err(NO_TREE.to_owned()));

    let mut rig = Rig::kept(["Go", "Go"]);
    let ambiguous = rig.send("ui.click", json!({"control": {"name": "Go"}})).expect_err("two match");
    assert!(ambiguous.starts_with(AMBIGUOUS), "{ambiguous}");
    let missing = rig.send("ui.click", json!({"control": {"name": "Stop"}})).expect_err("none matches");
    assert!(missing.starts_with(NO_CONTROL), "{missing}");
    assert!(rig.host.injected_idle(), "nothing was queued");
    assert_eq!(rig.counts(), [0, 0]);

    rig.send("ui.click", json!({"control": {"name": "Go", "index": 1}})).expect("valid");
    assert_eq!(rig.counts(), [0, 1], "index 1 is the second in reading order");
}
