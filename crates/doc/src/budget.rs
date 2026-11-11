//! The resource budget of one call and counted, depth-checked deserialization.

use super::*;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{self, DeserializeOwned, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

/// The bytes one call may still consume from gzip or ZIP entries, or return as renditions.
pub(crate) struct Budget<'a> {
    pub(crate) limits: &'a Limits,
    pub(crate) control: Option<&'a cancel::Control<'a>>,
    left: u64,
}

impl<'a> Budget<'a> {
    pub(super) fn new(limits: &'a Limits) -> Self {
        Budget { limits, control: None, left: limits.max_total }
    }

    pub(super) fn controlled(limits: &'a Limits, control: &'a cancel::Control<'a>) -> Self {
        Budget { limits, control: Some(control), left: limits.max_total }
    }

    pub(crate) fn check_cancelled(&self) -> Result<(), Error> {
        if let Some(control) = self.control {
            control.check()?;
        }
        Ok(())
    }

    /// Copy bytes whose size the caller has already checked against the budget.
    pub(crate) fn copy(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        if self.control.is_none() {
            return Ok(bytes.to_vec());
        }
        self.check_cancelled()?;
        let mut out = Vec::with_capacity(bytes.len());
        for chunk in bytes.chunks(cancel::CHUNK) {
            self.check_cancelled()?;
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }

    /// Bytes still allowed.
    pub(crate) fn left(&self) -> u64 {
        self.left
    }

    /// Count `n` bytes of `what` against the total.
    pub(crate) fn take(&mut self, n: u64, what: &str) -> Result<(), Error> {
        self.check_cancelled()?;
        self.left = self
            .left
            .checked_sub(n)
            .ok_or_else(|| Error::LimitExceeded(format!("{what}: more than max_total ({} bytes) in all", self.limits.max_total)))?;
        Ok(())
    }

    /// Check one rendition or ZIP entry of `n` bytes against `max_entry`.
    pub(crate) fn check_entry(&self, n: u64, what: &str) -> Result<(), Error> {
        self.check_cancelled()?;
        if n > self.limits.max_entry {
            return Err(Error::LimitExceeded(format!("{what} is {n} bytes, more than max_entry ({})", self.limits.max_entry)));
        }
        Ok(())
    }

    /// Check one rendition or ZIP entry of `n` bytes against `max_entry`, then count it.
    pub(crate) fn take_entry(&mut self, n: u64, what: &str) -> Result<(), Error> {
        self.check_entry(n, what)?;
        self.take(n, what)
    }

    /// Check `n` bytes of JSON against `max_json`.
    pub(crate) fn check_json(&self, n: u64, what: &str) -> Result<(), Error> {
        self.check_cancelled()?;
        if n > self.limits.max_json {
            return Err(Error::LimitExceeded(format!("{what} is more than max_json ({} bytes)", self.limits.max_json)));
        }
        Ok(())
    }
}

/// Is the JSON text nested at most `max` levels deep (arrays and objects), and at most
/// [`MAX_DEPTH`]?
pub(crate) fn check_depth(json: &[u8], max: u32, control: Option<&cancel::Control<'_>>) -> Result<(), Error> {
    let max = max.min(MAX_DEPTH);
    let (mut depth, mut in_str, mut escaped) = (0u32, false, false);
    for chunk in json.chunks(cancel::CHUNK) {
        if let Some(control) = control {
            control.check()?;
        }
        for &b in chunk {
            if in_str {
                match b {
                    _ if escaped => escaped = false,
                    b'\\' => escaped = true,
                    b'"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => in_str = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > max {
                        return Err(Error::LimitExceeded(format!("JSON nested deeper than max_depth ({max} levels)")));
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

thread_local! {
    /// The elements [`counted`] lists may still read in the current [`parse`], and whether a
    /// list went past them.
    static ITEMS: Cell<(u64, bool)> = const { Cell::new((u64::MAX, false)) };
}

/// Run `f` with `max` elements for [`counted`] lists; when `f` fails after a list went past
/// them, the error is [`Error::LimitExceeded`].
pub(super) fn counting<T>(max: u64, f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    let saved = ITEMS.with(|c| c.replace((max, false)));
    let r = f();
    let (_, over) = ITEMS.with(|c| c.replace(saved));
    match r {
        Err(_) if over => Err(Error::LimitExceeded(format!("the file lists more than max_items ({max}) items, layers and pages"))),
        r => r,
    }
}

/// Count one list element against [`Limits::max_items`].
fn take_item<E: de::Error>() -> Result<(), E> {
    ITEMS.with(|c| match c.get().0.checked_sub(1) {
        Some(left) => {
            c.set((left, false));
            Ok(())
        }
        None => {
            c.set((0, true));
            Err(E::custom("more than max_items list elements"))
        }
    })
}

/// A `T` read after one element is counted with [`take_item`].
struct Item<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Item<T> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
        take_item()?;
        T::deserialize(d)
    }
}

/// A list read by [`parse`] (as `#[serde(deserialize_with = "counted")]`), each element counted
/// against [`Limits::max_items`] before it is read.
pub(crate) fn counted<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    struct List<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for List<T> {
        type Value = Vec<T>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a list")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            while let Some(v) = seq.next_element_seed(Item(PhantomData))? {
                out.push(v);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(List(PhantomData))
}

/// [`counted`] for a list that may be `null`.
pub(crate) fn counted_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Vec<T>>, D::Error> {
    struct Counted<T>(Vec<T>);
    impl<'de, T: Deserialize<'de>> Deserialize<'de> for Counted<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            counted(d).map(Counted)
        }
    }
    Ok(Option::<Counted<T>>::deserialize(d)?.map(|c| c.0))
}

/// [`counted`] for a map, each value counted before it is read.
pub(crate) fn counted_map<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<BTreeMap<String, T>, D::Error> {
    struct Map<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Map<T> {
        type Value = BTreeMap<String, T>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<BTreeMap<String, T>, A::Error> {
            let mut out = BTreeMap::new();
            while let Some(k) = map.next_key::<String>()? {
                let v = map.next_value_seed(Item(PhantomData))?;
                out.insert(k, v);
            }
            Ok(out)
        }
    }
    d.deserialize_map(Map(PhantomData))
}

/// Parse one JSON value of type `T` from `json`, after checking its nesting against the limits.
pub(crate) fn parse<T: DeserializeOwned>(json: &[u8], budget: &Budget) -> Result<T, Error> {
    check_depth(json, budget.limits.max_depth, budget.control)?;
    counting(budget.limits.max_items, || parse_checked(json, budget).map_err(malformed))
}

/// Parse JSON whose nesting [`check_depth`] has accepted.
pub(super) fn parse_checked<T: DeserializeOwned>(json: &[u8], budget: &Budget) -> Result<T, serde_json::Error> {
    fn read<'de, T: DeserializeOwned, R: serde_json::de::Read<'de>>(mut de: serde_json::Deserializer<R>) -> Result<T, serde_json::Error> {
        de.disable_recursion_limit();
        let v = T::deserialize(&mut de)?;
        de.end()?;
        Ok(v)
    }
    match budget.control {
        Some(control) => read(serde_json::Deserializer::from_reader(cancel::Reader::new(json, Some(control)))),
        None => read(serde_json::Deserializer::from_slice(json)),
    }
}
