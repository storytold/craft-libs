//! `PresetPicker`: the popover hangs under its field at its grid's width; a click on a preset in it picks it and
//! closes; Escape closes and keeps the value; inside a popover, the picker's own popover nests in the parent, which
//! stays open through the pick.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
use craft_ui::patterns::presets::{PickerField, PresetFns, PresetPicker, PresetRow, PresetSource, PresetState, PresetView};
use craft_ui::popover::Popover;
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, last_pass_runs, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CORAL: u32 = 12;
const TEAL: u32 = 21;

struct Host {
    value: Option<u32>,
    state: PresetState<u32>,
    picked: Vec<u32>,
    open: bool,
}

#[trial]
fn a_click_picks_and_closes_and_escape_closes_keeping_the_value() {
    let host = Rc::new(RefCell::new(Host { value: Some(CORAL), state: PresetState::new(), picked: Vec::new(), open: false }));
    let shown = Rc::clone(&host);
    // A library longer than a row of the grid, so the room would hold every tile on one line.
    let rows = PresetSource::new(
        (),
        PresetFns::new(|_, _| {
            let mut rows = vec![PresetRow::item(CORAL, "Coral", 0), PresetRow::item(TEAL, "Teal", 0)];
            rows.extend((100..130).map(|key| PresetRow::item(key, format!("Preset {key}"), 0)));
            rows
        }),
    );
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { value, state, picked, open } = &mut *host;
        let out = PresetPicker::new("Swatch", Bind::bound(value), &rows, Bind::bound(state)).show(ui);
        picked.extend(out.picked);
        *open = out.open;
    });
    let room = Vec2::new(1200.0, 800.0);
    fixture.room(room);
    fixture.settle();
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the field shows");

    fixture.click(&field);
    fixture.idle(3);
    assert!(fixture.by_label("Teal").is_some(), "the field opens the popover with the presets");
    let popover = fixture.by_role(Role::DIALOG).first().map(|popover| popover.rect).expect("the popover shows");
    assert!(popover.width() < room.x / 2.0, "the popover takes its grid's width, not the room's: {popover:?}");
    assert!(popover.width() >= field.rect.width(), "and is at least as wide as its field: {popover:?} {:?}", field.rect);
    assert!(popover.top() >= field.rect.bottom() && (popover.left() - field.rect.left()).abs() < 1.0, "it hangs under its field: {popover:?}");
    let last = fixture.by_label("Preset 129").expect("the last preset shows").rect;
    assert!(popover.contains_rect(last), "the popover is as tall as its grid's rows at that width: {popover:?} {last:?}");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(host.borrow().value, Some(CORAL), "Escape keeps the value");
    assert!(fixture.by_role(Role::DIALOG).is_empty(), "and closes the popover");
    assert!(!host.borrow().open, "the picker reports it closed");

    fixture.click(&field);
    fixture.idle(3);
    let teal = fixture.by_label("Teal").expect("the popover lists the presets");
    fixture.click(&teal);
    fixture.idle(3);
    let host = host.borrow();
    assert_eq!((host.value, host.picked.as_slice()), (Some(TEAL), &[TEAL][..]), "the click picks");
    assert!(fixture.by_role(Role::DIALOG).is_empty(), "and closes the popover");
}

#[trial]
fn a_picker_in_a_popover_opens_and_picks_with_the_popover_kept_open() {
    let (parent, value) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(Some(1u32))));
    let (parent_seen, value_seen) = (parent.clone(), value.clone());
    let rows = PresetSource::new((), PresetFns::new(|_, _| vec![PresetRow::item(1u32, "Alpha", 0), PresetRow::item(2, "Beta", 0)]));
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Fill").show(ui);
        let mut open = parent_seen.get();
        Popover::new("Paint", Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
            let mut picked = value_seen.get();
            let _ = PresetPicker::new("Pattern", Bind::bound(&mut picked), &rows, Bind::bound(&mut state)).show(ui);
            value_seen.set(picked);
        });
        parent_seen.set(open);
    });
    fixture.settle();
    let field = fixture.by_label("Pattern").expect("the picker's field shows in the popover");
    fixture.click(&field).settle();
    assert!(parent.get(), "opening the picker keeps the popover open");
    let beta = fixture.by_label("Beta").expect("the picker's presets show");
    fixture.click(&beta).settle();
    assert_eq!(value.get(), Some(2), "the click picks Beta");
    assert!(fixture.by_label("Alpha").is_none(), "the pick closes the picker: only the field shows, with Beta");
    assert!(parent.get(), "the pick keeps the popover open");
}

/// A chip whose caption follows a dragged value (a brush size) holds the width of the widest number of its range: as
/// the caption gains digits, and whatever its digits are, the chip and what follows it on the bar stay in place, and
/// each frame runs one pass (a caption that changed the chip's width made the bar's kept measure stale, and the frame
/// ran again).
#[trial]
fn a_chip_with_a_caption_range_keeps_its_width_as_the_caption_grows() {
    let size = Rc::new(Cell::new(9u32));
    let shown = Rc::clone(&size);
    let rows = PresetSource::new((), PresetFns::new(|_, _| vec![PresetRow::item(CORAL, "Coral", 0), PresetRow::item(TEAL, "Teal", 0)]));
    let mut state = PresetState::<u32>::new();
    let mut value = Some(CORAL);
    let mut fixture = Fixture::new(move |ui| {
        Bar::new("Brush options", BarKind::Options).show(ui, |ui, _| {
            let _ = PresetPicker::new("Brush", Bind::bound(&mut value), &rows, Bind::bound(&mut state))
                .field(PickerField::Chip)
                .caption(shown.get().to_string())
                .caption_range((1.0..=5000.0, 0))
                .show(ui);
            let _ = Button::new("Mode").show(ui);
        });
    });
    fixture.settle();
    let place = |fixture: &Fixture| {
        let chip = fixture.by_label("Brush").expect("the chip shows").rect;
        let mode = fixture.by_label("Mode").expect("the button after it shows").rect;
        (chip.width(), mode.left())
    };
    let first = place(&fixture);
    for next in [10, 100, 1000, 4888, 7] {
        size.set(next);
        fixture.frame(Vec::new());
        assert_eq!(fixture.passes(), 1, "caption {next}: the frame runs one pass");
        assert_eq!(place(&fixture), first, "caption {next}: the chip and the button after it stay in place");
    }
}

/// The views of `props` that ran in the last frame, and the list rows that ran in its last pass.
fn list_runs(fixture: &Fixture, props: &str) -> (usize, u32) {
    let views = last_frame_memo_runs(fixture.ctx()).into_iter().filter(|(name, _, run)| name.contains(props) && *run == MemoRun::Ran).count();
    let rows = last_pass_runs(fixture.ctx()).into_iter().filter(|(name, _)| name.contains("ListRow")).map(|(_, runs)| runs).sum();
    (views, rows)
}

/// While nothing changes an open picker's list replays: no list row runs. The search field takes the focus on open, as
/// in the Brush Preset picker: a list that holds the focus runs in every pass.
#[trial]
fn an_open_picker_with_nothing_changing_runs_no_list_row() {
    let mut value = Some(CORAL);
    let mut state = PresetState::<u32>::new();
    let rows = PresetSource::new((), PresetFns::new(|_, _| (0..12).map(|key| PresetRow::item(CORAL + key, format!("Preset {key}"), 0)).collect()));
    let mut fixture = Fixture::new(move |ui| {
        let _ = PresetPicker::new("Swatch", Bind::bound(&mut value), &rows, Bind::bound(&mut state)).view(PresetView::List).toolbar(true).show(ui);
    });
    fixture.room(Vec2::new(800.0, 800.0));
    fixture.settle();
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the field shows");
    fixture.click(&field);
    fixture.settle();
    fixture.frame(vec![]);
    assert!(fixture.by_label("Preset 3").is_some(), "the open popover lists the presets");
    assert_eq!(list_runs(&fixture, "ListProps"), (0, 0), "an idle frame runs neither the list view nor a row");
    let replayed = last_frame_memo_runs(fixture.ctx()).into_iter().any(|(name, _, run)| name.contains("ListProps") && run == MemoRun::Replayed);
    assert!(replayed, "the list view replays");
}
