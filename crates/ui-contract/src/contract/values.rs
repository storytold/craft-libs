//! Values and edits (chapter 10): the binding, the output of a value component and the types that carry a value
//! between the app and the pipeline.

use crate::runtime::env::Message;
use crate::runtime::memory::{MemoryStage, Savable, SaveKey, StageAccess};
use crate::runtime::reads::{self, BindKey};
use craft_behavior::{Applied, CanvasView, EditEvent, EditEvents, EditPolicy, Mixed, Value};
use egui::Id;
use egui::Response;
use std::any::TypeId;
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

type Check<'a, T> = dyn Fn(&T) -> Validity<T> + 'a;

/// Where a binding's value lives.
enum Source<'a, T> {
    /// Value in, event out.
    Given(Mixed<T>),
    /// Write-through.
    Mut(&'a mut T),
    /// Write-through for a multi-selection field.
    MutMixed(&'a mut Mixed<T>),
    /// No caller value: session memory under the component id, or the saved tier.
    Kept,
}

/// The binding of a value component: a value the app owns, the check on it and the value a reset restores. The same
/// binding produces the same events whichever source it has.
pub struct Bind<'a, T> {
    source: Source<'a, T>,
    policy: EditPolicy,
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

    /// No caller value: the component keeps it in session memory under its id.
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

    /// The check on a value: `Fix` replaces it, `Warn` and `Refuse` set the component's state.
    pub fn check(mut self, check: impl Fn(&T) -> Validity<T> + 'a) -> Self {
        self.check = Some(Box::new(check));
        self
    }

    /// The value a reset gesture restores.
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
            Source::Kept => Source::Kept,
        };
        let check = self.check.as_deref().map(|check| Box::new(move |value: &T| check(value)) as Box<Check<'_, T>>);
        Bind { source, policy: self.policy, check, save: self.save.clone(), reset: self.reset.clone() }
    }

    /// The value a reset restores.
    #[doc(hidden)]
    pub fn reset_value(&self) -> Option<&T> {
        self.reset.as_ref()
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
            Source::MutMixed(Mixed::Same(value)) => value == shown,
            _ => false,
        }
    }

    /// The binding as the pipeline reads and writes it.
    pub fn reborrow(&mut self) -> BindMut<'_, T> {
        let source = match &mut self.source {
            Source::Given(value) => SourceMut::Given(value),
            Source::Mut(value) => SourceMut::Mut(&mut **value),
            Source::MutMixed(value) => SourceMut::MutMixed(&mut **value),
            Source::Kept => SourceMut::Kept,
        };
        BindMut { source, policy: self.policy, check: self.check.as_deref(), save: self.save.as_ref(), reset: self.reset.as_ref() }
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
            SourceMut::Given(Mixed::Same(value)) | SourceMut::MutMixed(Mixed::Same(value)) => value == shown,
            SourceMut::Mut(value) => **value == *shown,
            _ => false,
        }
    }

    /// The check's verdict on `value`.
    #[doc(hidden)]
    pub fn validity(&self, value: &T) -> Validity<T> {
        self.check.map_or(Validity::Ok, |check| check(value))
    }

    /// The value of a kept binding: the saved tier when it has a `SaveKey`, else session memory under `id`. `held`
    /// is the shown copy, reused when equal.
    pub(crate) fn kept(&self, stage: &mut MemoryStage, id: Id, held: &Held<T>) -> Option<Arc<T>> {
        match self.save {
            Some(save) => (save.load)(stage, &save.key, held),
            None => stage.session::<Option<T>>(id).as_ref().map(|value| reuse_arc(held, value)),
        }
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
            SourceMut::Mut(_) | SourceMut::MutMixed(_) => self.policy == EditPolicy::WriteThrough || !matches!(event, EditEvent::Update { .. }),
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
            SourceMut::Given(_) | SourceMut::Kept => Applied::Unchanged,
        }
    }
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
    /// Tab (`Some(true)`) or Shift+Tab (`Some(false)`) moved focus off the text part in this pass, which ended its
    /// edit; a list's rename continues on the next or previous row.
    pub left_by_tab: Option<bool>,
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
}

impl<T> EditResponse<T> {
    pub(crate) fn new(response: Response, events: EditEvents<T>, validity: Validity<T>) -> Self {
        EditResponse {
            response,
            events,
            submitted: false,
            submit_mods: craft_keymap::Mods::NONE,
            left_by_tab: None,
            escaped: false,
            unparsed: false,
            validity,
            shown: None,
            value_text: None,
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
            left_by_tab: None,
            escaped: false,
            unparsed: false,
            validity: Validity::Ok,
            shown,
            value_text: None,
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
            left_by_tab: self.left_by_tab,
            escaped: self.escaped,
            unparsed: self.unparsed,
            validity,
            shown,
            value_text: self.value_text,
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
