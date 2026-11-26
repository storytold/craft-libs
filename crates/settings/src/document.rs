//! TOML document handling: `schema_version`, per-section parsing and comment-preserving merges.

use std::collections::{HashMap, HashSet};

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value};

use crate::Settings;
use crate::error::SectionError;

/// How [`merge_changes`] merges one item path when another process wrote the file meanwhile. The app names the
/// rule per path in [`Settings::merge_rule`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum MergeRule {
    /// A table merges key by key, each key by its own rule; an array or a scalar is replaced whole.
    ByKey,
    /// The item as a whole: when both instances changed it, ours replaces theirs, so a table is never a mix of two
    /// instances' keys (a window's size from one and its place from the other).
    Whole,
    /// A most-recently-used list, most recent first: the entries this instance added or used again go to the
    /// front, entries it removed stay removed, the other instance's keep their order behind, and the list is cut
    /// to `cap`. An entry whose `key` is `None` is dropped.
    Recent {
        /// The most entries kept.
        cap: usize,
        /// The identity of an entry; `None` for an entry without one.
        key: fn(&toml_edit::Value) -> Option<String>,
    },
    /// A list of user entries merged by identity: entries either instance added are kept, entries either removed
    /// stay removed, an entry both changed takes ours, and an entry added on one side follows the entry before it
    /// there. Entries with the same key are told apart by their order. An entry whose `key` is `None` is dropped.
    Entries {
        /// The identity of an entry; `None` for an entry without one.
        key: fn(&toml_edit::Value) -> Option<String>,
    },
}

/// The key written first in every `config.toml`.
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Parses `text` as TOML, keeping comments and formatting.
pub fn parse(text: &str) -> Result<DocumentMut, toml_edit::TomlError> {
    text.parse()
}

/// Reads `schema_version`; `None` when the key is missing or not a non-negative integer.
pub fn schema_version(doc: &DocumentMut) -> Option<u32> {
    doc.get(SCHEMA_VERSION_KEY)?.as_integer().and_then(|v| u32::try_from(v).ok())
}

/// Sets `schema_version`, keeping its position if it already exists.
pub fn set_schema_version(doc: &mut DocumentMut, version: u32) {
    doc[SCHEMA_VERSION_KEY] = toml_edit::value(i64::from(version));
}

/// An empty document carrying only `schema_version`.
pub fn new_document(version: u32) -> DocumentMut {
    let mut doc = DocumentMut::new();
    set_schema_version(&mut doc, version);
    doc
}

/// Turns `doc` into `T`, one value at a time.
///
/// A value that does not fit `T` is reported with its dotted key path (`general.autosave_minutes`, a map's
/// `shortcuts.file.open`) and read as its default; every other value is kept. This is what keeps one hand-edited
/// value, or a renamed enum variant, from resetting its neighbours.
pub fn deserialize_sections<T: Settings>(doc: &DocumentMut) -> (T, Vec<SectionError>) {
    let (value, resets) = read::<T>(doc);
    (value, resets.into_iter().map(Reset::into_error).collect())
}

/// [`deserialize_sections`] for a document the store will save: every value it reset is also removed from `doc`,
/// so the next save writes the default in its place (or, for a map's entry, leaves it out).
pub fn repair_sections<T: Settings>(doc: &mut DocumentMut) -> (T, Vec<SectionError>) {
    let (value, resets) = read::<T>(doc);
    for reset in &resets {
        remove_at(doc.as_table_mut(), &reset.path);
    }
    (value, resets.into_iter().map(Reset::into_error).collect())
}

/// A value of the document that `T` could not read: its key path and the deserialization error.
struct Reset {
    path: Vec<String>,
    message: String,
}

impl Reset {
    fn into_error(self) -> SectionError {
        SectionError { section: self.path.join("."), message: self.message }
    }
}

fn read<T: Settings>(doc: &DocumentMut) -> (T, Vec<Reset>) {
    if let Ok(value) = toml_edit::de::from_document::<T>(doc.clone()) {
        return (value, Vec::new());
    }
    let defaults = T::default();
    let Ok(mut base) = toml_edit::ser::to_document(&defaults) else {
        return (defaults, Vec::new());
    };
    expand_inline_tables(base.as_table_mut());
    let mut merged = base.clone();
    let mut resets = Vec::new();
    for (key, item) in doc.iter() {
        if key == SCHEMA_VERSION_KEY {
            continue;
        }
        let mut found = Vec::new();
        accept::<T>(&base, &mut merged, &mut vec![key.to_owned()], item, &mut found);
        // The values of a section are tried one by one; together they must still fit, or the section is reset whole.
        let mut candidate = base.clone();
        if let Some(kept) = merged.get(key) {
            candidate[key] = kept.clone();
        }
        if let Err(e) = toml_edit::de::from_document::<T>(candidate) {
            match base.get(key) {
                Some(default) => merged[key] = default.clone(),
                None => {
                    merged.remove(key);
                }
            }
            found = vec![Reset { path: vec![key.to_owned()], message: e.to_string() }];
        }
        resets.extend(found);
    }
    let value = toml_edit::de::from_document::<T>(merged).unwrap_or(defaults);
    (value, resets)
}

/// Puts `item` at `path` in `merged` when the defaults with only that value in place still read as `T`. A table that
/// does not is tried key by key the same way; any other value is recorded as reset.
///
/// Each value is tried against the defaults alone, never against the values accepted before it, so a damaged
/// section costs the defaults plus one value per value.
fn accept<T: Settings>(base: &DocumentMut, merged: &mut DocumentMut, path: &mut Vec<String>, item: &Item, resets: &mut Vec<Reset>) {
    let mut candidate = base.clone();
    place(candidate.as_table_mut(), path, item.clone());
    let error = match toml_edit::de::from_document::<T>(candidate) {
        Ok(_) => {
            place(merged.as_table_mut(), path, item.clone());
            return;
        }
        Err(e) => e,
    };
    match item.as_table_like() {
        Some(table) if !table.is_empty() && holds_tables(base.as_table(), path) => {
            for (key, sub) in table.iter() {
                path.push(key.to_owned());
                accept::<T>(base, merged, path, sub, resets);
                path.pop();
            }
        }
        _ => resets.push(Reset { path: path.clone(), message: error.to_string() }),
    }
}

/// True when every key along `path` in `table` is a table or missing, so values can be put under it.
fn holds_tables(table: &dyn TableLike, path: &[String]) -> bool {
    let Some((key, rest)) = path.split_first() else { return true };
    match table.get(key) {
        None => true,
        Some(item) => item.as_table_like().is_some_and(|sub| holds_tables(sub, rest)),
    }
}

/// Sets `item` at `path` in `table`, adding the tables along it that are missing.
fn place(table: &mut dyn TableLike, path: &[String], item: Item) {
    let Some((key, rest)) = path.split_first() else { return };
    if rest.is_empty() {
        table.insert(key, item);
        return;
    }
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    if let Some(sub) = table.get_mut(key).and_then(Item::as_table_like_mut) {
        place(sub, rest, item);
    }
}

fn remove_at(table: &mut dyn TableLike, path: &[String]) {
    match path {
        [] => {}
        [key] => {
            table.remove(key);
        }
        [key, rest @ ..] => {
            if let Some(sub) = table.get_mut(key).and_then(Item::as_table_like_mut) {
                remove_at(sub, rest);
            }
        }
    }
}

/// What `T` reads from `doc`, written back as a document: the keys it knows, each with the value it took (a value it
/// reset as its default, a key `doc` lacks as its default). Keys `T` does not know are not in it.
fn known<T: Settings>(doc: &DocumentMut) -> Option<DocumentMut> {
    let (value, _) = read::<T>(doc);
    let mut known = toml_edit::ser::to_document(&value).ok()?;
    expand_inline_tables(known.as_table_mut());
    Some(known)
}

/// Writes `value` into `doc`, keeping comments, key order and keys `T` does not know.
///
/// Tables merge key by key; arrays, arrays of tables and scalars are replaced whole. A key `T` reads from `doc` that
/// `value` no longer writes (a map's removed entry, an option set to none) is removed.
pub fn merge_into<T: Settings>(doc: &mut DocumentMut, value: &T) -> Result<(), toml_edit::ser::Error> {
    let mut fresh = toml_edit::ser::to_document(value)?;
    expand_inline_tables(fresh.as_table_mut());
    if let Some(known) = known::<T>(doc) {
        remove_dropped(doc.as_table_mut(), known.as_table(), fresh.as_table());
    }
    merge_tables(doc.as_table_mut(), fresh.as_table());
    Ok(())
}

/// Removes from `doc` each key of `known` that `fresh` lacks, table by table.
fn remove_dropped(doc: &mut dyn TableLike, known: &dyn TableLike, fresh: &dyn TableLike) {
    for (key, known_item) in known.iter() {
        let Some(fresh_item) = fresh.get(key) else {
            doc.remove(key);
            continue;
        };
        if let (Some(known), Some(fresh), Some(doc)) =
            (known_item.as_table_like(), fresh_item.as_table_like(), doc.get_mut(key).and_then(Item::as_table_like_mut))
        {
            remove_dropped(doc, known, fresh);
        }
    }
}

/// Serde writes nested structs as inline tables (`general = { … }`); files are nicer to read
/// and edit as `[general]` sections, so new sections are converted before merging.
pub(crate) fn expand_inline_tables(table: &mut Table) {
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_owned()).collect();
    for key in keys {
        let Some(item) = table.get_mut(&key) else { continue };
        if let Some(inline) = item.as_inline_table() {
            *item = Item::Table(inline.clone().into_table());
        }
        if let Some(sub) = item.as_table_mut() {
            expand_inline_tables(sub);
        }
    }
}

fn merge_tables(into: &mut dyn TableLike, from: &dyn TableLike) {
    for (key, item) in from.iter() {
        match into.get_mut(key) {
            Some(existing) if existing.is_table_like() && item.is_table_like() => {
                if let (Some(existing), Some(fresh)) = (existing.as_table_like_mut(), item.as_table_like()) {
                    merge_tables(existing, fresh);
                }
            }
            Some(existing) => {
                // Keep the spacing and trailing comment of the line being replaced.
                let mut fresh = item.clone();
                if let (Some(old), Some(new)) = (existing.as_value(), fresh.as_value_mut()) {
                    *new.decor_mut() = old.decor().clone();
                }
                *existing = fresh;
            }
            None => {
                into.insert(key, item.clone());
            }
        }
    }
}

/// Merges this instance's changes into the file another instance wrote (gap 31, PhotoCraft's
/// multi-instance merge): `base` is the document as this store last read or wrote it, `ours` what
/// it would write now, `theirs` the file on disk, which becomes the result.
///
/// - `ours == base`: nothing changes; `theirs` is kept.
/// - `base` counts as `T` reads it: a key it lacks holds its default, so a section an older file
///   did not have is not taken as all of this instance's change.
/// - Each changed item merges by the rule [`Settings::merge_rule`] names for its key path. Under
///   [`MergeRule::ByKey`], the default, tables merge key by key: a key in both `base` and `theirs`
///   recurses; a key removed on disk and unchanged here stays removed; any other key of `ours` is
///   written (`ours` wins at the leaf, scalars and arrays replaced whole); a key `base` had and
///   `ours` dropped is removed. [`MergeRule::Whole`], [`MergeRule::Recent`] and
///   [`MergeRule::Entries`] say how their item merges instead.
/// - Comments and formatting of `theirs` stay, as in [`merge_into`].
///
/// When the disk's `schema_version` is newer than `base`'s, `base` is migrated to it with
/// [`Settings::migrate`] first, so a migration this instance ran is not written over another
/// instance's choice (PhotoCraft's `without_stale_migration`).
// pc: crates/ui-egui/src/prefs/prefs_ui/save.rs:151 merge_changes, :134 without_stale_migration, :115 write_prefs
pub fn merge_changes<T: Settings>(base: &DocumentMut, ours: &DocumentMut, theirs: &mut DocumentMut) {
    if same_table(base.as_table(), ours.as_table()) {
        return;
    }
    let base = as_read::<T>(&migrated_base::<T>(base, theirs));
    merge_changed_tables::<T>(base.as_table(), ours.as_table(), theirs.as_table_mut(), &mut Vec::new());
}

/// `doc` with each key `T` reads and `doc` lacks filled in as `T` reads it.
fn as_read<T: Settings>(doc: &DocumentMut) -> DocumentMut {
    let Some(mut filled) = known::<T>(doc) else {
        return doc.clone();
    };
    merge_tables(filled.as_table_mut(), doc.as_table());
    filled
}

/// `base` migrated up to the version `theirs` has, when `theirs` is newer; otherwise `base`.
fn migrated_base<T: Settings>(base: &DocumentMut, theirs: &DocumentMut) -> DocumentMut {
    let (Some(from), Some(to)) = (schema_version(base), schema_version(theirs)) else {
        return base.clone();
    };
    let mut migrated = base.clone();
    for step in from..to.min(T::SCHEMA_VERSION) {
        if T::migrate(step, &mut migrated).is_err() {
            return base.clone();
        }
        set_schema_version(&mut migrated, step + 1);
    }
    migrated
}

/// Merges the keys of `ours` that changed since `base` into `theirs`; `path` is the key path of the tables, so
/// each item's [`MergeRule`] can be asked.
fn merge_changed_tables<T: Settings>(base: &dyn TableLike, ours: &dyn TableLike, theirs: &mut dyn TableLike, path: &mut Vec<String>) {
    for (key, our_item) in ours.iter() {
        path.push(key.to_owned());
        match (base.get(key), theirs.get_mut(key)) {
            (Some(base_item), Some(their_item)) => merge_changed_items::<T>(base_item, our_item, their_item, path),
            // Removed on disk and unchanged here: it stays removed.
            (Some(base_item), None) if same_item(base_item, our_item) => {}
            // A key neither `base` nor `T`'s defaults have (a map entry both instances added): merged against nothing,
            // so under `ByKey` every leaf of ours is ours and a list keeps the entries of both.
            (None, Some(their_item)) => merge_changed_items::<T>(&empty_like(our_item), our_item, their_item, path),
            (_, None) => {
                theirs.insert(key, our_item.clone());
            }
        }
        path.pop();
    }
    let removed: Vec<String> = base.iter().map(|(k, _)| k).filter(|k| !ours.contains_key(k)).map(str::to_owned).collect();
    for key in removed {
        theirs.remove(&key);
    }
}

/// Merges the item at `path` by its [`MergeRule`] when ours changed it since `base`.
fn merge_changed_items<T: Settings>(base: &Item, ours: &Item, theirs: &mut Item, path: &mut Vec<String>) {
    if same_item(base, ours) {
        return;
    }
    let rule = {
        let keys: Vec<&str> = path.iter().map(String::as_str).collect();
        T::merge_rule(&keys)
    };
    match rule {
        MergeRule::ByKey => match (base.as_table_like(), ours.as_table_like(), theirs.as_table_like_mut()) {
            (Some(base), Some(ours), Some(theirs)) => merge_changed_tables::<T>(base, ours, theirs, path),
            _ => replace_item(theirs, ours),
        },
        MergeRule::Whole => replace_item(theirs, ours),
        MergeRule::Recent { cap, key } => merge_lists(base, ours, theirs, |b, o, t| merge_recent(b, o, t, cap, key)),
        MergeRule::Entries { key } => merge_lists(base, ours, theirs, |b, o, t| merge_entries(b, o, t, key)),
    }
}

/// An empty item of `item`'s kind: the base of a key this instance and another both added.
fn empty_like(item: &Item) -> Item {
    match item {
        Item::ArrayOfTables(_) => Item::ArrayOfTables(ArrayOfTables::new()),
        Item::Value(Value::Array(_)) => Item::Value(Value::Array(Array::new())),
        _ if item.is_table_like() => Item::Table(Table::new()),
        _ => Item::None,
    }
}

/// Merges three lists with `merge`, writing the result over `theirs` in the shape of `ours`. When `theirs` is not a
/// list (a hand edit put something else there), ours replaces it; the base of a list `base` lacks is empty.
fn merge_lists(base: &Item, ours: &Item, theirs: &mut Item, merge: impl FnOnce(&[Value], &[Value], &[Value]) -> Vec<Value>) {
    let (Some(our_list), Some(their_list)) = (list(ours), list(theirs)) else {
        replace_item(theirs, ours);
        return;
    };
    let base_list = list(base).unwrap_or_default();
    let merged = list_item(ours, merge(&base_list, &our_list, &their_list));
    replace_item(theirs, &merged);
}

/// The entries of an array or an array of tables, each as a value (a table as an inline table).
fn list(item: &Item) -> Option<Vec<Value>> {
    match item {
        Item::Value(Value::Array(array)) => Some(array.iter().cloned().collect()),
        Item::ArrayOfTables(tables) => Some(tables.iter().map(|t| Value::InlineTable(t.clone().into_inline_table())).collect()),
        _ => None,
    }
}

/// `values` as an item shaped like `shape`: an array of tables stays one while every entry is a table.
fn list_item(shape: &Item, values: Vec<Value>) -> Item {
    if shape.is_array_of_tables() {
        let tables: Option<Vec<Table>> = values.iter().map(|v| v.as_inline_table().map(|t| t.clone().into_table())).collect();
        if let Some(tables) = tables {
            let mut array = ArrayOfTables::new();
            for table in tables {
                array.push(table);
            }
            return Item::ArrayOfTables(array);
        }
    }
    let mut array: Array = values.into_iter().collect();
    // Entries come from two documents with their own spacing; lay the merged list out afresh.
    array.fmt();
    Item::Value(Value::Array(array))
}

/// The identity of a list entry; `None` for an entry that has none, which a merge drops.
type EntryKey = fn(&Value) -> Option<String>;

/// The entries of `list` that have a key, with it. An entry without one cannot be matched across the lists and is
/// dropped, so a value the settings type cannot read is not carried into the merged list.
fn keyed(list: &[Value], key: EntryKey) -> Vec<(String, Value)> {
    list.iter().filter_map(|v| Some((key(v)?, v.clone()))).collect()
}

/// A most-recently-used list: the entries this instance put at the front of `ours` ([`touched_len`]), then those
/// of `theirs` it did not remove (an entry of `base` no longer in `ours`), without duplicates and at most `cap`.
fn merge_recent(base: &[Value], ours: &[Value], theirs: &[Value], cap: usize, key: EntryKey) -> Vec<Value> {
    let (base, ours, theirs) = (keyed(base, key), keyed(ours, key), keyed(theirs, key));
    let has = |list: &[(String, Value)], k: &str| list.iter().any(|(x, _)| x == k);
    let removed = |k: &str| has(&base, k) && !has(&ours, k);
    let touched = &ours[..touched_len(&base, &ours)];
    let mut merged: Vec<(String, Value)> = Vec::new();
    for (k, v) in touched.iter().chain(theirs.iter().filter(|(k, _)| !removed(k))) {
        if merged.len() >= cap {
            break;
        }
        if !has(&merged, k) {
            merged.push((k.clone(), v.clone()));
        }
    }
    merged.into_iter().map(|(_, v)| v).collect()
}

/// How many of `ours`'s first entries this instance added or used again. Each use moves an entry to the front of a
/// recent list, so what it left alone is a tail of `ours` in `base`'s order: the longest such tail is untouched, and
/// everything before it (new entries, and entries that moved ahead of one loaded before them) was touched. A use
/// that leaves the order as loaded (the first entry used again) cannot be told from no use, and merges as none.
fn touched_len(base: &[(String, Value)], ours: &[(String, Value)]) -> usize {
    let mut end = base.len();
    for (i, (k, _)) in ours.iter().enumerate().rev() {
        match base[..end].iter().rposition(|(b, _)| b == k) {
            Some(at) => end = at,
            None => return i + 1,
        }
    }
    0
}

/// A list entry: its key, and how many entries before it have the same key, so equal entries stay apart by order.
type EntryId = (String, usize);

/// The keyed entries of `list` by [`EntryId`], in order.
fn entry_ids(list: &[Value], key: EntryKey) -> Vec<(EntryId, Value)> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    keyed(list, key)
        .into_iter()
        .map(|(k, v)| {
            let n = seen.entry(k.clone()).or_default();
            let id = (k, *n);
            *n += 1;
            (id, v)
        })
        .collect()
}

/// The three lists of an `Entries` merge, by entry.
struct EntryLists {
    base: HashMap<EntryId, Value>,
    ours: HashMap<EntryId, Value>,
    theirs: HashMap<EntryId, Value>,
}

impl EntryLists {
    /// The merged value of entry `id`: added or changed here wins, removed here stays removed, and an entry left
    /// alone here is what `theirs` holds (gone when the other instance removed it).
    fn value(&self, id: &EntryId) -> Option<&Value> {
        match (self.base.get(id), self.ours.get(id), self.theirs.get(id)) {
            (Some(b), Some(o), t) if same_value(b, o) => t,
            (_, Some(o), _) => Some(o),
            (Some(_), None, _) => None,
            (None, None, t) => t,
        }
    }
}

/// A list of user entries, entry by entry: entries either instance added are kept, entries either removed stay
/// removed, and an entry this instance changed takes its version. The order is ours when this instance reordered
/// the entries it loaded, else theirs; an entry only the other list has goes after the nearest entry before it there
/// that the merge keeps (at the start when there is none).
fn merge_entries(base: &[Value], ours: &[Value], theirs: &[Value], key: EntryKey) -> Vec<Value> {
    let (base_ids, ours_ids, theirs_ids) = (entry_ids(base, key), entry_ids(ours, key), entry_ids(theirs, key));
    let ids = |list: &[(EntryId, Value)]| -> Vec<EntryId> { list.iter().map(|(id, _)| id.clone()).collect() };
    let (base_order, ours_order, theirs_order) = (ids(&base_ids), ids(&ours_ids), ids(&theirs_ids));
    let lists = EntryLists { base: base_ids.into_iter().collect(), ours: ours_ids.into_iter().collect(), theirs: theirs_ids.into_iter().collect() };
    let common =
        |order: &[EntryId], other: &HashMap<EntryId, Value>| -> Vec<EntryId> { order.iter().filter(|id| other.contains_key(*id)).cloned().collect() };
    let reordered = common(&ours_order, &lists.base) != common(&base_order, &lists.ours);
    let (first, second) = if reordered { (&ours_order, &theirs_order) } else { (&theirs_order, &ours_order) };
    anchored(&lists, first, second)
}

/// `first`'s kept entries in order, each followed by the entries only `second` has that come after it there.
fn anchored(lists: &EntryLists, first: &[EntryId], second: &[EntryId]) -> Vec<Value> {
    let in_first: HashSet<&EntryId> = first.iter().collect();
    let kept: HashSet<&EntryId> = first.iter().filter(|id| lists.value(id).is_some()).collect();
    let mut after: HashMap<Option<&EntryId>, Vec<&EntryId>> = HashMap::new();
    let mut anchor = None;
    for id in second {
        if kept.contains(id) {
            anchor = Some(id);
        } else if !in_first.contains(id) && lists.value(id).is_some() {
            after.entry(anchor).or_default().push(id);
        }
    }
    let mut merged = Vec::new();
    let mut emit = |ids: &[&EntryId]| merged.extend(ids.iter().filter_map(|id| lists.value(id).cloned()));
    emit(after.get(&None).map_or(&[], Vec::as_slice));
    for id in first.iter().filter(|id| kept.contains(id)) {
        emit(&[id]);
        emit(after.get(&Some(id)).map_or(&[], Vec::as_slice));
    }
    merged
}

/// Replaces `theirs` with `ours`, keeping the spacing and trailing comment of a replaced value.
fn replace_item(theirs: &mut Item, ours: &Item) {
    let mut fresh = ours.clone();
    if let (Some(old), Some(new)) = (theirs.as_value(), fresh.as_value_mut()) {
        *new.decor_mut() = old.decor().clone();
    }
    *theirs = fresh;
}

/// Equal in content, ignoring comments, spacing and key order.
fn same_table(a: &dyn TableLike, b: &dyn TableLike) -> bool {
    a.len() == b.len() && a.iter().all(|(k, ai)| b.get(k).is_some_and(|bi| same_item(ai, bi)))
}

fn same_item(a: &Item, b: &Item) -> bool {
    if let (Some(a), Some(b)) = (a.as_table_like(), b.as_table_like()) {
        return same_table(a, b);
    }
    match (a, b) {
        (Item::Value(a), Item::Value(b)) => same_value(a, b),
        (Item::ArrayOfTables(a), Item::ArrayOfTables(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same_table(a, b)),
        _ => false,
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a.value() == b.value(),
        (Value::Integer(a), Value::Integer(b)) => a.value() == b.value(),
        (Value::Float(a), Value::Float(b)) => a.value() == b.value(),
        (Value::Boolean(a), Value::Boolean(b)) => a.value() == b.value(),
        (Value::Datetime(a), Value::Datetime(b)) => a.value() == b.value(),
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same_value(a, b)),
        (Value::InlineTable(a), Value::InlineTable(b)) => same_table(a, b),
        _ => false,
    }
}

/// Renders `doc` as text.
pub fn render(doc: &DocumentMut) -> String {
    doc.to_string()
}
