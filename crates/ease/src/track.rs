//! [`Track`]: keyed values with an ease between each key and the next.

use crate::{EaseError, Easing, Vec2, finite};

/// Most keys one track holds.
pub const MAX_TRACK_KEYS: usize = 100_000;

/// A value a track can animate: interpolated in a straight line between keys, the ease giving
/// how far along. Eases that overshoot pass `t` outside `0..=1`, which extrapolates.
pub trait Animatable: Clone {
    /// The value `t` of the way from `self` to `to`.
    fn lerp(&self, to: &Self, t: f64) -> Self;
    /// Whether every component is finite.
    fn is_finite(&self) -> bool;
}

impl Animatable for f32 {
    fn lerp(&self, to: &f32, t: f64) -> f32 {
        crate::lerp(f64::from(*self), f64::from(*to), t) as f32
    }
    fn is_finite(&self) -> bool {
        f32::is_finite(*self)
    }
}

impl Animatable for f64 {
    fn lerp(&self, to: &f64, t: f64) -> f64 {
        crate::lerp(*self, *to, t)
    }
    fn is_finite(&self) -> bool {
        f64::is_finite(*self)
    }
}

impl Animatable for Vec2 {
    fn lerp(&self, to: &Vec2, t: f64) -> Vec2 {
        Vec2::lerp(*self, *to, t)
    }
    fn is_finite(&self) -> bool {
        Vec2::is_finite(*self)
    }
}

impl<const N: usize> Animatable for [f32; N] {
    fn lerp(&self, to: &[f32; N], t: f64) -> [f32; N] {
        let mut out = *self;
        for (o, b) in out.iter_mut().zip(to) {
            *o = Animatable::lerp(&*o, b, t);
        }
        out
    }
    fn is_finite(&self) -> bool {
        self.iter().all(|v| v.is_finite())
    }
}

impl<const N: usize> Animatable for [f64; N] {
    fn lerp(&self, to: &[f64; N], t: f64) -> [f64; N] {
        let mut out = *self;
        for (o, b) in out.iter_mut().zip(to) {
            *o = crate::lerp(*o, *b, t);
        }
        out
    }
    fn is_finite(&self) -> bool {
        self.iter().all(|v| v.is_finite())
    }
}

/// A key: a value at a time (seconds), and the ease to the next key ([`Easing::Hold`] keeps the
/// value until the next key).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Key<T> {
    pub time: f64,
    pub value: T,
    #[cfg_attr(feature = "serde", serde(default))]
    pub ease: Easing,
}

/// Keys sorted by time, one per time, at most [`MAX_TRACK_KEYS`], every time and value finite.
///
/// Before the first key a track gives the first key's value, after the last key the last's.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Track<T> {
    keys: Vec<Key<T>>,
}

impl<T> Default for Track<T> {
    fn default() -> Self {
        Track { keys: Vec::new() }
    }
}

impl<T: Animatable> Track<T> {
    pub fn new() -> Self {
        Track::default()
    }

    /// A track from keys a caller or a file supplies: an error for a time or value that is not
    /// finite, two keys at one time, or more than [`MAX_TRACK_KEYS`]. The keys may come in any
    /// order.
    pub fn from_keys(mut keys: Vec<Key<T>>) -> Result<Self, EaseError> {
        if keys.len() > MAX_TRACK_KEYS {
            return Err(EaseError::Full(MAX_TRACK_KEYS));
        }
        for k in &mut keys {
            check(k.time, &k.value)?;
            // -0.0 and 0.0 are one time.
            if k.time == 0.0 {
                k.time = 0.0;
            }
        }
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        if let Some(w) = keys.windows(2).find(|w| matches!(w, [a, b] if a.time == b.time)) {
            return Err(EaseError::SameTime(w.first().map_or(0.0, |k| k.time)));
        }
        Ok(Track { keys })
    }

    pub fn keys(&self) -> &[Key<T>] {
        &self.keys
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Sets the value at `time`, keeping that key's ease, or adds a key there that eases like
    /// the segment it splits. Returns the key's index. An error for a time or value that is not
    /// finite, or a full track.
    pub fn insert(&mut self, time: f64, value: T) -> Result<usize, EaseError> {
        check(time, &value)?;
        // -0.0 and 0.0 are one time.
        let time = if time == 0.0 { 0.0 } else { time };
        match self.keys.binary_search_by(|k| k.time.total_cmp(&time)) {
            Ok(i) => {
                if let Some(k) = self.keys.get_mut(i) {
                    k.value = value;
                }
                Ok(i)
            }
            Err(i) => {
                if self.keys.len() >= MAX_TRACK_KEYS {
                    return Err(EaseError::Full(MAX_TRACK_KEYS));
                }
                let ease = i.checked_sub(1).and_then(|p| self.keys.get(p)).map_or_else(Easing::default, |k| k.ease.clone());
                self.keys.insert(i, Key { time, value, ease });
                Ok(i)
            }
        }
    }

    /// Sets the ease from key `i` to the next one. `None` for a missing key.
    pub fn set_ease(&mut self, i: usize, ease: Easing) -> Option<()> {
        self.keys.get_mut(i)?.ease = ease;
        Some(())
    }

    /// Removes key `i`, returning it.
    pub fn remove(&mut self, i: usize) -> Option<Key<T>> {
        (i < self.keys.len()).then(|| self.keys.remove(i))
    }

    /// The index of the key at exactly `time` (−0 and +0 are one time).
    pub fn key_at(&self, time: f64) -> Option<usize> {
        let time = if time == 0.0 { 0.0 } else { time };
        self.keys.binary_search_by(|k| k.time.total_cmp(&time)).ok()
    }

    /// The value at `time`. An error for an empty track, a `time` that is not finite, or an
    /// overshooting ease that carries the value past what the type can hold.
    pub fn sample(&self, time: f64) -> Result<T, EaseError> {
        let time = finite(time, "time")?;
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else { return Err(EaseError::Empty) };
        if time <= first.time {
            return Ok(first.value.clone());
        }
        if time >= last.time {
            return Ok(last.value.clone());
        }
        let i = self.keys.partition_point(|k| k.time <= time);
        let (Some(a), Some(b)) = (i.checked_sub(1).and_then(|j| self.keys.get(j)), self.keys.get(i)) else { return Ok(last.value.clone()) };
        if a.ease == Easing::Hold {
            return Ok(a.value.clone());
        }
        // Keys at opposite ends of the f64 range overflow the span; halving both sides keeps it.
        let span = b.time - a.time;
        let t = if span.is_finite() { (time - a.time) / span } else { (time * 0.5 - a.time * 0.5) / (b.time * 0.5 - a.time * 0.5) };
        let v = a.value.lerp(&b.value, a.ease.apply(t.clamp(0.0, 1.0)));
        if v.is_finite() { Ok(v) } else { Err(EaseError::Overflow) }
    }
}

fn check<T: Animatable>(time: f64, value: &T) -> Result<(), EaseError> {
    finite(time, "time")?;
    if value.is_finite() { Ok(()) } else { Err(EaseError::NotFinite("value")) }
}

#[cfg(feature = "serde")]
mod serde_impl {
    use super::{Animatable, Key, MAX_TRACK_KEYS, Track};
    use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
    use std::marker::PhantomData;

    /// The key list, read one key at a time and refused once it passes [`MAX_TRACK_KEYS`], so a
    /// hostile file cannot make it allocate more.
    struct Keys<T>(Vec<Key<T>>);

    impl<'de, T: Deserialize<'de>> Deserialize<'de> for Keys<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V<T>(PhantomData<T>);
            impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
                type Value = Keys<T>;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(f, "a list of at most {MAX_TRACK_KEYS} keys")
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Keys<T>, A::Error> {
                    let mut keys = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1024));
                    while let Some(k) = seq.next_element()? {
                        if keys.len() >= MAX_TRACK_KEYS {
                            return Err(A::Error::custom(format!("more than {MAX_TRACK_KEYS} keys")));
                        }
                        keys.push(k);
                    }
                    Ok(Keys(keys))
                }
            }
            d.deserialize_seq(V(PhantomData))
        }
    }

    impl<'de, T: Animatable + Deserialize<'de>> Deserialize<'de> for Track<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V<T>(PhantomData<T>);
            impl<'de, T: Animatable + Deserialize<'de>> Visitor<'de> for V<T> {
                type Value = Track<T>;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(f, "a track")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Track<T>, A::Error> {
                    let mut keys: Option<Vec<Key<T>>> = None;
                    while let Some(name) = map.next_key::<String>()? {
                        if name == "keys" {
                            keys = Some(map.next_value::<Keys<T>>()?.0);
                        } else {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                    Track::from_keys(keys.unwrap_or_default()).map_err(A::Error::custom)
                }
                /// Formats that write a struct as its fields in order.
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Track<T>, A::Error> {
                    let keys = seq.next_element::<Keys<T>>()?.ok_or_else(|| A::Error::invalid_length(0, &self))?.0;
                    if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                        return Err(A::Error::invalid_length(2, &self));
                    }
                    Track::from_keys(keys).map_err(A::Error::custom)
                }
            }
            d.deserialize_struct("Track", &["keys"], V(PhantomData))
        }
    }
}
