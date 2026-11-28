//! What a replay at another place moves (`Shift`): a settled region's record replays where its room is now, so every
//! registration it journaled (`runtime::journal`) and what a memoized view returned move by the replay's one
//! `RecordShift`, the same one egui's records move by.

use egui::{Pos2, RecordShift, Rect};
use smallvec::SmallVec;
use std::sync::Arc;

/// A value a record holds, at the place a replay adds the record (`RecordShift`): rects and points move with the record,
/// clip rects (`Clip`) go through the replay's clip, and everything else stays. There is no default: a type states how
/// it moves. `Vec2` has no impl, as a size and a move are both `Vec2`; a type holding one says which it is.
pub trait Shift {
    /// `self` at the place `by` names.
    #[must_use]
    fn shifted(&self, by: &RecordShift) -> Self;

    /// `self` with each coordinate `shifted` moves, and each distance a hit shape holds (`reach`, `min_across`), taken
    /// from `to` where the two lie within `within` points; everything else, a bare float included, stays. The replay
    /// audit compares a moved record with a real run this way (`==` after it): positions off the float grid moved by a
    /// whole pixel differ from the same positions computed at the new place by rounding only.
    #[must_use]
    fn snapped(&self, to: &Self, within: f32) -> Self;
}

/// `value`, or `to` when the two lie within `within` of each other.
pub fn snap(value: f32, to: f32, within: f32) -> f32 {
    if (value - to).abs() <= within { to } else { value }
}

/// A clip rect a record holds: the clip of the code around the record becomes the replay's clip, a clip set inside it
/// moves with it (`RecordShift::clip`). A clip with no area is `Rect::NOTHING`, however it came about: one clip passes
/// nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip(Rect);

impl Clip {
    /// The record's clip for the clip rect `clip`.
    pub fn new(clip: Rect) -> Self {
        Clip(if clip.is_positive() { clip } else { Rect::NOTHING })
    }

    /// The clip rect.
    pub fn rect(self) -> Rect {
        self.0
    }
}

impl Shift for Clip {
    fn shifted(&self, by: &RecordShift) -> Self {
        Clip::new(by.clip(self.0))
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        Clip(self.0.snapped(&to.0, within))
    }
}

impl Shift for Rect {
    fn shifted(&self, by: &RecordShift) -> Self {
        by.rect(*self)
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        Rect::from_min_max(self.min.snapped(&to.min, within), self.max.snapped(&to.max, within))
    }
}

impl Shift for Pos2 {
    fn shifted(&self, by: &RecordShift) -> Self {
        by.pos(*self)
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        Pos2::new(snap(self.x, to.x, within), snap(self.y, to.y, within))
    }
}

/// Types with no place: they stay as they are at any place.
#[macro_export]
#[doc(hidden)]
macro_rules! stays {
    ($($ty:ty),* $(,)?) => {
        $(impl $crate::runtime::Shift for $ty {
            #[allow(clippy::clone_on_copy)]
            fn shifted(&self, _: &$crate::runtime::RecordShift) -> Self {
                self.clone()
            }

            #[allow(clippy::clone_on_copy)]
            fn snapped(&self, _: &Self, _: f32) -> Self {
                self.clone()
            }
        })*
    };
}

// A scalar is a length, a fraction, a zoom, a time or a count, which a replay at another place never changes, so the
// audit compares it exactly; a coordinate is a `Pos2` or a `Rect`.
stays!(craft_behavior::PartRef, craft_behavior::Sense);
stays!((), bool, f32, f64, u8, u16, u32, u64, usize, i32, i64, String, Arc<str>, &'static str, egui::Id, egui::LayerId, egui::TextureId);

impl<T: Shift> Shift for Option<T> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.as_ref().map(|value| value.shifted(by))
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        match (self, to) {
            (Some(value), Some(to)) => Some(value.snapped(to, within)),
            _ => self.as_ref().map(|value| value.snapped(value, within)),
        }
    }
}

/// `values`, each taken from the item of `to` at its index by `snapped` when both hold as many.
fn snapped_items<'a, T: Shift + 'a>(values: impl ExactSizeIterator<Item = &'a T>, to: impl ExactSizeIterator<Item = &'a T>, within: f32) -> Vec<T> {
    if values.len() == to.len() {
        values.zip(to).map(|(value, to)| value.snapped(to, within)).collect()
    } else {
        values.map(|value| value.snapped(value, within)).collect()
    }
}

impl<T: Shift, const N: usize> Shift for [T; N] {
    fn shifted(&self, by: &RecordShift) -> Self {
        std::array::from_fn(|index| self[index].shifted(by))
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        std::array::from_fn(|index| self[index].snapped(&to[index], within))
    }
}

impl<T: Shift> Shift for Vec<T> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.iter().map(|value| value.shifted(by)).collect()
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        snapped_items(self.iter(), to.iter(), within)
    }
}

impl<T: Shift, const N: usize> Shift for SmallVec<[T; N]> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.iter().map(|value| value.shifted(by)).collect()
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        snapped_items(self.iter(), to.iter(), within).into_iter().collect()
    }
}

macro_rules! tuples {
    ($(($($name:ident $index:tt),+)),*) => {
        $(impl<$($name: Shift),+> Shift for ($($name,)+) {
            fn shifted(&self, by: &RecordShift) -> Self {
                ($(self.$index.shifted(by),)+)
            }

            fn snapped(&self, to: &Self, within: f32) -> Self {
                ($(self.$index.snapped(&to.$index, within),)+)
            }
        })*
    };
}

impl Shift for craft_behavior::HitShape {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.translated(by.offset)
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        self.snapped(to, within)
    }
}

/// A hit shape in a body layer's space (`local`) moves too: a record holding a transformed body never moves, as the
/// body's own layer blocks it.
impl<O: Clone> Shift for craft_behavior::Shape<O> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.translated(by.offset)
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        self.snapped(to, within)
    }
}

/// A wheel taker's rect is its view; how far it can still scroll is a distance.
impl<O: Clone> Shift for craft_behavior::WheelTaker<O>
where
    craft_behavior::WheelTaker<O>: Clone,
{
    fn shifted(&self, by: &RecordShift) -> Self {
        let mut taker = self.clone();
        taker.rect = by.rect(self.rect);
        taker
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        let mut taker = self.clone();
        taker.rect = self.rect.snapped(&to.rect, within);
        taker
    }
}

tuples!((A 0), (A 0, B 1), (A 0, B 1, C 2), (A 0, B 1, C 2, D 3), (A 0, B 1, C 2, D 3, E 4), (A 0, B 1, C 2, D 3, E 4, F 5));
