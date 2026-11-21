//! `LayersPanel` in a host's memoized view: what the panel derives from its tracked tree, and when.

use craft_layers::ui::filter::{FilterBy, LayerFilter};
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::{LayersEvent, LayersPanel};
use craft_ui::contract::{Bind, MemoProps, MemoView, Outbox, Tracked};
use craft_ui::testing::{Fixture, MemoRun, last_frame_derived_builds, last_frame_memo_runs, pointer, trial};
use egui::{Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The panel's room: wide enough for a row's parts, tall enough for the sample tree and the footer.
const PANEL: Vec2 = Vec2::new(420.0, 560.0);

type Shared<T> = Rc<RefCell<T>>;

/// What a host's view of the panel reads: the tracked tree, the filter the app stores, and an outbox for the events.
#[derive(MemoProps)]
struct Host<'a> {
    tree: &'a Tracked<FakeLayers>,
    filter: Bind<'a, LayerFilter>,
    events: Outbox<'a, LayersEvent<u32>>,
}

fn host(props: &mut Host<'_>, ui: &mut Ui) {
    let out = LayersPanel::new(props.tree).filter(props.filter.reborrowed()).show(ui);
    for event in out.events {
        props.events.send(event);
    }
}

/// A fixture showing the panel over `tree` in a host view, with the shared tree, filter and delivered events.
struct Rig {
    fixture: Fixture,
    tree: Shared<Tracked<FakeLayers>>,
    filter: Shared<LayerFilter>,
    seen: Shared<Vec<LayersEvent<u32>>>,
}

fn rig(tree: FakeLayers) -> Rig {
    let tree = Rc::new(RefCell::new(Tracked::new(tree)));
    let filter: Shared<LayerFilter> = Rc::default();
    let seen: Shared<Vec<LayersEvent<u32>>> = Rc::default();
    let (shown, by, sink) = (Rc::clone(&tree), Rc::clone(&filter), Rc::clone(&seen));
    let mut fixture = Fixture::new(move |ui| {
        let tree = shown.borrow();
        let mut filter = by.borrow_mut();
        let mut events = sink.borrow_mut();
        MemoView::new("layers", Host { tree: &tree, filter: Bind::bound(&mut filter), events: Outbox::new(&mut events) }, host).show(ui);
    });
    fixture.room(PANEL);
    fixture.settle().idle(2);
    Rig { fixture, tree, filter, seen }
}

impl Rig {
    /// The builds of every pass of the last frame.
    fn builds(&self) -> u32 {
        last_frame_derived_builds(self.fixture.ctx()).iter().map(|(_, builds)| builds).sum()
    }

    /// The host view's runs and replays in the last frame.
    fn host_runs(&self) -> (u32, u32) {
        let runs = last_frame_memo_runs(self.fixture.ctx());
        let count = |wanted: MemoRun| runs.iter().filter(|(name, _, run)| name.contains("Host") && *run == wanted).count() as u32;
        (count(MemoRun::Ran), count(MemoRun::Replayed))
    }

    /// One frame with no input, after the edit of the trial.
    fn frame(&mut self) -> &mut Self {
        self.fixture.frame(Vec::new());
        self
    }
}

/// An idle panel of 50 layers builds nothing and replays; so do the frames after it.
#[trial]
fn an_idle_panel_builds_nothing_and_replays() {
    let mut rig = rig(FakeLayers::fifty());
    for _ in 0..3 {
        rig.frame();
        assert_eq!(rig.builds(), 0, "an idle frame builds no display order");
        assert_eq!(rig.host_runs(), (0, 1), "the host view replays");
    }
}

/// One layer edit through the `Tracked` builds the display order once and runs the view once, then it replays.
#[trial]
fn one_layer_edit_builds_once() {
    let mut rig = rig(FakeLayers::fifty());
    rig.tree.borrow_mut().layers[3].info.name = String::from("Renamed");
    rig.frame();
    assert_eq!(rig.builds(), 1, "the edit builds the display order once");
    assert_eq!(rig.host_runs().0, 1, "and runs the view once");
    rig.frame();
    assert_eq!(rig.builds(), 0, "the next pass builds nothing");
    assert_eq!(rig.host_runs(), (0, 1), "and replays");
}

/// A filter change builds the display order once.
#[trial]
fn a_filter_change_builds_once() {
    let mut rig = rig(FakeLayers::fifty());
    {
        let mut filter = rig.filter.borrow_mut();
        filter.by = FilterBy::Name;
        filter.name = String::from("mark");
    }
    rig.frame();
    assert_eq!(rig.builds(), 1, "the filter builds the display order once");
    rig.frame();
    assert_eq!(rig.builds(), 0, "the next pass builds nothing");
    assert_eq!(rig.host_runs(), (0, 1), "and the view replays");
}

/// A pass that runs the view for another reason, the pointer over it, keeps the display order.
#[trial]
fn a_pass_that_runs_the_view_for_the_pointer_builds_nothing() {
    let mut rig = rig(FakeLayers::fifty());
    rig.fixture.frame(vec![pointer::moved(egui::Pos2::new(100.0, 200.0))]);
    assert_eq!(rig.host_runs().0, 1, "the pointer over the panel runs the view");
    assert_eq!(rig.builds(), 0, "and the display order is kept");
}

/// An event from a click reaches the host once, in the run that produced it, and not again while the view replays.
#[trial]
fn a_clicks_event_is_delivered_once_across_the_run_and_the_replays() {
    let mut rig = rig(FakeLayers::sample());
    let row = rig.fixture.by_label("Title").expect("the Title row shows");
    rig.seen.borrow_mut().clear();
    rig.fixture.click(&row).idle(4);
    let selects = rig.seen.borrow().iter().filter(|event| matches!(event, LayersEvent::Select(_))).count();
    assert_eq!(selects, 1, "{:?}", rig.seen.borrow());
}

/// A pointer move beside the panel replays its view: the panel reads the modifiers it shows its rows with, not the
/// pointer, so only a modifier change runs it (#2812). Before, every pointer move in the window ran it.
#[trial]
fn a_pointer_move_beside_the_panel_replays_it_and_a_modifier_runs_it() {
    let tree = Tracked::new(FakeLayers::fifty());
    let mut fixture = Fixture::new(move |ui| {
        let mut filter = LayerFilter::default();
        let mut events = Vec::new();
        let at = egui::Rect::from_min_size(ui.max_rect().min, PANEL);
        ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
            MemoView::new("layers", Host { tree: &tree, filter: Bind::bound(&mut filter), events: Outbox::new(&mut events) }, host).show(ui);
        });
    });
    fixture.room(Vec2::new(PANEL.x + 400.0, PANEL.y));
    fixture.settle().idle(2);
    let runs = |fixture: &Fixture| {
        let runs = last_frame_memo_runs(fixture.ctx());
        let count = |wanted: MemoRun| runs.iter().filter(|(name, _, run)| name.contains("Host") && *run == wanted).count();
        (count(MemoRun::Ran), count(MemoRun::Replayed))
    };
    // The pointer enters the window beside the panel; in an app it is already there.
    fixture.frame(vec![pointer::moved(egui::Pos2::new(PANEL.x + 200.0, 90.0))]);
    for step in 0..3 {
        fixture.frame(vec![pointer::moved(egui::Pos2::new(PANEL.x + 200.0, 100.0 + step as f32 * 10.0))]);
        assert_eq!(runs(&fixture), (0, 1), "pointer move {step} beside the panel replays its view");
    }
    fixture.hold(egui::Modifiers::ALT).frame(Vec::new());
    assert_eq!(runs(&fixture).0, 1, "Alt held runs the view");
    fixture.frame(Vec::new());
    assert_eq!(runs(&fixture), (0, 1), "and it replays while Alt stays held");
}
