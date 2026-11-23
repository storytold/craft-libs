//! Curves display choices through the real shared panel: drawing, pointer capture, keyboard,
//! accessible states and the dialog/Properties layouts. No renderer or screenshot is needed.

use craft_behavior::Mixed;
use craft_imaging::ui::adjust::{AdjustContext, CurvesMode, CurvesPanel, HostedAdjustDialog, ToneChannels, ToneCurves, ToneSpace};
use craft_imaging::ui::curve::Curve;
use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::{DialogHost, DialogResult};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, Shape, Vec2, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CHECKS: [&str; 4] = ["Channel Overlays", "Histogram", "Baseline", "Intersection Line"];
const SNAP: f32 = 1.0;

fn curves() -> ToneCurves {
    let mut value = ToneCurves::identity(&ToneChannels::of(ToneSpace::Rgb, false));
    for (curve, middle) in value.curves.iter_mut().zip([[104.0, 154.0], [80.0, 30.0], [120.0, 190.0], [180.0, 80.0]]) {
        *curve = Curve::smooth(vec![[0.0, 0.0], middle, [255.0, 255.0]]);
    }
    value
}

struct Rig {
    fixture: Fixture,
    value: Rc<RefCell<ToneCurves>>,
    channel: Rc<Cell<usize>>,
    dialog: Rc<Cell<bool>>,
    visible: Rc<Cell<bool>>,
}

fn rig(width: f32) -> Rig {
    let value = Rc::new(RefCell::new(curves()));
    let channel = Rc::new(Cell::new(0));
    let dialog = Rc::new(Cell::new(true));
    let visible = Rc::new(Cell::new(true));
    let (v, c, d, shown) = (value.clone(), channel.clone(), dialog.clone(), visible.clone());
    let histograms: Vec<[u32; 256]> = (0..4).map(|channel| std::array::from_fn(|i| ((i + channel * 41) % 128 + 1) as u32)).collect();
    let mut fixture = Fixture::new(move |ui| {
        if !shown.get() { return; }
        let mut value = v.borrow().clone();
        let mut channel = c.get();
        let context = if d.get() { AdjustContext::dialog(true) } else { AdjustContext::properties(true) };
        let output = CurvesPanel::new("Curves", Bind::bound(&mut value), ToneChannels::of(ToneSpace::Rgb, false))
            .histograms(&histograms).channel(Bind::bound(&mut channel)).context(context).show(ui);
        if let Some(mode) = output.mode { value.mode = mode; }
        c.set(channel);
        *v.borrow_mut() = value;
    });
    fixture.room(vec2(width, 1000.0)).settle();
    Rig { fixture, value, channel, dialog, visible }
}

fn checked(fixture: &Fixture, label: &str) -> bool {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} control"));
    assert_eq!(fixture.tree().role_of(target.node), Some(if label.ends_with("grid") { Role::BUTTON } else { Role::CHECK_BOX }));
    match fixture.tree().toggled_of(target.node) {
        Some(Mixed::Same(value)) => value,
        other => panic!("{label} reports its checked state: {other:?}"),
    }
}

fn click(fixture: &mut Fixture, label: &str) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} control"));
    fixture.click(&target).settle();
}

/// The two endpoint nodes calibrate the actual plot, without repeating its padding or bar geometry.
fn area(fixture: &Fixture) -> Rect {
    let first = fixture.by_label("Point 1 of 3").expect("first point").center();
    let last = fixture.by_label("Point 3 of 3").expect("last point").center();
    Rect::from_min_max(pos2(first.x, last.y), pos2(last.x, first.y))
}

fn segments(shapes: &[Shape]) -> Vec<[Pos2; 2]> {
    shapes.iter().filter_map(|shape| match shape {
        Shape::LineSegment { points, .. } => Some(*points),
        Shape::Path(path) if path.points.len() == 2 => Some([path.points[0], path.points[1]]),
        _ => None,
    }).collect()
}

fn near(a: f32, b: f32) -> bool { (a - b).abs() <= SNAP }

/// Full-width/height rules inside the calibrated plot; points, icons, bars and field borders do not qualify.
fn grid(shapes: &[Shape], area: Rect) -> (usize, usize) {
    let lines = segments(shapes);
    let vertical = lines.iter().filter(|[a,b]| near(a.x,b.x) && a.x > area.left()+SNAP && a.x < area.right()-SNAP
        && near(a.y.min(b.y),area.top()) && near(a.y.max(b.y),area.bottom())).count();
    let horizontal = lines.iter().filter(|[a,b]| near(a.y,b.y) && a.y > area.top()+SNAP && a.y < area.bottom()-SNAP
        && near(a.x.min(b.x),area.left()) && near(a.x.max(b.x),area.right())).count();
    (vertical,horizontal)
}

fn diagonal(shapes: &[Shape], area: Rect) -> usize {
    segments(shapes).iter().filter(|[a,b]| (a.distance(area.left_bottom()) < SNAP && b.distance(area.right_top()) < SNAP)
        || (b.distance(area.left_bottom()) < SNAP && a.distance(area.right_top()) < SNAP)).count()
}

fn curve_paths(shapes: &[Shape], area: Rect) -> usize {
    shapes.iter().filter(|shape| matches!(shape, Shape::Path(path) if path.points.len() == 256
        && path.points.iter().all(|p| area.expand(SNAP).contains(*p)))).count()
}

fn histogram_meshes(shapes: &[Shape], area: Rect) -> usize {
    shapes.iter().filter(|shape| matches!(shape, Shape::Mesh(mesh) if mesh.vertices.len() > 4
        && !mesh.vertices.is_empty() && mesh.vertices.iter().all(|vertex| area.expand(SNAP).contains(vertex.pos)))).count()
}

#[trial]
fn fresh_display_defaults_and_grid_choices_draw_the_measured_rules() {
    let mut rig = rig(700.0);
    for label in CHECKS { assert!(checked(&rig.fixture, label), "{label} starts on"); }
    assert!(checked(&rig.fixture,"4x4 grid"));
    assert!(!checked(&rig.fixture,"10x10 grid"));
    let plot = area(&rig.fixture);
    assert_eq!(grid(&rig.fixture.frame_shapes(),plot),(3,3));
    click(&mut rig.fixture,"10x10 grid");
    assert_eq!(grid(&rig.fixture.frame_shapes(),plot),(9,9));
    assert!(!checked(&rig.fixture,"4x4 grid"));
    assert!(checked(&rig.fixture,"10x10 grid"));
    click(&mut rig.fixture,"4x4 grid");
    assert_eq!(grid(&rig.fixture.frame_shapes(),plot),(3,3));
}

#[trial]
fn show_choices_change_only_their_backdrop_or_overlay_and_not_the_curve() {
    let mut rig = rig(700.0);
    let before = rig.value.borrow().clone();
    let plot = area(&rig.fixture);
    let all = rig.fixture.frame_shapes();
    assert_eq!(diagonal(&all,plot),1);
    assert_eq!(curve_paths(&all,plot),4,"composite and three changed colour curves");
    assert_eq!(histogram_meshes(&all,plot),1,"the original channel's histogram");
    click(&mut rig.fixture,"Baseline");
    let no_baseline = rig.fixture.frame_shapes();
    assert_eq!(diagonal(&no_baseline,plot),0);
    assert_eq!(curve_paths(&no_baseline,plot),4);
    assert_eq!(grid(&no_baseline,plot),(3,3));
    click(&mut rig.fixture,"Histogram");
    assert_eq!(histogram_meshes(&rig.fixture.frame_shapes(),plot),0);
    click(&mut rig.fixture,"Channel Overlays");
    assert_eq!(curve_paths(&rig.fixture.frame_shapes(),plot),1,"the edited composite stays");
    for label in ["Baseline","Histogram","Channel Overlays"] { click(&mut rig.fixture,label); }
    let all = rig.fixture.frame_shapes();
    assert_eq!((diagonal(&all,plot),curve_paths(&all,plot),histogram_meshes(&all,plot)),(1,4,1));
    assert_eq!(*rig.value.borrow(),before,"view choices never change the adjustment");
}

#[trial]
fn single_channel_and_pencil_disable_the_options_without_losing_their_choices() {
    let mut rig = rig(700.0);
    click(&mut rig.fixture,"Channel Overlays");
    click(&mut rig.fixture,"Intersection Line");
    rig.channel.set(1);
    rig.value.borrow_mut().mode = CurvesMode::Pencil;
    rig.fixture.settle();
    for label in ["Channel Overlays","Intersection Line"] {
        let target = rig.fixture.by_label(label).expect("disabled control stays visible");
        assert!(rig.fixture.tree().is_disabled_key(target.node),"{label} is disabled");
        click(&mut rig.fixture,label);
        assert!(!checked(&rig.fixture,label),"disabled click does not change {label}");
    }
    rig.channel.set(0);
    rig.value.borrow_mut().mode = CurvesMode::Points;
    rig.fixture.settle();
    for label in ["Channel Overlays","Intersection Line"] {
        let target = rig.fixture.by_label(label).expect("enabled control");
        assert!(!rig.fixture.tree().is_disabled_key(target.node));
        assert!(!checked(&rig.fixture,label));
        click(&mut rig.fixture,label);
        assert!(checked(&rig.fixture,label));
    }
}

#[trial]
fn intersection_guides_follow_only_an_active_point_drag_and_clear_on_release_or_cancel() {
    let mut rig = rig(700.0);
    let plot = area(&rig.fixture);
    let point = rig.fixture.by_label("Point 2 of 3").expect("middle point").center();
    rig.fixture.frame(vec![moved(point)]).frame(vec![press(point)]);
    let dragged = point + vec2(18.0,-24.0);
    let shapes = rig.fixture.frame_shapes_with(vec![moved(dragged)]);
    assert_eq!(grid(&shapes,plot),(4,4),"one guide in each direction, through the dragged point");
    rig.fixture.frame(vec![release(dragged)]).settle();
    assert_eq!(grid(&rig.fixture.frame_shapes(),plot),(3,3),"release leaves no guides");
    assert_ne!(*rig.value.borrow(),curves(),"the actual point drag edited the curve");
    let now = rig.value.borrow().clone();
    let point = rig.fixture.by_label("Point 2 of 3").expect("moved point").center();
    rig.fixture.frame(vec![moved(point)]).frame(vec![press(point)]).frame(vec![moved(point+vec2(15.0,18.0))]);
    rig.fixture.key(Key::Escape,Modifiers::NONE).settle();
    assert_eq!(grid(&rig.fixture.frame_shapes(),plot),(3,3),"cancel leaves no guides");
    assert_eq!(*rig.value.borrow(),now,"cancel restores the actual adjustment");
    rig.fixture.frame(vec![release(point)]);
    click(&mut rig.fixture,"Intersection Line");
    let point = rig.fixture.by_label("Point 2 of 3").expect("point after cancel").center();
    rig.fixture.frame(vec![moved(point)]).frame(vec![press(point)]);
    let shapes = rig.fixture.frame_shapes_with(vec![moved(point+vec2(18.0,-24.0))]);
    assert_eq!(grid(&shapes,plot),(3,3),"the off choice hides guides during a real drag");
    rig.fixture.frame(vec![release(point+vec2(18.0,-24.0))]);
}

#[trial]
fn display_controls_fit_a_300_point_width_and_the_graph_stays_square_when_resized() {
    let mut rig = rig(300.0);
    for width in [300.0,700.0,360.0] {
        rig.fixture.room(vec2(width,1000.0)).settle();
        let plot = area(&rig.fixture);
        assert!(near(plot.width(),plot.height()),"square plot at width {width}: {plot:?}");
        assert!(plot.width()>0.0);
        let room = Rect::from_min_size(Pos2::ZERO,vec2(width,1000.0));
        for label in CHECKS.into_iter().chain(["4x4 grid","10x10 grid","Input","Output"]) {
            let target = rig.fixture.by_label(label).unwrap_or_else(|| panic!("visible {label}"));
            assert!(room.contains_rect(target.rect),"{label} fits {width} points: {:?}",target.rect);
        }
    }
}

#[trial]
fn properties_uses_its_full_graph_and_ignores_dialog_display_memory() {
    let mut rig = rig(300.0);
    click(&mut rig.fixture,"10x10 grid");
    for label in CHECKS { click(&mut rig.fixture,label); }
    rig.dialog.set(false);
    rig.fixture.settle();
    for label in CHECKS.into_iter().chain(["4x4 grid","10x10 grid"]) { assert!(rig.fixture.by_label(label).is_none()); }
    let plot = area(&rig.fixture);
    let shapes = rig.fixture.frame_shapes();
    assert_eq!(grid(&shapes,plot),(3,3));
    assert_eq!((diagonal(&shapes,plot),curve_paths(&shapes,plot),histogram_meshes(&shapes,plot)),(1,4,1));
    rig.dialog.set(true);
    rig.fixture.settle();
    assert!(checked(&rig.fixture,"10x10 grid"),"the dialog's own choices survive");
    for label in CHECKS { assert!(!checked(&rig.fixture,label)); }
}

#[trial]
fn reopening_the_panel_retains_session_choices_without_changing_the_adjustment() {
    let mut rig = rig(700.0);
    click(&mut rig.fixture,"10x10 grid");
    click(&mut rig.fixture,"Baseline");
    rig.visible.set(false);
    rig.fixture.idle(2);
    rig.visible.set(true);
    rig.fixture.settle();
    assert!(checked(&rig.fixture,"10x10 grid"));
    assert!(!checked(&rig.fixture,"Baseline"));
    assert_eq!(*rig.value.borrow(),curves());
}

#[trial]
fn checkboxes_are_keyboard_reachable_and_space_changes_the_accessible_checked_state() {
    let mut rig = rig(700.0);
    let target = rig.fixture.by_label("Histogram").expect("Histogram checkbox");
    for _ in 0..32 {
        if rig.fixture.tree().focused_key()==target.node { break; }
        rig.fixture.key(Key::Tab,Modifiers::NONE).settle();
    }
    assert_eq!(rig.fixture.tree().focused_key(),target.node,"Tab reaches the standard checkbox");
    rig.fixture.key(Key::Space,Modifiers::NONE).settle();
    assert!(!checked(&rig.fixture,"Histogram"));
    rig.fixture.key(Key::Space,Modifiers::NONE).settle();
    assert!(checked(&rig.fixture,"Histogram"));
    rig.fixture.key(Key::Tab,Modifiers::NONE).settle();
    assert_ne!(rig.fixture.tree().focused_key(),target.node,"Tab leaves the checkbox without a trap");
}

fn underlined(shapes: &[Shape], label: &str) -> String {
    shapes.iter().filter_map(|shape| match shape {
        Shape::Text(text) if text.galley.job.text==label => Some(text.galley.job.sections.iter()
            .filter(|section| section.format.underline.width>0.0)
            .map(|section| text.galley.job.text[section.byte_range.clone()].to_owned()).collect::<String>()),
        _ => None,
    }).collect()
}

#[trial]
fn alt_underlines_only_the_four_translated_measured_letters_while_held() {
    let mut rig = rig(700.0);
    for modifiers in [Modifiers::NONE,Modifiers::ALT,Modifiers::NONE] {
        rig.fixture.hold(modifiers).settle();
        let shapes = rig.fixture.frame_shapes();
        for (label,letter) in CHECKS.into_iter().zip(["v","H","B","n"]) {
            assert_eq!(underlined(&shapes,label),if modifiers.alt { letter } else { "" },"{label}");
        }
    }
}

type Dialog = HostedAdjustDialog<ToneCurves,()>;

#[trial]
fn a_hosted_curves_dialog_keeps_the_graph_fields_and_display_controls_inside_its_bounds() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = host.clone();
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1000.0,1000.0));
    host.borrow_mut().open(Dialog::new("curve-display-options","Curves",curves(),|ui,value,cx| {
        CurvesPanel::new("Curves",Bind::bound(value),ToneChannels::of(ToneSpace::Rgb,false)).context(cx.context).show(ui);
        None
    }).resizable(true));
    fixture.settle();
    let dialog = fixture.by_role(Role::DIALOG).first().expect("dialog").rect;
    for label in CHECKS.into_iter().chain(["4x4 grid","10x10 grid","Input","Output","Point 2 of 3"]) {
        let part = fixture.by_label(label).unwrap_or_else(|| panic!("{label} inside dialog"));
        assert!(dialog.contains_rect(part.rect),"{label}: {:?} inside {dialog:?}",part.rect);
    }
    fixture.key(Key::Escape,Modifiers::NONE).settle();
    assert!(matches!(host.borrow_mut().take::<Dialog>(),Some(DialogResult::Cancel)));
}
