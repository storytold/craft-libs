//! `SectionKey`, `SectionState`, `SectionLayout` and `SectionEvent`: the user's arrangement of a sidebar's sections
//! (order, hidden, folded and pinned), made whole against the sections the app registers.

use std::sync::Arc;

/// The identity of a section: the id the app saves and the stack keys the section's state by (`"albums"`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SectionKey(Arc<str>);

impl SectionKey {
    /// The key with id `id`.
    pub fn new(id: impl AsRef<str>) -> Self {
        SectionKey(Arc::from(id.as_ref()))
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SectionKey {
    fn from(id: &str) -> Self {
        SectionKey::new(id)
    }
}

/// One section's place in the arrangement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SectionState {
    /// The section.
    pub key: SectionKey,
    /// Hidden from the sidebar.
    pub hidden: bool,
    /// Folded shut.
    pub folded: bool,
    /// Kept above the sections that are not pinned.
    pub pinned: bool,
}

impl SectionState {
    /// A section that is shown, open and not pinned.
    pub fn new(key: SectionKey) -> Self {
        SectionState { key, hidden: false, folded: false, pinned: false }
    }
}

/// What [`ReorderableSections`](super::ReorderableSections) reported, in order. View state, never an undo step: the
/// host applies the events ([`SectionLayout::apply`]) or, with a bound layout, finds them already applied.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SectionEvent {
    /// Move `key` before `before` (`None`: to the end). From a header drag, Alt with the arrow keys, or Move Up and
    /// Move Down.
    #[non_exhaustive]
    Moved {
        /// The section moved.
        key: SectionKey,
        /// The section it now precedes; `None` for the end.
        before: Option<SectionKey>,
    },
    /// The section was hidden.
    Hidden(SectionKey),
    /// The section was shown again.
    Shown(SectionKey),
    /// The section was folded or unfolded.
    #[non_exhaustive]
    Folded {
        /// The section.
        key: SectionKey,
        /// Whether the section is now folded.
        folded: bool,
    },
    /// The section was pinned above the others.
    Pinned(SectionKey),
    /// The section was released from the top.
    Unpinned(SectionKey),
}

impl SectionEvent {
    /// `key` moved before `before` (`None`: to the end).
    pub fn moved(key: SectionKey, before: Option<SectionKey>) -> Self {
        SectionEvent::Moved { key, before }
    }

    /// `key` folded (`folded`) or unfolded.
    pub fn folded(key: SectionKey, folded: bool) -> Self {
        SectionEvent::Folded { key, folded }
    }
}

/// The arrangement of a sidebar's sections, top to bottom. Pinned sections come first, in their own order, then the
/// others; every key appears once.
///
/// Serde: written as a list of `{ id, hidden, folded, pinned }`. Read leniently from any self-describing format
/// (TOML, JSON): a list whose entries are an id string or a map with `id` and optional `hidden`, `folded` (also
/// `collapsed`) and `pinned`; a flag that is not a bool is off; the first entry for an id counts; an entry without an
/// id is dropped; anything that is not a list is an empty arrangement. A file always opens. Reading cannot know the
/// registered sections, so the host calls [`reconcile`](Self::reconcile) after reading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionLayout(Vec<SectionState>);

impl SectionLayout {
    /// The usual arrangement of `registered`: in that order, every section shown, open and not pinned.
    pub fn new(registered: impl IntoIterator<Item = SectionKey>) -> Self {
        Self::from_states(registered.into_iter().map(SectionState::new))
    }

    /// The layout these states describe: the first state of each key counts.
    pub fn from_states(states: impl IntoIterator<Item = SectionState>) -> Self {
        let mut all: Vec<SectionState> = Vec::new();
        for state in states {
            if !all.iter().any(|held| held.key == state.key) {
                all.push(state);
            }
        }
        let mut layout = SectionLayout(all);
        layout.normalise();
        layout
    }

    /// Makes the layout hold exactly the sections of `registered`: unknown sections are dropped and sections it
    /// lacks are appended, shown and open, in registration order. Returns whether the layout changed.
    pub fn reconcile(&mut self, registered: &[SectionKey]) -> bool {
        let was = self.0.len();
        self.0.retain(|state| registered.contains(&state.key));
        let dropped = self.0.len() != was;
        let mut added = false;
        for key in registered {
            if !self.0.iter().any(|state| state.key == *key) {
                self.0.push(SectionState::new(key.clone()));
                added = true;
            }
        }
        let before = self.0.clone();
        self.normalise();
        dropped || added || before != self.0
    }

    /// Every section top to bottom, hidden ones included.
    pub fn states(&self) -> &[SectionState] {
        &self.0
    }

    /// The shown sections, top to bottom.
    pub fn shown(&self) -> impl Iterator<Item = &SectionKey> + '_ {
        self.0.iter().filter(|state| !state.hidden).map(|state| &state.key)
    }

    /// Whether `key` is hidden.
    pub fn is_hidden(&self, key: &SectionKey) -> bool {
        self.state(key).is_some_and(|state| state.hidden)
    }

    /// Whether `key` is folded.
    pub fn is_folded(&self, key: &SectionKey) -> bool {
        self.state(key).is_some_and(|state| state.folded)
    }

    /// Whether `key` is pinned.
    pub fn is_pinned(&self, key: &SectionKey) -> bool {
        self.state(key).is_some_and(|state| state.pinned)
    }

    /// Hides or shows `key`.
    pub fn set_hidden(&mut self, key: &SectionKey, hidden: bool) -> bool {
        self.change(key, |state| &mut state.hidden, hidden)
    }

    /// Folds or unfolds `key`.
    pub fn set_folded(&mut self, key: &SectionKey, folded: bool) -> bool {
        self.change(key, |state| &mut state.folded, folded)
    }

    /// Pins `key` above the sections that are not pinned, or releases it.
    pub fn set_pinned(&mut self, key: &SectionKey, pinned: bool) -> bool {
        let changed = self.change(key, |state| &mut state.pinned, pinned);
        self.normalise();
        changed
    }

    /// Shows and unfolds `key` (a command that needs it in view).
    pub fn reveal(&mut self, key: &SectionKey) -> bool {
        let shown = self.set_hidden(key, false);
        self.set_folded(key, false) || shown
    }

    /// Moves `key` before `before` (`None`: to the end). A section never crosses between the pinned and the other
    /// sections. Returns false and changes nothing when the shown order would stay the same (a hidden section between
    /// them does not count).
    pub fn move_before(&mut self, key: &SectionKey, before: Option<&SectionKey>) -> bool {
        if before == Some(key) {
            return false;
        }
        let Some(from) = self.0.iter().position(|state| state.key == *key) else { return false };
        let was = self.clone();
        let moved = self.0.remove(from);
        let hidden = moved.hidden;
        let at = before.and_then(|before| self.0.iter().position(|state| state.key == *before)).unwrap_or(self.0.len());
        self.0.insert(at, moved);
        self.normalise();
        if !hidden && self.shown().eq(was.shown()) {
            *self = was;
            return false;
        }
        *self != was
    }

    /// Moves `key` so it is the `to`th shown section once it has moved (the index a stack's move reports).
    pub fn move_to(&mut self, key: &SectionKey, to: usize) -> bool {
        self.moved_to(key, to).is_some_and(|event| self.apply(&event))
    }

    /// Moves `key` one shown place earlier (`up`) or later, stepping over hidden sections so every move is visible;
    /// with `to_end`, to the first or last shown place. Returns whether it moved.
    pub fn move_by(&mut self, key: &SectionKey, up: bool, to_end: bool) -> bool {
        self.moved_by(key, up, to_end).is_some_and(|event| self.apply(&event))
    }

    /// The event that [`move_to`](Self::move_to) applies, or `None` when it would change nothing shown.
    pub fn moved_to(&self, key: &SectionKey, to: usize) -> Option<SectionEvent> {
        let mut shown: Vec<SectionKey> = self.shown().cloned().collect();
        let from = shown.iter().position(|held| held == key)?;
        let moved = shown.remove(from);
        let to = to.min(shown.len());
        shown.insert(to, moved);
        self.moved(key, shown.get(to + 1).cloned())
    }

    /// The event that [`move_by`](Self::move_by) applies, or `None` when it would change nothing shown.
    pub fn moved_by(&self, key: &SectionKey, up: bool, to_end: bool) -> Option<SectionEvent> {
        let shown: Vec<SectionKey> = self.shown().cloned().collect();
        let at = shown.iter().position(|held| held == key)?;
        let before = if up {
            Some(shown.get(if to_end { 0 } else { at.checked_sub(1)? })?.clone())
        } else {
            if at + 1 >= shown.len() {
                return None;
            }
            // below the next one: above the one after it, or last
            if to_end { None } else { shown.get(at + 2).cloned() }
        };
        self.moved(key, before)
    }

    /// Whether [`move_by`](Self::move_by) would move `key` (Move Up and Move Down are disabled at the ends).
    pub fn can_move(&self, key: &SectionKey, up: bool) -> bool {
        let mut shown = self.shown();
        match shown.position(|held| held == key) {
            Some(at) if up => at > 0,
            Some(_) => shown.next().is_some(),
            None => false,
        }
    }

    /// Applies one event; returns whether the layout changed (the host saves it then).
    pub fn apply(&mut self, event: &SectionEvent) -> bool {
        match event {
            SectionEvent::Moved { key, before } => self.move_before(key, before.as_ref()),
            SectionEvent::Hidden(key) => self.set_hidden(key, true),
            SectionEvent::Shown(key) => self.set_hidden(key, false),
            SectionEvent::Folded { key, folded } => self.set_folded(key, *folded),
            SectionEvent::Pinned(key) => self.set_pinned(key, true),
            SectionEvent::Unpinned(key) => self.set_pinned(key, false),
        }
    }

    /// `Moved { key, before }` when it changes the shown order.
    fn moved(&self, key: &SectionKey, before: Option<SectionKey>) -> Option<SectionEvent> {
        self.clone().move_before(key, before.as_ref()).then(|| SectionEvent::Moved { key: key.clone(), before })
    }

    fn state(&self, key: &SectionKey) -> Option<&SectionState> {
        self.0.iter().find(|state| state.key == *key)
    }

    /// Sets one flag of `key`; returns whether it changed.
    fn change(&mut self, key: &SectionKey, flag: impl FnOnce(&mut SectionState) -> &mut bool, value: bool) -> bool {
        let Some(state) = self.0.iter_mut().find(|state| state.key == *key) else { return false };
        let flag = flag(state);
        let changed = *flag != value;
        *flag = value;
        changed
    }

    /// Keeps the pinned sections ahead of the others, each group in its own order.
    fn normalise(&mut self) {
        self.0.sort_by_key(|state| !state.pinned);
    }
}

impl serde::Serialize for SectionLayout {
    fn serialize<Z: serde::Serializer>(&self, serializer: Z) -> Result<Z::Ok, Z::Error> {
        #[derive(serde::Serialize)]
        struct Saved<'a> {
            id: &'a str,
            hidden: bool,
            folded: bool,
            pinned: bool,
        }
        serializer.collect_seq(self.0.iter().map(|s| Saved { id: s.key.as_str(), hidden: s.hidden, folded: s.folded, pinned: s.pinned }))
    }
}

impl<'de> serde::Deserialize<'de> for SectionLayout {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = deserializer.deserialize_any(lenient::List)?;
        Ok(Self::from_states(entries.into_iter().filter_map(|entry| {
            let key = SectionKey::new(entry.id?);
            Some(SectionState { key, hidden: entry.hidden, folded: entry.folded, pinned: entry.pinned })
        })))
    }
}

/// Reading a saved arrangement from any format without failing: whatever is not understood is
/// left out. A value is taken by `deserialize_any`, so each visitor says what it accepts and
/// drains the rest.
mod lenient {
    use std::fmt;

    use serde::de::{Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

    /// One saved entry, before its id is looked up.
    pub(super) struct Entry {
        pub id: Option<String>,
        pub hidden: bool,
        pub folded: bool,
        pub pinned: bool,
    }

    /// A value that is a bool, a string or something else.
    enum Scalar {
        Bool(bool),
        Str(String),
        Other,
    }

    impl Scalar {
        fn flag(&self) -> bool {
            matches!(self, Scalar::Bool(true))
        }
    }

    impl<'de> Deserialize<'de> for Scalar {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Scalar;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("anything")
                }
                fn visit_bool<E>(self, v: bool) -> Result<Scalar, E> {
                    Ok(Scalar::Bool(v))
                }
                fn visit_str<E>(self, v: &str) -> Result<Scalar, E> {
                    Ok(Scalar::Str(v.to_owned()))
                }
                fn visit_i64<E>(self, _: i64) -> Result<Scalar, E> {
                    Ok(Scalar::Other)
                }
                fn visit_u64<E>(self, _: u64) -> Result<Scalar, E> {
                    Ok(Scalar::Other)
                }
                fn visit_f64<E>(self, _: f64) -> Result<Scalar, E> {
                    Ok(Scalar::Other)
                }
                fn visit_unit<E>(self) -> Result<Scalar, E> {
                    Ok(Scalar::Other)
                }
                fn visit_none<E>(self) -> Result<Scalar, E> {
                    Ok(Scalar::Other)
                }
                fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Scalar, D::Error> {
                    d.deserialize_any(self)
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Scalar, A::Error> {
                    while seq.next_element::<IgnoredAny>()?.is_some() {}
                    Ok(Scalar::Other)
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Scalar, A::Error> {
                    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                    Ok(Scalar::Other)
                }
            }
            deserializer.deserialize_any(V)
        }
    }

    impl<'de> Deserialize<'de> for Entry {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Entry;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a section id or a map")
                }
                fn visit_str<E>(self, v: &str) -> Result<Entry, E> {
                    Ok(Entry { id: Some(v.to_owned()), hidden: false, folded: false, pinned: false })
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Entry, A::Error> {
                    let mut entry = Entry { id: None, hidden: false, folded: false, pinned: false };
                    while let Some(key) = map.next_key::<Scalar>()? {
                        let value = map.next_value::<Scalar>()?;
                        let Scalar::Str(key) = key else { continue };
                        match key.as_str() {
                            "id" if entry.id.is_none() => {
                                if let Scalar::Str(id) = value {
                                    entry.id = Some(id);
                                }
                            }
                            "hidden" => entry.hidden = value.flag(),
                            "pinned" => entry.pinned = value.flag(),
                            "folded" | "collapsed" => entry.folded |= value.flag(),
                            _ => {}
                        }
                    }
                    Ok(entry)
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Entry, A::Error> {
                    while seq.next_element::<IgnoredAny>()?.is_some() {}
                    Ok(Entry { id: None, hidden: false, folded: false, pinned: false })
                }
                fn visit_bool<E>(self, _: bool) -> Result<Entry, E> {
                    Ok(Entry { id: None, hidden: false, folded: false, pinned: false })
                }
                fn visit_i64<E>(self, _: i64) -> Result<Entry, E> {
                    Ok(Entry { id: None, hidden: false, folded: false, pinned: false })
                }
                fn visit_u64<E>(self, _: u64) -> Result<Entry, E> {
                    Ok(Entry { id: None, hidden: false, folded: false, pinned: false })
                }
                fn visit_f64<E>(self, _: f64) -> Result<Entry, E> {
                    Ok(Entry { id: None, hidden: false, folded: false, pinned: false })
                }
                fn visit_unit<E>(self) -> Result<Entry, E> {
                    Ok(Entry { id: None, hidden: false, folded: false, pinned: false })
                }
                fn visit_none<E>(self) -> Result<Entry, E> {
                    Ok(Entry { id: None, hidden: false, folded: false, pinned: false })
                }
                fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Entry, D::Error> {
                    d.deserialize_any(self)
                }
            }
            deserializer.deserialize_any(V)
        }
    }

    /// The saved list; anything that is not a list reads as an empty one.
    pub(super) struct List;

    impl<'de> Visitor<'de> for List {
        type Value = Vec<Entry>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a list of sections")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<Entry>, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = seq.next_element::<Entry>()? {
                entries.push(entry);
            }
            Ok(entries)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<Entry>, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(Vec::new())
        }
        fn visit_bool<E>(self, _: bool) -> Result<Vec<Entry>, E> {
            Ok(Vec::new())
        }
        fn visit_i64<E>(self, _: i64) -> Result<Vec<Entry>, E> {
            Ok(Vec::new())
        }
        fn visit_u64<E>(self, _: u64) -> Result<Vec<Entry>, E> {
            Ok(Vec::new())
        }
        fn visit_f64<E>(self, _: f64) -> Result<Vec<Entry>, E> {
            Ok(Vec::new())
        }
        fn visit_str<E>(self, _: &str) -> Result<Vec<Entry>, E> {
            Ok(Vec::new())
        }
        fn visit_unit<E>(self) -> Result<Vec<Entry>, E> {
            Ok(Vec::new())
        }
        fn visit_none<E>(self) -> Result<Vec<Entry>, E> {
            Ok(Vec::new())
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Vec<Entry>, D::Error> {
            d.deserialize_any(self)
        }
    }
}
