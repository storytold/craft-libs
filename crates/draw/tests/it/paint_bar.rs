//! `PaintBar`: which members each tool disables, the smoothing options, the gestures, the combinations list, what the
//! load swatch shows, a click on its menu and the percents each field holds.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_draw::ui::{EraserMode, MIXER_COMBINATIONS, MixerLoad, PaintBar, PaintTool, PaintValues};
use craft_theme::Srgba;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::choice::SwatchImage;
use craft_ui::contract::{Entry, ShowContainer as _};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Room, trial};
use craft_ui::text::RangeAlert;
use egui::{Color32, ColorImage, Key, Modifiers, Pos2, Shape, TextureId};

struct Host {
    values: PaintValues<u32>,
    eraser: EraserMode,
    edits: Vec<EditEvent<PaintValues<u32>>>,
    /// How many passes reported Clean Brush.
    cleans: u32,
    /// The range alerts the passes reported.
    alerts: Vec<RangeAlert>,
}

fn harness(tool: PaintTool, eraser: EraserMode, prepare: impl FnOnce(&mut PaintValues<u32>)) -> (Fixture, Rc<RefCell<Host>>) {
    let mut values = PaintValues::new(0u32);
    prepare(&mut values);
    let host = Rc::new(RefCell::new(Host { values, eraser, edits: Vec::new(), cleans: 0, alerts: Vec::new() }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { values, eraser, edits, cleans, alerts } = &mut *host;
        let modes = [Entry::new(0u32, "Normal"), Entry::new(1u32, "Multiply")];
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let mut bar = PaintBar::new(tool, &mut *values).modes(&modes);
            if tool == PaintTool::Eraser {
                bar = bar.eraser_mode(&mut *eraser);
            }
            let out = bar.show(ui);
            edits.extend(out.edits);
            *cleans += u32::from(out.mixer_clean);
            alerts.extend(out.out_of_range);
        });
    });
    // The options bar spans an application window; its members still run past it, and the bar scrolls.
    fixture.in_room(Room::Window).settle();
    (fixture, host)
}

/// Pages the options bar to its end, so its last members are in view where a click reaches them.
fn page_to_end(fixture: &mut Fixture) {
    let bar = fixture.by_label("Options").expect("the options bar");
    fixture.access(AccessActionKind::ScrollRight, &bar).settle();
}

fn disabled(fixture: &Fixture, label: &str) -> bool {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("the bar shows {label}"));
    fixture.tree().is_disabled_key(target.node)
}

/// The members each tool disables, the smoothing options that wait for an amount, one terminating event per field
/// gesture, and one `Set` for a combinations pick.
#[trial]
fn paint_bar_rules() {
    let cases: [(PaintTool, EraserMode, &[&str], &[&str]); 5] = [
        (PaintTool::Brush, EraserMode::Brush, &[], &["Opacity", "Flow", "Mode", "Build-up"]),
        (PaintTool::Pencil, EraserMode::Brush, &[], &["Opacity", "Mode", "Pressure controls size"]),
        (PaintTool::Eraser, EraserMode::Brush, &[], &["Opacity", "Flow", "Mode"]),
        (PaintTool::Eraser, EraserMode::Pencil, &["Flow", "Build-up"], &["Opacity", "Mode"]),
        (PaintTool::Eraser, EraserMode::Block, &["Opacity", "Flow", "Pressure controls opacity"], &["Mode"]),
    ];
    for (tool, eraser, off, on) in cases {
        let (fixture, _) = harness(tool, eraser, |_| {});
        for label in off {
            assert!(disabled(&fixture, label), "{tool:?} {eraser:?}: {label} is disabled");
        }
        for label in on {
            assert!(!disabled(&fixture, label), "{tool:?} {eraser:?}: {label} is enabled");
        }
    }

    let options = ["Pulled String Mode", "Stroke Catch-up", "Catch-up on Stroke End", "Adjust for Zoom"];
    for (amount, expect_disabled) in [(0.0, true), (0.1, false)] {
        let (mut fixture, host) = harness(PaintTool::Brush, EraserMode::Brush, |v| {
            v.smoothing.amount = amount;
            v.smoothing.pulled_string = true;
            v.smoothing.adjust_for_zoom = true;
        });
        page_to_end(&mut fixture);
        let gear = fixture.by_label("Smoothing options").expect("the bar shows the smoothing gear");
        fixture.click(&gear);
        fixture.idle(3);
        for label in options {
            assert_eq!(disabled(&fixture, label), expect_disabled, "amount {amount}: {label}");
        }
        let host = host.borrow();
        assert!(host.values.smoothing.pulled_string && host.values.smoothing.adjust_for_zoom, "the options keep their values");
        assert!(host.edits.is_empty());
    }

    let (mut fixture, host) = harness(PaintTool::Brush, EraserMode::Brush, |_| {});
    let field = fixture.by_label("Opacity").expect("the bar shows Opacity");
    fixture.click(&field);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text("50");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(3);
    let host = host.borrow();
    assert!((host.values.opacity - 0.5).abs() < 1e-9, "{:?}", host.values);
    let begins = host.edits.iter().filter(|e| matches!(e, EditEvent::Begin { .. })).count();
    let commits = host.edits.iter().filter(|e| matches!(e, EditEvent::Commit { .. })).count();
    let sets = host.edits.iter().filter(|e| matches!(e, EditEvent::Set { .. })).count();
    assert_eq!(begins, commits, "every Begin has its Commit");
    assert_eq!(commits + sets, 1, "one terminating event per gesture: {} events", host.edits.len());
    drop(host);

    let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| v.mixer.flow = 0.42);
    let list = fixture.by_label("Useful mixer brush combinations").expect("the bar shows the combinations");
    fixture.click(&list);
    fixture.idle(3);
    let entry = fixture.by_label("Moist, Light Mix").expect("the list shows the entry");
    fixture.click(&entry);
    fixture.idle(3);
    let host = host.borrow();
    let (wet, load, mix) = MIXER_COMBINATIONS[5].expect("the entry sets values");
    assert_eq!((host.values.mixer.wet, host.values.mixer.load, host.values.mixer.mix), (wet, load, mix));
    assert_eq!(host.values.mixer.flow, 0.42, "Flow stays");
    assert_eq!(host.values.mixer.combination, 5);
    assert!(matches!(host.edits.as_slice(), [EditEvent::Set { .. }]), "one Set for the pick: {} events", host.edits.len());
}

/// A click at `at`: hover, press and release in one pass, as egui_kittest's `click` sends them, or in separate passes.
fn click_at(fixture: &mut Fixture, at: Pos2, one_pass: bool) {
    if one_pass {
        fixture.frame(vec![moved(at), press(at), release(at)]).frame(Vec::new());
    } else {
        fixture.click_at(at);
    }
}

/// A pointer click on the load swatch's menu row Clean Brush reports it once, also when the click comes in one pass.
#[trial]
fn a_click_on_the_load_swatch_menus_clean_brush_row_reports_it_once() {
    for one_pass in [true, false] {
        let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |_| {});
        let swatch = fixture.by_label("Current brush load").expect("the bar shows the load swatch");
        click_at(&mut fixture, swatch.center(), one_pass);
        fixture.idle(3);
        let row = fixture.by_label("Clean Brush").expect("the click opens the load swatch's menu");
        click_at(&mut fixture, row.center(), one_pass);
        fixture.idle(3);
        assert_eq!(host.borrow().cleans, 1, "one pass: {one_pass}");
    }
}

/// Every shape of the frame, groups opened.
fn flat(shapes: Vec<Shape>) -> Vec<Shape> {
    shapes.into_iter().flat_map(|shape| if let Shape::Vec(inner) = shape { flat(inner) } else { vec![shape] }).collect()
}

/// What the load swatch shows: its accessible value and the pictures painted in it (the checkerboard, a sampled image).
fn load_swatch(fixture: &mut Fixture) -> (Option<String>, usize) {
    let swatch = fixture.by_label("Current brush load").expect("the bar shows the load swatch");
    let value = fixture.tree().value_of(swatch.node).map(str::to_owned);
    let well = swatch.rect.expand(0.5);
    let mut pictures: Vec<TextureId> = flat(fixture.frame_shapes())
        .iter()
        .filter(|shape| well.contains_rect(shape.visual_bounding_rect()))
        .map(Shape::texture_id)
        .filter(|texture| *texture != TextureId::default())
        .collect();
    pictures.sort_by_key(|texture| format!("{texture:?}"));
    pictures.dedup();
    (value, pictures.len())
}

/// The load swatch shows what the brush holds: a clean brush the checkerboard alone, a colour that colour, a sampled
/// tip image that picture over the checkerboard.
#[trial]
fn the_load_swatch_shows_each_mixer_load() {
    let green = Srgba::rgb(0.0, 1.0, 0.0);
    let tip = SwatchImage::new(ColorImage::filled([64, 64], Color32::from_rgb(0, 0, 255)));
    let cases =
        [(MixerLoad::Clean, Some("Empty".to_owned()), 1), (MixerLoad::Color(green), Some(green.to_hex()), 0), (MixerLoad::Image(tip), None, 2)];
    for (load, value, pictures) in cases {
        let shown = load.clone();
        let (mut fixture, _) = harness(PaintTool::MixerBrush, EraserMode::Brush, move |v| v.mixer.current_load = shown);
        assert_eq!(load_swatch(&mut fixture), (value, pictures), "{load:?}");
    }
}

fn type_into(fixture: &mut Fixture, label: &str, text: &str) {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("the bar shows {label}"));
    fixture.click(&field);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
}

/// Opacity and Flow hold 1 to 100 % on every tool that shows them (photocraft#16809): a typed 0 or -5 keeps 1 % and
/// 150 keeps 100 %, each reported once with a range that starts at 1 %. Smoothing still takes 0 with no alert.
#[trial]
fn opacity_and_flow_hold_one_to_a_hundred_percent_and_smoothing_still_takes_zero() {
    let fields = [
        (PaintTool::Brush, "Opacity"),
        (PaintTool::Brush, "Flow"),
        (PaintTool::Eraser, "Opacity"),
        (PaintTool::Eraser, "Flow"),
        (PaintTool::Pencil, "Opacity"),
    ];
    for (tool, label) in fields {
        for (typed, kept) in [("0", 0.01), ("-5", 0.01), ("150", 1.0)] {
            let (mut fixture, host) = harness(tool, EraserMode::Brush, |v| {
                v.opacity = 0.5;
                v.flow = 0.5;
            });
            type_into(&mut fixture, label, typed);
            let host = host.borrow();
            let value = if label == "Opacity" { host.values.opacity } else { host.values.flow };
            assert_eq!(value, kept, "{tool:?} {label}: {typed} keeps {kept}");
            assert_eq!(host.alerts.len(), 1, "{tool:?} {label}: {typed} is reported once: {:?}", host.alerts);
            assert_eq!(host.alerts[0].range, 1.0..=100.0, "{tool:?} {label}: the alert names 1 to 100 %");
        }
    }

    let (mut fixture, host) = harness(PaintTool::Brush, EraserMode::Brush, |v| v.smoothing.amount = 0.5);
    page_to_end(&mut fixture);
    type_into(&mut fixture, "Smoothing", "0");
    let host = host.borrow();
    assert_eq!(host.values.smoothing.amount, 0.0, "Smoothing takes 0");
    assert!(host.alerts.is_empty(), "and reports nothing: {:?}", host.alerts);
}
