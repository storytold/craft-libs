//! Values and edits (chapter 10): the binding, the output of a value component and the types that carry a value
//! between the app and the pipeline.

use crate::contract::{MemoKey, MemoProps};
use crate::runtime::env::Message;
use crate::runtime::memory::{MemoryStage, Savable, SaveKey, StageAccess};
use crate::runtime::reads::{self, BindKey};
use craft_behavior::{Applied, CanvasView, EditEvent, EditEvents, EditPolicy, Mixed, Value};
use egui::Id;
use egui::{Response, Ui};
use std::any::TypeId;
use std::cell::Cell;
use std::sync::Arc;

/// What a check says about a value, with the message type of craft-ui.
pub type Validity<T> = craft_behavior::Validity<T, Message>;

/// The value type of a component with no value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoValue;

/// What `show` returns: the component's response and the type of value it edits.
pub trait Outcome {
    /// The value type; `NoValue` for a component with none.
    type Value: Value;

    /// The component's response.
    fn response(&self) -> &Response;

    /// The output of a layer-like container that shows nothing this pass (closed, past its exit fade): `response` is
    /// its id's inert response. A container whose output gives one runs no pipeline while closed; `None` runs it.
    fn unshown(response: Response) -> Option<Self>
    where
        Self: Sized,
    {
        let _ = response;
        None
    }

    /// The shown value as the component words it to assistive technology; `None` for a component that gives none.
    fn value_text(&self) -> Option<&str> {
        None
    }

    /// The facts a story reports about this output: its id, click, edit kinds, collection kinds and visible range.
    /// Outputs that hold an `EditResponse` or a `CollectionResponse` forward to it.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput::from_response(self.response())
    }
}

impl Outcome for Response {
    type Value = NoValue;

    fn response(&self) -> &Response {
        self
    }
}

/// Whether `response` reports a click, by pointer or by keyboard activation.
pub fn clicked(response: &Response) -> bool {
    crate::runtime::egui::response::clicked(response)
}

/// The `Response` of the component `id` in the running pass: its root's egui widget, or the response its arena gives a
/// member of one (a component in a collection's rows or in an arena body, which registers no egui widget). A
/// memoized view's replay shows its components again, so a host reads the response of a component a replayed view
/// showed. `None` for an id no component showed in this pass or the last.
pub fn response_of(ui: &Ui, id: Id) -> Option<Response> {
    ui.ctx().read_response(id).or_else(|| crate::primitives::sensing::member_response(ui, id))
}

/// Whether a pointer press went down on `response`'s component and is still held.
pub fn pressed(response: &Response) -> bool {
    crate::runtime::egui::response::is_pressed(response)
}

/// The value type of `C`'s output.
pub type ValueOf<C> = <<C as super::Component>::Output as Outcome>::Value;

/// `held` when it equals `value`, else a new copy of it.
pub(crate) fn reuse_arc<V: Value>(held: &Option<Arc<V>>, value: &V) -> Arc<V> {
    match held {
        Some(held) if **held == *value => held.clone(),
        _ => Arc::new(value.clone()),
    }
}

/// The shown copy a step 4 comparison keeps.
type Held<T> = Option<Arc<T>>;

/// What `Bind::saved` captures while `T: Savable` is known, so the generic pipeline reaches the saved tier without
/// the bound.
struct SaveSpec<T> {
    key: SaveKey,
    /// Reads the saved value, reusing the held copy when it is equal.
    load: fn(&mut MemoryStage, &SaveKey, &Held<T>) -> Option<Arc<T>>,
    store: fn(&mut MemoryStage, &SaveKey, T),
}

impl<T> Clone for SaveSpec<T> {
    fn clone(&self) -> Self {
        SaveSpec { key: self.key.clone(), load: self.load, store: self.store }
    }
}

/// A binding's check: its verdict on a value, and the key of its inputs and its `fn`, which only a memoized view that
/// holds the binding takes.
trait Checker<T> {
    fn validity(&self, value: &T) -> Validity<T>;
    fn key(&self, key: &mut MemoKey);
}

/// A check `fn` and the inputs it reads (`Bind::check_with`).
struct WithInputs<I, T: 'static> {
    inputs: I,
    check: fn(&I, &T) -> Validity<T>,
}

impl<I: MemoProps, T> Checker<T> for WithInputs<I, T> {
    fn validity(&self, value: &T) -> Validity<T> {
        (self.check)(&self.inputs, value)
    }

    fn key(&self, key: &mut MemoKey) {
        self.inputs.key(key);
        key.write(&(self.check as usize).to_le_bytes());
    }
}

/// A borrowed check, for a reborrowed binding.
impl<T, C: Checker<T> + ?Sized> Checker<T> for &C {
    fn validity(&self, value: &T) -> Validity<T> {
        (**self).validity(value)
    }

    fn key(&self, key: &mut MemoKey) {
        (**self).key(key);
    }
}

type Check<'a, T> = dyn Checker<T> + 'a;

/// Where a binding's value lives.
enum Source<'a, T> {
    /// Value in, event out.
    Given(Mixed<T>),
    /// Write-through.
    Mut(&'a mut T),
    /// Write-through for a multi-selection field.
    MutMixed(&'a mut Mixed<T>),
    /// Write-through to a field of a `Tracked` (`bind_tracked!`), whose write count moves only when an edit changes
    /// the field.
    Tracked { value: &'a mut T, count: &'a Cell<u64>, at: TrackedAt },
    /// `Tracked` for a field that may be mixed; an edit makes it concrete.
    TrackedMixed { value: &'a mut Mixed<T>, count: &'a Cell<u64>, at: TrackedAt },
    /// No caller value: session memory under the component id, or the saved tier.
    Kept,
}

/// Where a `bind_tracked!` binding's field is: the source id of the `Tracked` that holds it and the field's offset in
/// bytes from the start of that value. It names the field in every pass, wherever the value lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TrackedAt {
    source: u64,
    offset: usize,
}

/// The binding of a value component: a value the app owns, the check on it and the value a reset restores. The same
/// binding produces the same events whichever source it has.
pub struct Bind<'a, T> {
    source: Source<'a, T>,
    policy: EditPolicy,
    /// The check and the inputs it reads (`check_with`).
    check: Option<Box<Check<'a, T>>>,
    save: Option<SaveSpec<T>>,
    reset: Option<T>,
}

impl<'a, T: Value> Bind<'a, T> {
    /// A value in; every edit comes out as an event.
    pub fn given(v: impl Into<Mixed<T>>) -> Self {
        Self::from_source(Source::Given(v.into()))
    }

    /// Edits write through to `v`.
    pub fn bound(v: &'a mut T) -> Self {
        Self::from_source(Source::Mut(v))
    }

    /// Edits write through to a field that may be mixed; an edit makes it concrete.
    pub fn bound_mixed(v: &'a mut Mixed<T>) -> Self {
        Self::from_source(Source::MutMixed(v))
    }

    /// Called by [`bind_tracked!`](crate::bind_tracked) only: edits write through to `value`, a field `offset` bytes
    /// into the value of the `Tracked` with source id `source`, and an edit that changes it moves `count`, that
    /// `Tracked`'s write count.
    #[doc(hidden)]
    pub fn __tracked(value: &'a mut T, count: &'a Cell<u64>, source: u64, offset: usize) -> Self {
        Self::from_source(Source::Tracked { value, count, at: TrackedAt { source, offset } })
    }

    /// `Bind::__tracked` for a field that may be mixed: an edit makes it concrete.
    #[doc(hidden)]
    pub fn __tracked_mixed(value: &'a mut Mixed<T>, count: &'a Cell<u64>, source: u64, offset: usize) -> Self {
        Self::from_source(Source::TrackedMixed { value, count, at: TrackedAt { source, offset } })
    }

    /// No caller value: the component keeps it in session memory under its id. Until something is stored, it reads
    /// as the [`reset`](Self::reset) value when one is given, else as nothing.
    pub fn kept() -> Self {
        Self::from_source(Source::Kept)
    }

    fn from_source(source: Source<'a, T>) -> Self {
        Bind { source, policy: EditPolicy::default(), check: None, save: None, reset: None }
    }

    /// A kept value that is also saved across runs under `key`. The `Savable` bound sits here; the pipeline reaches
    /// the saved tier through the captured load and store functions.
    pub fn saved(mut self, key: SaveKey) -> Self
    where
        T: Savable,
    {
        self.source = Source::Kept;
        self.save = Some(SaveSpec {
            key,
            load: |stage, key, held| stage.saved::<Option<T>>(key).as_ref().map(|value| reuse_arc(held, value)),
            store: |stage, key, value| *stage.saved::<Option<T>>(key) = Some(value),
        });
        self
    }

    /// When a write-through source is written.
    pub fn policy(mut self, policy: EditPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The check on a value: `Fix` replaces it; `Warn`, `Refuse` and `Error` set the component's state, and `Refuse`
    /// also refuses a commit. `check` is a `fn`, which captures nothing; a check that reads other values takes them as
    /// inputs ([`check_with`](Self::check_with)).
    pub fn check(self, check: fn(&T) -> Validity<T>) -> Self
    where
        T: 'a,
    {
        self.check_with(CheckFn(check), |check, value| (check.0)(value))
    }

    /// The check `check` on a value, reading `inputs` beside it (a limit, the names already taken). The inputs are
    /// props ([`MemoProps`]) and `check` is a `fn`, which captures nothing, so a memoized view that holds the binding
    /// sees every input of its check.
    pub fn check_with<I: MemoProps + 'a>(mut self, inputs: I, check: fn(&I, &T) -> Validity<T>) -> Self
    where
        T: 'a,
    {
        self.check = Some(Box::new(WithInputs { inputs, check }));
        self
    }

    /// The value a reset gesture restores, and the value a kept binding reads while nothing is stored.
    pub fn reset(mut self, default: T) -> Self {
        self.reset = Some(default);
        self
    }

    /// A binding with the same source, check, policy and reset that borrows this one, so a composite can hand its
    /// own binding to a child component for the length of one pass. A given value is copied; a write-through
    /// source is the same place.
    pub fn reborrowed(&mut self) -> Bind<'_, T> {
        let source = match &mut self.source {
            Source::Given(value) => Source::Given(value.clone()),
            Source::Mut(value) => Source::Mut(&mut **value),
            Source::MutMixed(value) => Source::MutMixed(&mut **value),
            Source::Tracked { value, count, at } => Source::Tracked { value: &mut **value, count, at: *at },
            Source::TrackedMixed { value, count, at } => Source::TrackedMixed { value: &mut **value, count, at: *at },
            Source::Kept => Source::Kept,
        };
        let check = self.check.as_deref().map(|check| Box::new(check) as Box<Check<'_, T>>);
        Bind { source, policy: self.policy, check, save: self.save.clone(), reset: self.reset.clone() }
    }

    /// The value a reset restores.
    #[doc(hidden)]
    pub fn reset_value(&self) -> Option<&T> {
        self.reset.as_ref()
    }

    /// The one value the binding holds before this pass's edits; `None` for a mixed or kept binding.
    pub fn value(&self) -> Option<&T> {
        match &self.source {
            Source::Given(Mixed::Same(value)) | Source::MutMixed(Mixed::Same(value)) => Some(value),
            Source::Mut(value) => Some(&**value),
            Source::Tracked { value, .. } => Some(&**value),
            _ => None,
        }
    }

    /// The binding holds one value equal to `shown` this pass: what a component reads in `Component::keys`, which has
    /// no context to read the value from.
    #[doc(hidden)]
    pub fn holds(&self, shown: &T) -> bool
    where
        T: PartialEq,
    {
        match &self.source {
            Source::Given(Mixed::Same(value)) => value == shown,
            Source::Mut(value) => **value == *shown,
            Source::MutMixed(Mixed::Same(value)) | Source::TrackedMixed { value: Mixed::Same(value), .. } => value == shown,
            Source::Tracked { value, .. } => **value == *shown,
            _ => false,
        }
    }

    /// The binding as the pipeline reads and writes it.
    pub fn reborrow(&mut self) -> BindMut<'_, T> {
        let source = match &mut self.source {
            Source::Given(value) => SourceMut::Given(value),
            Source::Mut(value) => SourceMut::Mut(&mut **value),
            Source::MutMixed(value) => SourceMut::MutMixed(&mut **value),
            Source::Tracked { value, count, at } => SourceMut::Tracked { value: &mut **value, count, at: *at },
            Source::TrackedMixed { value, count, at } => SourceMut::TrackedMixed { value: &mut **value, count, at: *at },
            Source::Kept => SourceMut::Kept,
        };
        BindMut { source, policy: self.policy, check: self.check.as_deref(), save: self.save.as_ref(), reset: self.reset.as_ref() }
    }
}

/// A binding's check as props (`BindMut::check_of`): by the key of its inputs and its `fn`.
pub struct BindCheck<'c, T> {
    check: Option<&'c Check<'c, T>>,
}

impl<T> BindCheck<'_, T> {
    /// The check's verdict on `value`.
    pub fn validity(&self, value: &T) -> Validity<T> {
        self.check.map_or(Validity::Ok, |check| check.validity(value))
    }
}

impl<T> MemoProps for BindCheck<'_, T> {
    fn key(&self, key: &mut MemoKey) {
        key_check(self.check, key);
    }
}

/// Feeds a binding's check, or its absence.
fn key_check<T>(check: Option<&Check<'_, T>>, key: &mut MemoKey) {
    match check {
        None => key.write_u32(0),
        Some(check) => {
            key.write_u32(1);
            check.key(key);
        }
    }
}

/// A check with no inputs, as the inputs of the one mechanism (`Bind::check_with`): keyed by the `fn`'s address.
struct CheckFn<T: 'static>(fn(&T) -> Validity<T>);

impl<T> MemoProps for CheckFn<T> {
    fn key(&self, key: &mut MemoKey) {
        key.write(&(self.0 as usize).to_le_bytes());
    }
}

/// A binding in a memoized view's props: by what the view reads through it. A given value keys by value; a
/// write-through source by its value and its address, the place the framework knows its reads by (a large value is
/// bound through `bind_tracked!` or held as `&Tracked<T>` instead: its value is hashed for each key); a `Tracked` field
/// (`bind_tracked!`) by its `Tracked`'s source, the field's place in the value and the write count; a kept value by the writes the context counts
/// outside a component's edit: of its saved value (`set_saved_value`, `forget_saved`), or of session memory
/// (`forget`). The framework tracks the component's own reads and edits. The policy, the reset value, the save key and
/// the check's inputs and `fn` (`check_with`) feed too.
impl<T: Value + MemoProps> MemoProps for Bind<'_, T> {
    fn key(&self, key: &mut MemoKey) {
        let Bind { source, policy, check, save, reset } = self;
        let address = |value: &T| std::ptr::from_ref(value).addr();
        match source {
            Source::Given(value) => {
                key.write_u32(0);
                Mixed::<T>::key(value, key);
            }
            Source::Mut(value) => {
                key.write_u32(1);
                T::key(value, key);
                address(value).key(key);
            }
            Source::MutMixed(value) => {
                key.write_u32(2);
                Mixed::<T>::key(value, key);
                std::ptr::from_ref::<Mixed<T>>(value).addr().key(key);
            }
            Source::Tracked { count, at, .. } | Source::TrackedMixed { count, at, .. } => {
                key.write_u32(3);
                at.source.key(key);
                at.offset.key(key);
                count.get().key(key);
            }
            Source::Kept => {
                key.write_u32(4);
                let writes = key.context().map(|ctx| crate::runtime::memory::writes(ctx, save.as_ref().map(|save| &save.key)));
                match (writes, save) {
                    (Some((saved, _)), Some(_)) => saved.key(key),
                    (Some((_, session)), None) => session.key(key),
                    (None, _) => {}
                }
            }
        }
        key.feed_hash(policy);
        reset.key(key);
        save.as_ref().map(|save| save.key.as_str()).key(key);
        key_check(check.as_deref(), key);
    }
}

impl<'a> From<CanvasView> for Bind<'a, CanvasView> {
    fn from(view: CanvasView) -> Self {
        Bind::given(view)
    }
}

enum SourceMut<'a, T> {
    Given(&'a Mixed<T>),
    Mut(&'a mut T),
    MutMixed(&'a mut Mixed<T>),
    Tracked { value: &'a mut T, count: &'a Cell<u64>, at: TrackedAt },
    TrackedMixed { value: &'a mut Mixed<T>, count: &'a Cell<u64>, at: TrackedAt },
    Kept,
}

/// A reborrow of a [`Bind`] the pipeline reads at steps 4, 11, 15 and 17. `Component::bind` returns it, and returns the
/// same binding on every call.
pub struct BindMut<'a, T> {
    source: SourceMut<'a, T>,
    policy: EditPolicy,
    check: Option<&'a Check<'a, T>>,
    save: Option<&'a SaveSpec<T>>,
    reset: Option<&'a T>,
}

impl<T: Value> BindMut<'_, T> {
    /// The binding's value: `Mixed` for a multi-selection field, none for a kept binding.
    #[doc(hidden)]
    pub fn get(&self) -> Option<Mixed<T>> {
        match &self.source {
            SourceMut::Given(value) => Some((*value).clone()),
            SourceMut::Mut(value) => Some(Mixed::Same((**value).clone())),
            SourceMut::MutMixed(value) => Some((**value).clone()),
            SourceMut::Tracked { value, .. } => Some(Mixed::Same((**value).clone())),
            SourceMut::TrackedMixed { value, .. } => Some((**value).clone()),
            SourceMut::Kept => None,
        }
    }

    /// The identity of the binding's value in this pass, for the read record: the bound place, or where the kept value
    /// lives (`id` is the component it is kept under). A given value has none.
    pub(crate) fn key(&self, id: Id) -> Option<BindKey> {
        match &self.source {
            SourceMut::Given(_) => None,
            SourceMut::Mut(value) => Some(BindKey::Place(std::ptr::from_ref::<T>(value) as usize, TypeId::of::<T>())),
            SourceMut::MutMixed(value) => Some(BindKey::Place(std::ptr::from_ref::<Mixed<T>>(value) as usize, TypeId::of::<Mixed<T>>())),
            SourceMut::Tracked { at, .. } => Some(BindKey::Tracked(at.source, at.offset, size_of::<T>(), TypeId::of::<T>())),
            SourceMut::TrackedMixed { at, .. } => Some(BindKey::Tracked(at.source, at.offset, size_of::<Mixed<T>>(), TypeId::of::<Mixed<T>>())),
            SourceMut::Kept => Some(match self.save {
                Some(save) => BindKey::Saved(Id::NULL.with(&save.key)),
                None => BindKey::Kept(id),
            }),
        }
    }

    pub(crate) fn is_kept(&self) -> bool {
        matches!(self.source, SourceMut::Kept)
    }

    pub(crate) fn is_given(&self) -> bool {
        matches!(self.source, SourceMut::Given(_))
    }

    /// The binding holds one value equal to `shown`: a compare that clones nothing, so step 4 copies the value only
    /// when it changed.
    pub(crate) fn shown_matches(&self, shown: &T) -> bool {
        match &self.source {
            SourceMut::Given(Mixed::Same(value))
            | SourceMut::MutMixed(Mixed::Same(value))
            | SourceMut::TrackedMixed { value: Mixed::Same(value), .. } => value == shown,
            SourceMut::Mut(value) => **value == *shown,
            SourceMut::Tracked { value, .. } => **value == *shown,
            _ => false,
        }
    }

    /// The check's verdict on `value`.
    #[doc(hidden)]
    pub fn validity(&self, value: &T) -> Validity<T> {
        self.check.map_or(Validity::Ok, |check| check.validity(value))
    }

    /// The binding's check, as the inputs of a child's check (`Bind::check_with`): a composite hands its own check to
    /// the children that edit its value.
    pub fn check_of(&self) -> BindCheck<'_, T> {
        BindCheck { check: self.check }
    }

    /// The value of a kept binding: the saved tier when it has a `SaveKey`, else session memory under `id`, else the
    /// reset value while nothing is stored. `held` is the shown copy, reused when equal.
    pub(crate) fn kept(&self, stage: &mut MemoryStage, id: Id, held: &Held<T>) -> Option<Arc<T>> {
        let stored = match self.save {
            Some(save) => (save.load)(stage, &save.key, held),
            None => stage.session::<Option<T>>(id).as_ref().map(|value| reuse_arc(held, value)),
        };
        stored.or_else(|| self.reset.map(|value| reuse_arc(held, value)))
    }

    /// Stores the value of a kept binding where `kept` reads it.
    pub(crate) fn keep(&self, stage: &mut MemoryStage, id: Id, value: T) {
        match self.save {
            Some(save) => (save.store)(stage, &save.key, value),
            None => *stage.session::<Option<T>>(id) = Some(value),
        }
    }

    /// What the binding does with one event of the pass: the check may replace a value or refuse a commit.
    pub(crate) fn checked(&self, event: &mut EditEvent<T>) -> Checked {
        let (EditEvent::Update { value, .. } | EditEvent::Commit { after: value, .. } | EditEvent::Set { after: value, .. }) = event else {
            return Checked::Keep;
        };
        match self.validity(value) {
            Validity::Fix(fixed) => {
                *value = fixed;
                Checked::Keep
            }
            Validity::Refuse(_) => match event {
                EditEvent::Commit { .. } => Checked::Cancel,
                EditEvent::Set { .. } => Checked::Drop,
                _ => Checked::Keep,
            },
            _ => Checked::Keep,
        }
    }

    /// The binding's value for code that runs no pipeline step: a given or write-through value, or the kept value
    /// under the base id of `cx` (the saved tier when the binding has a `SaveKey`). `None` for a kept binding that
    /// holds nothing yet.
    pub fn read(&self, cx: &mut impl StageAccess) -> Option<Mixed<T>> {
        let id = cx.base();
        if let Some(key) = self.key(id) {
            let ctx = cx.stage_mut().ctx().clone();
            reads::note(&ctx, key, id, crate::runtime::measure::active(&ctx), &[]);
        }
        match &self.source {
            SourceMut::Kept => {
                let id = cx.base();
                self.kept(cx.stage_mut(), id, &None).map(|value| Mixed::Same((*value).clone()))
            }
            _ => self.get(),
        }
    }

    /// Writes `event` as the pipeline's apply step does: the check may replace the value (`Fix`) or refuse it
    /// (nothing is written). A write-through source takes the event under its policy; a kept binding stores the
    /// value the event leaves under the base id of `cx`; a given binding never writes, so the caller reports the
    /// event. Returns whether the value changed.
    pub fn write(&mut self, cx: &mut impl StageAccess, event: &EditEvent<T>) -> bool {
        let mut event = event.clone();
        if !matches!(self.checked(&mut event), Checked::Keep) {
            return false;
        }
        match self.source {
            SourceMut::Given(_) => false,
            SourceMut::Kept => {
                let Some(value) = event.landed() else { return false };
                let id = cx.base();
                if self.kept(cx.stage_mut(), id, &None).is_some_and(|held| *held == *value) {
                    return false;
                }
                self.keep(cx.stage_mut(), id, value.clone());
                true
            }
            _ => self.apply(&event) != Applied::Unchanged,
        }
    }

    /// The value a reset restores.
    pub(crate) fn reset_value(&self) -> Option<&T> {
        self.reset
    }

    /// Whether a write-through source takes `event`: every value under `WriteThrough`, commits and sets under `OnCommit`.
    pub(crate) fn writes(&self, event: &EditEvent<T>) -> bool {
        match self.source {
            SourceMut::Given(_) | SourceMut::Kept => false,
            SourceMut::Mut(_) | SourceMut::MutMixed(_) | SourceMut::Tracked { .. } | SourceMut::TrackedMixed { .. } => {
                self.policy == EditPolicy::WriteThrough || !matches!(event, EditEvent::Update { .. })
            }
        }
    }

    /// Applies `event` to a write-through source; a given or kept binding has nothing to write.
    #[doc(hidden)]
    pub fn apply(&mut self, event: &EditEvent<T>) -> Applied {
        if !self.writes(event) {
            return Applied::Unchanged;
        }
        match &mut self.source {
            SourceMut::Mut(value) => event.apply(&mut **value),
            SourceMut::MutMixed(value) => event.apply_mixed(&mut **value),
            SourceMut::Tracked { value, count, .. } => write_tracked(event, count, |event| event.apply(&mut **value)),
            SourceMut::TrackedMixed { value, count, .. } => write_tracked(event, count, |event| event.apply_mixed(&mut **value)),
            SourceMut::Given(_) | SourceMut::Kept => Applied::Unchanged,
        }
    }
}

/// Applies `event` to a `Tracked` field through `apply`: a gesture's start and a cancel with nothing to restore never
/// write, and a write that changes the field moves the `Tracked`'s write count.
fn write_tracked<T: Value>(event: &EditEvent<T>, count: &Cell<u64>, apply: impl FnOnce(&EditEvent<T>) -> Applied) -> Applied {
    if matches!(event, EditEvent::Begin { .. } | EditEvent::Cancel { before: None, .. }) {
        return Applied::Unchanged;
    }
    let applied = apply(event);
    if applied != Applied::Unchanged {
        count.set(count.get().wrapping_add(1));
    }
    applied
}

/// The output of a component that edits a value. Only `cx.edit_response()` builds it.
#[derive(Debug)]
#[non_exhaustive]
pub struct EditResponse<T> {
    /// The component's response.
    pub response: Response,
    /// The pass's events in order (`EditEvents`); empty when nothing was edited.
    pub events: EditEvents<T>,
    /// The value was submitted (Enter in a text part).
    pub submitted: bool,
    /// The modifiers of the Enter that submitted the value (Shift+Enter, Cmd+Enter); none unless `submitted` came
    /// from a key.
    pub submit_mods: craft_keymap::Mods,
    /// The modifiers held at the activation (click, Enter, Space) that applied this pass's value; a text Enter's stay
    /// in `submit_mods`.
    pub activation_mods: craft_keymap::Mods,
    /// Tab (`Some(true)`) or Shift+Tab (`Some(false)`) moved focus off the text part in this pass, which ended its
    /// edit; a list's rename continues on the next or previous row.
    pub left_by_tab: Option<bool>,
    /// The text part lost key focus in this pass, with or without an edit: a click elsewhere, Tab, or an Escape that
    /// keeps the text (`keep_on_escape`). It decides nothing; `submitted`, `escaped` and `left_by_tab` stay their own
    /// facts. False for a component with no text part.
    pub text_lost_focus: bool,
    /// The edit was abandoned with Escape.
    pub escaped: bool,
    /// A text part holds text that does not parse: the value is the last one that did, and a form that commits the
    /// value refuses to until the text parses or is abandoned.
    pub unparsed: bool,
    /// What the binding's check says about the shown value.
    pub validity: Validity<T>,
    /// The value the component showed after the pass, with the pass's edits applied; `None` for a component with
    /// no value to show. A composite reads a child's new state here in the pass it changes. The value is shared with
    /// the component, so building the response copies nothing.
    pub shown: Option<Mixed<Arc<T>>>,
    /// The shown value as the component words it to assistive technology ("40.0", "50 %"); `None` for a component
    /// that gives no such text.
    pub value_text: Option<String>,
    /// A `TabKey::Reports` text part kept focus on Tab (`Some(true)`) or Shift+Tab (`Some(false)`) in this pass;
    /// `left_by_tab` stays the fact that focus left by Tab, which such a part never does.
    pub tab: Option<bool>,
    /// The bound caret (byte offset) after the pass.
    pub caret: Option<usize>,
    /// The pointer's stay over the control.
    pub hover: Hover,
}

/// A command-bound control's report of the pointer's stay over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Hover {
    /// The pointer is over the control in this pass.
    pub over: bool,
    /// The pointer entered the control in this pass.
    pub entered: bool,
    /// The pointer left the control in this pass.
    pub left: bool,
    /// The control was clicked with the pointer, which has stayed over it since.
    pub clicked: bool,
}

impl<T> EditResponse<T> {
    pub(crate) fn new(response: Response, events: EditEvents<T>, validity: Validity<T>) -> Self {
        EditResponse {
            response,
            events,
            submitted: false,
            submit_mods: craft_keymap::Mods::NONE,
            activation_mods: craft_keymap::Mods::NONE,
            left_by_tab: None,
            text_lost_focus: false,
            escaped: false,
            unparsed: false,
            validity,
            shown: None,
            value_text: None,
            tab: None,
            caret: None,
            hover: Hover::default(),
        }
    }

    /// The response of a composite that edits one value through several child components: `events` are the edits of
    /// the whole value the composite lifted from its children's events, `shown` the value it showed after the pass, and
    /// `response` its own. The flags are clear and the validity is `Ok`; the composite sets the flags it reads from
    /// its children.
    pub fn lifted(response: Response, events: EditEvents<T>, shown: Option<Mixed<Arc<T>>>) -> Self {
        EditResponse {
            response,
            events,
            submitted: false,
            submit_mods: craft_keymap::Mods::NONE,
            activation_mods: craft_keymap::Mods::NONE,
            left_by_tab: None,
            text_lost_focus: false,
            escaped: false,
            unparsed: false,
            validity: Validity::Ok,
            shown,
            value_text: None,
            tab: None,
            caret: None,
            hover: Hover::default(),
        }
    }

    /// The latest value the pass's events carry: a proposal, a commit or a set; `None` when nothing was edited or no
    /// event carries one (a gesture's begin or cancel).
    pub fn value(&self) -> Option<&T> {
        self.events.value()
    }
}

impl<T: Clone> EditResponse<T> {
    /// The response of a child component's edit as the response of the edit of a value built from it: the events, the
    /// shown value and a fix are mapped by `f`; the response and flags are the child's own.
    pub fn map<U>(self, f: impl Fn(T) -> U) -> EditResponse<U> {
        let validity = match self.validity {
            Validity::Ok => Validity::Ok,
            Validity::Fix(value) => Validity::Fix(f(value)),
            Validity::Warn(message) => Validity::Warn(message),
            Validity::Refuse(message) => Validity::Refuse(message),
            Validity::Error(message) => Validity::Error(message),
            _ => Validity::Ok,
        };
        let shown = self.shown.map(|shown| match shown {
            Mixed::Same(value) => Mixed::Same(Arc::new(f(Arc::unwrap_or_clone(value)))),
            Mixed::Mixed => Mixed::Mixed,
        });
        EditResponse {
            response: self.response,
            events: self.events.map(&f),
            submitted: self.submitted,
            submit_mods: self.submit_mods,
            activation_mods: self.activation_mods,
            left_by_tab: self.left_by_tab,
            text_lost_focus: self.text_lost_focus,
            escaped: self.escaped,
            unparsed: self.unparsed,
            validity,
            shown,
            value_text: self.value_text,
            tab: self.tab,
            caret: self.caret,
            hover: self.hover,
        }
    }
}

impl<T: Value> Outcome for EditResponse<T> {
    type Value = T;

    fn response(&self) -> &Response {
        &self.response
    }

    fn value_text(&self) -> Option<&str> {
        self.value_text.as_deref()
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput { value_text: self.value_text.clone(), ..crate::testing::StoryOutput::of_edit(&self.response, &self.events) }
    }
}

/// What an item's output may report beyond its response.
pub trait ItemOutput<E>: Outcome {
    /// The event the collection reports for this item, `None` when the item reports nothing.
    fn into_event(self) -> Option<E>;

    /// The item asks to open or close its children, as a click on its disclosure does. The collection reports it
    /// as `CollectionEvent::Open`, the same form the keys and the screen reader produce.
    fn open_request(&self) -> Option<super::OpenChange<()>> {
        None
    }

    /// The item asks for its menu from one of its parts (a row's "More" button): the part's rect, in the item's layer.
    /// The collection makes the context request a `Trigger::Context` opener reads at that rect and reports
    /// `CollectionEvent::ContextMenu` at its bottom start corner, as it does for the Menu key.
    fn menu_request(&self) -> Option<egui::Rect> {
        None
    }

    /// The rect of the item's label, where a rename editor goes; `None` puts it over the whole row.
    fn label_rect(&self) -> Option<egui::Rect> {
        None
    }
}

impl<E> ItemOutput<E> for Response {
    fn into_event(self) -> Option<E> {
        None
    }
}

/// What a binding does with one event of the pass.
pub(crate) enum Checked {
    Keep,
    /// A refused commit becomes `Cancel(Refused)`.
    Cancel,
    /// A refused one-shot change reaches nothing.
    Drop,
}

/// Write-through bindings to fields of a [`Tracked`](crate::contract::Tracked) value: `bind_tracked!(doc => size)`
/// gives one [`Bind`], `bind_tracked!(doc => title, style.size)` a tuple with one per field path (field names and
/// tuple indices only), and `bind_tracked!(name)` one binding to the whole value. Making and reading the bindings
/// leaves the revision; an edit that changes the bound value moves it at the write, so bindings made every pass don't
/// invalidate the value's readers. Each binding borrows its own field, so the paths must not overlap.
///
/// ```
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::{PlainData, Tracked, bind_tracked};
///
/// #[derive(PlainData)]
/// struct Doc {
///     title: String,
///     style: (f32, bool),
/// }
///
/// let mut doc = Tracked::new(Doc { title: "a".into(), style: (12.0, false) });
/// let (title, size) = bind_tracked!(doc => title, style.0);
/// # let _ = (title, size);
///
/// let mut name = Tracked::new(String::from("a"));
/// let before = name.revision();
/// let whole = bind_tracked!(name);
/// # drop(whole);
/// assert_eq!(name.revision(), before, "binding leaves the revision");
/// ```
///
/// Overlapping paths don't compile:
///
/// ```compile_fail,E0499
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::{Tracked, bind_tracked};
/// let mut doc = Tracked::new((1_u32, 2_u32));
/// let (a, b) = bind_tracked!(doc => 0, 0);
/// # let _ = (a, b);
/// ```
///
/// A method call is not a field path (a macro with no matching rule reports no error code):
///
/// ```compile_fail
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::{Tracked, bind_tracked};
/// let mut names = Tracked::new(vec![String::new()]);
/// let first = bind_tracked!(names => first());
/// # let _ = first;
/// ```
#[macro_export]
macro_rules! bind_tracked {
    ($tracked:expr => $first:tt $(. $rest:tt)*) => {{
        let (model, count, source) = $tracked.__split_for_bind();
        let base = ::std::ptr::from_ref(&*model).addr();
        let field = &mut model.$first $(.$rest)*;
        let offset = ::std::ptr::from_ref(&*field).addr() - base;
        $crate::contract::Bind::__tracked(field, count, source, offset)
    }};
    ($tracked:expr => $($first:tt $(. $rest:tt)*),+ $(,)?) => {{
        let (model, count, source) = $tracked.__split_for_bind();
        let base = ::std::ptr::from_ref(&*model).addr();
        ($({
            let field = &mut model.$first $(.$rest)*;
            let offset = ::std::ptr::from_ref(&*field).addr() - base;
            $crate::contract::Bind::__tracked(field, count, source, offset)
        },)+)
    }};
    ($tracked:expr) => {{
        let (model, count, source) = $tracked.__split_for_bind();
        $crate::contract::Bind::__tracked(model, count, source, 0)
    }};
}
