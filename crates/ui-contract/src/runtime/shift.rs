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
}

impl Shift for Rect {
    fn shifted(&self, by: &RecordShift) -> Self {
        by.rect(*self)
    }
}

impl Shift for Pos2 {
    fn shifted(&self, by: &RecordShift) -> Self {
        by.pos(*self)
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
        })*
    };
}

// A scalar is a length or a count; a coordinate is a `Pos2` or a `Rect`.
stays!(craft_behavior::PartRef, craft_behavior::Sense);
stays!((), bool, u8, u16, u32, u64, usize, i32, i64, f32, f64, String, Arc<str>, &'static str, egui::Id, egui::LayerId);

impl<T: Shift> Shift for Option<T> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.as_ref().map(|value| value.shifted(by))
    }
}

impl<T: Shift> Shift for Vec<T> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.iter().map(|value| value.shifted(by)).collect()
    }
}

impl<T: Shift, const N: usize> Shift for SmallVec<[T; N]> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.iter().map(|value| value.shifted(by)).collect()
    }
}

macro_rules! tuples {
    ($(($($name:ident $index:tt),+)),*) => {
        $(impl<$($name: Shift),+> Shift for ($($name,)+) {
            fn shifted(&self, by: &RecordShift) -> Self {
                ($(self.$index.shifted(by),)+)
            }
        })*
    };
}

impl Shift for craft_behavior::HitShape {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.translated(by.offset)
    }
}

/// A hit shape in a body layer's space (`local`) moves too: a record holding a transformed body never moves, as the
/// body's own layer blocks it.
impl<O: Clone> Shift for craft_behavior::Shape<O> {
    fn shifted(&self, by: &RecordShift) -> Self {
        self.translated(by.offset)
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
}

tuples!((A 0), (A 0, B 1), (A 0, B 1, C 2), (A 0, B 1, C 2, D 3), (A 0, B 1, C 2, D 3, E 4), (A 0, B 1, C 2, D 3, E 4, F 5));
