//! `PaintBar`: which members each tool disables, the smoothing options, the gestures, the combinations list, what the
//! load swatch shows, a click on its menu, the Mixer Brush fields' ranges and tooltips.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::EditEvent;
use craft_draw::ui::{EraserMode, MIXER_COMBINATIONS, MixerLoad, PaintBar, PaintTool, PaintValues};
use craft_host::AppInfo;
use craft_i18n::Lang;
use craft_theme::Srgba;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::choice::SwatchImage;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Entry, Role, ShowContainer as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Room, Target, trial};
use craft_ui::text::RangeAlert;
use egui::{Color32, ColorImage, Key, Modifiers, Pos2, Shape, TextureId};

struct Host {
    values: PaintValues<u32>,
    eraser: EraserMode,
    edits: Vec<EditEvent<PaintValues<u32>>>,
    /// How many passes reported Clean Brush.
    cleans: u32,
    /// The range alerts the bar reported, one per clamped commit.
    alerts: Vec<RangeAlert>,
}

/// The bar's host, English whatever the machine's locale is: the trials type digits and read the digits the fields
/// paint, so the number format has to be the one they write.
fn english_host() -> Arc<craft_host::Host> {
    Arc::new(craft_host::Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).lang(Lang::EN).build())
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
            alerts.extend(out.out_of_range);
            *cleans += u32::from(out.mixer_clean);
        });
    });
    fixture.host(english_host());
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

/// The bar's percent control named `label`, by its spin button node: the field keeps that node while the bar disables
/// it, so a trial reads the control and not its caption.
fn control(fixture: &Fixture, label: &str) -> Target {
    fixture
        .by_role(Role::SPIN_BUTTON)
        .into_iter()
        .find(|field| fixture.tree().name_of(field.node) == Some(label))
        .unwrap_or_else(|| panic!("the bar shows the {label} field"))
}

/// Whether the bar's percent control named `label` paints digits inside its own bounds.
fn shows_digits(fixture: &Fixture, label: &str) -> bool {
    let field = control(fixture, label);
    fixture.texts().iter().any(|run| field.rect.intersects(run.rect) && run.text.chars().any(|c| c.is_ascii_digit()))
}

/// The number the bar's percent control named `label` reports to assistive technology, in shown percent; `None` when
/// the field has no number to read.
fn shown_number(fixture: &Fixture, label: &str) -> Option<f64> {
    fixture.tree().numeric_of(control(fixture, label).node).and_then(|numeric| numeric.value)
}

/// The middle of the bar's painted caption named `label`: the scrub handle a drag starts on.
fn handle_of(fixture: &Fixture, label: &str) -> Pos2 {
    let caption = fixture.texts().iter().find(|painted| painted.text == label).unwrap_or_else(|| panic!("the bar shows the {label} caption"));
    caption.rect.center()
}

/// Types `text` into the bar's percent field named `label` and commits it with Enter.
fn type_into(fixture: &mut Fixture, label: &str, text: &str) {
    let field = fixture.visible(&control(fixture, label)).expect("the field is on screen");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE).settle();
}

/// What the bar adds while the pointer rests on the control named `label`, after the tooltip delay, as one line.
fn hover_texts(fixture: &mut Fixture, label: &str) -> String {
    let target = fixture.visible(&fixture.by_label(label).expect("the bar shows the control")).expect("the control is on screen");
    let before: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    fixture.frame(vec![moved(target.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    let added: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).filter(|text| !before.contains(text)).collect();
    added.join(" ")
}

/// The value the bar's values hold for the field named `label`, of the Brush bar's fields.
fn value_of(values: &PaintValues<u32>, label: &str) -> f64 {
    match label {
        "Opacity" => values.opacity,
        "Flow" => values.flow,
        "Smoothing" => values.smoothing.amount,
        other => panic!("{other} is not one of the fields this trial types into"),
    }
}

/// Measured on the reference application: the Mixer Brush's Load and Flow refuse a value below 1 % and keep the
/// nearest one, the clamp reported once with the range it belongs to; a 1 % entry itself commits without an alert.
#[trial]
fn a_mixer_load_or_flow_below_one_percent_keeps_one_percent_and_reports_the_clamp() {
    let cases: [(&str, &str, f64, f64, usize); 4] =
        [("Load", "0", 0.0, 0.01, 1), ("Flow", "0", 0.0, 0.01, 1), ("Load", "-8", -8.0, 0.01, 1), ("Load", "1", 1.0, 0.01, 0)];
    for (label, typed, typed_number, kept, alerts) in cases {
        let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| {
            v.mixer.load = 0.5;
            v.mixer.flow = 0.42;
        });
        page_to_end(&mut fixture);
        type_into(&mut fixture, label, typed);
        let host = host.borrow();
        let stored = if label == "Load" { host.values.mixer.load } else { host.values.mixer.flow };
        assert_eq!(stored, kept, "{label}: {typed} stores {kept}");
        assert_eq!(host.alerts.len(), alerts, "{label}: {typed} reports {alerts} alerts: {:?}", host.alerts);
        if let Some(alert) = host.alerts.first() {
            assert_eq!(alert.typed, typed_number, "{label}: the alert names what was typed");
            assert_eq!(alert.kept, 1.0, "{label}: the alert names the nearest value");
            assert_eq!(alert.range, 1.0..=100.0, "{label}: the field holds 1 to 100 %");
            assert!(alert.whole, "{label}: the field commits whole percents");
            assert_eq!(alert.unit_kind, Some(UnitKind::Percent), "{label}: the range is worded in percent");
        }
    }

    // A value above the range keeps the top end, with the same alert for the two fields.
    for label in ["Load", "Flow"] {
        let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| {
            v.mixer.load = 0.5;
            v.mixer.flow = 0.42;
        });
        page_to_end(&mut fixture);
        type_into(&mut fixture, label, "150");
        let host = host.borrow();
        let stored = if label == "Load" { host.values.mixer.load } else { host.values.mixer.flow };
        assert_eq!(stored, 1.0, "{label}: 150 stores 100 %");
        assert_eq!(host.alerts.len(), 1, "{label}: the clamp is reported once");
        assert_eq!(host.alerts[0].range, 1.0..=100.0, "{label}: the range ends at 100 %");
    }
}

/// Measured: Wet and Mix keep 0 as their own value with no alert, and only a value past 0 or past 100 reports the
/// clamp of a 0 to 100 % range.
#[trial]
fn the_mixer_wet_and_mix_fields_keep_zero_and_report_only_values_past_their_range() {
    for label in ["Wet", "Mix"] {
        for (typed, kept, alerts) in [("0", 0.0, 0), ("50", 0.5, 0), ("150", 1.0, 1), ("-5", 0.0, 1)] {
            let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| {
                v.mixer.wet = 0.2;
                v.mixer.mix = 0.5;
            });
            page_to_end(&mut fixture);
            type_into(&mut fixture, label, typed);
            let host = host.borrow();
            let stored = if label == "Wet" { host.values.mixer.wet } else { host.values.mixer.mix };
            assert_eq!(stored, kept, "{label}: {typed} stores {kept}");
            assert_eq!(host.alerts.len(), alerts, "{label}: {typed} reports {alerts} alerts: {:?}", host.alerts);
            if let Some(alert) = host.alerts.first() {
                assert_eq!(alert.range, 0.0..=100.0, "{label}: the field holds 0 to 100 %");
                assert_eq!(alert.kept, kept * 100.0, "{label}: the alert names the nearest value");
            }
        }
    }
}

/// Measured: while Wet is 0 the Mix field is greyed out and shows an empty field, and it comes back with the value it
/// kept once Wet is above 0 again.
#[trial]
fn the_mixer_mix_field_blanks_and_closes_while_wet_is_zero_and_reopens_with_its_value() {
    let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| {
        v.mixer.wet = 0.0;
        v.mixer.mix = 0.5;
    });
    page_to_end(&mut fixture);
    assert!(shows_digits(&fixture, "Wet"), "the trial reads a field that shows its number");
    assert!(fixture.tree().is_disabled_key(control(&fixture, "Mix").node), "Mix is greyed out while Wet is 0");
    assert!(!shows_digits(&fixture, "Mix"), "and shows an empty field");
    assert_eq!(shown_number(&fixture, "Mix"), None, "while closed the field reports no number to assistive technology");
    assert_eq!(fixture.tree().value_of(control(&fixture, "Mix").node), Some(""), "and its text value is empty");
    let held = host.borrow();
    assert_eq!(held.values.mixer.mix, 0.5, "the closed field keeps its value");
    assert!(held.edits.is_empty(), "showing a closed field edits nothing");
    drop(held);

    type_into(&mut fixture, "Wet", "40");
    assert!(!fixture.tree().is_disabled_key(control(&fixture, "Mix").node), "Mix opens again while Wet is above 0");
    assert!(shows_digits(&fixture, "Mix"), "and shows its number again");
    assert_eq!(shown_number(&fixture, "Mix"), Some(50.0), "and reads 50 % out again");
    let host = host.borrow();
    assert_eq!((host.values.mixer.wet, host.values.mixer.mix), (0.4, 0.5), "Wet took the typed value, Mix kept its own");
}

/// The side handle is the other path a percent value moves by, and it follows the same two rules: the closed Mix
/// handle scrubs nothing, and a Load handle dragged down from 5 % stops at 1 %. A scrub keeps its value inside the
/// range without a range report, which only a typed entry past it earns.
#[trial]
fn a_scrub_on_the_side_handle_keeps_the_closed_field_still_and_stops_at_one_percent() {
    let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| {
        v.mixer.wet = 0.0;
        v.mixer.mix = 0.5;
    });
    page_to_end(&mut fixture);
    fixture.drag_at(handle_of(&fixture, "Mix"), egui::Vec2::new(-120.0, 0.0)).settle();
    fixture.idle(3);
    let host = host.borrow();
    assert_eq!(host.values.mixer.mix, 0.5, "a drag on the closed Mix handle changes nothing");
    assert!(host.edits.is_empty(), "and asks for no edit: {} events", host.edits.len());
    assert!(host.alerts.is_empty(), "nor reports a range: {:?}", host.alerts);
    drop(host);

    let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| v.mixer.load = 0.05);
    page_to_end(&mut fixture);
    assert_eq!(shown_number(&fixture, "Load"), Some(5.0), "the field starts at 5 %");
    fixture.drag_at(handle_of(&fixture, "Load"), egui::Vec2::new(-120.0, 0.0)).settle();
    fixture.idle(3);
    let host = host.borrow();
    assert_eq!(host.values.mixer.load, 0.01, "a scrub down past 1 % stops at 1 %");
    assert_eq!(shown_number(&fixture, "Load"), Some(1.0), "and the field reads 1 %");
    assert!(host.alerts.is_empty(), "a scrub is not a typed entry past the range: {:?}", host.alerts);
}

/// Measured: the five Mixer Brush controls carry these tooltips, as the accessible description of the control and as
/// the text hovering it shows.
#[trial]
fn the_mixer_bars_five_controls_carry_their_measured_tooltips() {
    let cases: [(&str, &str); 5] = [
        ("Wet", "Set the amount of paint picked up from the Canvas"),
        ("Load", "Set the amount of paint on the brush"),
        ("Mix", "Set the color mixing ratio for stroke"),
        ("Flow", "Set flow rate for stroke"),
        ("Sample All Layers", "Pick up wet paint from all layers"),
    ];
    for (label, tip) in cases {
        let (mut fixture, _) = harness(PaintTool::MixerBrush, EraserMode::Brush, |_| {});
        page_to_end(&mut fixture);
        let node = fixture.by_label(label).expect("the bar shows the control").node;
        assert_eq!(fixture.tree().description_of(node), Some(tip), "{label}: the tooltip is the control's description");
        let shown = hover_texts(&mut fixture, label);
        let one_line = shown.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(one_line.contains(tip), "{label}: resting the pointer shows {tip:?}, the bar shows {shown:?}");
    }
}

/// The bar's other tools keep the ranges and the plain fields they had: their percent fields take 0 with no alert,
/// still clamp past their ends with a range that starts at 0 %, and none of them takes a Mixer Brush tooltip.
#[trial]
fn the_other_bars_percent_fields_keep_zero_and_carry_no_mixer_tooltip() {
    for (tool, label) in [(PaintTool::Brush, "Opacity"), (PaintTool::Brush, "Flow"), (PaintTool::Brush, "Smoothing"), (PaintTool::Pencil, "Opacity")]
    {
        for (typed, alerts) in [("0", 0), ("-5", 1)] {
            let (mut fixture, host) = harness(tool, EraserMode::Brush, |_| {});
            page_to_end(&mut fixture);
            type_into(&mut fixture, label, typed);
            let host = host.borrow();
            assert_eq!(value_of(&host.values, label), 0.0, "{tool:?} {label}: {typed} stores 0");
            assert_eq!(host.alerts.len(), alerts, "{tool:?} {label}: {typed} reports {alerts} alerts: {:?}", host.alerts);
            if let Some(alert) = host.alerts.first() {
                assert_eq!(alert.range, 0.0..=100.0, "{tool:?} {label}: the field still holds 0 to 100 %");
            }
        }
    }

    let (mut fixture, host) = harness(PaintTool::Brush, EraserMode::Brush, |_| {});
    page_to_end(&mut fixture);
    type_into(&mut fixture, "Opacity", "150");
    let host = host.borrow();
    assert_eq!(host.values.opacity, 1.0, "the Brush bar's Opacity still clamps at its top end");
    assert_eq!(host.alerts.len(), 1, "and reports the clamp once");
    assert_eq!(host.alerts[0].range, 0.0..=100.0, "the Brush bar's range starts at 0 %");
    drop(host);

    let (mut fixture, _) = harness(PaintTool::Brush, EraserMode::Brush, |_| {});
    page_to_end(&mut fixture);
    let shown = hover_texts(&mut fixture, "Flow");
    for tip in ["Set flow rate for stroke", "Set the amount of paint on the brush", "Set the color mixing ratio for stroke"] {
        assert!(!shown.contains(tip), "the Brush bar's Flow shows {tip:?}: {shown:?}");
    }
}

/// A host that stored 0 in Load or Flow keeps seeing that 0 until it stores something else: the bar holds what the
/// user types and steps, it does not rewrite the values it was given. Guard test, passes before and after.
#[trial]
fn a_mixer_load_the_host_stored_below_the_range_is_shown_as_it_was_stored() {
    let (mut fixture, host) = harness(PaintTool::MixerBrush, EraserMode::Brush, |v| {
        v.mixer.load = 0.0;
        v.mixer.flow = 0.0;
    });
    page_to_end(&mut fixture);
    fixture.idle(3);
    let host = host.borrow();
    assert_eq!((host.values.mixer.load, host.values.mixer.flow), (0.0, 0.0), "showing the bar keeps the stored values");
    assert!(host.edits.is_empty(), "and edits nothing: {} events", host.edits.len());
    assert!(host.alerts.is_empty(), "nor reports a range alert");
}
