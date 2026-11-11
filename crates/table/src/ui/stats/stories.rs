//! Stories of the selection statistics readout.

use super::{SelectionStats, SelectionStatsReadout, StatsShown, StatsWords, stats_parts};
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Key, Modifiers, Ui};

fn stats() -> SelectionStats {
    SelectionStats::new(4, 2, 7.0, 2.0, 5.0)
}

fn readout(ui: &mut Ui, data: &mut StoryData, initial: StatsShown) -> StoryOutput {
    let shown = *data.value("shown", initial);
    let parts = stats_parts(&stats(), &shown, &StatsWords::default(), |value| format!("{value:.1}"));
    SelectionStatsReadout::new("Selection statistics", &parts, Bind::bound(data.value("shown", initial))).show(ui).into()
}

#[story(SelectionStatsReadout<'static>, covers = [Default])]
fn every_statistic(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    readout(ui, data, StatsShown::all())
}

#[story(SelectionStatsReadout<'static>, covers = [], drive = Drive::Never)]
fn some_hidden(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    readout(ui, data, StatsShown::default())
}

#[story(SelectionStatsReadout<'static>, covers = [Invalid], drive = Drive::Never)]
fn no_statistic_shown_is_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = *data.value("shown", StatsShown::default());
    let parts = stats_parts(&stats(), &shown, &StatsWords::default(), |value| format!("{value:.1}"));
    let refuse_none = |shown: &StatsShown| {
        if *shown == StatsShown::default() { Validity::Refuse(Message::new("Show at least one statistic")) } else { Validity::Ok }
    };
    SelectionStatsReadout::new("Selection statistics", &parts, Bind::bound(data.value("shown", StatsShown::default())).check(refuse_none))
        .show(ui)
        .into()
}

fn open_menu(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::F10, Modifiers::SHIFT);
    fixture.idle(2);
}

fn toggle_average(fixture: &mut Fixture) {
    open_menu(fixture);
    if let Some(item) = fixture.by_label("Average") {
        fixture.click(&item);
    }
    fixture.idle(2);
}

#[story(SelectionStatsReadout<'static>, covers = [Open], drive = Drive::Custom(open_menu))]
fn menu_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    readout(ui, data, StatsShown::all())
}

#[story(SelectionStatsReadout<'static>, covers = [], drive = Drive::Custom(toggle_average))]
fn menu_toggles_a_statistic(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    readout(ui, data, StatsShown::all())
}
