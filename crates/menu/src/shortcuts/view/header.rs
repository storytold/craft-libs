//! The header: Show (a select, or tabs with `Options::area_tabs`), Filter, Search with Search by key, the
//! keyboard pane's toggle, and Hold Shortcuts when the app gives its hold setting.

use craft_ui::buttons::{IconButton, ToggleButton};
use craft_ui::choice::Switch;
use craft_ui::contract::{Bind, BodyCx, Entry, Props as _, Show as _, ShowContainer as _, Sizing, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::primitives::IconSpec;
use craft_ui::text::{NumberField, TextField};
use egui::Ui;

use crate::shortcuts::recorder::ChordRecorder;
use crate::shortcuts::rows::{AreaId, Filter};
use crate::shortcuts::state::{DialogState, Env, Target};
use crate::shortcuts::view::{recording_text, tab_strip};

/// What one arrow press or wheel notch changes the hold delay by, in seconds.
const HOLD_DELAY_STEP: f64 = 0.05;

/// The header for one pass.
pub(super) fn show(ui: &mut Ui, _cx: &mut BodyCx<'_>, now: f64, env: &mut Env<'_>, state: &mut DialogState) {
    if env.options.area_tabs {
        area_tabs(ui, state);
    }
    Row::new().show(ui, |ui, _| {
        if !env.options.area_tabs {
            area_select(ui, env, state);
        }
        filter_select(ui, env, state);
    });
    Row::new().show(ui, |ui, _| {
        search(ui, now, env, state);
        keyboard_toggle(ui, env, state);
    });
    holds(ui, env, state);
}

/// With the app's hold setting: the Hold Shortcuts switch, the delay while it is on, and the sentence that says what
/// a hold is.
fn holds(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) {
    let Some(setting) = env.holds.as_deref() else { return };
    let (was_on, was_delay, range) = (setting.on, setting.delay, setting.range.clone());
    let words = env.text.words();
    let mut on = was_on;
    let mut delay = Some(was_delay);
    Row::new().show(ui, |ui, _| {
        Switch::new(Bind::bound(&mut on)).label(words.hold_shortcuts.clone()).id_salt("holds").show(ui);
        if was_on {
            let label = words.hold_delay.clone();
            Field::new(label.clone()).id_salt("shortcuts.holdDelay").sizing(Sizing::Content).show(ui, |ui, _| {
                NumberField::new(label.clone(), Bind::bound(&mut delay)).id_salt("holdDelay").range(range.clone()).step(HOLD_DELAY_STEP).show(ui);
            });
        }
    });
    Text::new(words.hold_explanation.clone()).shade(Shade::Faint).show(ui);
    if on != was_on {
        state.set_holds_on(env, on);
    }
    if let Some(delay) = delay
        && delay != was_delay
    {
        state.set_hold_delay(env, delay);
    }
}

/// Show as tabs.
fn area_tabs(ui: &mut Ui, state: &mut DialogState) {
    let titles: Vec<String> = state.areas.iter().map(|area| area.label.clone()).collect();
    let shown = state.areas.iter().position(|area| area.id == state.area).unwrap_or(0);
    if let Some(i) = tab_strip(ui, "shortcuts.area", &titles, shown)
        && let Some(area) = state.areas.get(i)
    {
        let id = area.id.clone();
        state.set_area(id);
    }
}

fn area_select(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState) {
    let label = env.text.words().show.clone();
    let entries: Vec<Entry<AreaId>> = state.areas.iter().map(|area| Entry::new(area.id.clone(), area.label.clone())).collect();
    let mut area = state.area.clone();
    Field::new(label.clone()).id_salt("shortcuts.area").sizing(Sizing::Content).show(ui, |ui, _| {
        Select::new(label.clone(), Bind::bound(&mut area), &entries).id_salt("area").show(ui);
    });
    if area != state.area {
        state.set_area(area);
    }
}

fn filter_select(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState) {
    let label = env.text.words().filter.clone();
    let entries: Vec<Entry<Filter>> = Filter::ALL.iter().map(|filter| Entry::new(*filter, filter.label(env.text))).collect();
    let mut filter = state.filter;
    Field::new(label.clone()).id_salt("shortcuts.filter").sizing(Sizing::Content).show(ui, |ui, _| {
        Select::new(label.clone(), Bind::bound(&mut filter), &entries).id_salt("filter").show(ui);
    });
    if filter != state.filter {
        state.set_filter(filter);
    }
}

/// Search, and Search by key: while it is on, the chord pressed becomes the query.
fn search(ui: &mut Ui, now: f64, env: &mut Env<'_>, state: &mut DialogState) {
    let words = env.text.words();
    let label = words.search.clone();
    let mut query = state.query.clone();
    Field::new(label.clone()).id_salt("shortcuts.search").sizing(Sizing::Content).show(ui, |ui, _| {
        TextField::search(label.clone(), Bind::bound(&mut query)).id_salt("search").show(ui);
    });
    if query != state.query {
        state.set_query(query);
    }
    let searching = matches!(state.recording.as_ref().map(|recording| &recording.target), Some(Target::Search));
    let name = words.search_by_key.clone();
    let toggle = IconButton::new(IconSpec::svg(craft_behavior::IconId::new("keyboard")))
        .id_salt("searchKey")
        .label(name.clone())
        .tooltip(name.clone())
        .selected(searching)
        .show(ui);
    if clicked(&toggle) {
        if searching {
            state.stop_recording();
        } else {
            state.record(env, Target::Search);
        }
    }
    if matches!(state.recording.as_ref().map(|recording| &recording.target), Some(Target::Search)) {
        let shown = recording_text(env, state);
        let out = ChordRecorder::new(name, true, shown).id_salt("searchRecorder").show(ui);
        if let Some(recording) = state.recording.as_mut() {
            recording.started = true;
        }
        if !out.chords.is_empty() {
            state.feed(env, &out.chords, now);
        } else if out.response.lost_focus() {
            state.stop_recording();
        }
    }
}

/// The toggle that shows and hides the keyboard pane.
fn keyboard_toggle(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState) {
    if !env.options.keyboard {
        return;
    }
    let mut shown = !state.keyboard.hidden;
    ToggleButton::new(env.text.words().keyboard.clone(), Bind::bound(&mut shown)).id_salt("keyboard").show(ui);
    state.keyboard.hidden = !shown;
}
