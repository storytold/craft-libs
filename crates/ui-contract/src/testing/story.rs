//! Stories: named ways to show one registered type, and the data and output the runner reads from them.

use super::Fixture;
use crate::contract::{CollectionEvent, Inner, Outcome, TooltipSpec};
use craft_behavior::{EditEvent, EditEvents, PartKey, PartRef, Value};
use egui::{Id, Response, Ui, Vec2};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::{Range, RangeInclusive};

/// A state a registered type must show in some story.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Required {
    /// The ordinary look.
    Default,
    /// A value that differs across a selection.
    Mixed,
    /// A value the check refuses.
    Invalid,
    /// Text longer than its room.
    LongText,
    /// An open popup.
    Open,
    /// A collection with no rows.
    Empty,
    /// A collection with one row.
    One,
    /// A collection with many rows.
    Many,
}

/// How the runner changes the value a story shows.
#[non_exhaustive]
#[derive(Clone, Copy)]
pub enum Drive {
    /// Chosen from the type's facts.
    Auto,
    /// Primary click, Enter, Space and AccessKit `Click`, each in its own run.
    Activate,
    /// AccessKit Increment, then the forward key.
    Step,
    /// A drag of the grip's part by twice the slop along its direction, and a second run cancelled by Escape.
    Drag(Grip),
    /// A drag of the grip's part on a component the subject shows inside itself (a colour area's point picker), as
    /// [`Drive::Drag`] drags a part of the subject.
    DragChild(Grip),
    /// Typed text.
    Type(&'static str),
    /// Open, then Down and Enter on the row; a second run clicks the row.
    Pick {
        /// The row to pick.
        row: usize,
    },
    /// The story's own drive.
    Custom(fn(&mut Fixture)),
    /// The story shows a state and changes nothing.
    Never,
}

/// The part a [`Drive::Drag`] or [`Drive::DragChild`] presses and the way the drag moves it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grip {
    /// The pressed part.
    pub part: PartRef,
    /// The drag's direction, in units of the travel (twice the slop): `vec2(1.0, 0.0)` moves the travel along x,
    /// `vec2(1.0, 1.0)` the travel along each axis.
    pub toward: Vec2,
}

impl Grip {
    /// The part `key`, a part that occurs once, dragged along x.
    pub const fn new(key: PartKey) -> Self {
        Grip::part(PartRef { key, index: None })
    }

    /// The part `part`, one of a repeated part (`HANDLE.at(i)`), dragged along x.
    pub const fn part(part: PartRef) -> Self {
        Grip { part, toward: Vec2::X }
    }

    /// The grip dragged along `toward`.
    pub const fn toward(mut self, toward: Vec2) -> Self {
        self.toward = toward;
        self
    }
}

/// A named way to show one registered type.
#[non_exhaustive]
#[derive(Clone, Copy)]
pub struct Story {
    /// The story's name.
    pub name: &'static str,
    /// The required states it shows.
    pub covers: &'static [Required],
    /// Shows the type.
    pub show: fn(&mut Ui, &mut StoryData) -> StoryOutput,
    /// How the runner changes the shown value.
    pub drive: Drive,
    /// The reason a disabled control shows (`disabled_text` of its tooltip), when the story gives its control one.
    pub disabled_text: Option<&'static str>,
    /// The caption a viewer shows over the story; `None` for the title made of its name (`contract::story_title`).
    pub title: Option<&'static str>,
    /// The group a viewer shows the story in, with the other stories of the type that name it, as the cases of one
    /// old gallery story; `None` for a story that is its own group, under its caption.
    pub group: Option<&'static str>,
}

impl Story {
    /// A story that shows with `show`.
    pub const fn new(name: &'static str, show: fn(&mut Ui, &mut StoryData) -> StoryOutput) -> Self {
        Story { name, covers: &[], show, drive: Drive::Auto, disabled_text: None, title: None, group: None }
    }

    /// The caption a viewer shows over the story.
    pub const fn title(mut self, title: &'static str) -> Self {
        self.title = Some(title);
        self
    }

    /// The group a viewer shows the story in.
    pub const fn group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    /// The name of the story's group as a story of the type whose display name is `type_name`: its group, else its
    /// caption.
    pub fn group_name(&self, type_name: &str) -> String {
        self.group.map_or_else(|| self.caption(type_name), str::to_owned)
    }

    /// The story's caption as a story of the type whose display name is `type_name`: its title, else the one its
    /// name makes ([`crate::contract::story_title`]).
    pub fn caption(&self, type_name: &str) -> String {
        self.title.map_or_else(|| crate::contract::story_title(type_name, self.name), str::to_owned)
    }

    /// The states the story shows.
    pub const fn covers(mut self, covers: &'static [Required]) -> Self {
        self.covers = covers;
        self
    }

    /// The reason the story's control shows while disabled. The story gives its tooltip this text through
    /// [`StoryData::tooltip`], and the `disabled` case then checks that the control stays a Tab stop and shows it.
    pub const fn disabled_text(mut self, text: &'static str) -> Self {
        self.disabled_text = Some(text);
        self
    }

    /// The drive the story names.
    pub const fn drive(mut self, drive: Drive) -> Self {
        self.drive = drive;
        self
    }
}

/// A story registered by `#[story]`, linked to its type by the type's registered name.
#[doc(hidden)]
#[non_exhaustive]
pub struct StoryRegistration {
    /// The package that declares the story.
    pub crate_name: &'static str,
    /// The crate target that declares it.
    pub crate_target: &'static str,
    /// The registered name of the type the story shows.
    pub type_name: fn() -> &'static str,
    /// The story.
    pub story: Story,
    /// The source file of the declaration.
    pub file: &'static str,
    /// The line of the declaration in `file`.
    pub line: u32,
}

impl StoryRegistration {
    /// The registration `#[story]` writes.
    pub const fn new(crate_name: &'static str, crate_target: &'static str, type_name: fn() -> &'static str, story: Story) -> Self {
        StoryRegistration { crate_name, crate_target, type_name, story, file: "", line: 0 }
    }

    /// The registration declared at `line` of `file`.
    pub const fn at(mut self, file: &'static str, line: u32) -> Self {
        self.file = file;
        self.line = line;
        self
    }
}

inventory::collect!(StoryRegistration);

/// Every story linked into the running binary, by type name, then in the order they are declared in.
#[doc(hidden)]
pub fn stories() -> Vec<&'static StoryRegistration> {
    let mut all: Vec<_> = inventory::iter::<StoryRegistration>().collect();
    all.sort_by_key(|entry| ((entry.type_name)(), entry.file, entry.line));
    all
}

/// The values a story shows, by name. Each is a `Value`, so the runner compares them before and after a drive.
#[derive(Default)]
pub struct StoryData {
    #[doc(hidden)]
    pub values: HashMap<(&'static str, TypeId), Box<dyn Shown>>,
    /// What a story said about how a value is edited: a range or a list of choices.
    #[doc(hidden)]
    pub how: HashMap<(&'static str, TypeId), Edit>,
    /// The story's `disabled_text`.
    disabled_text: Option<&'static str>,
}

/// How the Properties editor edits a value, beyond what its type says.
#[doc(hidden)]
pub enum Edit {
    /// A number stays inside this range.
    Range(RangeInclusive<f64>),
    /// The value is one of these, by label.
    Choice(Vec<(&'static str, Box<dyn Shown>)>),
}

/// A story value with its type erased: it copies and compares itself.
#[doc(hidden)]
pub trait Shown {
    fn copy(&self) -> Box<dyn Shown>;
    fn assign(&mut self, from: &dyn Shown);
    fn same(&self, other: &dyn Shown) -> bool;
    fn any(&self) -> &dyn Any;
    fn any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Value> Shown for T {
    fn copy(&self) -> Box<dyn Shown> {
        Box::new(self.clone())
    }

    fn assign(&mut self, from: &dyn Shown) {
        if let Some(value) = from.any().downcast_ref::<T>() {
            self.clone_from(value);
        }
    }

    fn same(&self, other: &dyn Shown) -> bool {
        other.any().downcast_ref::<T>() == Some(self)
    }

    fn any(&self) -> &dyn Any {
        self
    }

    fn any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl StoryData {
    /// The data of `story`'s runs.
    pub(crate) fn of(story: &Story) -> Self {
        StoryData { disabled_text: story.disabled_text, ..StoryData::default() }
    }

    /// A tooltip of `text`, with the story's `disabled_text` as the text it shows while disabled.
    pub fn tooltip(&self, text: &'static str) -> TooltipSpec {
        let spec = TooltipSpec::new(text);
        match self.disabled_text {
            Some(reason) => spec.disabled_text(reason),
            None => spec,
        }
    }

    /// The value named `name` of type `T`, created from `init` on first use. A name used with two types names two
    /// values.
    #[allow(clippy::unreachable, reason = "the slot key holds the type, so the downcast cannot fail")]
    pub fn value<T: Value>(&mut self, name: &'static str, init: T) -> &mut T {
        let slot = self.values.entry((name, TypeId::of::<T>())).or_insert_with(|| Box::new(init));
        match slot.any_mut().downcast_mut::<T>() {
            Some(value) => value,
            None => unreachable!("a slot's value has the type its key names"),
        }
    }

    /// The value named `name` of type `T` when a run has created it; never creates one, so asking does not make a
    /// value the story did not have.
    #[allow(clippy::unreachable, reason = "the slot key holds the type, so the downcast cannot fail")]
    pub fn present<T: Value>(&mut self, name: &'static str) -> Option<&mut T> {
        let slot = self.values.get_mut(&(name, TypeId::of::<T>()))?;
        match slot.any_mut().downcast_mut::<T>() {
            Some(value) => Some(value),
            None => unreachable!("a slot's value has the type its key names"),
        }
    }

    /// Like [`StoryData::value`] for a number the Properties editor keeps inside `range`.
    pub fn ranged<T: Value>(&mut self, name: &'static str, init: T, range: RangeInclusive<f64>) -> &mut T {
        self.how.entry((name, TypeId::of::<T>())).or_insert(Edit::Range(range));
        self.value(name, init)
    }

    /// Like [`StoryData::value`] for a value that is one of `options`, which the Properties editor offers by label.
    pub fn choice<T: Value>(&mut self, name: &'static str, init: T, options: &[(&'static str, T)]) -> &mut T {
        let list = || Edit::Choice(options.iter().map(|(label, value)| (*label, Box::new(value.clone()) as Box<dyn Shown>)).collect());
        self.how.entry((name, TypeId::of::<T>())).or_insert_with(list);
        self.value(name, init)
    }

    /// Puts every value that was there in `before` back to what it was then. A value added since stays.
    pub fn restore(&mut self, before: &StorySnapshot) {
        for (key, was) in &before.values {
            if let Some(now) = self.values.get_mut(key) {
                now.assign(&**was);
            }
        }
    }

    /// The names of the values that differ from `before`; a value added since counts as changed.
    pub fn changed_since(&self, before: &StorySnapshot) -> Vec<&'static str> {
        let mut names: Vec<_> = self
            .values
            .iter()
            .filter(|(key, now)| !before.values.iter().any(|(old, was)| old == *key && now.same(&**was)))
            .map(|((name, _), _)| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// The names of the values that were there in `before` and differ now. A value a story adds on first use is not a
    /// change.
    pub(crate) fn changed_values_since(&self, before: &StorySnapshot) -> Vec<&'static str> {
        let mut names: Vec<_> = self
            .values
            .iter()
            .filter(|(key, now)| before.values.iter().any(|(old, was)| old == *key && !now.same(&**was)))
            .map(|((name, _), _)| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// A copy of every value.
    pub fn snapshot(&self) -> StorySnapshot {
        StorySnapshot { values: self.values.iter().map(|(key, value)| (*key, value.copy())).collect() }
    }
}

type Held = ((&'static str, TypeId), Box<dyn Shown>);

/// The values of a [`StoryData`] at one moment.
pub struct StorySnapshot {
    values: Vec<Held>,
}

/// The kind of an edit event.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// A gesture began.
    Begin,
    /// A gesture proposed a value.
    Update,
    /// A gesture ended with its value.
    Commit,
    /// A gesture was abandoned.
    Cancel,
    /// A one-step change.
    Set,
}

impl EditKind {
    /// The kind of `event`.
    pub fn of<T>(event: &EditEvent<T>) -> Option<EditKind> {
        Some(match event {
            EditEvent::Begin { .. } => EditKind::Begin,
            EditEvent::Update { .. } => EditKind::Update,
            EditEvent::Commit { .. } => EditKind::Commit,
            EditEvent::Cancel { .. } => EditKind::Cancel,
            EditEvent::Set { .. } => EditKind::Set,
            _ => return None,
        })
    }
}

/// The kind of a collection event, with the edit kind of the events that carry one.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind {
    /// The selection changed.
    Selection,
    /// The lead moved.
    LeadMoved,
    /// Enter or a double click on an item.
    Activate,
    /// A context menu was asked for.
    ContextMenu,
    /// Delete was pressed on items.
    Delete,
    /// A tree row opened or closed.
    Open,
    /// An item moved to another place.
    Reorder(EditKind),
    /// The selected items of a free layout moved.
    Move(EditKind),
    /// A drop onto an item.
    Dropped,
    /// The wheel zoomed the collection.
    Zoom,
    /// Files from the OS dropped on the collection.
    FilesDropped,
    /// A rename started, committed or was cancelled.
    Rename(EditKind),
    /// A toggle column's cell flipped.
    Toggle(EditKind),
    /// A column changed.
    Column,
    /// A column header's context menu was asked for.
    HeaderMenu,
    /// An item reported an event.
    Item,
}

impl CollectionKind {
    /// The kind of `event`, `None` for an edit event of a kind the runner does not know.
    pub fn of<K, E>(event: &CollectionEvent<K, E>) -> Option<CollectionKind> {
        Some(match event {
            CollectionEvent::Selection(_) => CollectionKind::Selection,
            CollectionEvent::LeadMoved { .. } => CollectionKind::LeadMoved,
            CollectionEvent::Activate(_) => CollectionKind::Activate,
            CollectionEvent::ContextMenu(_) => CollectionKind::ContextMenu,
            CollectionEvent::Delete(_) => CollectionKind::Delete,
            CollectionEvent::Open(_) => CollectionKind::Open,
            CollectionEvent::Reorder(edit) => CollectionKind::Reorder(EditKind::of(edit)?),
            CollectionEvent::Move(edit) => CollectionKind::Move(EditKind::of(edit)?),
            CollectionEvent::Dropped { .. } => CollectionKind::Dropped,
            CollectionEvent::Zoom { .. } => CollectionKind::Zoom,
            CollectionEvent::FilesDropped { .. } => CollectionKind::FilesDropped,
            CollectionEvent::Rename(edit) => CollectionKind::Rename(EditKind::of(edit)?),
            CollectionEvent::Toggle(edit) => CollectionKind::Toggle(EditKind::of(edit)?),
            CollectionEvent::Column(_) => CollectionKind::Column,
            CollectionEvent::HeaderMenu(_) => CollectionKind::HeaderMenu,
            CollectionEvent::Item(..) => CollectionKind::Item,
        })
    }
}

/// What a story showed, type-erased: the response id, the kinds of its edit events and of its collection events.
#[derive(Clone, Debug, Default)]
pub struct StoryOutput {
    /// The shown component's id.
    pub id: Option<Id>,
    /// The edit events' kinds, in order.
    pub edits: Vec<EditKind>,
    /// The pass reported a click, by pointer or by keyboard activation.
    pub clicked: bool,
    /// The collection events' kinds, in drawing order.
    pub collection: Vec<CollectionKind>,
    /// The positions a collection showed.
    pub visible: Option<Range<usize>>,
    /// The shown value as the component words it ([`crate::contract::Outcome::value_text`]).
    pub value_text: Option<String>,
}

impl StoryOutput {
    pub(crate) fn from_response(response: &Response) -> Self {
        StoryOutput { id: Some(response.id), clicked: super::contract::clicked(response), ..StoryOutput::default() }
    }

    #[doc(hidden)]
    pub fn of_edit<T>(response: &Response, events: &EditEvents<T>) -> Self {
        StoryOutput { edits: events.iter().filter_map(EditKind::of).collect(), ..Self::from_response(response) }
    }

    pub(crate) fn of_collection<K, E>(response: &Response, events: &[CollectionEvent<K, E>], visible: &Range<usize>) -> Self {
        StoryOutput {
            collection: events.iter().filter_map(CollectionKind::of).collect(),
            visible: Some(visible.clone()),
            ..Self::from_response(response)
        }
    }
}

impl<O: Outcome> From<O> for StoryOutput {
    fn from(output: O) -> Self {
        output.story_facts()
    }
}

impl<O: Outcome, R> From<Inner<O, R>> for StoryOutput {
    fn from(shown: Inner<O, R>) -> Self {
        shown.output.story_facts()
    }
}

/// Shows `row` as the only item of a collection: the item step gives an item its id and its place in the set, which
/// a stand-alone story of an item type would otherwise lack.
pub fn show_as_only_item<C>(ui: &mut Ui, row: C) -> C::Output
where
    C: crate::contract::Component + crate::contract::Props + crate::contract::Show,
{
    let id = ui.scope_id().with("only item");
    let facts = crate::runtime::env::ItemFacts {
        item: id,
        flags: crate::style::Flags::empty(),
        position: crate::runtime::access::SetPosition::new(0).of(1),
        columns: Vec::new(),
        toggles: Vec::new(),
    };
    let installed =
        crate::runtime::env::Env::install_for(ui, crate::runtime::env::Env::of(ui).derive_local(|scoped| scoped.item_facts = Some(facts)));
    let shown = row.id(id).show(ui);
    crate::runtime::env::Env::restore(ui.ctx(), installed);
    shown
}
