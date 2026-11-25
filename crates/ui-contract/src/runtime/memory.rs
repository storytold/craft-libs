//! Ids and memory: `Remember`, the memory stage, `Facts`, eviction and saved state.

use crate::runtime::egui::memory::{self as store, FastMap, Map, Removed, take_raw};
use crate::runtime::egui::pass;
use crate::runtime::env::Env;
use crate::runtime::frame::{self, PassStep, PerViewport, Stamped, Stored};
use crate::runtime::pass_state;
use crate::runtime::pending::{self, Job, Pending};
use craft_behavior::eviction::owner_due;
use craft_host::Host;
use craft_settings::Remembered;
use egui::{Context, Id, IdSet, ViewportId};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::RefCell;
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
    /// The saved slot of `key`: it goes only through `forget_saved`. With a host in the frame it is loaded from the
    /// host's settings on first use and stored there when a pass changes it.
    fn saved<T: Savable>(&mut self, key: &SaveKey) -> &mut T;
    /// A read-only copy of the shown slot, for paint.
    fn recall<T: Slot>(&self) -> T;
    /// The result of the job `start` returns, one per `key`: it is submitted once, cancelled when the owner is
    /// evicted, and a landed result asks for a pass.
    fn pending<K: Hash, T: Slot, E: Slot>(&mut self, key: K, start: impl FnOnce() -> Job<T, E>) -> &Pending<T, E>;
    /// True on the first pass a shown stage exists for this id, and on the first pass after the id was left unshown
    /// for a frame of its viewport (a page left and opened again), through that frame's passes.
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
    /// A press entry whose press ended is dropped at write-back instead of kept.
    ended: Option<fn(&(dyn Any + Send + Sync)) -> bool>,
    /// The slot was in context data when the stage took it: the registry holds its record.
    recorded: bool,
    /// A SAVED entry of a real stage on a viewport with a host: what write-back needs to store a changed value.
    persist: Option<Persist>,
}

/// How a SAVED entry reaches the host's store. The fn pointers are monomorphised where `T: Savable` is known, so
/// write-back can serialize and store an entry whose type it does not name.
struct Persist {
    key: SaveKey,
    host: Arc<Host>,
    /// The serialized form of the value the stage took; `None` when it does not serialize.
    before: Option<toml::Value>,
    form: fn(&(dyn Any + Send + Sync)) -> Option<toml::Value>,
    store: fn(&(dyn Any + Send + Sync), &Host, &SaveKey),
}

/// The serialized form of a SAVED entry's value, the one the store keeps: TOML has no null, so an `Option` set to
/// `None` has none.
fn saved_form<T: Savable>(value: &(dyn Any + Send + Sync)) -> Option<toml::Value> {
    value.downcast_ref::<Wrapped<SAVED, T>>().and_then(|wrapped| toml::Value::try_from(&wrapped.0).ok())
}

fn store_saved<T: Savable>(value: &(dyn Any + Send + Sync), host: &Host, key: &SaveKey) {
    if let Some(wrapped) = value.downcast_ref::<Wrapped<SAVED, T>>() {
        Remembered::<T>::new(key.as_str()).store(host, &wrapped.0);
    }
}

/// Removes the stored value of `key`, whatever its type: `Remembered::forget` reads no value.
fn forget_stored(host: &Host, key: &SaveKey) {
    Remembered::<toml::Value>::new(key.as_str()).forget(host);
}

/// The SAVED tier's one load: when the slot of `key` is not in context data and the running viewport has a host, the
/// stored value enters context data with its registry record, so later passes take it from there and `forget_saved`
/// finds it. A stored value that no longer fits `T`, or none, loads as `T::default()`. Without a host the tier is
/// session only.
fn load_saved<T: Savable>(ctx: &Context, key: &SaveKey) {
    let id = key.id();
    if store::exists::<Wrapped<SAVED, T>>(ctx, id) {
        return;
    }
    let Some(host) = Env::host_of(ctx) else { return };
    let value = Remembered::<T>::new(key.as_str()).recall(&host).unwrap_or_default();
    store::put(ctx, id, Wrapped::<SAVED, T>(value));
    pass_state::with::<Registry, _>(ctx, |reg| {
        reg.slots.insert((id, SAVED, TypeId::of::<T>()), SlotRecord { owner: id, remove: remove_slot::<SAVED, T> });
    });
}

/// Stores every SAVED entry whose serialized form changed since the stage took it, and forgets the stored value of
/// one that no longer serializes (`Remembered::store` skips such a value and would keep the old one). An unchanged
/// value writes nothing. Called outside every lock: a host's `SectionStore` takes its own.
fn persist_changed(entries: &[Entry]) {
    for entry in entries {
        let Some(persist) = &entry.persist else { continue };
        let after = (persist.form)(&*entry.value);
        if after == persist.before {
            continue;
        }
        match after {
            Some(_) => (persist.store)(&*entry.value, &persist.host, &persist.key),
            None => forget_stored(&persist.host, &persist.key),
        }
    }
}

/// The last registration of an owner: the component id registered at step 3 whose pipeline opened a stage.
struct OwnerRecord {
    viewport: ViewportId,
    pass: u64,
    at: f64,
    /// The pass of the owner's registration before `pass`, in the same viewport.
    before: Option<u64>,
}

/// One entry the runtime keeps for a component, and the owner that is tested for its eviction.
struct SlotRecord {
    owner: Id,
    remove: Remover,
}

#[derive(Default)]
pub(crate) struct Registry {
    owners: FastMap<Id, OwnerRecord>,
    /// Keyed by `(slot id, tier, type)`.
    slots: FastMap<(Id, u8, TypeId), SlotRecord>,
    /// Evicted slots, moved out inside the end hook; the first pipeline run of the next pass drops them.
    graveyard: Vec<Removed>,
    /// Moves each time owners' shown lives end (`evict`, `end_shown`).
    ended_generation: u64,
    /// The generation at which each owner's shown life ended, until it registers again: a kept run that registered it
    /// before then showed state that is gone (`ended_since`).
    ended: FastMap<Id, u64>,
    /// Writes of the saved value under each key's id made outside a component's edit (`set_saved_value`,
    /// `forget_saved`), which a saved binding in a memoized view's props feeds.
    saved_writes: FastMap<Id, u64>,
    /// Session slots forgotten (`forget`), which a kept binding in a memoized view's props feeds.
    session_writes: u64,
    /// Slot records write-back has written, for the count trials.
    #[cfg(feature = "testing")]
    written: usize,
}

/// Marks a base whose stage was written back, for `appeared`: the frame it appeared in and the last frame it was
/// shown in. A discard pass repeats the frame, so `appeared` stays true through it. A frame of its viewport that did
/// not show the base (a page left and opened again before its owner was evicted) makes its next showing an
/// appearance again, as WordCraft told a reopened page by a gap in the frames it was drawn in.
#[derive(Clone, Copy)]
struct Seen {
    first: u64,
    last: u64,
}

impl Seen {
    /// Whether the frames since the base was last shown leave a gap before `frame`.
    fn gap_before(self, frame: u64) -> bool {
        self.last.saturating_add(1) < frame
    }

    /// Whether a stage of this base in `frame` is the base appearing.
    fn appears(self, frame: u64) -> bool {
        self.first == frame || self.gap_before(frame)
    }

    /// The mark after a write-back in `frame`.
    fn shown(old: Option<Seen>, frame: u64) -> Seen {
        match old {
            Some(seen) if !seen.gap_before(frame) => Seen { first: seen.first, last: frame.max(seen.last) },
            _ => Seen { first: frame, last: frame },
        }
    }
}

/// Registers `owner`'s pass, at step 3: eviction tests only owners. Shown entries of an owner not registered
/// for 3 passes of the viewport that last showed it and 0.5 s are evicted by `evict`.
pub(crate) fn register_owner(ctx: &Context, owner: Id) {
    crate::runtime::replay::journal(ctx, || owner, |ctx, &owner| register_owner(ctx, owner));
    crate::runtime::replay::note_owner(ctx, owner);
    let facts = frame::facts(ctx);
    pass_state::with::<Registry, _>(ctx, |reg| {
        let before = match reg.owners.get(&owner) {
            Some(last) if last.viewport == facts.viewport && last.pass == facts.pass_nr => last.before,
            Some(last) if last.viewport == facts.viewport => Some(last.pass),
            _ => None,
        };
        reg.owners.insert(owner, OwnerRecord { viewport: facts.viewport, pass: facts.pass_nr, at: facts.now, before });
        reg.ended.remove(&owner);
    });
}

/// Whether `owner` was registered in the pass before the running one of its viewport, by a run or a replay: its
/// component was shown there.
pub(crate) fn shown_last_pass(ctx: &Context, owner: Id) -> bool {
    let facts = frame::facts(ctx);
    pass_state::with::<Registry, _>(ctx, |reg| {
        reg.owners.get(&owner).filter(|last| last.viewport == facts.viewport).is_some_and(|last| {
            let shown = if last.pass == facts.pass_nr { last.before } else { Some(last.pass) };
            shown.is_some_and(|shown| shown + 1 >= facts.pass_nr)
        })
    })
}

/// The generation of ended shown lives now: a kept run stores it, and `ended_since` compares with it.
pub(crate) fn ended_generation(ctx: &Context) -> u64 {
    pass_state::with::<Registry, _>(ctx, |reg| reg.ended_generation)
}

/// Whether the shown life of one of `owners` ended after the generation `since`: a run that registered them showed
/// shown-tier state (remembered values, drafts, text state) that memory has dropped since.
pub(crate) fn ended_since(ctx: &Context, owners: &[Id], since: u64) -> bool {
    pass_state::with::<Registry, _>(ctx, |reg| {
        !reg.ended.is_empty() && owners.iter().any(|owner| reg.ended.get(owner).is_some_and(|ended| *ended > since))
    })
}

/// How many times the saved value under `key` was written outside a component's edit, and how many times session slots
/// were forgotten: what a saved or kept binding in a memoized view's props feeds.
pub(crate) fn writes(ctx: &Context, key: Option<&SaveKey>) -> (u64, u64) {
    pass_state::with::<Registry, _>(ctx, |reg| (key.and_then(|key| reg.saved_writes.get(&key.id()).copied()).unwrap_or(0), reg.session_writes))
}

/// Ends the shown lives of `owners` in the registry's generation count.
fn end_lives(reg: &mut Registry, owners: impl IntoIterator<Item = Id>) {
    reg.ended_generation += 1;
    let generation = reg.ended_generation;
    reg.ended.extend(owners.into_iter().map(|owner| (owner, generation)));
}

/// Eviction, run by the end hook. An owner that `owner_due` finds due loses its shown and press entries (a press
/// entry whose press never ended goes with its owner). Session and saved entries go only through `forget` and
/// `forget_saved`.
pub(crate) fn evict(ctx: &Context) {
    let now = frame::clock(ctx);
    let live = pass::live_viewports(ctx);
    pass_state::with::<Registry, _>(ctx, |reg| {
        let due = |r: &OwnerRecord| owner_due(r.pass, r.at, pass::pass_nr(ctx, r.viewport), now, live.contains(&r.viewport));
        let dead: IdSet = reg.owners.iter().filter(|(_, r)| due(r)).map(|(id, _)| *id).collect();
        if dead.is_empty() {
            return;
        }
        reg.owners.retain(|id, _| !dead.contains(id));
        end_lives(reg, dead.iter().copied());
        let moved = take_shown(ctx, reg, |owner| dead.contains(&owner));
        QUEUED.fetch_add(moved.len(), Ordering::Release);
        reg.graveyard.extend(moved);
    });
}

/// Ends the shown life of `owner` at once, as eviction does once its grace ran out: its shown and press entries go
/// with its record. A container that shows nothing ends its own (`run::typed::unshown`), and a dialog host the one of
/// a dialog it removes (`overlay::end_stack_dialog`), so a reopen is a first pass. Both run outside every lock and
/// hook, so the slots drop here and their dispose hooks (a running gesture's removal) report in this pass. False when
/// `owner` has no record: its life ended already, or it never ran.
pub(crate) fn end_shown(ctx: &Context, owner: Id) -> bool {
    let moved = pass_state::with::<Registry, _>(ctx, |reg| {
        reg.owners.remove(&owner).map(|_| {
            end_lives(reg, [owner]);
            take_shown(ctx, reg, |of| of == owner)
        })
    });
    moved.is_some()
}

/// Takes the shown and press slots of the owners `ended` picks out of context data (a press entry whose press never
/// ended goes with its owner).
fn take_shown(ctx: &Context, reg: &mut Registry, ended: impl Fn(Id) -> bool) -> Vec<Removed> {
    let gone: Vec<(Id, u8, TypeId)> = reg.slots.iter().filter(|(k, r)| ended(r.owner) && matches!(k.1, SHOWN | PRESS)).map(|(k, _)| *k).collect();
    let mut moved: Vec<Removed> = Vec::new();
    for key in gone {
        if let Some(rec) = reg.slots.remove(&key) {
            moved.extend(store::locked(ctx, |data| (rec.remove)(data, key.0)));
        }
    }
    moved
}

/// Slots queued for disposal across all contexts, so a run with nothing queued takes no lock.
static QUEUED: AtomicUsize = AtomicUsize::new(0);

/// Removes the session slots under `id` (all types). The only way a session slot goes.
pub(crate) fn forget(ctx: &Context, id: Id) {
    forget_tier(ctx, id, SESSION);
    pass_state::with::<Registry, _>(ctx, |reg| reg.session_writes += 1);
}

/// How long a remembered value lives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Lifetime {
    /// The value goes through the host's saved state (the `SaveKey` tier).
    #[default]
    Saved,
    /// The value lasts until the app quits.
    Run,
}

/// The run-only value under `key`, in the session tier. It never reaches the host. It gives the default until a value
/// is set, and it is readable outside a pass, as [`saved_value`] is.
pub fn run_value<T: Slot>(ctx: &Context, key: &SaveKey) -> T {
    MemoryStage::peek_session(ctx, global("craft_ui.run", key))
}

/// Sets the run-only value under `key`; see [`run_value`].
pub fn set_run_value<T: Slot>(ctx: &Context, key: &SaveKey, value: T) {
    let id = global("craft_ui.run", key);
    let mut stage = MemoryStage::new(ctx, id, id, false);
    // No component shows under the key's id, so the stage stamps no `Seen` for it.
    stage.touched.get_mut().clear();
    *stage.session::<T>(id) = value;
    stage.write_back();
}

/// The saved value under `key`, outside a dialog: an app reads one SAVED value through the same store the dialogs'
/// remembered fields use.
pub fn saved_value<T: Savable>(ctx: &Context, key: &SaveKey) -> T {
    MemoryStage::peek_saved(ctx, key)
}

/// Writes `value` as the saved value under `key`, outside a dialog. It is stored through the host when its
/// serialized form differs from the saved one.
pub fn set_saved_value<T: Savable>(ctx: &Context, key: &SaveKey, value: T) {
    let id = key.id();
    let mut stage = MemoryStage::new(ctx, id, id, false);
    // No component shows under the key's id, so the stage stamps no `Seen` for it.
    stage.touched.get_mut().clear();
    *stage.saved::<T>(key) = value;
    stage.write_back();
    saved_written(ctx, key);
}

/// Notes a write of the saved value under `key` outside a component's edit: a memoized view whose props bind it runs.
fn saved_written(ctx: &Context, key: &SaveKey) {
    pass_state::with::<Registry, _>(ctx, |reg| *reg.saved_writes.entry(key.id()).or_default() += 1);
    crate::runtime::lazy::note_edit(ctx, crate::runtime::reads::BindKey::Saved(Id::NULL.with(key)));
}

/// Removes the saved slots of `key` and the value stored under it. The only way a saved slot goes.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "used by the commands (F3)"))]
pub(crate) fn forget_saved(ctx: &Context, key: &SaveKey) {
    forget_tier(ctx, key.id(), SAVED);
    if let Some(host) = Env::host_of(ctx) {
        forget_stored(&host, key);
    }
    saved_written(ctx, key);
}

fn forget_tier(ctx: &Context, id: Id, tier: u8) {
    let moved = pass_state::with::<Registry, _>(ctx, |reg| {
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
    pass_state::with::<Registry, _>(ctx, |reg| {
        reg.slots.insert((id, SHOWN, TypeId::of::<T>()), SlotRecord { owner, remove: remove::<T> });
    });
}

/// Moves the shown slot of type `T` under `id` out of context data into the graveyard, as eviction does.
pub(crate) fn dispose_shown<T: Slot>(ctx: &Context, id: Id) {
    let moved = pass_state::with::<Registry, _>(ctx, |reg| {
        reg.slots.remove(&(id, SHOWN, TypeId::of::<T>()));
        store::locked(ctx, |data| remove_slot::<SHOWN, T>(data, id))
    });
    if let Some(moved) = moved {
        QUEUED.fetch_add(1, Ordering::Release);
        pass_state::with::<Registry, _>(ctx, |reg| reg.graveyard.push(moved));
    }
}

/// How many slot records write-back has written in `ctx` so far.
#[cfg(feature = "testing")]
pub(crate) fn records_written(ctx: &Context) -> usize {
    pass_state::with::<Registry, _>(ctx, |reg| reg.written)
}

/// The pass of the viewport that last registered `owner` at step 3, `None` once the owner is evicted.
pub(crate) fn owner_pass(ctx: &Context, owner: Id) -> Option<u64> {
    pass_state::with::<Registry, _>(ctx, |reg| reg.owners.get(&owner).map(|record| record.pass))
}

/// Drops the evicted slots, outside every lock and hook, so a slot's `Drop` may touch the `Context`.
pub(crate) fn dispose_evicted(ctx: &Context) {
    if QUEUED.load(Ordering::Acquire) == 0 {
        return;
    }
    let queue = pass_state::with::<Registry, _>(ctx, |reg| std::mem::take(&mut reg.graveyard));
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
    /// The stage belongs to another component than its base, so what it writes back is recorded under that owner
    /// even when the registry holds a record already.
    retags: bool,
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

    /// A stage over `base` that claims nothing and stamps no `Seen` for `base` (`appeared()` is unchanged): opened
    /// after the pass's real run wrote its own back, by a container that shows nothing, or for a slot of another
    /// component (a hold opener's dwell). The slots it creates are recorded under `owner`, so they are evicted with it.
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
            retags: owner != base,
            face: false,
        }
    }

    /// Debug assertion: no id is the base of two open stages in one pass of its viewport.
    pub(crate) fn claim(&self, base: Id) {
        if self.peek_only || !cfg!(debug_assertions) || self.claimed.borrow().contains(&base) {
            return;
        }
        self.claimed.borrow_mut().push(base);
        claim_id(&self.ctx, base, "an id is the base of two open stages in one pass");
    }

    /// Read-only access for `measure_in`: opens no stage.
    pub(crate) fn peek<T: Slot>(ctx: &Context, id: Id) -> T {
        store::peek_clone::<Wrapped<SHOWN, T>>(ctx, id).map(|wrapped| wrapped.0).unwrap_or_default()
    }

    /// Like `peek`, but `f` reads the slot in place and only its result is copied; a missing slot reads as the default.
    pub(crate) fn peek_with<T: Slot, R>(ctx: &Context, id: Id, f: impl FnOnce(&T) -> R) -> R {
        store::peek_with::<Wrapped<SHOWN, T>, R>(ctx, id, |wrapped| match wrapped {
            Some(wrapped) => f(&wrapped.0),
            None => f(&T::default()),
        })
    }

    /// Read-only access to a session slot, as `peek` does for a shown one.
    pub(crate) fn peek_session<T: Slot>(ctx: &Context, id: Id) -> T {
        store::peek_clone::<Wrapped<SESSION, T>>(ctx, id).map(|wrapped| wrapped.0).unwrap_or_default()
    }

    /// Read-only access to a saved slot, as `peek` does for a shown one.
    #[doc(hidden)]
    pub fn peek_saved<T: Savable>(ctx: &Context, key: &SaveKey) -> T {
        load_saved::<T>(ctx, key);
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

    /// True for the first pass a base id is shown, and for the first after a frame that did not show it; per base id,
    /// so a body and its component differ.
    pub(crate) fn appeared_at(&self, base: Id) -> bool {
        let mut touched = self.touched.borrow_mut();
        if !touched.contains(&base) {
            touched.push(base);
        }
        let frame = pass::frame_nr(&self.ctx, pass::current_viewport(&self.ctx));
        store::peek_copy::<Seen>(&self.ctx, base).is_none_or(|seen| seen.appears(frame))
    }

    fn entry(&mut self, key: (u8, TypeId, Id), create: fn(&Context, bool, Id) -> Entry) -> &mut Entry {
        let at = match self.entries.iter().position(|entry| entry.key == key) {
            Some(at) => at,
            None => {
                self.entries.push(create(&self.ctx, self.peek_only, key.2));
                self.entries.len() - 1
            }
        };
        &mut self.entries[at]
    }

    #[allow(clippy::unreachable, reason = "the entry key holds the lifetime and the type, so the downcast cannot fail")]
    fn slot<const TIER: u8, T: Slot>(&mut self, id: Id) -> &mut T {
        fn create<const TIER: u8, T: Slot>(ctx: &Context, peek: bool, id: Id) -> Entry {
            let (value, recorded): (Box<dyn Any + Send + Sync>, bool) = if peek {
                (Box::new(store::peek_clone::<Wrapped<TIER, T>>(ctx, id).unwrap_or_default()), false)
            } else {
                // The box the slot was stored in stays the entry's value.
                match store::take_boxed::<Wrapped<TIER, T>>(ctx, id) {
                    Some(boxed) if boxed.is::<Wrapped<TIER, T>>() => (boxed, true),
                    _ => (Box::new(Wrapped::<TIER, T>::default()), false),
                }
            };
            Entry {
                key: (TIER, TypeId::of::<T>(), id),
                value,
                write_back: write_back::<TIER, T>,
                remove: remove_slot::<TIER, T>,
                ended: None,
                recorded,
                persist: None,
            }
        }
        let entry = self.entry((TIER, TypeId::of::<T>(), id), create::<TIER, T>);
        match entry.value.downcast_mut::<Wrapped<TIER, T>>() {
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
    pub fn saved<T: Savable>(&mut self, key: &SaveKey) -> &mut T {
        let id = key.id();
        let entry_key = (SAVED, TypeId::of::<T>(), id);
        if !self.entries.iter().any(|entry| entry.key == entry_key) {
            load_saved::<T>(&self.ctx, key);
            self.slot::<SAVED, T>(id);
            // A peek stage writes nothing back, so it never stores.
            let host = if self.peek_only { None } else { Env::host_of(&self.ctx) };
            if let Some(host) = host
                && let Some(entry) = self.entries.iter_mut().find(|entry| entry.key == entry_key)
            {
                let before = saved_form::<T>(&*entry.value);
                entry.persist = Some(Persist { key: key.clone(), host, before, form: saved_form::<T>, store: store_saved::<T> });
            }
        }
        self.slot::<SAVED, T>(id)
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
        pass_state::with::<Registry, _>(&self.ctx, |reg| reg.slots.remove(&(id, SHOWN, ty)));
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

    /// A read-only copy of the press slot of `id`, as `recall` reads a shown one.
    pub(crate) fn recall_press<T: Slot>(&self, id: Id) -> Option<T> {
        let open = self
            .entries
            .iter()
            .find(|entry| entry.key == (PRESS, TypeId::of::<Option<T>>(), id))
            .and_then(|entry| entry.value.downcast_ref::<Wrapped<PRESS, Option<T>>>());
        match open {
            Some(wrapped) => wrapped.0.clone(),
            None => store::peek_clone::<Wrapped<PRESS, Option<T>>>(&self.ctx, id).and_then(|wrapped| wrapped.0),
        }
    }

    /// Step 17, after `finish` returned: every slot goes back to context data, and the registry gains a record
    /// for each slot and `Seen` mark it lacks. A slot the stage found in context data is recorded already; a stage
    /// owned by another component than its base records everything it wrote under that owner.
    #[doc(hidden)]
    pub fn write_back(self) {
        if self.peek_only {
            return;
        }
        let frame = pass::frame_nr(&self.ctx, pass::current_viewport(&self.ctx));
        let (owner, retags) = (self.owner, self.retags);
        let touched = self.touched.into_inner();
        let ctx = self.ctx.clone();
        persist_changed(&self.entries);
        let mut ended: Vec<(Id, u8, TypeId)> = Vec::new();
        let mut record: Vec<((Id, u8, TypeId), Remover)> = Vec::new();
        // One data lock for every slot; the values they replace and the ended presses drop outside it.
        let mut replaced: Vec<Removed> = Vec::new();
        let mut dropped: Vec<Box<dyn Any + Send + Sync>> = Vec::new();
        store::locked(&ctx, |data| {
            for entry in self.entries {
                let (tier, ty, id) = entry.key;
                // An ended press leaves no entry behind; its value drops outside the lock.
                if entry.ended.is_some_and(|is_ended| is_ended(&*entry.value)) {
                    ended.push((id, tier, ty));
                    dropped.push(entry.value);
                    continue;
                }
                if retags || !entry.recorded {
                    record.push(((id, tier, ty), entry.remove));
                }
                replaced.extend((entry.write_back)(entry.value, data, id));
            }
            for base in touched {
                if store::mark_seen(data, base, |old| Seen::shown(old, frame)) || retags {
                    record.push(((base, SHOWN, TypeId::of::<Seen>()), take_raw::<Seen>));
                }
            }
        });
        if !ended.is_empty() || !record.is_empty() {
            pass_state::with::<Registry, _>(&ctx, |reg| {
                for key in &ended {
                    reg.slots.remove(key);
                }
                #[cfg(feature = "testing")]
                {
                    reg.written += record.len();
                }
                for (key, remove) in record {
                    reg.slots.insert(key, SlotRecord { owner, remove });
                }
            });
        }
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

/// A salt for an id made from `key`, a value that is only `Hash`: the ids egui makes also ask for `Debug`, which an
/// app's key type need not have. Equal keys give equal salts within a run, so the id stays the same from pass to pass.
pub fn key_salt(key: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

/// The runtime sub-id of `base` for `tag`: the one derivation of every id the runtime makes from a component
/// id (body, children, header, footer, layer, rows, items, popups, the gesture, `memory_at` slots, animation
/// slots, edge and transform widgets, text and access sub-nodes).
#[doc(hidden)]
pub fn sub(base: Id, tag: impl Hash + Debug) -> Id {
    base.with(Sub(tag))
}

/// The slot a container with no binding or save key keeps its open state in for the session (`body::open`). A dialog
/// host forgets it for each layer of a dialog it removed (`overlay::end_stack_dialog`), so a dialog that closed itself
/// and opens again under the same id opens.
pub(crate) fn open_slot(id: Id) -> Id {
    sub(id, "open")
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

impl<T: Send + 'static> Stored for FactRecord<T> where FactRecord<T>: Default {}

impl<T> Default for FactRecord<T> {
    fn default() -> Self {
        FactRecord { this: Vec::new(), last: Vec::new() }
    }
}

impl<T: Clone + Send + Sync + 'static> Facts<T> {
    /// Registers `fact` for the next pass of the running viewport. A settled view's replay registers it again.
    #[doc(hidden)]
    pub fn register(ctx: &Context, fact: T)
    where
        T: PartialEq + std::fmt::Debug + crate::runtime::Shift,
    {
        crate::runtime::replay::journal(ctx, || fact.clone(), |ctx, fact| Self::register(ctx, fact.clone()));
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

/// Debug assertion: `id` is claimed once in a pass of its viewport, by an open stage or a memoized view's entry; a
/// second claim fails with `what`.
pub(crate) fn claim_id(ctx: &Context, id: Id, what: &'static str) {
    if !cfg!(debug_assertions) {
        return;
    }
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let duplicate = PerViewport::<Stamped<IdSet>>::with_pass(ctx, viewport, pass_nr, |claimed, _| claimed.clear(), |claimed| !claimed.insert(id));
    debug_assert!(!duplicate, "{what}");
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
