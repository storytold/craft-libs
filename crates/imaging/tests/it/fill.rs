//! Content-Aware Fill: what its own logic adds to the full-window parts, the tools and the controls.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Color32, ColorImage, Modifiers, Pos2, vec2};

use craft_behavior::PixelSize;
use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::kurbo::Rect;
use craft_imaging::ui::fill::{
    ContentAwareFillWorkspace, FillAreaEdit, FillCommands, FillEvent, FillLimit, FillLimits, FillOpen, FillOverlay, FillPreviewRequest, FillSession,
    FillSettings, FillSource, FillStrokeEvent, SamplingPlane, SamplingStroke,
};
use craft_imaging::ui::select_mask::EditMode;
use craft_imaging::ui::wide_angle::PreviewDone;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::dialog::{DialogHost, DialogResult, FullWindowWorkspace as _, HostedWorkspace};
use craft_ui::primitives::Revision;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::{self, FireSource};
use craft_ui::runtime::memory::{SaveKey, saved_value};
use craft_ui::runtime::pending::Job;
use craft_ui::testing::{Fixture, pointer, trial};

const NONE: Modifiers = Modifiers::NONE;
/// The layer the workspace opens on, in document pixels.
const LAYER: PixelSize = PixelSize::new(400, 300);
/// The room the workspace is shown in.
const ROOM: egui::Vec2 = vec2(1400.0, 900.0);

/// The level of the flat grey picture.
const GREY: u8 = 128;

/// A flat grey picture.
struct Grey;

impl RasterSource for Grey {
    fn size(&self) -> PixelSize {
        LAYER
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::from_gray(GREY));
    }
}

/// The app's side: a picture and no planes; its preview job fails.
struct Source;

impl FillSource for Source {
    fn image(&self) -> &dyn RasterSource {
        &Grey
    }

    fn sampling(&self) -> Option<&SamplingPlane> {
        None
    }

    fn fill_area(&self) -> Option<&SamplingPlane> {
        None
    }

    fn sampling_window(&self) -> Option<Rect> {
        None
    }

    fn preview(&self) -> Option<&dyn RasterSource> {
        None
    }

    fn preview_job(&self, _: &FillPreviewRequest) -> Job<PreviewDone, String> {
        Job::new(|_, _| Err("no engine in this test".to_owned()))
    }
}

/// The hosted workspace.
type Hosted = HostedWorkspace<ContentAwareFillWorkspace>;

/// The workspace on a dialog host shown in a fixture, with every pass's events kept.
// A pass's `WorkspaceCx` is built by the host only, so this runs the workspace on a `DialogHost`.
struct Rig {
    fixture: Fixture,
    host: Rc<RefCell<DialogHost>>,
    events: Rc<RefCell<Vec<FillEvent>>>,
}

impl Rig {
    fn new(settings: FillSettings, limits: FillLimits) -> Rig {
        let open = FillOpen::new([LAYER.width, LAYER.height], "Layer 1", limits).settings(settings);
        Rig::over(ContentAwareFillWorkspace::new(open))
    }

    /// `workspace` on the host, settled.
    fn over(workspace: ContentAwareFillWorkspace) -> Rig {
        let source: Arc<dyn FillSource> = Arc::new(Source);
        let mut host = DialogHost::new();
        host.open(HostedWorkspace::new(workspace, source));
        let host = Rc::new(RefCell::new(host));
        let events = Rc::new(RefCell::new(Vec::new()));
        let (shown, kept) = (Rc::clone(&host), Rc::clone(&events));
        let mut fixture = Fixture::new(move |ui| {
            let mut host = shown.borrow_mut();
            host.run(ui);
            let taken = host.get_mut::<Hosted>().map(Hosted::take_events).unwrap_or_default();
            kept.borrow_mut().extend(taken);
        });
        fixture.room(ROOM).settle();
        let rig = Rig { fixture, host, events };
        rig.events();
        rig
    }

    /// The middle of the main canvas.
    fn canvas(&self) -> Pos2 {
        self.fixture.by_role(Role::CANVAS).into_iter().next().expect("the workspace shows a canvas").center()
    }

    /// The events since the last call.
    fn events(&self) -> Vec<FillEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    /// Runs `answer` on the workspace, as the app does between passes.
    fn with<R>(&self, answer: impl FnOnce(&mut ContentAwareFillWorkspace) -> R) -> R {
        let mut host = self.host.borrow_mut();
        answer(host.get_mut::<Hosted>().expect("the workspace is open").workspace_mut())
    }

    /// Presses at `from`, moves through `through` and releases at the last point, with `modifiers` held.
    fn stroke(&mut self, from: Pos2, through: &[Pos2], modifiers: Modifiers) {
        self.fixture.hold(modifiers);
        self.fixture.frame(vec![pointer::moved(from)]);
        self.fixture.frame(vec![pointer::press(from)]);
        for at in through {
            self.fixture.frame(vec![pointer::moved(*at)]);
        }
        self.fixture.frame(vec![pointer::release(through.last().copied().unwrap_or(from))]);
        self.fixture.hold(NONE);
        self.fixture.idle(1);
    }

    /// A stroke along a line to the right of the canvas centre in `steps` moves.
    fn line(&mut self, steps: usize) {
        let through: Vec<Pos2> = (1..=steps).map(|i| self.canvas() + vec2(12.0 * i as f32, 0.0)).collect();
        self.stroke(self.canvas(), &through, NONE);
    }

    fn set(&self, field: &str, value: serde_json::Value) {
        let ctx = self.fixture.ctx().clone();
        self.with(|workspace| workspace.set_json_field(&ctx, &Source, field, value)).expect("the workspace accepts the control field");
    }

    fn tool(&self) -> String {
        self.with(|workspace| workspace.json_fields(&Source).unwrap()["tool"].as_str().unwrap().to_owned())
    }
}

/// The stroke a pass committed, if one did.
fn committed(events: &[FillEvent]) -> Option<&SamplingStroke> {
    events.iter().find_map(|event| match event {
        FillEvent::Stroke(FillStrokeEvent::Commit(stroke)) => Some(stroke),
        _ => None,
    })
}

fn stroke_of(points: &[[f64; 2]]) -> SamplingStroke {
    SamplingStroke::new(EditMode::Add, 20.0, points.to_vec())
}

#[trial]
fn fill_limits_refuse_at_the_brush() {
    let limits = FillLimits::new(2, 5);
    let mut session = FillSession::default();
    session.add_stroke(stroke_of(&[[0.0, 0.0]]));
    assert_eq!(session.may_begin_stroke(&limits), Ok(()));
    session.add_stroke(stroke_of(&[[0.0, 0.0], [1.0, 1.0]]));
    assert_eq!(session.may_begin_stroke(&limits), Err(FillLimit::Strokes(2)), "two strokes are the limit");

    // Points are counted over strokes and lasso edits; a total equal to the limit passes an edit.
    let limits = FillLimits::new(10, 5);
    let mut session = FillSession::default();
    session.add_stroke(stroke_of(&[[0.0, 0.0], [1.0, 1.0]]));
    let triangle = || {
        let Some(edit) = FillAreaEdit::lasso(SelectionOperation::Add, vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]]) else {
            panic!("three points are a lasso")
        };
        edit
    };
    assert_eq!(session.add_fill_edit(triangle(), &limits), Ok(()));
    assert_eq!(session.points_used(None), 5);
    assert_eq!(session.may_begin_stroke(&limits), Err(FillLimit::Vertices(5)));
    assert_eq!(session.add_fill_edit(triangle(), &limits), Err(FillLimit::Vertices(5)));
    assert_eq!(session.fill_edits.len(), 1, "a refused edit records nothing");

    // The brush: no stroke begins at the stroke limit.
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(1, 100));
    rig.line(2);
    assert!(committed(&rig.events()).is_some(), "the first stroke fits");
    rig.line(2);
    let events = rig.events();
    assert!(events.contains(&FillEvent::Refused(FillLimit::Strokes(1))), "{events:?}");
    assert!(committed(&events).is_none());

    // A stroke at the points limit stops taking points; the message comes once.
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 3));
    rig.line(6);
    let events = rig.events();
    let refusals = events.iter().filter(|event| matches!(event, FillEvent::Refused(FillLimit::Vertices(3)))).count();
    assert_eq!(refusals, 1, "{events:?}");
    assert_eq!(committed(&events).map(|stroke| stroke.points.len()), Some(3));
}

// The Apply button reports one `Apply` and the workspace stays open; the app's answer shows its message or clears the
// history; OK is not held by it. A change made between passes is reported once, however many passes the frames run.
#[trial]
fn answer_apply_answers_the_apply_button() {
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 1000));
    rig.line(2);
    rig.events();

    let apply = rig.fixture.by_label("Apply").expect("the footer has Apply");
    rig.fixture.click(&apply);
    let events = rig.events();
    let applied: Vec<_> = events.iter().filter_map(|event| if let FillEvent::Apply(command) = event { Some(command) } else { None }).collect();
    assert_eq!(applied.len(), 1, "{events:?}");
    assert_eq!(applied[0].strokes.len(), 1, "the command carries the stroke");
    assert!(rig.host.borrow().is_open::<Hosted>(), "Apply keeps the workspace open");

    rig.with(|work| work.answer_apply(Err("the fill failed".to_owned())));
    rig.fixture.idle(2);
    assert!(rig.fixture.by_label("the fill failed").is_some(), "the message shows under the panel");

    rig.with(|work| {
        work.answer_apply(Ok(()));
        work.undo();
        assert_eq!(work.session().strokes.len(), 1, "an applied fill is one document step: undo cannot take the stroke back");
    });
    rig.fixture.idle(2);
    assert!(rig.fixture.by_label("the fill failed").is_none(), "the answer clears the message");

    rig.with(|work| work.settings_mut().opacity = 10.0);
    rig.fixture.idle(3);
    let changed = rig.events().into_iter().filter(|event| matches!(event, FillEvent::SettingsChanged(settings) if settings.opacity == 10.0)).count();
    assert_eq!(changed, 1, "a change between passes is reported once");

    let ok = rig.fixture.by_label("OK").expect("the footer has OK");
    rig.fixture.click(&ok);
    let result = rig.host.borrow_mut().take::<Hosted>();
    assert!(matches!(result, Some(DialogResult::Ok(command)) if command.strokes.len() == 1), "OK answers the command");
}

#[trial]
fn fill_overlay_tints_the_sampling_area_or_the_excluded_area() {
    // A 4 by 1 plane of one-pixel cells: sampled, sampled, not sampled, sampled.
    let sampling = SamplingPlane::new([0, 0], [4, 1], 1, vec![true, true, false, true]);
    // The fill area covers the second cell.
    let fill_area = SamplingPlane::new([0, 0], [4, 1], 1, vec![false, true, false, false]);
    let document = PixelSize::new(6, 1);
    let tinted = |excluded: bool| -> Vec<bool> {
        let overlay = FillOverlay::new(&sampling, Some(&fill_area), document).excluded(excluded);
        let mut out = ColorImage::filled([6, 1], Color32::from_white_alpha(0));
        overlay.read(PixelRect::of_size(document), 1, &mut out);
        out.pixels.iter().map(|pixel| pixel.a() > 0).collect()
    };
    // Sampling Area: the sampled cells the fill area does not cover; nothing outside the plane.
    assert_eq!(tinted(false), [true, false, false, true, false, false]);
    // Excluded Area: the cells inside the plane that are not sampled and not filled.
    assert_eq!(tinted(true), [false, false, true, false, false, false]);
}

const REMEMBERED: SaveKey = SaveKey::new("trial.fill");

/// With `remember`, every close stores the settings and the next Content-Aware Fill opens with them: the footer's
/// Cancel, and a cancel from the host (the app's control route) with no pass after the change.
#[trial]
fn fill_remembers_its_settings_at_every_close() {
    let remembered =
        || ContentAwareFillWorkspace::new(FillOpen::new([LAYER.width, LAYER.height], "Layer 1", FillLimits::new(10, 1000))).remember(REMEMBERED);
    let mut rig = Rig::over(remembered());
    rig.with(|work| work.settings_mut().opacity = 10.0);
    rig.fixture.idle(2);
    assert_eq!(saved_value::<Option<FillSettings>>(rig.fixture.ctx(), &REMEMBERED), None, "a change alone stores nothing");

    let cancel = rig.fixture.by_label("Cancel").expect("the footer has Cancel");
    rig.fixture.click(&cancel);
    let stored = saved_value::<Option<FillSettings>>(rig.fixture.ctx(), &REMEMBERED);
    assert!(stored.is_some_and(|settings| settings.opacity == 10.0), "Cancel stores the settings");

    let _ = rig.host.borrow_mut().take::<Hosted>();
    let source: Arc<dyn FillSource> = Arc::new(Source);
    rig.host.borrow_mut().open(HostedWorkspace::new(remembered(), source));
    rig.fixture.idle(3);
    assert_eq!(rig.with(|work| work.settings().opacity), 10.0, "the next opening takes the stored settings");

    rig.with(|work| work.settings_mut().opacity = 40.0);
    let ctx = rig.fixture.ctx().clone();
    assert!(rig.host.borrow_mut().cancel::<Hosted>(&ctx), "the host cancels the workspace");
    let stored = saved_value::<Option<FillSettings>>(&ctx, &REMEMBERED);
    assert!(stored.is_some_and(|settings| settings.opacity == 40.0), "a cancel from the host stores the settings");
}

/// The automation keys `expand` and `contract` edit the fill area by that many pixels, like the bar's buttons; a
/// value that is not a number is refused.
#[trial]
fn expand_and_contract_keys_edit_the_fill_area() {
    let rig = Rig::new(FillSettings::default(), FillLimits::new(10, 100));
    let ctx = rig.fixture.ctx().clone();
    let source: Arc<dyn FillSource> = Arc::new(Source);
    let set = |field: &str, value: serde_json::Value| rig.with(|workspace| workspace.set_json_field(&ctx, &*source, field, value));
    assert_eq!(set("expand", serde_json::json!(3)), Ok(()));
    assert_eq!(set("contract", serde_json::json!(2)), Ok(()));
    assert!(set("expand", serde_json::json!("wide")).is_err(), "text is not an amount");
    let edits = rig.with(|workspace| workspace.session().fill_edits.clone());
    assert_eq!(edits, vec![FillAreaEdit::grow(true, 3.0), FillAreaEdit::grow(false, 2.0)]);
}

/// A real rail flyout, not a separate fifth slot: both opening gestures share the order and remember the choice.
#[trial]
fn the_fill_lasso_flyout_remembers_its_polygonal_tool() {
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 1000));
    let lasso = rig.fixture.by_label("Lasso Tool").expect("the Lasso slot shows");
    let at = lasso.center();
    rig.fixture.frame(vec![pointer::moved(at)]);
    rig.fixture.frame(vec![pointer::secondary(at, true)]);
    rig.fixture.frame(vec![pointer::secondary(at, false)]);
    rig.fixture.idle(3);
    let rows = rig.fixture.by_role(Role::MENU_ITEM_CHECK_BOX);
    let names: Vec<_> = rows.iter().map(|row| rig.fixture.tree().name_of(row.node)).collect();
    assert_eq!(names, [Some("Lasso Tool"), Some("Polygonal Lasso Tool")]);
    rig.fixture.click(&rows[1]).idle(2);
    assert_eq!(rig.tool(), "polygonalLasso");

    let hand = rig.fixture.by_label("Hand Tool").expect("the Hand slot shows");
    rig.fixture.click(&hand).idle(2);
    assert_eq!(rig.tool(), "hand");
    // Rail clicks select tools without taking keyboard focus. Establish the real keyboard target
    // after the flyout before testing the workspace's letter shortcut.
    rig.fixture.access(craft_ui::testing::focus::AccessActionKind::Focus, &hand).idle(2);
    assert_eq!(craft_ui::testing::access::focused_key(rig.fixture.tree()), hand.node, "the Hand button owns keyboard focus");
    rig.fixture.key(egui::Key::L, NONE).idle(2);
    assert_eq!(rig.tool(), "polygonalLasso", "L recalls the selected member of the Lasso slot");

    let polygon = rig.fixture.by_label("Polygonal Lasso Tool").expect("the slot shows its last choice");
    rig.fixture.press(&polygon).advance(0.7).idle(3);
    let rows = rig.fixture.by_role(Role::MENU_ITEM_CHECK_BOX);
    let names: Vec<_> = rows.iter().map(|row| rig.fixture.tree().name_of(row.node)).collect();
    assert_eq!(names, [Some("Lasso Tool"), Some("Polygonal Lasso Tool")], "holding opens the same ordered flyout");
    rig.fixture.frame(vec![pointer::moved(rows[0].center())]);
    rig.fixture.release_at(rows[0].center()).idle(2);
    assert_eq!(rig.tool(), "lasso");
    rig.set("tool", serde_json::json!("hand"));
    rig.fixture.idle(2);
    let lasso = rig.fixture.by_label("Lasso Tool").expect("the slot keeps the last freehand choice");
    rig.fixture.click(&lasso).idle(2);
    assert_eq!(rig.tool(), "lasso");

    // A named command selects that member; only the plain letter recalls the slot's last choice.
    for source in [FireSource::Automation, FireSource::Menu] {
        for (command, expected) in [(FillCommands::PolygonalLasso, "polygonalLasso"), (FillCommands::Lasso, "lasso")] {
            assert!(commands::fire(rig.fixture.ctx(), command, source), "the explicit member command is queued");
            rig.fixture.frame(Vec::new()).idle(2);
            assert_eq!(rig.tool(), expected, "an explicit {source:?} command selects its named member");
            assert!(commands::fire(rig.fixture.ctx(), FillCommands::Hand, source));
            rig.fixture.frame(Vec::new()).idle(2);
            assert_eq!(rig.tool(), "hand");
            let hand = rig.fixture.by_label("Hand Tool").expect("the Hand tool remains available");
            rig.fixture.access(craft_ui::testing::focus::AccessActionKind::Focus, &hand).idle(2);
            rig.fixture.key(egui::Key::L, NONE).idle(2);
            assert_eq!(rig.tool(), expected, "L recalls the last explicitly selected member");
        }
    }
}

/// Clicks survive their releases, close once, and reach the same operation/history path as a freehand lasso.
#[trial]
fn polygonal_fill_commits_each_bar_mode_as_one_edit() {
    for (mode, label, closing) in [
        (SelectionOperation::New, "New selection", 0),
        (SelectionOperation::Add, "Add to selection", 1),
        (SelectionOperation::Subtract, "Subtract from selection", 2),
        (SelectionOperation::Intersect, "Intersect with selection", 3),
    ] {
        let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 1000));
        rig.set("tool", serde_json::json!("polygonalLasso"));
        rig.fixture.idle(2);
        assert_eq!(rig.tool(), "polygonalLasso", "the control route reports its selected tool");
        let choice = rig.fixture.by_label(label).expect("the polygon uses the Lasso's mode buttons");
        rig.fixture.click(&choice).idle(2);
        let a = rig.canvas() + vec2(-40.0, -30.0);
        let b = rig.canvas() + vec2(35.0, -30.0);
        let c = rig.canvas() + vec2(0.0, 40.0);
        for at in [a, b, c] {
            rig.fixture.click_at(at);
            assert!(rig.with(|workspace| workspace.session().fill_edits.is_empty()), "a release does not close the polygon");
        }
        assert!(!rig.events().iter().any(|event| matches!(event, FillEvent::FillArea(_))));
        match closing {
            0 => {
                rig.fixture.click_at(a);
            }
            1 => {
                rig.fixture.click_at(c);
            }
            2 => {
                rig.fixture.key(egui::Key::Enter, NONE);
            }
            _ => {
                let d = rig.canvas() + vec2(60.0, 30.0);
                rig.fixture.click_at(d);
                rig.fixture.click_at(d);
            }
        }
        rig.fixture.idle(2);
        assert!(rig.host.borrow().is_open::<Hosted>(), "Enter closes the polygon, not the workspace");
        let edits: Vec<_> = rig
            .events()
            .into_iter()
            .filter_map(|event| match event {
                FillEvent::FillArea(edit) => Some(edit),
                _ => None,
            })
            .collect();
        assert_eq!(edits.len(), 1, "one completed polygon is one fill-area event");
        let correct_mode = matches!(
            (&edits[0], mode),
            (FillAreaEdit::New { .. }, SelectionOperation::New)
                | (FillAreaEdit::Add { .. }, SelectionOperation::Add)
                | (FillAreaEdit::Subtract { .. }, SelectionOperation::Subtract)
                | (FillAreaEdit::Intersect { .. }, SelectionOperation::Intersect)
        );
        assert!(correct_mode, "the bar's operation survives every click: {edits:?}");
        let points = match &edits[0] {
            FillAreaEdit::New { points, .. }
            | FillAreaEdit::Add { points, .. }
            | FillAreaEdit::Subtract { points, .. }
            | FillAreaEdit::Intersect { points, .. } => points,
            _ => panic!("the pointer committed a polygon"),
        };
        assert_eq!(points.len(), if closing == 3 { 4 } else { 3 }, "only clicked vertices enter the edit");
        assert!((points[0][1] - points[1][1]).abs() < 1e-6, "the horizontal edge stays horizontal");
        assert!(points[0][0] < points[2][0] && points[2][0] < points[1][0] && points[2][1] > points[0][1], "the third corner stays below that edge");
        assert_eq!(rig.with(|workspace| workspace.session().fill_edits.clone()), edits);
        rig.with(ContentAwareFillWorkspace::undo);
        assert!(rig.with(|workspace| workspace.session().fill_edits.is_empty()), "one undo removes the whole polygon");
        rig.with(ContentAwareFillWorkspace::redo);
        assert_eq!(rig.with(|workspace| workspace.session().fill_edits.clone()), edits, "redo restores the same edit");
    }

    // A double-click on the last vertex closes on its first press. Its continuation must not
    // leave a new one-point draft that steals the next independent polygon's starting point.
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 1000));
    rig.set("tool", serde_json::json!("polygonalLasso"));
    rig.fixture.idle(2);
    let new_selection = rig.fixture.by_label("New selection").expect("the polygon's operation is explicit");
    rig.fixture.click(&new_selection).idle(2);
    let a = rig.canvas() + vec2(-40.0, -30.0);
    let b = rig.canvas() + vec2(35.0, -30.0);
    let c = rig.canvas() + vec2(0.0, 40.0);
    for at in [a, b, c] {
        rig.fixture.click_at(at);
    }
    let click_delay = rig.fixture.ctx().options(|options| options.input_options.max_double_click_delay);
    rig.fixture.advance(click_delay + 1.0);
    for _ in 0..2 {
        rig.fixture.frame(vec![pointer::moved(c)]);
        rig.fixture.frame(vec![pointer::press(c)]);
        rig.fixture.frame(vec![pointer::release(c)]);
    }
    rig.fixture.idle(2);
    let first = rig.with(|workspace| workspace.session().fill_edits.clone());
    assert_eq!(first.len(), 1, "the proximity double-click commits only one polygon");
    let FillAreaEdit::New { points: first_points, .. } = &first[0] else {
        panic!("the first polygon uses New selection");
    };
    assert_eq!(first_points.len(), 3);

    // Start a separate three-corner polygon, at a point one quarter along the old top edge.
    rig.fixture.advance(click_delay + 1.0);
    for at in [a + (b - a) * 0.25, a + (b - a) * 0.75, rig.canvas() + vec2(0.0, 10.0)] {
        rig.fixture.click_at(at);
    }
    rig.fixture.key(egui::Key::Enter, NONE).idle(2);
    let edits = rig.with(|workspace| workspace.session().fill_edits.clone());
    assert_eq!(edits.len(), 2, "the next independent polygon commits separately");
    let FillAreaEdit::New { points, .. } = &edits[1] else {
        panic!("the next polygon keeps New selection");
    };
    assert_eq!(points.len(), 3, "no continuation vertex leaks into the next polygon");
    let expected_start = [first_points[0][0] * 0.75 + first_points[1][0] * 0.25, first_points[0][1]];
    assert!(
        (points[0][0] - expected_start[0]).abs() < 1e-3 && (points[0][1] - expected_start[1]).abs() < 1e-3,
        "the next polygon starts at the new click, not at the old closing vertex"
    );
}

#[trial]
fn polygonal_fill_retracts_and_cancels_without_committing_a_partial_edit() {
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 1000));
    rig.set("tool", serde_json::json!("polygonalLasso"));
    rig.fixture.idle(2);
    let a = rig.canvas() + vec2(-40.0, -30.0);
    let b = rig.canvas() + vec2(35.0, -30.0);
    let c = rig.canvas() + vec2(0.0, 40.0);
    for retract in 0..3 {
        for at in [a, b, c] {
            rig.fixture.click_at(at);
        }
        match retract {
            0 => {
                rig.fixture.key(egui::Key::Backspace, NONE);
            }
            1 => {
                rig.fixture.key(egui::Key::Delete, NONE);
            }
            _ => {
                rig.fixture.frame(vec![pointer::secondary(c, true)]);
                rig.fixture.frame(vec![pointer::secondary(c, false)]);
                assert!(rig.fixture.by_role(Role::MENU).is_empty(), "the secondary click retracts, rather than opening a menu");
            }
        }
        rig.fixture.key(egui::Key::Enter, NONE);
        assert!(rig.host.borrow().is_open::<Hosted>());
        assert!(rig.with(|workspace| workspace.session().fill_edits.is_empty()), "each retract leaves only two vertices, which cannot commit");
    }

    for at in [a, b, c] {
        rig.fixture.click_at(at);
    }
    rig.fixture.key(egui::Key::Escape, NONE);
    assert!(rig.host.borrow().is_open::<Hosted>(), "Escape drops the outline before it can close the workspace");
    assert!(rig.with(|workspace| workspace.session().fill_edits.is_empty()));
    for at in [a, b, c] {
        rig.fixture.click_at(at);
    }
    rig.fixture.key(egui::Key::H, NONE).idle(2);
    assert_eq!(rig.tool(), "hand", "tool letters still work between polygon clicks");
    assert!(rig.with(|workspace| workspace.session().fill_edits.is_empty()));
    rig.set("tool", serde_json::json!("polygonalLasso"));
    rig.fixture.idle(2);
    for at in [a, b, c] {
        rig.fixture.click_at(at);
    }
    rig.set("tool", serde_json::json!("lasso"));
    rig.fixture.idle(2);
    assert!(rig.with(|workspace| workspace.session().fill_edits.is_empty()), "a tool switch drops the unfinished polygon");
    let through = [b, c, a];
    rig.stroke(a, &through, NONE);
    assert_eq!(rig.with(|workspace| workspace.session().fill_edits.len()), 1, "freehand still closes on release");
}

#[trial]
fn polygonal_fill_uses_the_existing_edit_limits() {
    let mut rig = Rig::new(FillSettings::default(), FillLimits::new(10, 2));
    rig.set("tool", serde_json::json!("polygonalLasso"));
    rig.fixture.idle(2);
    for delta in [vec2(-40.0, -30.0), vec2(35.0, -30.0), vec2(0.0, 40.0)] {
        let at = rig.canvas() + delta;
        rig.fixture.click_at(at);
    }
    rig.fixture.key(egui::Key::Enter, NONE);
    assert!(rig.with(|workspace| workspace.session().fill_edits.is_empty()));
    assert!(rig.events().iter().any(|event| matches!(event, FillEvent::Refused(FillLimit::Vertices(2)))), "the existing refusal reaches the host");
}
