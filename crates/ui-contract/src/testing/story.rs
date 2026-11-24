//! Stories: named ways to show one registered type, and the data and output the runner reads from them.

use super::Fixture;
use crate::contract::{CollectionEvent, Inner, Outcome, TooltipSpec};
use crate::runtime::commands::{CommandRef, CommandStates};
use craft_behavior::{EditEvent, EditEvents, Mixed, PartKey, PartRef, Value};
use craft_tracked::{Revision, Tracked};
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
    /// Sets this story's app-scope command states; runners apply the shown stories' functions in order to one
    /// `CommandStates` before the first frame and hold it as the app's.
    pub commands: Option<fn(&mut CommandStates)>,
    /// The caption a viewer shows over the story; `None` for the title made of its name (`contract::story_title`).
    pub title: Option<&'static str>,
    /// The group a viewer shows the story in, with the other stories of the type that name it, as the cases of one
    /// old gallery story; `None` for a story that is its own group, under its caption.
    pub group: Option<&'static str>,
}

impl Story {
    /// A story that shows with `show`.
    pub const fn new(name: &'static str, show: fn(&mut Ui, &mut StoryData) -> StoryOutput) -> Self {
        Story { name, covers: &[], show, drive: Drive::Auto, disabled_text: None, commands: None, title: None, group: None }
    }

    /// Sets the function that sets the story's app-scope command states.
    pub const fn commands(mut self, add: fn(&mut CommandStates)) -> Self {
        self.commands = Some(add);
        self
    }

    /// The command states of `stories` shown together: each story's command function applied in order to empty
    /// states, so a later story's state for a command replaces an earlier one's. Runners call this before the first frame.
    pub fn command_states<'a>(stories: impl IntoIterator<Item = &'a Story>) -> CommandStates {
        let mut states = CommandStates::new();
        for add in stories.into_iter().filter_map(|story| story.commands) {
            add(&mut states);
        }
        states
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
    #[cfg(feature = "suite")]
    pub fn group_name(&self, type_name: &str) -> String {
        self.group.map_or_else(|| self.caption(type_name), str::to_owned)
    }

    /// The story's caption as a story of the type whose display name is `type_name`: its title, else the one its
    /// name makes ([`crate::contract::story_title`]).
    #[cfg(feature = "suite")]
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
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct StoryRegistration {
    /// The package that declares the story.
    pub crate_name: &'static str,
    /// The crate target that declares it.
    pub crate_target: &'static str,
    /// The registered name of the type the story shows.
    pub type_name: fn() -> &'static str,
    /// The registration of the type shown by this story.
    pub registration: fn() -> crate::contract::Registration,
    /// The story.
    pub story: Story,
    /// The source file of the declaration.
    pub file: &'static str,
    /// The line of the declaration in `file`.
    pub line: u32,
}

impl StoryRegistration {
    /// The registration `#[story]` writes.
    pub const fn new(
        crate_name: &'static str,
        crate_target: &'static str,
        type_name: fn() -> &'static str,
        registration: fn() -> crate::contract::Registration,
        story: Story,
    ) -> Self {
        StoryRegistration { crate_name, crate_target, type_name, registration, story, file: "", line: 0 }
    }

    /// The registration declared at `line` of `file`.
    pub const fn at(mut self, file: &'static str, line: u32) -> Self {
        self.file = file;
        self.line = line;
        self
    }
}

/// The values a story shows, by name. Each is a `Value`, so the runner compares them before and after a drive.
#[derive(Default)]
pub struct StoryData {
    #[doc(hidden)]
    pub values: HashMap<(&'static str, TypeId), Box<dyn Shown>>,
    /// What a story said about how a value is edited: a range or a list of choices.
    #[doc(hidden)]
    pub how: HashMap<(&'static str, TypeId), Edit>,
    /// What the story keeps about what happened in it ([`StoryData::note`]).
    notes: HashMap<(&'static str, TypeId), Box<dyn Any>>,
    /// The story's `disabled_text`.
    disabled_text: Option<&'static str>,
    /// Moves when [`StoryData::settle`] finds the values changed: the revision a memoized view of the story keys on.
    written: Tracked<()>,
    /// The values as the last [`StoryData::settle`] found them.
    settled: Option<StorySnapshot>,
    /// The revision after the last [`StoryData::settle`]: a write through a binding ([`StoryData::bind`]) moved it
    /// since, and settle moves it no further for that write.
    settled_at: Option<Revision>,
}

/// A story value as the Properties editor binds it ([`StoryData::__slot`]).
#[doc(hidden)]
pub struct Slot<'a> {
    pub value: &'a mut dyn Shown,
    pub how: Option<&'a Edit>,
    pub count: &'a std::cell::Cell<u64>,
    pub source: u64,
    pub offset: usize,
}

impl Slot<'_> {
    /// A binding of the value as `T`, as [`StoryData::bind`] makes it; `None` when the value is not a `T`.
    pub fn bind<T: Value>(&mut self) -> Option<crate::contract::Bind<'_, T>> {
        let value = self.value.any_mut().downcast_mut::<T>()?;
        Some(crate::contract::Bind::__tracked(value, self.count, self.source, self.offset))
    }
}

/// Where a story value lies ([`StoryData::value_at`]): its data's source, its name and its place in memory, so a binding
/// of the value or of a field of it names that field by source and the field's offset in the value, as `bind_tracked!`
/// names a field of a `Tracked`. A write through such a binding moves the story's revision.
#[derive(Clone, Copy)]
pub struct ValueAt<'a> {
    count: &'a std::cell::Cell<u64>,
    source: u64,
    /// The value's key in the data's source (`slot_offset`): a part of it is keyed by this plus its offset.
    key: usize,
    base: usize,
    size: usize,
}

impl<'a> ValueAt<'a> {
    /// Where `value`, the story value named `name`, lies in the data whose writes `count` counts under `source`.
    fn of<T: 'static>(count: &'a std::cell::Cell<u64>, source: u64, name: &'static str, value: &T) -> Self {
        ValueAt { count, source, key: slot_offset(name, TypeId::of::<T>()), base: std::ptr::from_ref::<T>(value).addr(), size: size_of::<T>() }
    }

    /// A binding of `part`, the value itself or a field inside it.
    ///
    /// # Panics
    ///
    /// When `part` does not lie inside the value (a field behind a pointer, or another value).
    pub fn bind<'b, F: Value>(self, part: &'b mut F) -> crate::contract::Bind<'b, F>
    where
        Self: 'b,
    {
        let offset = self.offset_of(std::ptr::from_ref::<F>(part).addr(), size_of::<F>());
        crate::contract::Bind::__tracked(part, self.count, self.source, offset)
    }

    /// Like [`ValueAt::bind`] for a field that may be mixed: an edit makes it concrete.
    ///
    /// # Panics
    ///
    /// When `part` does not lie inside the value.
    pub fn bind_mixed<'b, F: Value>(self, part: &'b mut Mixed<F>) -> crate::contract::Bind<'b, F>
    where
        Self: 'b,
    {
        let offset = self.offset_of(std::ptr::from_ref::<Mixed<F>>(part).addr(), size_of::<Mixed<F>>());
        crate::contract::Bind::__tracked_mixed(part, self.count, self.source, offset)
    }

    /// The key of the part at address `at`, `size` bytes long: the value's key plus the part's offset in it.
    fn offset_of(self, at: usize, size: usize) -> usize {
        let offset = at.wrapping_sub(self.base);
        assert!(offset <= self.size && size <= self.size - offset, "a story value binding names a part inside the value");
        self.key.wrapping_add(offset)
    }
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

    /// The note named `name` of type `T`, created from `init` on first use: what the story keeps about what happened in
    /// it for its readout ([`StoryOutput::readout`]), such as the last event or a click count. A note is not a value:
    /// the runner's drives and a viewer's Reset compare the values only, so a cancelled drag that the readout reports
    /// leaves the values as they were.
    #[allow(clippy::unreachable, reason = "the slot key holds the type, so the downcast cannot fail")]
    pub fn note<T: Any>(&mut self, name: &'static str, init: T) -> &mut T {
        let slot = self.notes.entry((name, TypeId::of::<T>())).or_insert_with(|| Box::new(init));
        match slot.downcast_mut::<T>() {
            Some(note) => note,
            None => unreachable!("a slot's note has the type its key names"),
        }
    }

    /// A binding of the value named `name` of type `T`, created from `init` on first use. Every view that binds a value
    /// shows and edits one value: the story's view, and the Properties editor in the same pass. The binding names
    /// the value by this data's source, so a reader that replays sees an edit made later in its pass, and an edit
    /// moves the [`revision`](Self::revision) at the write.
    #[allow(clippy::unreachable, reason = "the slot key holds the type, so the downcast cannot fail")]
    pub fn bind<T: Value>(&mut self, name: &'static str, init: T) -> crate::contract::Bind<'_, T> {
        let StoryData { values, written, .. } = self;
        let slot = values.entry((name, TypeId::of::<T>())).or_insert_with(|| Box::new(init));
        let (_, count, source) = written.__split_for_bind();
        match slot.any_mut().downcast_mut::<T>() {
            Some(value) => crate::contract::Bind::__tracked(value, count, source, slot_offset(name, TypeId::of::<T>())),
            None => unreachable!("a slot's value has the type its key names"),
        }
    }

    /// Like [`StoryData::bind`] for a value that may be mixed, as a multi-selection field shows it; an edit makes it
    /// concrete.
    #[allow(clippy::unreachable, reason = "the slot key holds the type, so the downcast cannot fail")]
    pub fn bind_mixed<T: Value>(&mut self, name: &'static str, init: Mixed<T>) -> crate::contract::Bind<'_, T> {
        let StoryData { values, written, .. } = self;
        let slot = values.entry((name, TypeId::of::<Mixed<T>>())).or_insert_with(|| Box::new(init));
        let (_, count, source) = written.__split_for_bind();
        match slot.any_mut().downcast_mut::<Mixed<T>>() {
            Some(value) => crate::contract::Bind::__tracked_mixed(value, count, source, slot_offset(name, TypeId::of::<Mixed<T>>())),
            None => unreachable!("a slot's value has the type its key names"),
        }
    }

    /// Like [`StoryData::bind`] for a number the Properties editor keeps inside `range`.
    pub fn bind_ranged<T: Value>(&mut self, name: &'static str, init: T, range: RangeInclusive<f64>) -> crate::contract::Bind<'_, T> {
        self.how.entry((name, TypeId::of::<T>())).or_insert(Edit::Range(range));
        self.bind(name, init)
    }

    /// Called by the Properties editor only: the value under `key` with what binds it as [`StoryData::bind`] does (the
    /// write count, the source and the value's place in the data).
    #[doc(hidden)]
    pub fn __slot(&mut self, key: (&'static str, TypeId)) -> Option<Slot<'_>> {
        let StoryData { values, how, written, .. } = self;
        let value = values.get_mut(&key)?.as_mut();
        let (_, count, source) = written.__split_for_bind();
        Some(Slot { value, how: how.get(&key), count, source, offset: slot_offset(key.0, key.1) })
    }

    /// Called by the Properties editor only: it wrote the value under `key` outside a binding (an assignment after a
    /// picked choice or an edited number). The revision moves, and a view that read the value earlier in the pass
    /// runs again.
    #[doc(hidden)]
    pub fn __wrote(&mut self, ctx: &egui::Context, key: (&'static str, TypeId)) {
        *self.written = ();
        let size = self.values.get(&key).map_or(0, |value| size_of_val(&**value));
        let read = crate::runtime::reads::BindKey::Tracked(self.written.source(), slot_offset(key.0, key.1), size, key.1);
        crate::runtime::inspect::edited(ctx, read, Id::NULL);
    }

    /// The value named `name` of type `T`, created from `init` on first use, with where it lies ([`ValueAt`]): a
    /// binding of the value or of a part of it (`at.bind(&mut value.0)`) names that part by this data's source, as
    /// [`StoryData::bind`] does, while the story keeps the value to read and pass on.
    #[allow(clippy::unreachable, reason = "the slot key holds the type, so the downcast cannot fail")]
    pub fn value_at<T: Value>(&mut self, name: &'static str, init: T) -> (&mut T, ValueAt<'_>) {
        let StoryData { values, written, .. } = self;
        let slot = values.entry((name, TypeId::of::<T>())).or_insert_with(|| Box::new(init));
        let (_, count, source) = written.__split_for_bind();
        match slot.any_mut().downcast_mut::<T>() {
            Some(value) => {
                let at = ValueAt::of(count, source, name, value);
                (value, at)
            }
            None => unreachable!("a slot's value has the type its key names"),
        }
    }

    /// Two values by name, each created from its init on first use and returned with where it lies, as
    /// [`StoryData::value_at`] returns one: for a story that binds a value it must keep by its own name beside another,
    /// such as a dialog's `open`, which the gallery closes and opens by that name, beside the value the dialog edits.
    ///
    /// # Panics
    ///
    /// When both name the same value (the same name and type).
    #[allow(clippy::unreachable, reason = "both values exist and each slot key holds its type, so neither lookup can fail")]
    pub fn pair_at<A: Value, B: Value>(&mut self, a: (&'static str, A), b: (&'static str, B)) -> ((&mut A, ValueAt<'_>), (&mut B, ValueAt<'_>)) {
        let StoryData { values, written, .. } = self;
        let (key_a, key_b) = ((a.0, TypeId::of::<A>()), (b.0, TypeId::of::<B>()));
        values.entry(key_a).or_insert_with(|| Box::new(a.1));
        values.entry(key_b).or_insert_with(|| Box::new(b.1));
        let (_, count, source) = written.__split_for_bind();
        let [Some(slot_a), Some(slot_b)] = values.get_disjoint_mut([&key_a, &key_b]) else { unreachable!("both values were created above") };
        let (Some(value_a), Some(value_b)) = (slot_a.any_mut().downcast_mut::<A>(), slot_b.any_mut().downcast_mut::<B>()) else {
            unreachable!("a slot's value has the type its key names")
        };
        let (at_a, at_b) = (ValueAt::of(count, source, a.0, value_a), ValueAt::of(count, source, b.0, value_b));
        ((value_a, at_a), (value_b, at_b))
    }

    /// Like [`StoryData::value_at`] for a number the Properties editor keeps inside `range`.
    pub fn ranged_at<T: Value>(&mut self, name: &'static str, init: T, range: RangeInclusive<f64>) -> (&mut T, ValueAt<'_>) {
        self.how.entry((name, TypeId::of::<T>())).or_insert(Edit::Range(range));
        self.value_at(name, init)
    }

    /// Like [`StoryData::value_at`] for a value that is one of `options`, which the Properties editor offers by label.
    pub fn choice_at<T: Value>(&mut self, name: &'static str, init: T, options: &[(&'static str, T)]) -> (&mut T, ValueAt<'_>) {
        self.choice(name, init.clone(), options);
        self.value_at(name, init)
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
        self.settle();
    }

    /// Moves the [`revision`](Self::revision) when a value was added or changed since the last call. The accessors hand
    /// out `&mut T`, so a write leaves no trace of its own: comparing the values after code that may have written them
    /// (a story's run, an editor's pass) is the only signal that one happened. Notes are not compared: only a run
    /// writes them, and a run is what a reader of the revision would repeat.
    pub fn settle(&mut self) {
        let same = self.settled.as_ref().is_some_and(|last| {
            last.values.len() == self.values.len() && last.values.iter().all(|(key, was)| self.values.get(key).is_some_and(|now| now.same(&**was)))
        });
        if !same {
            if self.settled_at.is_none_or(|at| at == self.written.revision()) {
                *self.written = ();
            }
            self.settled = Some(self.snapshot());
        }
        self.settled_at = Some(self.written.revision());
    }

    /// The revision of the values: unique to this data, and moved by every [`settle`](Self::settle) that finds a
    /// change, so equal revisions mean the values a memoized view showed.
    pub fn revision(&self) -> Revision {
        self.written.revision()
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

/// A story's data as props, by its [`revision`](StoryData::revision). A view that shows the story settles the data at
/// the end of its run, so a run that changed a value runs once more, from the changed values, and then replays.
impl crate::contract::MemoProps for &mut StoryData {
    fn key(&self, key: &mut crate::contract::MemoKey) {
        crate::contract::MemoProps::key(&self.revision(), key);
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
    /// A click picked the sole selected item again.
    Picked,
    /// The lead moved.
    LeadMoved,
    /// The pointer rested on an item.
    Rested,
    /// Enter or a double click on an item.
    Activate,
    /// Enter on a disabled lead.
    Refused,
    /// A double click on the empty space.
    EmptyActivated,
    /// A secondary click on the empty space.
    EmptyContextMenu,
    /// A click on the line between two rows.
    EdgePressed,
    /// Space previewed the lead.
    Preview,
    /// A context menu was asked for.
    ContextMenu,
    /// A Cmd/Ctrl click on an item of a `command_click` collection.
    CommandClick,
    /// A middle click on an item.
    MiddleClick,
    /// Delete was pressed on items.
    Delete,
    /// A tree row opened or closed.
    Open,
    /// An item moved to another place.
    Reorder(EditKind),
    /// The selected items of a free layout moved.
    Move(EditKind),
    /// A drop a drop kind took.
    Dropped,
    /// The wheel zoomed the collection.
    Zoom,
    /// A rename started, committed or was cancelled.
    Rename(EditKind),
    /// A toggle column's cell flipped.
    Toggle(EditKind),
    /// A column changed.
    Column,
    /// An item reported an event.
    Item,
}

impl CollectionKind {
    /// The kind of `event`, `None` for an edit event of a kind the runner does not know.
    pub fn of<K, E>(event: &CollectionEvent<K, E>) -> Option<CollectionKind> {
        Some(match event {
            CollectionEvent::Selection { .. } => CollectionKind::Selection,
            CollectionEvent::Picked { .. } => CollectionKind::Picked,
            CollectionEvent::LeadMoved { .. } => CollectionKind::LeadMoved,
            CollectionEvent::Activate(_) => CollectionKind::Activate,
            CollectionEvent::Refused(_) => CollectionKind::Refused,
            CollectionEvent::Rested(_) => CollectionKind::Rested,
            CollectionEvent::EmptyActivated => CollectionKind::EmptyActivated,
            CollectionEvent::EmptyContextMenu { .. } => CollectionKind::EmptyContextMenu,
            CollectionEvent::EdgePressed { .. } => CollectionKind::EdgePressed,
            CollectionEvent::Preview(_) => CollectionKind::Preview,
            CollectionEvent::ContextMenu(_) => CollectionKind::ContextMenu,
            CollectionEvent::CommandClick { .. } => CollectionKind::CommandClick,
            CollectionEvent::MiddleClick(_) => CollectionKind::MiddleClick,
            CollectionEvent::Delete(_) => CollectionKind::Delete,
            CollectionEvent::Open(_) => CollectionKind::Open,
            CollectionEvent::Reorder(edit) => CollectionKind::Reorder(EditKind::of(edit)?),
            CollectionEvent::Move(edit) => CollectionKind::Move(EditKind::of(edit)?),
            CollectionEvent::Dropped { .. } => CollectionKind::Dropped,
            CollectionEvent::Zoom { .. } => CollectionKind::Zoom,
            CollectionEvent::Rename(edit) => CollectionKind::Rename(EditKind::of(edit)?),
            CollectionEvent::Toggle(edit) => CollectionKind::Toggle(EditKind::of(edit)?),
            CollectionEvent::Column(_) => CollectionKind::Column,
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
    /// The commands the story's pass fired, in order; the fixture that shows the story fills it in. A component bound
    /// to a command reports its activation as the command it fires, not as a click.
    pub fired: Vec<CommandRef>,
    /// The collection events' kinds, in drawing order.
    pub collection: Vec<CollectionKind>,
    /// The positions a collection showed.
    pub visible: Option<Range<usize>>,
    /// The shown value as the component words it ([`crate::contract::Outcome::value_text`]).
    pub value_text: Option<String>,
    /// The story's readout: a line in the story's own words about what it shows or what last happened in it ("Last
    /// event: …", "Clicked 2 times"), which a viewer shows under the story.
    pub readout: Option<String>,
}

impl StoryOutput {
    /// The same output with `text` as its readout.
    #[must_use]
    pub fn readout(mut self, text: impl Into<String>) -> Self {
        self.readout = Some(text.into());
        self
    }

    /// Whether the pass activated what the story shows: it reported a click, or it fired a command, as a component
    /// bound to one does in place of the click.
    pub fn activated(&self) -> bool {
        self.clicked || !self.fired.is_empty()
    }

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
    crate::contract::show_item_in_flow(ui, row, crate::runtime::access::SetPosition::new(0).of(1))
}

/// Shows `row` as the only item of a collection across `span`, as a collection shows its rows across its columns: a
/// row with column cells spans the cells it is given, where in the flow it would take its own width. The row is as
/// high as it measures at the span's width, as a collection measures it.
pub fn show_as_only_item_across<C>(ui: &mut Ui, mut row: C, span: egui::Rangef) -> C::Output
where
    C: crate::contract::Component + crate::contract::Props + crate::contract::Show,
{
    let at = crate::runtime::access::SetPosition::new(0).of(1);
    let height = crate::contract::measure_item(ui, &mut row, at, craft_behavior::Proposal::Width(span.span())).y;
    let top = ui.cursor().min.y;
    let rect = egui::Rect::from_x_y_ranges(span, top..=top + height);
    let shown = crate::contract::show_item(ui, rect, row, at);
    ui.advance_cursor_after_rect(rect);
    shown
}

/// A story value's place in its data, by its name and type, as the data keeps it: with the data's source it names the
/// value in every pass (`BindKey::Tracked`), and a part of it by this plus the part's offset.
fn slot_offset(name: &str, ty: TypeId) -> usize {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (name, ty).hash(&mut hasher);
    hasher.finish() as usize
}
