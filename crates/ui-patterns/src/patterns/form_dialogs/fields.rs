//! [`DialogFields`]: a dialog's values by key, so scripts and tests set and read them like a user, the dialog
//! reopens with its last-used values, and Reset restores the opening ones.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// One dialog value. The variants are the kinds a [`Param`](super::Param) produces; a dialog with
/// its own body uses the same map with the kinds it needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum FieldValue {
    /// A check box or a toggle.
    Bool(bool),
    /// Integer fields, grid cells, counts. Before `Number` so an untagged read keeps an integer
    /// an integer.
    Int(i64),
    /// Sliders, angles, resolutions: any real number.
    Number(f64),
    /// Free text and the id of a [`ParamKind::Choice`](super::ParamKind::Choice) option.
    Text(String),
    /// sRGB components in 0..=1, as a colour parameter is kept (`#rrggbb` is these times 255).
    Color([f32; 3]),
    /// A 2D point in the parameter's own space (percent of the image for a blur centre).
    Point([f64; 2]),
    /// A row-major grid of integers (Custom filter's 5×5 kernel).
    Grid(Vec<i64>),
    /// A list of values of any kind: Color Table's colours, Replace Color's sampled points. Last
    /// so an untagged read keeps the fixed-shape variants above; a list of integers or of two or
    /// three numbers reads back as `Grid`, `Point` or `Color`, which [`DialogFields::list`] turns
    /// back into the list.
    List(Vec<FieldValue>),
    /// Any JSON value: what a [`ParamKind::Custom`](super::ParamKind::Custom) row keeps under its key when its value is
    /// structured (Blur Gallery's pins, `[{x, y, blur, …}]`). Last so an untagged read keeps the fixed-shape variants
    /// above: an array of objects reads back as a `List` of `Json`, so a reader of the key takes that form as well as
    /// `Json` of an array.
    Json(serde_json::Value),
}

impl From<bool> for FieldValue {
    fn from(value: bool) -> Self {
        FieldValue::Bool(value)
    }
}

impl From<f64> for FieldValue {
    fn from(value: f64) -> Self {
        FieldValue::Number(value)
    }
}

impl From<i64> for FieldValue {
    fn from(value: i64) -> Self {
        FieldValue::Int(value)
    }
}

impl From<&str> for FieldValue {
    fn from(value: &str) -> Self {
        FieldValue::Text(value.to_owned())
    }
}

impl From<String> for FieldValue {
    fn from(value: String) -> Self {
        FieldValue::Text(value)
    }
}

/// The values of one dialog, keyed by the parameter key (`"radius"`, `"method"`). A dialog body
/// reads and writes these; automation sets them before OK and reads them after; a test asserts
/// them. Keys never start with `__`: the chrome keeps nothing in the map.
///
/// - [`snapshot`](DialogFields::snapshot) is taken when the dialog opens;
///   [`reset`](DialogFields::reset) restores it (Alt+Cancel, Photoshop).
/// - It serializes as the plain map (`{radius = 4.0, method = "gaussian"}`).
/// - A parameter dialog that remembers its values stores the map after the app's command succeeds and loads
///   it on open, then drops the keys the current parameter list does not have or whose value no longer fits.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DialogFields {
    values: BTreeMap<String, FieldValue>,
    #[serde(skip)]
    opening: BTreeMap<String, FieldValue>,
    /// The keys marked mixed ([`DialogFields::set_mixed`]).
    #[serde(skip)]
    mixed: BTreeSet<String>,
    /// The marks the opening snapshot kept, which Reset restores.
    #[serde(skip)]
    opening_mixed: BTreeSet<String>,
    /// The keys of the tabs a reset button reset ([`DialogFields::mark_reset`]).
    #[serde(skip)]
    reset: BTreeSet<String>,
}

/// Equal when the values are; the opening snapshot is bookkeeping, not content.
impl PartialEq for DialogFields {
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}

impl DialogFields {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds from `(key, value)` pairs.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<FieldValue>)>) -> Self {
        Self {
            values: pairs.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
            opening: BTreeMap::new(),
            mixed: BTreeSet::new(),
            opening_mixed: BTreeSet::new(),
            reset: BTreeSet::new(),
        }
    }

    /// Sets `key` to `value`, replacing any earlier value. An edit answers a mixed mark, so it clears it.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<FieldValue>) {
        let key = key.into();
        self.mixed.remove(&key);
        self.values.insert(key, value.into());
    }

    /// Marks `key` as differing over what the dialog or panel edits: its value is removed and its row shows blank. The
    /// marks are not serialized, so a mixed key is absent from `json_fields` and every answer.
    pub fn set_mixed(&mut self, key: impl Into<String>) {
        let key = key.into();
        self.values.remove(&key);
        self.mixed.insert(key);
    }

    /// Whether `key` is marked mixed.
    pub fn is_mixed(&self, key: &str) -> bool {
        self.mixed.contains(key)
    }

    /// The value under `key`, if any.
    pub fn get(&self, key: &str) -> Option<&FieldValue> {
        self.values.get(key)
    }

    /// The value under `key`, mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut FieldValue> {
        self.values.get_mut(key)
    }

    /// Removes `key` and returns its value; the key is no longer marked mixed.
    pub fn remove(&mut self, key: &str) -> Option<FieldValue> {
        self.mixed.remove(key);
        self.values.remove(key)
    }

    /// True when `key` has a value.
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    // The scalar readers also read the `Json` form of their value, which automation stores for a custom row's keys
    // (`Param::custom`): one read whatever form the value was set in.

    /// `Bool`, else `None`.
    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.values.get(key)? {
            FieldValue::Bool(b) => Some(*b),
            FieldValue::Json(value) => value.as_bool(),
            _ => None,
        }
    }

    /// `Number`, or an `Int` as a number.
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.values.get(key)? {
            FieldValue::Number(n) => Some(*n),
            FieldValue::Int(i) => Some(*i as f64),
            FieldValue::Json(value) => value.as_f64(),
            _ => None,
        }
    }

    /// `Int`, or a whole `Number`.
    pub fn int(&self, key: &str) -> Option<i64> {
        let whole = |n: f64| (n.fract() == 0.0 && n.abs() < 9.0e15).then_some(n as i64);
        match self.values.get(key)? {
            FieldValue::Int(i) => Some(*i),
            FieldValue::Number(n) => whole(*n),
            FieldValue::Json(value) => value.as_i64().or_else(|| value.as_f64().and_then(whole)),
            _ => None,
        }
    }

    /// `Text`, else `None`.
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.values.get(key)? {
            FieldValue::Text(s) => Some(s),
            FieldValue::Json(value) => value.as_str(),
            _ => None,
        }
    }

    /// `Color` as RGB in 0–1, else `None`.
    pub fn color(&self, key: &str) -> Option<[f32; 3]> {
        match self.values.get(key)? {
            FieldValue::Color(c) => Some(*c),
            _ => None,
        }
    }

    /// `Point` as `[x, y]`, else `None`.
    pub fn point(&self, key: &str) -> Option<[f64; 2]> {
        match self.values.get(key)? {
            FieldValue::Point(p) => Some(*p),
            _ => None,
        }
    }

    /// `Grid` cells in row order, else `None`.
    pub fn grid(&self, key: &str) -> Option<&[i64]> {
        match self.values.get(key)? {
            FieldValue::Grid(g) => Some(g),
            _ => None,
        }
    }

    /// A `List`, or a `Grid`, `Point` or `Color` as the list of its numbers (the shapes an
    /// untagged read gives a stored list of numbers, and an empty list), or a `Json` array read as
    /// those values. The key may have no parameter row: a host keeps its own data under it (a key
    /// without a row is not remembered).
    pub fn list(&self, key: &str) -> Option<Cow<'_, [FieldValue]>> {
        Some(match self.values.get(key)? {
            FieldValue::List(items) => Cow::Borrowed(items.as_slice()),
            FieldValue::Grid(cells) => Cow::Owned(cells.iter().map(|c| FieldValue::Int(*c)).collect()),
            FieldValue::Point(p) => Cow::Owned(p.iter().map(|n| FieldValue::Number(*n)).collect()),
            FieldValue::Color(c) => Cow::Owned(c.iter().map(|n| FieldValue::Number(f64::from(*n))).collect()),
            FieldValue::Json(serde_json::Value::Array(items)) => {
                Cow::Owned(items.iter().map(|item| serde_json::from_value(item.clone())).collect::<Result<_, _>>().ok()?)
            }
            _ => return None,
        })
    }

    /// Every `(key, value)` in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &FieldValue)> + '_ {
        self.values.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// The number of keys.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when there are no keys.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Marks the tab `key` as reset by a reset button.
    pub fn mark_reset(&mut self, key: impl Into<String>) {
        self.reset.insert(key.into());
    }

    /// Whether a reset button reset the tab `key`; `snapshot` and `reset` clear the marks.
    pub fn was_reset(&self, key: &str) -> bool {
        self.reset.contains(key)
    }

    /// Records the current values as the opening ones. Dialogs call it on open; a host that
    /// changes a value from outside the dialog (a script) does not move the snapshot.
    pub fn snapshot(&mut self) {
        self.opening = self.values.clone();
        self.opening_mixed = self.mixed.clone();
        self.reset.clear();
    }

    /// Restores the snapshot. Keys added since the snapshot are kept; keys removed since come
    /// back. Returns whether anything changed.
    pub fn reset(&mut self) -> bool {
        let mut changed = false;
        for (key, value) in &self.opening {
            if self.values.get(key) != Some(value) {
                self.values.insert(key.clone(), value.clone());
                self.mixed.remove(key);
                changed = true;
            }
        }
        for key in &self.opening_mixed {
            if self.mixed.insert(key.clone()) {
                self.values.remove(key);
                changed = true;
            }
        }
        self.reset.clear();
        changed
    }

    /// Whether any value differs from the snapshot (Reset is enabled only then).
    // new.
    pub fn changed(&self) -> bool {
        self.opening.iter().any(|(key, value)| self.values.get(key) != Some(value))
    }

    /// Keeps only the keys `allowed` names whose value passes `fits`: what a remembered map goes
    /// through before it fills a reopened dialog, so a renamed or retyped parameter starts from
    /// its default.
    pub fn retain_fitting(&mut self, allowed: &[&str], fits: impl Fn(&str, &FieldValue) -> bool) {
        self.values.retain(|key, value| allowed.contains(&key.as_str()) && fits(key, value));
    }
}

impl<K: Into<String>, V: Into<FieldValue>> FromIterator<(K, V)> for DialogFields {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self::from_pairs(iter)
    }
}
