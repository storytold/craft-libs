//! Named controls through the protocol (`control` targets, `ui.find`), `ui.inspect` over the window's kept access
//! tree, and `ui.click` and `ui.focus` on a node it names: tree order and filters, what an open modal hides, a click,
//! a context menu and a focus reaching their widget, and the targets refused before anything is queued.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_automation::access::{AMBIGUOUS, NO_CONTROL, NO_TREE, NodeTarget, access_method, inspect, resolve};
use craft_automation::synthetic::input_method;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::dialog::Dialog;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::egui::access::Declared;
use craft_ui::runtime::egui::injected::hook_without_pass;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::sections::{DialogRole, FooterButton};
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

/// The listed widgets of `params`, from the window's kept tree.
fn listed(host: &WindowHost, params: Value) -> Value {
    inspect(host, ViewportId::ROOT, &params).expect("an inspect reply")
}

/// The labels of a reply's widgets, in its order.
fn labels(reply: &Value) -> Vec<String> {
    reply["widgets"].as_array().into_iter().flatten().filter_map(|w| w["label"].as_str().map(str::to_owned)).collect()
}

/// The widget of a reply labelled `label`.
fn widget<'a>(reply: &'a Value, label: &str) -> &'a Value {
    reply["widgets"].as_array().into_iter().flatten().find(|w| w["label"] == label).unwrap_or_else(|| panic!("{label} is listed: {reply}"))
}

/// Where `label` sits in `order`.
fn place(order: &[String], label: &str) -> usize {
    order.iter().position(|l| l == label).unwrap_or_else(|| panic!("{label} is listed in {order:?}"))
}

/// Every reported node is listed once in tree order with its state, the root left out; `query`, `role` and `focused`
/// keep a subset, `limit` cuts the list but not the count; a node behind an open modal is neither listed nor
/// resolved, and the modal's own buttons are.
#[trial]
fn inspect_lists_reported_nodes_in_tree_order_with_their_filters() {
    let open = Rc::new(Cell::new(false));
    let (shown, name) = (open.clone(), Rc::new(RefCell::new(String::from("Draft"))));
    let mut fx = Fixture::new(move |ui| {
        Button::new("Open").show(ui);
        Button::new("Save").show(ui);
        Button::new("Archive").enabled(false).show(ui);
        TextField::new("Name", Bind::bound(&mut *name.borrow_mut())).show(ui);
        let mut now = shown.get();
        Dialog::new("Options").opened(Bind::bound(&mut now)).buttons([FooterButton::new("Apply", DialogRole::Accept)]).show(ui, |_, _| {});
        shown.set(now);
    });
    let host = WindowHost::of(fx.ctx());
    host.keep_access_tree();
    fx.settle();

    let all = listed(&host, json!({}));
    let order = labels(&all);
    assert!(place(&order, "Open") < place(&order, "Save") && place(&order, "Save") < place(&order, "Archive"), "tree order: {order:?}");
    let widgets = all["widgets"].as_array().expect("a widget list");
    assert_eq!(all["count"].as_u64(), Some(widgets.len() as u64), "{all}");
    assert_eq!(all["truncated"], false);
    assert!(widgets.iter().all(|w| w["depth"].as_u64().is_some_and(|depth| depth >= 1)), "the root is not listed: {all}");
    let ids: Vec<&str> = widgets.iter().filter_map(|w| w["id"].as_str()).collect();
    assert!(ids.iter().enumerate().all(|(i, id)| !ids[..i].contains(id)), "each node once: {ids:?}");
    let (save, archive) = (widget(&all, "Save"), widget(&all, "Archive"));
    assert!(save["enabled"] == true && save["clickable"] == true, "{save}");
    assert!(archive["enabled"] == false, "{archive}");
    let rect = |w: &Value| w["rect"].as_array().map(|r| r.iter().filter_map(Value::as_f64).collect::<Vec<_>>()).expect("a rect");
    assert!(rect(widget(&all, "Open"))[3] <= rect(save)[1] + 0.5, "Open lies above Save");

    let queried = listed(&host, json!({"query": "SAV"}));
    assert!(labels(&queried).contains(&"Save".to_owned()) && !labels(&queried).contains(&"Open".to_owned()), "{queried}");
    let buttons = listed(&host, json!({"role": "button"}));
    assert!(["Open", "Save", "Archive"].iter().all(|label| labels(&buttons).contains(&(*label).to_owned())), "{buttons}");
    assert!(buttons["widgets"].as_array().into_iter().flatten().all(|w| w["role"].as_str().is_some_and(|role| role.eq_ignore_ascii_case("button"))));
    let fields = listed(&host, json!({"role": "textinput"}));
    assert_eq!(fields["count"], 1, "the one text field, its role matched ignoring case: {fields}");
    assert!(buttons["widgets"].as_array().into_iter().flatten().all(|w| w["id"] != fields["widgets"][0]["id"]), "the field is no button");

    let (focused, unfocused) = (listed(&host, json!({"focused": true})), listed(&host, json!({"focused": false})));
    assert_eq!(focused["count"].as_u64().unwrap_or(0) + unfocused["count"].as_u64().unwrap_or(0), all["count"].as_u64().unwrap_or(0));
    assert!(focused["widgets"].as_array().into_iter().flatten().all(|w| w["focused"] == true), "{focused}");

    let first = listed(&host, json!({"limit": 1}));
    assert_eq!(first["widgets"].as_array().map(Vec::len), Some(1));
    assert_eq!((first["count"].clone(), first["truncated"].clone()), (all["count"].clone(), json!(true)));
    assert_eq!(first["widgets"][0], all["widgets"][0], "the limit keeps the first in tree order");
    assert!(inspect(&host, ViewportId::ROOT, &json!({"nodes": true})).is_err(), "an unknown field is refused");

    open.set(true);
    fx.idle(30);
    let tree = host.access_tree_view(ViewportId::ROOT).expect("the kept tree");
    let behind = listed(&host, json!({}));
    assert!(!labels(&behind).contains(&"Open".to_owned()), "a node behind the modal is not listed: {behind}");
    assert!(labels(&behind).contains(&"Apply".to_owned()), "the modal's own button is: {behind}");
    assert!(resolve(&tree, &NodeTarget::Label("Open".into()), Declared::Click).is_err(), "nor is it resolved");
    assert!(resolve(&tree, &NodeTarget::Label("apply".into()), Declared::Click).is_ok(), "a label matches ignoring case");
}

/// A click by label activates its button once; a secondary click opens the node's context menu; a focus by the id
/// `ui.inspect` lists moves the key focus there. A missing, ambiguous, disabled or undeclared target is refused and
/// queues nothing.
#[trial]
fn click_and_focus_by_label_or_id_reach_the_widget_and_refuse_what_they_cannot_name() {
    let saves = Rc::new(Cell::new(0));
    let (counted, name) = (saves.clone(), Rc::new(RefCell::new(String::new())));
    let mut fx = Fixture::new(move |ui| {
        if Button::new("Save").show(ui).clicked() {
            counted.set(counted.get() + 1);
        }
        let layer = Button::new("Layer").show(ui);
        Menu::context("Layer menu", &layer).show(ui, |ui, _| {
            MenuItem::new("Delete layer").show(ui);
        });
        Button::new("Copy").id_salt("first copy").show(ui);
        Button::new("Copy").id_salt("second copy").show(ui);
        Button::new("Archive").enabled(false).show(ui);
        TextField::new("Name", Bind::bound(&mut *name.borrow_mut())).show(ui);
    });
    let (host, ctx) = (WindowHost::of(fx.ctx()), fx.ctx().clone());
    host.keep_access_tree();
    fx.settle();
    let none = |_: usize, _: Pos2| None;
    let send = |fx: &mut Fixture, method: &str, params: Value| {
        input_method(method, &params, &host, ViewportId::ROOT, &none).expect("an input method").expect("a target it can name");
        fx.run_injected().settle();
    };

    send(&mut fx, "ui.click", json!({"label": "save"}));
    assert_eq!(saves.get(), 1, "the click by label activated Save once");

    let field = listed(&host, json!({"role": "TextInput"}))["widgets"][0]["id"].clone();
    assert!(field.is_string(), "the text field is listed with its id");
    let before = listed(&host, json!({"focused": true}));
    assert!(before["widgets"].as_array().into_iter().flatten().all(|w| w["id"] != field), "the field has no focus yet: {before}");
    send(&mut fx, "ui.focus", json!({"id": field}));
    let focused = listed(&host, json!({"focused": true}));
    assert_eq!(focused["widgets"].as_array().map(|w| w.iter().map(|w| w["id"].clone()).collect::<Vec<_>>()), Some(vec![field]), "{focused}");

    // Refused while no popup is open, so each is refused for what it names, not for a covered window.
    let refused = [
        ("ui.click", json!({"label": "Nothing here"})),
        ("ui.click", json!({"id": "1"})),
        ("ui.click", json!({"label": "copy"})),
        ("ui.click", json!({"label": "Archive"})),
        ("ui.click", json!({"label": "Save", "button": "secondary"})),
        ("ui.focus", json!({"label": "Nothing here"})),
    ];
    for (method, params) in refused {
        let answer = input_method(method, &params, &host, ViewportId::ROOT, &none);
        assert!(matches!(answer, Some(Err(_))), "{method} {params} is refused: {answer:?}");
        assert!(host.injected_idle(), "{method} {params} left nothing waiting");
        assert!(hook_without_pass(&ctx, ViewportId::ROOT).is_empty(), "{method} {params} queued nothing");
    }
    fx.settle();
    assert_eq!(saves.get(), 1, "no refused request clicked");

    assert_eq!(listed(&host, json!({"query": "Delete layer"}))["count"], 0, "the menu starts closed");
    send(&mut fx, "ui.click", json!({"label": "Layer", "button": "secondary"}));
    assert!(listed(&host, json!({"query": "Delete layer"}))["count"].as_u64().is_some_and(|n| n >= 1), "the context menu opened");
    assert_eq!(saves.get(), 1, "nothing else was clicked");
}
