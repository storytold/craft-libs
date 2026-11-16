//! `PaintBar`: which members each tool disables, the smoothing options, the gestures, the combinations list and a
//! click on the load swatch menu.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_draw::ui::{EraserMode, MIXER_COMBINATIONS, PaintBar, PaintTool, PaintValues};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::{Entry, ShowContainer as _};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Room, trial};
use egui::{Key, Modifiers, Pos2};

struct Host {
    values: PaintValues<u32>,
    eraser: EraserMode,
    edits: Vec<EditEvent<PaintValues<u32>>>,
    /// How many passes reported Clean Brush.
    cleans: u32,
}

fn harness(tool: PaintTool, eraser: EraserMode, prepare: impl FnOnce(&mut PaintValues<u32>)) -> (Fixture, Rc<RefCell<Host>>) {
    let mut values = PaintValues::new(0u32);
    prepare(&mut values);
    let host = Rc::new(RefCell::new(Host { values, eraser, edits: Vec::new(), cleans: 0 }));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { values, eraser, edits, cleans } = &mut *host;
        let modes = [Entry::new(0u32, "Normal"), Entry::new(1u32, "Multiply")];
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let mut bar = PaintBar::new(tool, &mut *values).modes(&modes);
            if tool == PaintTool::Eraser {
                bar = bar.eraser_mode(&mut *eraser);
            }
            let out = bar.show(ui);
            edits.extend(out.edits);
            *cleans += u32::from(out.mixer_clean);
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
        (PaintTool::Brush, EraserMode::Brush, &[], &["Size", "Hardness", "Opacity", "Flow", "Mode"]),
        (PaintTool::Pencil, EraserMode::Brush, &[], &["Size", "Opacity", "Mode"]),
        (PaintTool::Eraser, EraserMode::Brush, &[], &["Size", "Hardness", "Opacity", "Flow", "Mode"]),
        (PaintTool::Eraser, EraserMode::Pencil, &["Hardness", "Flow"], &["Size", "Opacity", "Mode"]),
        (PaintTool::Eraser, EraserMode::Block, &["Size", "Hardness", "Opacity", "Flow"], &["Mode"]),
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
