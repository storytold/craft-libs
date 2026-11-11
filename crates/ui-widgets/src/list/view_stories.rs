//! Stories of `List`, `FindingsList`, `FindingsSummary`, `EventList` and `UndoList`.

use super::view_words::{EventWords, FindingsWords};
use super::{
    EventLevel, EventList, EventLog, EventLogState, Finding, Findings, FindingsList, FindingsSummary, List, ListRow, LogEvent, Severity, UndoList,
    UndoMark, UndoRows,
};
use crate::contract::{Bind, Show as _, ShowCollection as _, WordSetCx as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{DisplayOrder, ItemSource};
use egui::Ui;

/// A list of names keyed by position.
struct Names<'a>(&'a [String]);

impl DisplayOrder<usize> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|name| std::borrow::Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for Names<'_> {}

fn names(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("Layer {n}")).collect()
}

fn list(ui: &mut Ui, count: usize) -> StoryOutput {
    let names = names(count);
    List::<usize>::new("Layers").empty("No layers").show_items(ui, &Names(&names), &mut |key, _| ListRow::new(names[*key].clone())).into()
}

#[story(List<'static, usize>, covers = [Default, Many], drive = Drive::Never)]
fn list_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, 40)
}

#[story(List<'static, usize>, covers = [One], drive = Drive::Never)]
fn list_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, 1)
}

#[story(List<'static, usize>, covers = [Empty], drive = Drive::Never)]
fn list_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    list(ui, 0)
}

fn findings(count: usize) -> Vec<Finding> {
    (0..count)
        .map(|n| {
            let id = n as u64;
            match n % 3 {
                0 => Finding::new(id, Severity::Info, format!("Layer {} has no name", n + 1)),
                1 => Finding::new(id, Severity::Warning, format!("Font {} is missing", n + 1)).detail(format!("page {}", n + 1)),
                _ => Finding::new(id, Severity::Error, format!("Page {} overflows its frame", n + 1)),
            }
        })
        .collect()
}

fn findings_list(ui: &mut Ui, findings: &[Finding]) -> StoryOutput {
    let source = Findings::new(findings);
    FindingsList::new("Findings").show_items(ui, &source, &mut |key, icx| findings[*key as usize].row(&icx.words::<FindingsWords>())).into()
}

#[story(FindingsList<'static>, covers = [Default, Many], drive = Drive::Never)]
fn findings_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    findings_list(ui, &findings(30))
}

#[story(FindingsList<'static>, covers = [One], drive = Drive::Never)]
fn findings_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    findings_list(ui, &findings(1))
}

#[story(FindingsList<'static>, covers = [Empty], drive = Drive::Never)]
fn findings_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    findings_list(ui, &[])
}

#[story(FindingsSummary<'static>, covers = [Default], drive = Drive::Never)]
fn findings_summary(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FindingsSummary::new(&findings(3)).show(ui).into()
}

/// The time the event stories take as now, in milliseconds.
const NOW_MS: u64 = 3_600_000;

fn events(count: usize) -> Vec<LogEvent> {
    let levels = [EventLevel::Info, EventLevel::Warning, EventLevel::Error];
    (0..count)
        .map(|n| {
            LogEvent::new(n as u64 + 1, levels[n % 3], "Render", format!("Event {}", n + 1), NOW_MS - (n as u64 + 1) * 60_000)
                .count(1 + (n % 2) as u32)
        })
        .collect()
}

fn event_list(ui: &mut Ui, count: usize) -> StoryOutput {
    let events = events(count);
    let log = EventLog::new(&events, &EventLogState::default());
    EventList::new("Events")
        .show_items(ui, &log, &mut |_, icx| {
            let Some(event) = log.event(icx.at) else { return ListRow::spacer() };
            let age = format!("{} min ago", event.age_seconds(NOW_MS) / 60);
            event.row(&age, &icx.words::<EventWords>(), icx.format(), icx.row.striped)
        })
        .into()
}

#[story(EventList<'static>, covers = [Default, Many], drive = Drive::Never)]
fn event_list_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    event_list(ui, 12)
}

#[story(EventList<'static>, covers = [One], drive = Drive::Never)]
fn event_list_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    event_list(ui, 1)
}

#[story(EventList<'static>, covers = [Empty], drive = Drive::Never)]
fn event_list_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    event_list(ui, 0)
}

fn undo_list(ui: &mut Ui, data: &mut StoryData, count: usize) -> StoryOutput {
    let labels: Vec<String> = (1..=count).rev().map(|n| format!("Action {n}")).collect();
    let mark = data.value("mark", UndoMark::default());
    let rows = UndoRows::new(&labels);
    UndoList::new("Undo History").selection(Bind::bound(mark)).show_items(ui, &rows, &mut |key, _| rows.row(*key)).into()
}

#[story(UndoList<'static>, covers = [Default, Many], drive = Drive::Never)]
fn undo_list_many(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    undo_list(ui, data, 20)
}

#[story(UndoList<'static>, covers = [One], drive = Drive::Never)]
fn undo_list_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    undo_list(ui, data, 1)
}

#[story(UndoList<'static>, covers = [Empty], drive = Drive::Never)]
fn undo_list_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    undo_list(ui, data, 0)
}
