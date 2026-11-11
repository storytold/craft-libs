//! Ids and memory: `Remember`, the memory stage, `Facts`, eviction and saved state.

use crate::runtime::egui::memory::{self as store, FastMap, Map, Removed, Service, take_raw};
use crate::runtime::egui::pass;
use crate::runtime::frame::{self, PassStep, PerViewport, Stamped};
use crate::runtime::pending::{self, Job, Pending};
use craft_behavior::eviction::owner_due;
use egui::{Context, Id, ViewportId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A value kept in memory. `Clone` is required because egui's temp store clones on insert; large caches hold `Arc`s.
pub trait Slot: Clone + Default + Send + Sync + 'static {}
impl<T: Clone + Default + Send + Sync + 'static> Slot for T {}

/// A slot that also lives in saved state.
pub trait Savable: Slot + Serialize + DeserializeOwned {}
impl<T: Slot + Serialize + DeserializeOwned> Savable for T {}

/// The key of a saved slot, such as `"craft_dialogs.export.place"`. The crate prefix is checked, and the key
/// is never derived from an `Id`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SaveKey(Cow<'static, str>);

impl SaveKey {
    /// A key `crate_prefix.name`; evaluated in a const context it rejects a malformed name at compile time. The
    /// prefix is lowercase ASCII letters, digits and underscores.
    pub const fn new(key: &'static str) -> Self {
        check(key);
        SaveKey(Cow::Borrowed(key))
    }

    /// A key `crate_prefix.name` built at run time, for slots kept per item, such as a window for each open
    /// document. The format is `new`'s; a malformed key panics.
    pub fn named(key: impl Into<Arc<str>>) -> Self {
        let key: Arc<str> = key.into();
        check(&key);
        SaveKey(Cow::Owned(String::from(&*key)))
    }

    /// The key's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id the key's slots are stored under: one derivation, whatever component writes them.
    fn id(&self) -> Id {
        global("craft_ui.saved", self)
    }
}

/// Panics unless `key` is `crate_prefix.name`.
const fn check(key: &str) {
    let bytes = key.as_bytes();
    let mut prefix = 0;
    while prefix < bytes.len() && bytes[prefix] != b'.' {
        let byte = bytes[prefix];
        assert!(
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_',
            "a SaveKey's crate prefix is lowercase ASCII, digits and underscores"
        );
        prefix += 1;
    }
    assert!(prefix > 0 && prefix + 1 < bytes.len(), "a SaveKey is `crate_prefix.name`");
}

/// Implemented by `Cx`, `BodyCx` and `ItemCx` over the pipeline's memory stage, each keyed by its own base id.
pub trait Remember {
    /// `base.with(part)`: child and relation ids.
    fn key(&self, part: impl Hash + Debug) -> Id;
    /// The shown slot of this component.
    fn memory<T: Slot>(&mut self) -> &mut T;
    /// A shown slot keyed `sub(base, ("mem", part))`.
    fn memory_at<T: Slot>(&mut self, part: impl Hash + Debug) -> &mut T;
    /// The shown slot, read once: it moves out, so it follows the shown lifetime's eviction.
    fn take<T: Slot>(&mut self) -> Option<T>;
    /// The press slot: it goes when its press ends.
    fn press_slot<T: Slot>(&mut self) -> &mut Option<T>;
    /// The session slot: it goes only through `forget`.
    fn keep<T: Slot>(&mut self) -> &mut T;
    /// The saved slot of `key`: it goes only through `forget_saved`.
    fn saved<T: Savable>(&mut self, key: &SaveKey) -> &mut T;
    /// A read-only copy of the shown slot, for paint.
    fn recall<T: Slot>(&self) -> T;
    /// The result of the job `start` returns, one per `key`: it is submitted once, cancelled when the owner is
    /// evicted, and a landed result asks for a pass.
    fn pending<K: Hash, T: Slot, E: Slot>(&mut self, key: K, start: impl FnOnce() -> Job<T, E>) -> &Pending<T, E>;
    /// True on the first pass a shown stage exists for this id.
    fn appeared(&self) -> bool;
}

mod tier {
    pub(super) const SHOWN: u8 = 0;
    pub(super) const PRESS: u8 = 1;
    pub(super) const SESSION: u8 = 2;
    pub(super) const SAVED: u8 = 3;
}
use tier::{PRESS, SAVED, SESSION, SHOWN};

/// One lifetime of one type under one id, so a type can sit in two lifetimes.
#[derive(Clone, Default)]
struct Wrapped<const TIER: u8, T>(T);

/// Moves one typed slot out of the map; the caller drops the value outside every lock.
type Remover = fn(&mut Map, Id) -> Option<Removed>;

fn remove_slot<const TIER: u8, T: Slot>(data: &mut Map, id: Id) -> Option<Removed> {
    take_raw::<Wrapped<TIER, T>>(data, id)
}

struct Entry {
    /// `(tier, type, id)`.
    key: (u8, TypeId, Id),
    value: Box<dyn Any + Send + Sync>,
    write_back: fn(Box<dyn Any + Send + Sync>, &mut Map, Id) -> Option<Removed>,
    remove: Remover,
    tier: u8,
    ty: TypeId,
    /// A press entry whose press ended is dropped at write-back instead of kept.
    ended: Option<fn(&(dyn Any + Send + Sync)) -> bool>,
}

/// The last registration of an owner: the component id registered at step 3 whose pipeline opened a stage.
struct OwnerRecord {
    viewport: ViewportId,
    pass: u64,
    at: f64,
}

/// One entry the runtime keeps for a component, and the owner that is tested for its eviction.
struct SlotRecord {
    owner: Id,
    remove: Remover,
}

#[derive(Default)]
struct Registry {
    owners: FastMap<Id, OwnerRecord>,
    /// Keyed by `(slot id, tier, type)`.
    slots: FastMap<(Id, u8, TypeId), SlotRecord>,
    /// Evicted slots, moved out inside the end hook; the first pipeline run of the next pass drops them.
    graveyard: Vec<Removed>,
}

/// Marks a base whose stage was written back: the frame it was first shown in, for `appeared`. A discard pass
/// repeats the frame, so `appeared` stays true through it.
#[derive(Clone, Copy)]
struct Seen(u64);

/// Registers `owner`'s pass, at step 3: eviction tests only owners. Shown entries of an owner not registered
/// for 3 passes of the viewport that last showed it and 0.5 s are evicted by `evict`.
pub(crate) fn register_owner(ctx: &Context, owner: Id) {
    let viewport = pass::current_viewport(ctx);
    let (pass_nr, at) = (pass::pass_nr(ctx, viewport), frame::clock(ctx));
    Service::<Registry>::run(ctx, |reg| {
        reg.owners.insert(owner, OwnerRecord { viewport, pass: pass_nr, at });
    });
}

/// Eviction, run by the end hook. An owner that `owner_due` finds due loses its shown and press entries (a press
/// entry whose press never ended goes with its owner). Session and saved entries go only through `forget` and
/// `forget_saved`.
pub(crate) fn evict(ctx: &Context) {
    let now = frame::clock(ctx);
    let live = pass::live_viewports(ctx);
    let registry = Service::<Registry>::of(ctx);
    registry.with(|reg| {
        let due = |r: &OwnerRecord| owner_due(r.pass, r.at, pass::pass_nr(ctx, r.viewport), now, live.contains(&r.viewport));
        let dead: HashSet<Id> = reg.owners.iter().filter(|(_, r)| due(r)).map(|(id, _)| *id).collect();
        if dead.is_empty() {
            return;
        }
        reg.owners.retain(|id, _| !dead.contains(id));
        let gone: Vec<(Id, u8, TypeId)> =
            reg.slots.iter().filter(|(k, r)| dead.contains(&r.owner) && matches!(k.1, SHOWN | PRESS)).map(|(k, _)| *k).collect();
        let mut moved: Vec<Removed> = Vec::new();
        for key in gone {
            if let Some(rec) = reg.slots.remove(&key) {
                moved.extend(store::locked(ctx, |data| (rec.remove)(data, key.0)));
            }
        }
        QUEUED.fetch_add(moved.len(), Ordering::Release);
        reg.graveyard.extend(moved);
    });
}

/// Slots queued for disposal across all contexts, so a run with nothing queued takes no lock.
static QUEUED: AtomicUsize = AtomicUsize::new(0);

/// Removes the session slots under `id` (all types). The only way a session slot goes.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "used by the pipeline (F3)"))]
pub(crate) fn forget(ctx: &Context, id: Id) {
    forget_tier(ctx, id, SESSION);
}

/// Removes the saved slots of `key`. The only way a saved slot goes.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "used by the commands (F3)"))]
pub(crate) fn forget_saved(ctx: &Context, key: &SaveKey) {
    forget_tier(ctx, key.id(), SAVED);
}

fn forget_tier(ctx: &Context, id: Id, tier: u8) {
    let moved = Service::<Registry>::run(ctx, |reg| {
        let keys: Vec<(Id, u8, TypeId)> = reg.slots.keys().filter(|k| k.0 == id && k.1 == tier).copied().collect();
        let mut moved: Vec<Removed> = Vec::new();
        for key in keys {
            if let Some(rec) = reg.slots.remove(&key) {
                moved.extend(store::locked(ctx, |data| (rec.remove)(data, key.0)));
            }
        }
        moved
    });
    drop(moved);
}

/// Registers a slot the motion service wrote straight into context data, so it goes with its owner's shown entries.
pub(crate) fn adopt<T: Any + Clone + Send + Sync>(ctx: &Context, owner: Id, id: Id) {
    fn remove<T: Any + Send + Sync>(data: &mut Map, id: Id) -> Option<Removed> {
        take_raw::<T>(data, id)
    }
    Service::<Registry>::run(ctx, |reg| {
        reg.slots.insert((id, SHOWN, TypeId::of::<T>()), SlotRecord { owner, remove: remove::<T> });
    });
}

/// Moves the shown slot of type `T` under `id` out of context data into the graveyard, as eviction does.
pub(crate) fn dispose_shown<T: Slot>(ctx: &Context, id: Id) {
    let moved = Service::<Registry>::run(ctx, |reg| {
        reg.slots.remove(&(id, SHOWN, TypeId::of::<T>()));
        store::locked(ctx, |data| remove_slot::<SHOWN, T>(data, id))
    });
    if let Some(moved) = moved {
        QUEUED.fetch_add(1, Ordering::Release);
        Service::<Registry>::run(ctx, |reg| reg.graveyard.push(moved));
    }
}

/// The pass of the viewport that last registered `owner` at step 3, `None` once the owner is evicted.
pub(crate) fn owner_pass(ctx: &Context, owner: Id) -> Option<u64> {
    Service::<Registry>::run(ctx, |reg| reg.owners.get(&owner).map(|record| record.pass))
}

/// Drops the evicted slots, outside every lock and hook, so a slot's `Drop` may touch the `Context`.
pub(crate) fn dispose_evicted(ctx: &Context) {
    if QUEUED.load(Ordering::Acquire) == 0 {
        return;
    }
    let queue = Service::<Registry>::run(ctx, |reg| std::mem::take(&mut reg.graveyard));
    QUEUED.fetch_sub(queue.len(), Ordering::AcqRel);
    drop(queue);
}

fn write_back<const TIER: u8, T: Slot>(value: Box<dyn Any + Send + Sync>, data: &mut Map, id: Id) -> Option<Removed> {
    value.downcast::<Wrapped<TIER, T>>().ok().and_then(|wrapped| store::put_boxed(data, id, wrapped))
}

fn press_ended<T: Slot>(value: &(dyn Any + Send + Sync)) -> bool {
    value.downcast_ref::<Wrapped<PRESS, Option<T>>>().is_some_and(|wrapped| wrapped.0.is_none())
}

/// Slots move into the stage on first use and back after `finish` returns. It appears in the sealed
/// `StageAccess` and has no public constructor.
#[doc(hidden)]
pub struct MemoryStage {
    ctx: Context,
    /// The component id registered at step 3 whose pipeline opened this stage; eviction tests only it.
    owner: Id,
    entries: Vec<Entry>,
    /// Base ids whose `appeared()` was asked this pass; they are marked seen at write-back.
    touched: RefCell<Vec<Id>>,
    /// Bases this stage claimed (the component's own, its body's, its items').
    claimed: RefCell<Vec<Id>>,
    /// A sizing pass or `measure_in`: reads clone from context data, nothing moves, nothing is written back.
    peek_only: bool,
    /// A face's stage: a `Remember` write is a debug assertion, because a face has no state of its own.
    pub(crate) face: bool,
}

impl MemoryStage {
    /// The egui context the stage belongs to.
    #[doc(hidden)]
    pub fn ctx(&self) -> &Context {
        &self.ctx
    }

    /// A stage whose entries belong to `base`.
    #[doc(hidden)]
    pub fn open(ctx: &Context, base: Id) -> Self {
        Self::open_owned(ctx, base, base)
    }

    /// A stage whose entries belong to `owner`: a popup body's stage belongs to the component that shows it.
    pub(crate) fn open_owned(ctx: &Context, base: Id, owner: Id) -> Self {
        let stage = Self::new(ctx, base, owner, false);
        stage.claim(base);
        stage
    }

    /// A stage over `base` opened after the pass's real run wrote its own back: it claims nothing and stamps no
    /// `Seen` for `base` (`appeared()` is unchanged), and the slots it creates are recorded under `owner`, so they
    /// are evicted with the container.
    pub(crate) fn open_after(ctx: &Context, base: Id, owner: Id) -> Self {
        let stage = Self::new(ctx, base, owner, false);
        stage.touched.borrow_mut().clear();
        stage
    }

    /// The stage of a sizing pass: it opens no lifetime stage, so a container's first appearance in a new
    /// layer cannot lose a kept value.
    pub(crate) fn open_peek(ctx: &Context, base: Id) -> Self {
        Self::new(ctx, base, base, true)
    }

    fn new(ctx: &Context, base: Id, owner: Id, peek_only: bool) -> Self {
        MemoryStage {
            ctx: ctx.clone(),
            owner,
            entries: Vec::new(),
            touched: RefCell::new(vec![base]),
            claimed: RefCell::new(Vec::new()),
            peek_only,
            face: false,
        }
    }

    /// Debug assertion: no id is the base of two open stages in one pass of its viewport.
    pub(crate) fn claim(&self, base: Id) {
        if self.peek_only || !cfg!(debug_assertions) || self.claimed.borrow().contains(&base) {
            return;
        }
        self.claimed.borrow_mut().push(base);
        let viewport = pass::current_viewport(&self.ctx);
        let pass_nr = pass::pass_nr(&self.ctx, viewport);
        let duplicate = PerViewport::<Stamped<HashSet<Id>>>::with_pass(
            &self.ctx,
            viewport,
            pass_nr,
            |claimed, _| claimed.clear(),
            |claimed| !claimed.insert(base),
        );
        debug_assert!(!duplicate, "an id is the base of two open stages in one pass");
    }

    /// Read-only access for `measure_in`: opens no stage.
    pub(crate) fn peek<T: Slot>(ctx: &Context, id: Id) -> T {
        store::peek_clone::<Wrapped<SHOWN, T>>(ctx, id).map(|wrapped| wrapped.0).unwrap_or_default()
    }

    /// Read-only access to a saved slot, as `peek` does for a shown one.
    #[doc(hidden)]
    pub fn peek_saved<T: Slot>(ctx: &Context, key: &SaveKey) -> T {
        store::peek_clone::<Wrapped<SAVED, T>>(ctx, key.id()).map(|wrapped| wrapped.0).unwrap_or_default()
    }

    /// A sizing pass's stage, which keeps nothing.
    #[doc(hidden)]
    pub fn is_peek(&self) -> bool {
        self.peek_only
    }

    pub(crate) fn owner(&self) -> Id {
        self.owner
    }

    /// True for the first pass a base id is shown; per base id, so a body and its component differ.
    pub(crate) fn appeared_at(&self, base: Id) -> bool {
        let mut touched = self.touched.borrow_mut();
        if !touched.contains(&base) {
            touched.push(base);
        }
        let frame = pass::frame_nr(&self.ctx, pass::current_viewport(&self.ctx));
        store::peek_copy::<Seen>(&self.ctx, base).is_none_or(|seen| seen.0 == frame)
    }

    #[allow(clippy::unreachable, reason = "the entry key holds the lifetime and the type, so the downcast cannot fail")]
    fn slot<const TIER: u8, T: Slot>(&mut self, id: Id) -> &mut T {
        let ctx = &self.ctx;
        let peek = self.peek_only;
        let key = (TIER, TypeId::of::<T>(), id);
        let at = match self.entries.iter().position(|entry| entry.key == key) {
            Some(at) => at,
            None => {
                let value: Box<dyn Any + Send + Sync> = if peek {
                    Box::new(store::peek_clone::<Wrapped<TIER, T>>(ctx, id).unwrap_or_default())
                } else {
                    // The box the slot was stored in stays the entry's value.
                    match store::locked(ctx, |data| take_raw::<Wrapped<TIER, T>>(data, id)) {
                        Some(boxed) if boxed.is::<Wrapped<TIER, T>>() => boxed,
                        _ => Box::new(Wrapped::<TIER, T>::default()),
                    }
                };
                self.entries.push(Entry {
                    key,
                    value,
                    write_back: write_back::<TIER, T>,
                    remove: remove_slot::<TIER, T>,
                    tier: TIER,
                    ty: TypeId::of::<T>(),
                    ended: None,
                });
                self.entries.len() - 1
            }
        };
        match self.entries[at].value.downcast_mut::<Wrapped<TIER, T>>() {
            Some(wrapped) => &mut wrapped.0,
            None => unreachable!("an entry's value has the type its key names"),
        }
    }

    #[doc(hidden)]
    pub fn shown<T: Slot>(&mut self, id: Id) -> &mut T {
        self.slot::<SHOWN, T>(id)
    }

    pub(crate) fn session<T: Slot>(&mut self, id: Id) -> &mut T {
        self.slot::<SESSION, T>(id)
    }

    pub(crate) fn press<T: Slot>(&mut self, id: Id) -> &mut Option<T> {
        self.slot::<PRESS, Option<T>>(id);
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == (PRESS, TypeId::of::<Option<T>>(), id)) {
            entry.ended = Some(press_ended::<T>);
        }
        self.slot::<PRESS, Option<T>>(id)
    }

    #[doc(hidden)]
    pub fn saved<T: Slot>(&mut self, key: &SaveKey) -> &mut T {
        self.slot::<SAVED, T>(key.id())
    }

    /// The shown slot of `id`, moved out: it is read once. `None` when nothing was written under it.
    pub(crate) fn take<T: Slot>(&mut self, id: Id) -> Option<T> {
        let ty = TypeId::of::<T>();
        if let Some(at) = self.entries.iter().position(|entry| entry.key == (SHOWN, ty, id)) {
            let entry = self.entries.swap_remove(at);
            return entry.value.downcast::<Wrapped<SHOWN, T>>().ok().map(|wrapped| wrapped.0);
        }
        if self.peek_only {
            return store::peek_clone::<Wrapped<SHOWN, T>>(&self.ctx, id).map(|wrapped| wrapped.0);
        }
        let moved = store::take::<Wrapped<SHOWN, T>>(&self.ctx, id)?;
        Service::<Registry>::run(&self.ctx, |reg| reg.slots.remove(&(id, SHOWN, ty)));
        Some(moved.0)
    }

    #[doc(hidden)]
    pub fn recall<T: Slot>(&self, id: Id) -> T {
        let open = self
            .entries
            .iter()
            .find(|entry| entry.key == (SHOWN, TypeId::of::<T>(), id))
            .and_then(|entry| entry.value.downcast_ref::<Wrapped<SHOWN, T>>());
        match open {
            Some(wrapped) => wrapped.0.clone(),
            None => store::peek_clone::<Wrapped<SHOWN, T>>(&self.ctx, id).map(|wrapped| wrapped.0).unwrap_or_default(),
        }
    }

    /// Step 17, after `finish` returned: every slot goes back to context data and is recorded under its owner.
    #[doc(hidden)]
    pub fn write_back(self) {
        if self.peek_only {
            return;
        }
        let frame = pass::frame_nr(&self.ctx, pass::current_viewport(&self.ctx));
        let owner = self.owner;
        let touched = self.touched.into_inner();
        let mut ended = Vec::new();
        Service::<Registry>::run(&self.ctx, |reg| {
            for base in &touched {
                reg.slots.insert((*base, SHOWN, TypeId::of::<Seen>()), SlotRecord { owner, remove: take_raw::<Seen> });
            }
            for entry in &self.entries {
                let key = (entry.key.2, entry.tier, entry.ty);
                if entry.ended.is_some_and(|is_ended| is_ended(&*entry.value)) {
                    reg.slots.remove(&key);
                    ended.push(key);
                } else {
                    reg.slots.insert(key, SlotRecord { owner, remove: entry.remove });
                }
            }
        });
        // One data lock for every slot; the values they replace and the ended presses drop outside it.
        let mut replaced: Vec<Removed> = Vec::new();
        let ctx = self.ctx.clone();
        let mut dropped: Vec<Box<dyn Any + Send + Sync>> = Vec::new();
        store::locked(&ctx, |data| {
            for entry in self.entries {
                let (tier, ty, id) = entry.key;
                // An ended press leaves no entry behind; its value drops outside the lock.
                if ended.contains(&(id, tier, ty)) {
                    dropped.push(entry.value);
                    continue;
                }
                replaced.extend((entry.write_back)(entry.value, data, id));
            }
            for base in touched {
                store::mark_seen(data, base, Seen(frame));
            }
        });
        drop(replaced);
        drop(dropped);
    }
}

pub(crate) mod sealed {
    /// Keeps `StageAccess` to the contexts of this crate.
    pub trait Sealed {}
}

/// The `Remember` calls are written once over this accessor. Sealed: only the pipeline's contexts implement it.
#[doc(hidden)]
pub trait StageAccess: sealed::Sealed {
    /// The open stage.
    fn stage(&self) -> &MemoryStage;
    /// The open stage, mutably.
    fn stage_mut(&mut self) -> &mut MemoryStage;
    /// The id this context keys its memory by: the component id for `Cx`, `sub(id, "body")` for a body, the
    /// item id for an item, the popup id for a popup body.
    fn base(&self) -> Id;
}

impl<X: StageAccess> Remember for X {
    fn key(&self, part: impl Hash + Debug) -> Id {
        self.base().with(part)
    }

    fn memory<T: Slot>(&mut self) -> &mut T {
        face_guard(self.stage());
        let id = self.base();
        self.stage_mut().shown(id)
    }

    fn memory_at<T: Slot>(&mut self, part: impl Hash + Debug) -> &mut T {
        face_guard(self.stage());
        let id = sub(self.base(), ("mem", part));
        self.stage_mut().shown(id)
    }

    fn take<T: Slot>(&mut self) -> Option<T> {
        face_guard(self.stage());
        let id = self.base();
        self.stage_mut().take(id)
    }

    fn press_slot<T: Slot>(&mut self) -> &mut Option<T> {
        face_guard(self.stage());
        let id = self.base();
        self.stage_mut().press(id)
    }

    fn keep<T: Slot>(&mut self) -> &mut T {
        face_guard(self.stage());
        let id = self.base();
        self.stage_mut().session(id)
    }

    fn saved<T: Savable>(&mut self, key: &SaveKey) -> &mut T {
        face_guard(self.stage());
        self.stage_mut().saved(key)
    }

    fn recall<T: Slot>(&self) -> T {
        self.stage().recall(self.base())
    }

    fn pending<K: Hash, T: Slot, E: Slot>(&mut self, key: K, start: impl FnOnce() -> Job<T, E>) -> &Pending<T, E> {
        face_guard(self.stage());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let id = sub(self.base(), ("pending", hasher.finish()));
        pending::poll(self.stage_mut(), id, start)
    }

    fn appeared(&self) -> bool {
        self.stage().appeared_at(self.base())
    }
}

fn face_guard(stage: &MemoryStage) {
    debug_assert!(!stage.face, "a face writes no memory: the composite owns the state a face shows");
}

/// The tag type of runtime sub-ids. Its `Hash` writes a private `TypeId` first, so no `cx.key(k)`, child id or
/// app-chosen key can equal a runtime sub-id.
#[derive(Debug)]
struct Sub<T>(T);

impl<T: Hash> Hash for Sub<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        TypeId::of::<Sub<()>>().hash(state);
        self.0.hash(state);
    }
}

/// The runtime sub-id of `base` for `tag`: the one derivation of every id the runtime makes from a component
/// id (body, children, header, footer, layer, rows, items, popups, the gesture, `memory_at` slots, animation
/// slots, edge and transform widgets, text and access sub-nodes).
#[doc(hidden)]
pub fn sub(base: Id, tag: impl Hash + Debug) -> Id {
    base.with(Sub(tag))
}

/// The id of a runtime singleton named `name`, optionally under a `scope` such as a viewport.
#[doc(hidden)]
pub fn global(name: &'static str, scope: impl Hash + Debug) -> Id {
    store::unique(name).with(scope)
}

/// Cross-widget facts, registered in one pass and read in the next. Runtime services only.
#[doc(hidden)]
pub struct Facts<T>(PhantomData<T>);

/// The facts of one viewport: this pass's registrations and last pass's.
struct FactRecord<T> {
    this: Vec<T>,
    last: Vec<T>,
}

impl<T> Default for FactRecord<T> {
    fn default() -> Self {
        FactRecord { this: Vec::new(), last: Vec::new() }
    }
}

impl<T: Clone + Send + Sync + 'static> Facts<T> {
    /// Registers `fact` for the next pass of the running viewport.
    #[doc(hidden)]
    pub fn register(ctx: &Context, fact: T) {
        Self::with_record(ctx, |record| record.this.push(fact));
    }

    /// The facts the running viewport's previous pass registered, in registration order.
    #[doc(hidden)]
    pub fn of_last_pass(ctx: &Context) -> Vec<T> {
        Self::with_record(ctx, |record| record.last.clone())
    }

    fn with_record<R>(ctx: &Context, f: impl FnOnce(&mut FactRecord<T>) -> R) -> R {
        let viewport = pass::current_viewport(ctx);
        let pass_nr = pass::pass_nr(ctx, viewport);
        let advance = |record: &mut FactRecord<T>, step: PassStep| match step {
            PassStep::Next => record.last = std::mem::take(&mut record.this),
            PassStep::Skipped => {
                record.last.clear();
                record.this.clear();
            }
        };
        PerViewport::<Stamped<FactRecord<T>>>::with_pass(ctx, viewport, pass_nr, advance, f)
    }
}

/// Runs `f` on the session slot of `id` for a service or pattern that has no component context: the slot is read,
/// changed and written back in one call, and it goes only through `forget`. Calls nest freely and repeat within a
/// pass, because no stage claims a base.
#[doc(hidden)]
pub fn with_session<T: Slot, R>(ctx: &Context, id: Id, f: impl FnOnce(&mut T) -> R) -> R {
    let mut stage = MemoryStage::new(ctx, id, id, false);
    let out = f(stage.session::<T>(id));
    stage.write_back();
    out
}

/// An id no other `source` shares: the id of something with a stable key and no component of its own.
#[doc(hidden)]
pub fn unique(source: impl std::hash::Hash + std::fmt::Debug) -> Id {
    store::unique(source)
}
