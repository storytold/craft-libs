//! The Path Options gear's rows: the optional Curve Fit row, what the popover reports for it, and the settings rows
//! that stay as they are beside it.

use craft_draw::ui::pen::{PathOptionsPopover, PathOptionsSettings};
use craft_host::{AppInfo, Host};
use craft_i18n::{Format, I18n};
use craft_ui::contract::Bind;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::RangeAlert;
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The host's side of the gear: the settings, who owns `open`, and the Curve Fit value a tool keeps.
#[derive(Debug)]
struct Held {
    settings: PathOptionsSettings,
    open: bool,
    curve_fit: f32,
}

/// What the popover reported over the trial.
#[derive(Clone, Debug, Default)]
struct Seen {
    /// The Curve Fit range alerts the popover surfaced, in the order it reported them.
    alerts: Vec<RangeAlert>,
    /// The passes the popover reported refused Curve Fit text in.
    refused: usize,
    /// The passes the popover reported a settings change in.
    edits: usize,
}

/// The gear, its host, and what it reported.
struct Gear {
    fixture: Fixture,
    host: Rc<RefCell<Held>>,
    seen: Rc<RefCell<Seen>>,
}

/// A closed gear over `settings`, with the Curve Fit row only when `with_curve_fit` says so. The host fixes the
/// number format to `en`, so a typed `.` is the decimal separator the field reads whatever machine runs the trial.
fn gear(with_curve_fit: bool, settings: PathOptionsSettings, curve_fit: f32) -> Gear {
    let (i18n, problems) = I18n::from_owned(std::iter::empty(), &["en"]);
    assert!(problems.is_empty(), "{problems:?}");
    i18n.set_format(Format::from_tag("en"));
    let host = Rc::new(RefCell::new(Held { settings, open: false, curve_fit }));
    let seen = Rc::new(RefCell::new(Seen::default()));
    let (shown, reports) = (Rc::clone(&host), Rc::clone(&seen));
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let Held { settings, open, curve_fit } = &mut *held;
        let mut popover = PathOptionsPopover::new("Path Options", Bind::bound(settings));
        if with_curve_fit {
            popover = popover.curve_fit(Bind::bound(curve_fit));
        }
        let out = popover.show(ui, open);
        let mut reports = reports.borrow_mut();
        reports.alerts.extend(out.curve_fit_out_of_range.clone());
        reports.refused += usize::from(out.curve_fit_rejected_text);
        reports.edits += usize::from(out.edit.is_some());
    });
    fixture.host(Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).build()));
    fixture.settle();
    Gear { fixture, host, seen }
}

impl Gear {
    /// Opens the popover; the host owns `open`.
    fn open(&mut self) {
        self.host.borrow_mut().open = true;
        self.fixture.idle(3);
    }

    fn settings(&self) -> PathOptionsSettings {
        self.host.borrow().settings
    }

    fn curve_fit(&self) -> f32 {
        self.host.borrow().curve_fit
    }

    fn seen(&self) -> Seen {
        self.seen.borrow().clone()
    }

    /// Types `text` over the Curve Fit field, then commits it with `end` unless `end` is `None`, which leaves the
    /// text standing in the field.
    fn type_over_curve_fit(&mut self, text: &str, end: Option<Key>) {
        let field = self.fixture.by_label("Curve Fit:").expect("the Curve Fit row");
        self.fixture.click(&field).settle();
        self.fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
        if let Some(end) = end {
            self.fixture.key(end, Modifiers::NONE).settle();
        }
    }

    /// Types `text` over the Thickness field and ends the edit with `end`.
    fn type_over_thickness(&mut self, text: &str, end: Key) {
        let field = self.fixture.by_label("Thickness").expect("the Thickness row");
        self.fixture.click(&field).settle();
        self.fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
        self.fixture.key(end, Modifiers::NONE).settle();
    }

    /// Clicks the Thickness row, which takes the focus from wherever it is.
    fn click_thickness(&mut self) {
        let field = self.fixture.by_label("Thickness").expect("the Thickness row");
        self.fixture.click(&field).settle();
    }

    /// The text the popover paints.
    fn painted(&self) -> Vec<String> {
        self.fixture.texts().iter().map(|text| text.text.clone()).collect()
    }
}

/// The Curve Fit row shows only for a host that passes `curve_fit`; the Thickness, Color and Rubber Band rows are
/// there either way, and a gear without the row reports nothing about Curve Fit.
#[trial]
fn only_a_host_that_asks_for_it_gets_a_curve_fit_row() {
    for with in [true, false] {
        let mut rig = gear(with, PathOptionsSettings::default(), 2.0);
        rig.open();
        assert_eq!(rig.fixture.by_label("Curve Fit:").is_some(), with, "the row shows with the builder call: {with}");
        assert!(
            rig.fixture.by_label("Thickness").is_some() && rig.fixture.by_label("Color").is_some() && rig.fixture.by_label("Rubber Band").is_some(),
            "the settings rows are there either way"
        );
        rig.type_over_thickness("6", Key::Enter);
        assert_eq!(rig.settings().thickness, Some(6.0), "the Thickness row works as it did");
        let seen = rig.seen();
        assert!(seen.alerts.is_empty() && seen.refused == 0, "no Curve Fit report without the row: {seen:?}");
    }
}

/// The row shows the value the host holds, and a typed number commits at one decimal: 3.33 becomes 3.3, kept in the
/// field's text and in the host's value. Nothing is reported for a value inside the range.
#[trial]
fn a_typed_curve_fit_commits_one_decimal() {
    let mut rig = gear(true, PathOptionsSettings::default(), 2.0);
    rig.open();
    assert!(rig.painted().iter().any(|text| text == "2"), "the field shows the host's 2 px: {:?}", rig.painted());
    rig.type_over_curve_fit("3.33", Some(Key::Enter));
    assert_eq!(rig.curve_fit(), 3.3, "3.33 commits as 3.3");
    assert!(rig.painted().iter().any(|text| text == "3.3"), "the field shows the one decimal it keeps: {:?}", rig.painted());
    let seen = rig.seen();
    assert!(seen.alerts.is_empty() && seen.refused == 0, "a value inside the range reports nothing: {seen:?}");
    rig.type_over_curve_fit("0.25", Some(Key::Enter));
    assert_eq!(rig.curve_fit(), 0.5, "one decimal typed below the range keeps the range start");
}

/// A typed Curve Fit outside 0.5 to 10 px commits the nearest end and surfaces one range alert for that pass; Enter,
/// Tab and a key step that runs into the end tell it apart: only a typed commit surfaces one.
#[trial]
fn a_curve_fit_past_the_range_clamps_and_reports_its_alert_once() {
    let mut rig = gear(true, PathOptionsSettings::default(), 2.0);
    rig.open();

    rig.type_over_curve_fit("11", Some(Key::Enter));
    assert_eq!(rig.curve_fit(), 10.0, "11 commits the range's end");
    {
        let seen = rig.seen();
        assert_eq!(seen.alerts.len(), 1, "Enter surfaces one alert: {seen:?}");
        let alert = &seen.alerts[0];
        assert_eq!((alert.typed, alert.kept, alert.range.clone()), (11.0, 10.0, 0.5..=10.0));
        assert_eq!(alert.unit_kind, Some(UnitKind::Pixels), "the row names the unit it counts");
        assert!(!alert.whole, "the row commits one decimal, not whole pixels");
    }

    rig.type_over_curve_fit("0", Some(Key::Tab));
    assert_eq!(rig.curve_fit(), 0.5, "0 commits the range's start");
    let seen = rig.seen();
    assert_eq!(seen.alerts.len(), 2, "Tab surfaces it too");
    assert_eq!(seen.alerts[1].typed, 0.0);

    rig.type_over_curve_fit("99", Some(Key::Enter));
    let past = rig.seen().alerts.len();
    assert_eq!(past, 3, "a third typed value surfaces a third alert");
    let field = rig.fixture.by_label("Curve Fit:").expect("the row");
    rig.fixture.click(&field).settle();
    for _ in 0..4 {
        rig.fixture.key(Key::ArrowUp, Modifiers::NONE);
    }
    rig.fixture.settle();
    assert_eq!(rig.curve_fit(), 10.0, "the steps run into the end");
    assert_eq!(rig.seen().alerts.len(), past, "key steps surface nothing: {:?}", rig.seen().alerts);
}

/// Curve Fit text that is no number keeps the value the host holds and is reported once in the pass its commit was
/// refused: by Enter, by Tab, and by the focus loss of a press on another row. Escape cancels the edit and owes no
/// report, and a step over refused text reports nothing.
#[trial]
fn curve_fit_text_that_is_no_number_is_reported_once_per_commit() {
    let mut rig = gear(true, PathOptionsSettings::default(), 2.0);
    rig.open();

    rig.type_over_curve_fit("abc", Some(Key::Enter));
    assert_eq!(rig.curve_fit(), 2.0, "the refused commit leaves the value");
    assert!(rig.painted().iter().any(|text| text == "abc"), "the field still shows the refused text: {:?}", rig.painted());
    assert_eq!(rig.seen().refused, 1, "Enter reports it in the pass it was refused");
    rig.fixture.idle(4);
    assert_eq!(rig.seen().refused, 1, "the passes after report it no more");

    rig.type_over_curve_fit("4", Some(Key::Enter));
    assert_eq!(rig.curve_fit(), 4.0, "a number typed after it commits");
    assert_eq!(rig.seen().refused, 1, "a value that reads owes no report");

    rig.type_over_curve_fit("abc", Some(Key::Tab));
    assert_eq!(rig.curve_fit(), 4.0, "Tab ends the edit with the value it held");
    assert_eq!(rig.seen().refused, 2, "Tab reports it");

    rig.type_over_curve_fit("abc", None);
    rig.click_thickness();
    assert_eq!(rig.curve_fit(), 4.0, "a press on another row leaves the value");
    assert_eq!(rig.seen().refused, 3, "and the focus loss reports the refused text");

    rig.type_over_curve_fit("abc", Some(Key::Escape));
    assert_eq!(rig.curve_fit(), 4.0, "Escape restores the value too");
    assert_eq!(rig.seen().refused, 3, "Escape cancels the edit: it is not a refused commit");

    rig.type_over_curve_fit("7", Some(Key::Enter));
    assert_eq!(rig.curve_fit(), 7.0, "the row goes back to work after the refused text");
    assert_eq!(rig.seen().refused, 3, "a value that reads reports nothing");
}

/// The Curve Fit row leaves the settings rows alone: the Rubber Band checkbox still toggles with the row shown, the
/// settings are not written by a Curve Fit edit, and the popover reports one settings change for the checkbox only.
#[trial]
fn the_curve_fit_row_leaves_the_settings_rows_as_they_are() {
    let start = PathOptionsSettings { rubber_band: false, thickness: Some(2.0), color: Some([10, 20, 30]), ..PathOptionsSettings::default() };
    let mut rig = gear(true, start, 2.0);
    rig.open();
    let checkbox = rig.fixture.by_label("Rubber Band").expect("the Rubber Band row");
    rig.fixture.click(&checkbox).settle();
    assert!(rig.settings().rubber_band, "the checkbox still turns with the Curve Fit row shown");
    let after = rig.settings();
    rig.type_over_curve_fit("3.33", Some(Key::Enter));
    rig.type_over_curve_fit("99", Some(Key::Tab));
    assert_eq!(rig.curve_fit(), 10.0, "the Curve Fit row wrote its own value");
    assert_eq!(rig.settings(), after, "no settings row moved");
    assert_eq!(rig.seen().edits, 1, "the settings change is the checkbox alone: {:?}", rig.seen());
    assert_eq!(rig.seen().alerts.len(), 1, "the clamped Curve Fit surfaced one alert");
}
