//! Behaviour of the RAW Develop workspace: the filmstrip, the footer's buttons, the history, Before and the scope.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_behavior::placement::Placement;
use craft_behavior::{Monitor, MonitorKey, PixelSize};
use craft_host::target::ShellInfo;
use craft_imaging::raw::{Demosaic, RawDevelopSettings};
use craft_imaging::scopes::ClippingMask;
use craft_imaging::ui::raw_develop::{
    RawDevelopEvent, RawDevelopOpen, RawDevelopSource, RawDevelopWorkspace, RawFilmItem, RawFinish, RawPreviewRequest, RawTarget,
};
use craft_imaging::ui::scopes::{ScopeFrame, ScopeReadout};
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_imaging::ui::{ChannelColor, HistogramSet, ScopeChannel};
use craft_ui::InstallSpec;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::dialog::{DialogHost, DialogResult, FullWindowWorkspace as _, HostedWorkspace, OWN_WINDOW_MIN_SIZE};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::windows::{FakeSystem, NoWindows};
use craft_ui::testing::{Fixture, Target, pointer, trial};
use craft_ui_test_kit::windows::Windows;
use craft_window::restore::Store;
use egui::{Color32, ColorImage, Event, Key, Modifiers, Rect, Vec2, ViewportId, ViewportInfo, pos2};

const IMAGE: [u32; 2] = [400, 200];

/// A flat picture that counts how often it is read.
struct Grey {
    level: u8,
    reads: AtomicUsize,
}

impl Grey {
    fn new(level: u8) -> Self {
        Grey { level, reads: AtomicUsize::new(0) }
    }
}

impl RasterSource for Grey {
    fn size(&self) -> PixelSize {
        PixelSize::new(IMAGE[0], IMAGE[1])
    }

    fn revision(&self) -> Revision {
        Revision::new(u64::from(self.level))
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        (since == self.revision()).then(PixelRect::default)
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        self.reads.fetch_add(1, Ordering::Relaxed);
        out.pixels.fill(Color32::from_gray(self.level));
    }
}

/// The app's side: two pictures, a count of the develops asked for, and whether the scope gets a histogram.
struct Source {
    after: Grey,
    before: Grey,
    develops: AtomicUsize,
    histogram: bool,
}

impl Source {
    /// A source whose scope shows no frame.
    fn new() -> Arc<Self> {
        Arc::new(Source { after: Grey::new(0x80), before: Grey::new(0x40), develops: AtomicUsize::new(0), histogram: false })
    }

    /// A source whose scope shows a flat histogram, so the plot and its tone zones show.
    fn with_histogram() -> Arc<Self> {
        Arc::new(Source { after: Grey::new(0x80), before: Grey::new(0x40), develops: AtomicUsize::new(0), histogram: true })
    }
}

impl RawDevelopSource for Source {
    fn after(&self) -> &dyn RasterSource {
        &self.after
    }

    fn before(&self) -> &dyn RasterSource {
        &self.before
    }

    fn develop(&self, _: &RawPreviewRequest) -> Job<PreviewDone, String> {
        self.develops.fetch_add(1, Ordering::Relaxed);
        Job::new(|_, _| Ok(PreviewDone::new(Duration::from_millis(5))))
    }

    fn auto(&self, settings: &RawDevelopSettings) -> RawDevelopSettings {
        settings.clone()
    }

    fn scope(&self, _: bool, _: bool, _: Option<[f64; 2]>, show: &mut dyn FnMut(Option<ScopeFrame<'_>>)) {
        if !self.histogram {
            return show(None);
        }
        let bins = [10_u32; 256];
        let channels = [ScopeChannel::new("rgb", "RGB", ChannelColor::Neutral, &bins, false)];
        show(Some(ScopeFrame::histogram(HistogramSet::new(&channels))));
    }

    fn clipping(&self) -> Option<(Arc<ClippingMask>, Revision)> {
        None
    }

    fn sample(&self, _: [f64; 2], before: bool, _: bool) -> Option<ScopeReadout> {
        let level = f32::from(if before { self.before.level } else { self.after.level }) / 255.0;
        Some(ScopeReadout::Rgb([level; 3]))
    }

    fn has_selection(&self) -> bool {
        false
    }
}

type Hosted = HostedWorkspace<RawDevelopWorkspace>;

fn open(target: RawTarget) -> RawDevelopOpen {
    let items = ["IMG_0001.CR3", "IMG_0002.CR3"].into_iter().map(RawFilmItem::new).collect();
    RawDevelopOpen::new("IMG_0001.CR3", PixelSize::new(IMAGE[0], IMAGE[1]), target, RawDevelopSettings::default())
        .demosaic(Demosaic::Ahd)
        .filmstrip(items, 0)
}

/// A host the fixture runs every pass; `rerun` asks the next pass to run twice inside its frame.
fn fixture(host: &Rc<RefCell<DialogHost>>, rerun: &Rc<Cell<bool>>) -> Fixture {
    let (host, rerun) = (Rc::clone(host), Rc::clone(rerun));
    let mut fixture = Fixture::new(move |ui| {
        if rerun.replace(false) {
            ui.ctx().request_discard("a second pass inside the frame");
        }
        host.borrow_mut().run(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture
}

fn finish(host: &Rc<RefCell<DialogHost>>) -> Option<RawFinish> {
    host.borrow_mut().take::<Hosted>().and_then(DialogResult::ok).map(|result| result.finish)
}

fn exposure(host: &Rc<RefCell<DialogHost>>) -> f32 {
    host.borrow_mut().get_mut::<Hosted>().map_or(f32::NAN, |hosted| hosted.workspace().session().exposure)
}

fn contrast(host: &Rc<RefCell<DialogHost>>) -> f32 {
    host.borrow_mut().get_mut::<Hosted>().map_or(f32::NAN, |hosted| hosted.workspace().session().contrast)
}

/// The first slider named `name`. A slider field's number field carries the same name and comes first, so the
/// lookup is by role. The scope's tone zones come before the panel's rows.
fn slider(fixture: &Fixture, name: &str) -> Target {
    let tree = fixture.tree();
    let found = fixture.by_role(Role::SLIDER).into_iter().find(|target| tree.name_of(target.node) == Some(name));
    found.unwrap_or_else(|| panic!("no {name} slider"))
}

/// A click to the right of the named slider's middle: it sets a positive value.
fn nudge(fixture: &mut Fixture, name: &str) {
    let slider = slider(fixture, name);
    fixture.click_at(slider.center() + Vec2::new(40.0, 0.0)).idle(4);
}

/// A click on the canvas, which takes the focus: the workspace's keys reach it only with the focus inside.
fn focus_canvas(fixture: &mut Fixture) -> Target {
    let canvas = fixture.by_role(Role::CANVAS).into_iter().next().expect("the workspace shows a canvas");
    fixture.click(&canvas).idle(2);
    canvas
}

#[trial]
fn raw_develop_remembers_its_native_window_between_open_and_filter() {
    let store = Arc::new(Mutex::new(Store::in_memory()));
    let context = || Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).placement(store.clone()));
    let first = context();
    let mut host = DialogHost::new();
    host.open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::OpenRaw)), Source::new()));

    let inner_size = Vec2::new(1040.0, 760.0);
    let outer_pos = pos2(118.0, 60.0);
    first.child_info(ViewportInfo {
        inner_rect: Some(Rect::from_min_size(pos2(120.0, 90.0), inner_size)),
        outer_rect: Some(Rect::from_min_size(outer_pos, inner_size + Vec2::new(4.0, 32.0))),
        native_pixels_per_point: Some(2.0),
        ..ViewportInfo::default()
    });
    for _ in 0..3 {
        first.frame(vec![], vec![], |ui| {
            host.run(ui);
        });
    }
    let saved = store.lock().unwrap().get("craft_imaging.raw_develop.window").expect("the Raw Develop window's placement was saved");
    assert_eq!(saved.normal.size, <[f32; 2]>::from(inner_size));
    assert_eq!(saved.position_px, Some([236, 120]));

    // Windows can report a parked rectangle just before the minimized window stops painting.
    let parked = Rect::from_min_size(pos2(-32000.0, -32000.0), Vec2::new(160.0, 40.0));
    first.child_info(ViewportInfo {
        inner_rect: Some(parked),
        outer_rect: Some(parked),
        native_pixels_per_point: Some(2.0),
        minimized: Some(true),
        ..ViewportInfo::default()
    });
    first.frame(vec![], vec![], |ui| {
        host.run(ui);
    });
    assert_eq!(store.lock().unwrap().get("craft_imaging.raw_develop.window"), Some(saved), "minimizing keeps the normal placement");

    let second = context();
    let mut host = DialogHost::new();
    host.open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::Filter)), Source::new()));
    let output = second.frame(vec![], vec![], |ui| {
        host.run(ui);
    });
    let (_, builder) = output.builders.iter().find(|(id, _)| *id != ViewportId::ROOT).expect("Raw Develop Filter opens a child window");
    assert_eq!(builder.inner_size, Some(inner_size));
    assert_eq!(builder.position, Some(outer_pos));
}

#[trial]
fn raw_develop_window_keeps_its_size_when_the_saved_screen_is_gone() {
    let store = Arc::new(Mutex::new(Store::in_memory()));
    let mut saved = Placement::new([1040.0, 760.0], 2.0);
    saved.position_px = Some([-4400, 120]);
    saved.monitor = Some(MonitorKey::new("Disconnected", [3840, 2160]));
    store.lock().unwrap().set("craft_imaging.raw_develop.window", &saved);

    let screen = Monitor::new(MonitorKey::new("Attached", [3840, 2160]), [0, 0], [3840, 2160], 2.0);
    let system = Arc::new(FakeSystem::new());
    system.set_monitors(vec![screen]);
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), system).placement(store));
    let mut host = DialogHost::new();
    host.open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::OpenRaw)), Source::new()));
    let output = windows.frame(vec![], vec![], |ui| {
        host.run(ui);
    });
    let (_, builder) = output.builders.iter().find(|(id, _)| *id != ViewportId::ROOT).expect("Raw Develop opens a child window");
    assert_eq!(builder.inner_size, Some(Vec2::new(1040.0, 760.0)));
    let pos = builder.position.expect("the recovered position is on the attached screen");
    assert!((0.0..=880.0).contains(&pos.x) && (0.0..=320.0).contains(&pos.y), "restored position is in reach: {pos:?}");
}

/// photocraft#16595: while the window shrank each frame, the placement store saved its size, 1 pt wide at 100 %
/// scaling. Restored, the window opens no smaller than a full-window dialog's own window may be, and the OS keeps it
/// there.
#[trial]
fn raw_develop_window_saved_while_shrinking_opens_at_its_minimum() {
    let store = Arc::new(Mutex::new(Store::in_memory()));
    store.lock().unwrap().set("craft_imaging.raw_develop.window", &Placement::new([1.0, 500.0], 1.0));
    let system = Arc::new(FakeSystem::new());
    system.set_monitors(vec![Monitor::new(MonitorKey::new("Attached", [1920, 1080]), [0, 0], [1920, 1080], 1.0)]);
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), system).placement(store));
    let mut host = DialogHost::new();
    host.open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::OpenRaw)), Source::new()));
    let output = windows.frame(vec![], vec![], |ui| {
        host.run(ui);
    });
    let (_, builder) = output.builders.iter().find(|(id, _)| *id != ViewportId::ROOT).expect("Raw Develop opens a child window");
    assert_eq!(builder.inner_size, Some(Vec2::new(OWN_WINDOW_MIN_SIZE.x, 500.0)), "the saved width is raised to the minimum");
    assert_eq!(builder.min_inner_size, Some(OWN_WINDOW_MIN_SIZE), "the OS keeps the window at least that big");
}

#[trial]
fn raw_develop_switches_raws_and_finishes_with_each_footer_button() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let rerun = Rc::new(Cell::new(false));
    let mut fixture = fixture(&host, &rerun);
    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::OpenRaw)), Source::new()));
    fixture.idle(10);

    let second = fixture.by_label("IMG_0002.CR3").expect("the filmstrip's second tile");
    fixture.click(&second).idle(4);
    let events = host.borrow_mut().get_mut::<Hosted>().map(Hosted::take_events).unwrap_or_default();
    assert!(events.contains(&RawDevelopEvent::Show(1)), "a filmstrip pick reports Show(1): {events:?}");

    let next = RawDevelopSettings { exposure: 0.5, ..Default::default() };
    host.borrow_mut().get_mut::<Hosted>().expect("open").workspace_mut().show_item(1, next.clone(), next, Some(Demosaic::Bilinear));
    fixture.idle(4);
    fixture.key(Key::Z, Modifiers::COMMAND).idle(4);
    assert_eq!(exposure(&host), 0.5, "show_item starts the history over: there is nothing to undo");

    nudge(&mut fixture, "Contrast");
    nudge(&mut fixture, "Exposure");
    let (settled_exposure, settled_contrast) = (exposure(&host), contrast(&host));
    assert!(settled_contrast > 0.0 && settled_exposure != 0.5, "the clicks edit the settings");
    rerun.set(true);
    fixture.key(Key::Z, Modifiers::COMMAND).idle(4);
    assert_eq!(exposure(&host), 0.5, "undo steps the newest edit back, once even when the frame ran its pass twice");
    assert_eq!(contrast(&host), settled_contrast, "the edit before it stays");

    let open_button = fixture.by_label("Open").expect("Open");
    fixture.click(&open_button).idle(4);
    assert_eq!(finish(&host), Some(RawFinish::Open));

    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::OpenRaw)), Source::new()));
    fixture.idle(10);
    let done = fixture.by_label("Done").expect("Done");
    fixture.click(&done).idle(4);
    assert_eq!(finish(&host), Some(RawFinish::Done));

    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::Filter)), Source::new()));
    fixture.idle(10);
    assert!(fixture.by_label("Done").is_none(), "a filter has no Done");
    let ok = fixture.by_label("OK").expect("OK");
    fixture.click(&ok).idle(4);
    assert_eq!(finish(&host), Some(RawFinish::Ok));
}

#[trial]
fn a_held_ok_files_its_answer_and_keeps_the_workspace_open_until_the_app_answers() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host, &Rc::new(Cell::new(false)));
    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::OpenRaw)), Source::new()).held_ok());
    fixture.idle(10);
    let done = fixture.by_label("Done").expect("Done");
    fixture.click(&done).idle(4);
    let held = host.borrow_mut().get_mut::<Hosted>().expect("Done keeps the workspace open").take_ok();
    assert_eq!(held.map(|result| result.finish), Some(RawFinish::Done), "Done files its answer for the app");

    host.borrow_mut().get_mut::<Hosted>().expect("open").answer_ok(Ok(()));
    fixture.idle(4);
    assert_eq!(finish(&host), Some(RawFinish::Done), "the app's answer closes the workspace with the filed result");
}

/// The key `key` going down or up, as a keyboard reports it.
fn key_event(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
}

#[trial]
fn ok_accepts_a_curve_left_as_stored() {
    let stacked = vec![[0.0, 0.0], [60.0, 40.0], [60.0, 200.0], [255.0, 255.0]];
    let stored = RawDevelopSettings { point_curve: stacked.clone(), ..RawDevelopSettings::default() };
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let fixture = fixture(&host, &Rc::new(Cell::new(false)));
    let open = RawDevelopOpen::new("IMG_0001.CR3", PixelSize::new(IMAGE[0], IMAGE[1]), RawTarget::SmartFilter, stored);
    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open), Source::new()));
    assert!(host.borrow_mut().confirm::<Hosted>(fixture.ctx()), "an older Smart Filter's curve left as stored passes OK");
    assert!(finish(&host).is_some());
}

#[trial]
fn before_shows_the_before_picture_and_edits_nothing() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host, &Rc::new(Cell::new(false)));
    let source = Source::new();
    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::Filter)), source.clone()));
    fixture.idle(10);
    focus_canvas(&mut fixture);
    let (develops, before_reads) = (source.develops.load(Ordering::Relaxed), source.before.reads.load(Ordering::Relaxed));
    assert_eq!(before_reads, 0, "the before picture is not read while the settings show");

    fixture.frame(vec![key_event(Key::Y, true)]).idle(6);
    assert!(source.before.reads.load(Ordering::Relaxed) > before_reads, "Y held shows the before picture");
    let after_reads = source.after.reads.load(Ordering::Relaxed);
    fixture.frame(vec![key_event(Key::Y, false)]).idle(4);
    assert!(source.after.reads.load(Ordering::Relaxed) > after_reads, "releasing Y shows the developed picture again");

    assert_eq!(source.develops.load(Ordering::Relaxed), develops, "Before starts no develop");
    let events = host.borrow_mut().get_mut::<Hosted>().map(Hosted::take_events).unwrap_or_default();
    assert!(events.iter().all(|event| !matches!(event, RawDevelopEvent::Settings(_))), "Before edits nothing: {events:?}");
    fixture.key(Key::Z, Modifiers::COMMAND).idle(4);
    assert_eq!(exposure(&host), 0.0, "Before records no history step");
}

#[trial]
fn a_sampler_reads_the_picture_on_show() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host, &Rc::new(Cell::new(false)));
    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::Filter)), Source::new()));
    fixture.idle(10);
    let canvas = focus_canvas(&mut fixture);
    fixture.key(Key::S, Modifiers::NONE).idle(2);
    fixture.click(&canvas).idle(4);
    let reading = |fixture: &Fixture| fixture.texts().iter().find(|run| run.text.starts_with("#1 ")).map(|run| run.text.to_string());
    let developed = reading(&fixture).expect("a click of the Color Sampler tool adds a sampler");

    fixture.frame(vec![key_event(Key::Y, true)]).idle(6);
    let before = reading(&fixture).expect("the sampler keeps reading while Before shows");
    assert_ne!(before, developed, "Before reads the undeveloped pixel");
    fixture.frame(vec![key_event(Key::Y, false)]).idle(4);
    assert_eq!(reading(&fixture), Some(developed), "releasing Y reads the developed pixel again");
}

/// A color sampler's readout shows under the zoom bar.
fn readout_shown(fixture: &Fixture) -> bool {
    fixture.texts().iter().any(|run| run.text.starts_with("#1 "))
}

fn scope_json(host: &Rc<RefCell<DialogHost>>) -> serde_json::Value {
    let fields = host.borrow_mut().get_mut::<Hosted>().and_then(|hosted| hosted.workspace().json_fields(&**hosted.source()));
    fields.and_then(|fields| fields.get("scope").cloned()).unwrap_or_default()
}

#[trial]
fn the_scope_floats_and_docks_back_with_its_settings() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host, &Rc::new(Cell::new(false)));
    let source = Source::new();
    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::Filter)), source.clone()));
    fixture.idle(10);
    let canvas = focus_canvas(&mut fixture);
    fixture.key(Key::S, Modifiers::NONE).idle(2);
    fixture.click(&canvas).idle(4);
    assert!(readout_shown(&fixture), "a click of the Color Sampler tool adds a sampler");

    let float = fixture.by_label("Float").expect("the docked scope offers Float");
    fixture.click(&float).idle(6);
    assert!(fixture.by_label("Float").is_none() && fixture.by_label("Dock").is_some(), "the floating scope offers Dock instead");
    let dock = fixture.by_label("Dock").expect("Dock");
    fixture.click(&dock).idle(6);
    assert!(fixture.by_label("Float").is_some(), "Dock puts the scope back in the column");

    fixture.key(Key::U, Modifiers::NONE).idle(4);
    assert_eq!(scope_json(&host)["shadowWarning"], true, "U toggles the shadow warning of the scope's settings");

    let probe: &dyn RawDevelopSource = &*source;
    let settings = serde_json::json!({ "shadowWarning": true, "vectorscope": true, "selectedRegion": true, "view": "expanded" });
    host.borrow_mut()
        .get_mut::<Hosted>()
        .expect("open")
        .workspace_mut()
        .set_json_field(fixture.ctx(), probe, "scope", settings)
        .expect("scope field");
    fixture.idle(4);
    fixture.key(Key::Escape, Modifiers::NONE).idle(4);
    assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Cancel));

    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::Filter)), source));
    fixture.idle(10);
    assert!(!readout_shown(&fixture), "samplers are never restored");
    let restored = scope_json(&host);
    assert_eq!(restored["shadowWarning"], true, "a second open restores the settings");
    assert_eq!(restored["view"], "expanded");
    assert_eq!(restored["vectorscope"], false, "the vectorscope starts off");
    assert_eq!(restored["selectedRegion"], false, "the selected region is off without a selection");
}

#[trial]
fn a_tone_zone_drag_moves_exposure_from_where_it_started() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let rerun = Rc::new(Cell::new(false));
    let mut fixture = fixture(&host, &rerun);
    host.borrow_mut().open(HostedWorkspace::new(RawDevelopWorkspace::new(open(RawTarget::Filter)), Source::with_histogram()));
    fixture.idle(10);
    let start = RawDevelopSettings { exposure: -2.0, ..Default::default() };
    host.borrow_mut().get_mut::<Hosted>().expect("open").workspace_mut().show_item(0, start.clone(), start, None);
    fixture.idle(4);

    // The Exposure zone spans the middle half of the plot: a drag of its width is half the plot's, 5 stops.
    let zone = slider(&fixture, "Exposure");
    let from = pos2(zone.rect.left() + 2.0, zone.center().y);
    let by = Vec2::new(zone.rect.width(), 0.0);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    fixture.frame(vec![pointer::moved(from + by * 0.25)]);
    rerun.set(true);
    fixture.frame(vec![pointer::moved(from + by * 0.5)]);
    fixture.frame(vec![pointer::moved(from + by)]);
    fixture.frame(vec![pointer::release(from + by)]);
    fixture.idle(4);
    let moved = exposure(&host);
    assert!((moved - 3.0).abs() < 0.01, "the drag counts from the press, a rerun pass inside a frame included: {moved}");
}
