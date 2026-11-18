//! A file a newer build saved: it opens, with a warning, and saving it loses nothing.
//!
//! A newer format number, or a change the file names that this build doesn't have, says a newer
//! build saved the file ([`crate::Family::newer`]). It may use nothing this build lacks, so the app
//! reads it as usual. When part of it can't be read, that part stays out of the way and the rest
//! opens: the app reads the elements of its lists it can ([`NewerFile::keep_element`]), then any
//! other top-level section it can't read is left out whole ([`leave_out_unreadable`],
//! [`read_around`]), and a field it doesn't know, which typed reading would drop, is noticed by
//! comparing the file with what was read ([`NewerFile::compare`]).
//!
//! All of these are named in [`NewerFile::not_understood`] and go back into the file when it is
//! saved ([`NewerFile::restore_into`]), as they came. What can't be put back because the file was
//! changed around it is what `restore_into` returns.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::Newer;

/// The most sections left out one at a time to find the one a file can't be read with.
const MAX_SECTION_TRIES: usize = 64;

/// What the file says and holds that this build doesn't read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewerFile {
    /// The app, as the notice names it.
    pub app: &'static str,
    /// The number the file says.
    pub format: u32,
    /// The number this build reads up to.
    pub reads_up_to: u32,
    /// The changes the file names that this build doesn't have.
    pub unknown_changes: Vec<String>,
    /// What this build couldn't read, by name.
    pub not_understood: Vec<String>,
    /// Fields known records lost in typed reading.
    kept: Option<Kept>,
    /// Top-level sections left out whole.
    sections: Vec<Section>,
    /// Elements left out of a top-level list: its key and the element.
    elements: Vec<(String, Value)>,
    /// The parts of a container this build has no use for: name and bytes. A blob only one of the
    /// kept pieces refers to is one of them.
    pub parts: Vec<(String, Arc<[u8]>)>,
    /// The file's `container.json`, when this build doesn't know what it says.
    pub container: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
struct Section {
    key: String,
    value: Value,
    /// This build's record of the section, without the file's: while it is unchanged, the file's
    /// goes back.
    empty: Option<Value>,
}

/// The fields of a record that typed reading dropped.
#[derive(Clone, Debug, PartialEq)]
enum Kept {
    Field(Value),
    Fields(BTreeMap<String, Kept>),
    Items(Vec<(Anchor, Kept)>),
}

/// Which element of a list some kept fields belong to.
#[derive(Clone, Debug, PartialEq)]
enum Anchor {
    Id(u64),
    Name(String),
    /// Its place, good only while the list keeps its length.
    Index {
        at: usize,
        of: usize,
    },
}

impl NewerFile {
    /// A file of `app` that `newer` says a newer build saved, nothing left out of it yet.
    pub fn new(app: &'static str, newer: &Newer) -> NewerFile {
        NewerFile {
            app,
            format: newer.format,
            reads_up_to: newer.reads_up_to,
            unknown_changes: newer.unknown_changes.clone(),
            ..NewerFile::default()
        }
    }

    /// What to tell the user, whatever else is said.
    pub fn notice(&self) -> String {
        let (app, reads, format) = (self.app, self.reads_up_to, self.format);
        match self.unknown_changes.first() {
            Some(change) if format <= reads => {
                format!("Saved by a newer {app} (format {format} with `{change}`, which this version doesn't know). If something's missing, update.")
            }
            _ => format!("Saved by a newer {app} (format {format}, this version reads up to {reads}). If something's missing, update."),
        }
    }

    /// Every line to report: the notice, then what couldn't be read, by name.
    pub fn warnings(&self) -> Vec<String> {
        let unread = self.not_understood.iter().map(|what| format!("Not read: {what}. Saving keeps it."));
        std::iter::once(self.notice()).chain(unread).collect()
    }

    /// What a reply says about it.
    pub fn report(&self) -> Value {
        json!({
            "format": self.format,
            "reads_up_to": self.reads_up_to,
            "unknown_changes": self.unknown_changes,
            "not_understood": self.not_understood,
            "notice": self.notice(),
        })
    }

    /// Whether the next save needs a container to keep what the file holds: plain JSON has no room
    /// for parts beside the file's own, nor for the container's own `container.json`.
    pub fn needs_container(&self) -> bool {
        !self.parts.is_empty() || self.container.is_some()
    }

    /// Whether anything kept back names this text (a blob's name in a container).
    pub fn mentions(&self, text: &str) -> bool {
        fn kept(k: &Kept, text: &str) -> bool {
            match k {
                Kept::Field(v) => value_mentions(v, text),
                Kept::Fields(m) => m.values().any(|x| kept(x, text)),
                Kept::Items(l) => l.iter().any(|(_, x)| kept(x, text)),
            }
        }
        self.kept.as_ref().is_some_and(|k| kept(k, text))
            || self.sections.iter().any(|s| value_mentions(&s.value, text))
            || self.elements.iter().any(|(_, v)| value_mentions(v, text))
    }

    /// Say that a section the reader couldn't finish reading is left out. `empty` is what the
    /// app writes for the section while it holds nothing, if anything.
    pub fn keep_section(&mut self, key: &str, value: Value, empty: Option<Value>, why: &str) {
        self.not_understood.push(format!("section `{key}` ({why})"));
        self.sections.push(Section { key: key.into(), value, empty });
    }

    /// Say that an element of the top-level list `key` is left out, as `note` names it; it goes
    /// back at the end of the list when the file is saved.
    pub fn keep_element(&mut self, key: &str, element: Value, note: String) {
        self.not_understood.push(note);
        self.elements.push((key.into(), element));
    }

    /// Compare the file as it reads (`file`, after the same upgrade as what was read) with what was
    /// read of it (`known`, its typed record written out): what `known` lacks is kept and named.
    /// An object whose `type` is `placeholder_tag` (an element the app kept whole) is not
    /// compared; nor are these top-level fields: those in `container_only` (what a container adds
    /// beside the file's own), `format` and `format_pending`, and the sections kept whole (they go
    /// back whole).
    pub fn compare(&mut self, file: &Value, known: &Value, placeholder_tag: &str, container_only: &[&str]) {
        let mut paths = Vec::new();
        let mut top_only: Vec<&str> = container_only.to_vec();
        top_only.extend(["format", "format_pending"]);
        top_only.extend(self.sections.iter().map(|s| s.key.as_str()));
        // a list typed writing leaves out holds only elements kept on their own: not kept twice
        top_only.extend(self.elements.iter().map(|(key, _)| key.as_str()).filter(|key| known.get(*key).is_none()));
        let kept = vanished(file, known, "", &mut paths, placeholder_tag, &top_only);
        self.kept = kept;
        self.not_understood.extend(paths);
        for section in &mut self.sections {
            section.empty = known.get(&section.key).cloned();
        }
    }

    /// Write back what was kept of the file into `value`, the file as this build writes it.
    /// Returns what couldn't go back, because the file was changed around it.
    pub fn restore_into(&self, value: &mut Value) -> Vec<String> {
        let mut lost = Vec::new();
        if let Some(kept) = &self.kept {
            apply(kept, value, "", &mut lost);
        }
        if let Some(top) = value.as_object_mut() {
            for section in &self.sections {
                if top.get(&section.key) == section.empty.as_ref() {
                    top.insert(section.key.clone(), section.value.clone());
                } else {
                    lost.push(format!("section `{}`, replaced by what was edited here", section.key));
                }
            }
            // each list's records indexed once, by the identity records are matched by
            let mut indexes: HashMap<&str, Positions> = HashMap::new();
            for (key, element) in &self.elements {
                let name = element.get("name");
                let anchor = anchor_of(element);
                match top.entry(key.clone()).or_insert_with(|| json!([])) {
                    Value::Array(list) => {
                        let index = indexes.entry(key.as_str()).or_insert_with(|| Positions::of(list));
                        if anchor.as_ref().is_some_and(|a| index.find(list.len(), a).is_some()) {
                            lost.push(format!(
                                "`{key}` element {}, replaced by what was made here",
                                name.and_then(Value::as_str).unwrap_or("unnamed")
                            ));
                        } else {
                            index.add(element, list.len());
                            list.push(element.clone());
                        }
                    }
                    _ => lost.push(format!("`{key}` element {}, replaced by what was made here", name.and_then(Value::as_str).unwrap_or("unnamed"))),
                }
            }
        }
        lost
    }
}

/// Whether a string anywhere in `v` is `text`; iterative, so no nesting depth overflows the stack.
fn value_mentions(v: &Value, text: &str) -> bool {
    let mut todo = vec![v];
    while let Some(v) = todo.pop() {
        match v {
            Value::String(s) if s == text => return true,
            Value::Array(a) => todo.extend(a),
            Value::Object(o) => todo.extend(o.values()),
            _ => {}
        }
    }
    false
}

/// The first line of a reader's error, without the list of every kind it knows, and without the
/// `"<noun> file: "` its reader starts it with.
pub fn short<'a>(why: &'a str, noun: &str) -> &'a str {
    let why = why.split(", expected").next().unwrap_or(why);
    why.strip_prefix(&format!("{noun} file: ")).unwrap_or(why)
}

/// Leave a section out and keep it. A section the file needs is left empty.
fn move_out(top: &mut Map<String, Value>, key: &str, why: &str, noun: &str, file: &mut NewerFile) {
    let Some(value) = top.remove(key) else { return };
    file.not_understood.push(format!("section `{key}` ({})", short(why, noun)));
    file.sections.push(Section { key: key.into(), value, empty: None });
}

/// Every top-level section of `top` that doesn't read by itself (`piece`, the app's reading of a
/// file holding just that section) is left out and kept in `file`. `noun` is what a file of the
/// app's is called, which its reader's errors start with.
pub fn leave_out_unreadable<E: std::fmt::Display>(
    top: &mut Map<String, Value>,
    file: &mut NewerFile,
    noun: &str,
    piece: impl Fn(&str, &Value) -> Result<(), E>,
) {
    let keys: Vec<String> = top.keys().cloned().collect();
    for key in &keys {
        let value = top.get(key).cloned().unwrap_or_default();
        if let Err(why) = piece(key, &value) {
            move_out(top, key, &why.to_string(), noun, file);
        }
    }
}

/// Read `top` with `attempt`, leaving out the sections it can't be read with: if it doesn't read,
/// each section but the `core` ones (which the file can't be read without) is tried left out alone,
/// and as a last resort every one of them is. What is left out is kept in `file`. The error is the
/// first attempt's, when even the core alone doesn't read.
pub fn read_around<D, E: std::fmt::Display>(
    top: &mut Map<String, Value>,
    file: &mut NewerFile,
    core: &[&str],
    noun: &str,
    attempt: impl Fn(&Map<String, Value>) -> Result<D, E>,
) -> Result<D, String> {
    let first = match attempt(top) {
        Ok(doc) => return Ok(doc),
        Err(why) => why.to_string(),
    };
    // Sections that are fine by themselves but not together with the rest.
    let keys: Vec<String> = top.keys().filter(|k| !core.contains(&k.as_str())).cloned().collect();
    // Each try reads the whole file: a file of many sections goes straight to the last resort.
    for key in keys.iter().filter(|_| keys.len() <= MAX_SECTION_TRIES) {
        let mut without = top.clone();
        without.remove(key);
        if let Ok(doc) = attempt(&without) {
            move_out(top, key, &first, noun, file);
            return Ok(doc);
        }
    }
    let mut bare = top.clone();
    bare.retain(|key, _| core.contains(&key.as_str()));
    match attempt(&bare) {
        Ok(doc) => {
            for key in &keys {
                move_out(top, key, &first, noun, file);
            }
            Ok(doc)
        }
        Err(_) => Err(first),
    }
}

/// A value a writer leaves out without losing anything: not worth telling anyone about.
pub fn looks_default(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

/// What `raw` has that `known`, what was read of it, doesn't. `paths` gets those fields' names.
/// `top_only` are top-level keys left out of the comparison.
fn vanished(raw: &Value, known: &Value, path: &str, paths: &mut Vec<String>, placeholder_tag: &str, top_only: &[&str]) -> Option<Kept> {
    match (raw, known) {
        (Value::Object(raw), Value::Object(known)) => {
            if known.get("type").and_then(Value::as_str) == Some(placeholder_tag) {
                return None;
            }
            let mut kept = BTreeMap::new();
            for (key, value) in raw {
                if path.is_empty() && top_only.contains(&key.as_str()) {
                    continue;
                }
                let inner = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                match known.get(key) {
                    None => {
                        if !looks_default(value) {
                            paths.push(format!("field `{inner}`"));
                        }
                        kept.insert(key.clone(), Kept::Field(value.clone()));
                    }
                    Some(known) => {
                        if let Some(k) = vanished(value, known, &inner, paths, placeholder_tag, top_only) {
                            kept.insert(key.clone(), k);
                        }
                    }
                }
            }
            (!kept.is_empty()).then_some(Kept::Fields(kept))
        }
        (Value::Array(raw), Value::Array(known)) => {
            let mut items = Vec::new();
            let positions = Positions::of(known);
            for (at, element) in raw.iter().enumerate() {
                // an identity typed reading kept: an id it dropped falls back to the name
                let anchor = surviving_anchor(element, &positions).unwrap_or(Anchor::Index { at, of: raw.len() });
                let Some(partner) = positions.find(known.len(), &anchor).and_then(|i| known.get(i)) else { continue };
                let label = match (element.get("name").and_then(Value::as_str), &anchor) {
                    (Some(name), _) => format!("{path}[{name}]"),
                    (None, Anchor::Id(id)) => format!("{path}[{id}]"),
                    _ => format!("{path}[{at}]"),
                };
                if let Some(k) = vanished(element, partner, &label, paths, placeholder_tag, top_only) {
                    items.push((anchor, k));
                }
            }
            (!items.is_empty()).then_some(Kept::Items(items))
        }
        _ => None,
    }
}

fn anchor_of(element: &Value) -> Option<Anchor> {
    if let Some(id) = element.get("id").and_then(Value::as_u64) {
        return Some(Anchor::Id(id));
    }
    element.get("name").and_then(Value::as_str).map(|name| Anchor::Name(name.to_string()))
}

/// The identity of `element` that `known`'s records still carry: its id, or else its name. When
/// typed reading kept neither, there is none, and the record is matched by its place.
fn surviving_anchor(element: &Value, known: &Positions) -> Option<Anchor> {
    let id = element.get("id").and_then(Value::as_u64).map(Anchor::Id);
    let name = element.get("name").and_then(Value::as_str).map(|n| Anchor::Name(n.to_string()));
    [id, name].into_iter().flatten().find(|anchor| known.find(usize::MAX, anchor).is_some())
}

/// Where each id and name first appears in a list, so matching records is not a scan per record.
struct Positions {
    ids: HashMap<u64, usize>,
    names: HashMap<String, usize>,
}

impl Positions {
    fn of(list: &[Value]) -> Positions {
        let (mut ids, mut names) = (HashMap::new(), HashMap::new());
        for (i, x) in list.iter().enumerate() {
            if let Some(id) = x.get("id").and_then(Value::as_u64) {
                ids.entry(id).or_insert(i);
            }
            if let Some(name) = x.get("name").and_then(Value::as_str) {
                names.entry(name.to_string()).or_insert(i);
            }
        }
        Positions { ids, names }
    }

    /// Count `x`, now at `i`, in.
    fn add(&mut self, x: &Value, i: usize) {
        if let Some(id) = x.get("id").and_then(Value::as_u64) {
            self.ids.entry(id).or_insert(i);
        }
        if let Some(name) = x.get("name").and_then(Value::as_str) {
            self.names.entry(name.to_string()).or_insert(i);
        }
    }

    /// The position of the record `anchor` names in a list of `len` records (an index anchor
    /// only while the list keeps its length).
    fn find(&self, len: usize, anchor: &Anchor) -> Option<usize> {
        match anchor {
            Anchor::Id(id) => self.ids.get(id).copied(),
            Anchor::Name(name) => self.names.get(name).copied(),
            Anchor::Index { at, of } => (len == *of && at < of).then_some(*at),
        }
    }
}

/// Put the kept fields back. An element that is gone was deleted here, its fields with it; one
/// whose place can't be told any more, and a field whose record is gone, go into `lost`.
fn apply(kept: &Kept, value: &mut Value, path: &str, lost: &mut Vec<String>) {
    match kept {
        Kept::Field(_) => {}
        Kept::Fields(fields) => {
            let Some(object) = value.as_object_mut() else {
                lost.push(format!("field `{path}`"));
                return;
            };
            for (key, sub) in fields {
                let inner = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                match (sub, object.get_mut(key)) {
                    (Kept::Field(v), None) => {
                        object.insert(key.clone(), v.clone());
                    }
                    // the edited value stays; a retained one it replaced is reported
                    (Kept::Field(v), Some(existing)) => {
                        if existing != v {
                            lost.push(format!("field `{inner}`"));
                        }
                    }
                    (sub, Some(inner_value)) => apply(sub, inner_value, &inner, lost),
                    // The list it was in is empty and not written: its elements were deleted.
                    (Kept::Items(_), None) => {}
                    (_, None) => lost.push(format!("field `{inner}`")),
                }
            }
        }
        Kept::Items(items) => {
            let Some(list) = value.as_array_mut() else {
                lost.push(format!("field `{path}`"));
                return;
            };
            let positions = Positions::of(list);
            let len = list.len();
            for (anchor, sub) in items {
                match (positions.find(len, anchor).and_then(|i| list.get_mut(i)), anchor) {
                    (Some(element), _) => apply(sub, element, path, lost),
                    (None, Anchor::Index { at, .. }) => lost.push(format!("element {at} of `{path}`")),
                    (None, _) => {}
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "newer_tests.rs"]
mod tests;
