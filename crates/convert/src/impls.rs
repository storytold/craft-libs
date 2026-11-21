//! [`Convert`] and [`ConvertRef`] for the primitives and the standard containers.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::{BuildHasher, Hash};
use std::rc::Rc;
use std::sync::Arc;

use crate::{Convert, ConvertRef, Floats};

/// Types that are the same on both sides: moved by value, cloned by reference.
macro_rules! same {
    ($($ty:ty),* $(,)?) => {$(
        impl<M> Convert<$ty, M> for $ty {
            #[inline]
            fn convert(self) -> $ty {
                self
            }
        }

        impl<M> ConvertRef<$ty, M> for $ty {
            #[inline]
            fn convert_ref(&self) -> $ty {
                self.clone()
            }
        }
    )*};
}

same!((), bool, char, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, String);

impl<M> ConvertRef<String, M> for str {
    #[inline]
    fn convert_ref(&self) -> String {
        self.to_owned()
    }
}

impl<M: Floats> Convert<f32, M> for f32 {
    #[inline]
    fn convert(self) -> f32 {
        M::f32(self)
    }
}

impl<M: Floats> ConvertRef<f32, M> for f32 {
    #[inline]
    fn convert_ref(&self) -> f32 {
        M::f32(*self)
    }
}

impl<M: Floats> Convert<f64, M> for f64 {
    #[inline]
    fn convert(self) -> f64 {
        M::f64(self)
    }
}

impl<M: Floats> ConvertRef<f64, M> for f64 {
    #[inline]
    fn convert_ref(&self) -> f64 {
        M::f64(*self)
    }
}

impl<A: Convert<B, M>, B, M> Convert<Option<B>, M> for Option<A> {
    fn convert(self) -> Option<B> {
        self.map(Convert::convert)
    }
}

impl<A: ConvertRef<B, M>, B, M> ConvertRef<Option<B>, M> for Option<A> {
    fn convert_ref(&self) -> Option<B> {
        self.as_ref().map(ConvertRef::convert_ref)
    }
}

impl<A: Convert<B, M>, B, M> Convert<Vec<B>, M> for Vec<A> {
    fn convert(self) -> Vec<B> {
        self.into_iter().map(Convert::convert).collect()
    }
}

impl<A: ConvertRef<B, M>, B, M> ConvertRef<Vec<B>, M> for Vec<A> {
    fn convert_ref(&self) -> Vec<B> {
        self.iter().map(ConvertRef::convert_ref).collect()
    }
}

impl<A: ConvertRef<B, M>, B, M> ConvertRef<Vec<B>, M> for [A] {
    fn convert_ref(&self) -> Vec<B> {
        self.iter().map(ConvertRef::convert_ref).collect()
    }
}

impl<A: Convert<B, M>, B, M> Convert<VecDeque<B>, M> for VecDeque<A> {
    fn convert(self) -> VecDeque<B> {
        self.into_iter().map(Convert::convert).collect()
    }
}

impl<A: ConvertRef<B, M>, B, M> ConvertRef<VecDeque<B>, M> for VecDeque<A> {
    fn convert_ref(&self) -> VecDeque<B> {
        self.iter().map(ConvertRef::convert_ref).collect()
    }
}

impl<A: Convert<B, M>, B, M> Convert<Box<B>, M> for Box<A> {
    fn convert(self) -> Box<B> {
        Box::new((*self).convert())
    }
}

impl<A: ConvertRef<B, M>, B, M> ConvertRef<Box<B>, M> for Box<A> {
    fn convert_ref(&self) -> Box<B> {
        Box::new((**self).convert_ref())
    }
}

/// One owner: the value is moved out. Shared: it is cloned, and the other owners keep theirs.
impl<A: Convert<B, M> + Clone, B, M> Convert<Arc<B>, M> for Arc<A> {
    fn convert(self) -> Arc<B> {
        Arc::new(Arc::unwrap_or_clone(self).convert())
    }
}

impl<A: ConvertRef<B, M>, B, M> ConvertRef<Arc<B>, M> for Arc<A> {
    fn convert_ref(&self) -> Arc<B> {
        Arc::new((**self).convert_ref())
    }
}

/// One owner: the value is moved out. Shared: it is cloned, and the other owners keep theirs.
impl<A: Convert<B, M> + Clone, B, M> Convert<Rc<B>, M> for Rc<A> {
    fn convert(self) -> Rc<B> {
        Rc::new(Rc::unwrap_or_clone(self).convert())
    }
}

impl<A: ConvertRef<B, M>, B, M> ConvertRef<Rc<B>, M> for Rc<A> {
    fn convert_ref(&self) -> Rc<B> {
        Rc::new((**self).convert_ref())
    }
}

impl<K: Convert<L, M>, L: Ord, A: Convert<B, M>, B, M> Convert<BTreeMap<L, B>, M> for BTreeMap<K, A> {
    fn convert(self) -> BTreeMap<L, B> {
        self.into_iter().map(|(key, value)| (key.convert(), value.convert())).collect()
    }
}

impl<K: ConvertRef<L, M>, L: Ord, A: ConvertRef<B, M>, B, M> ConvertRef<BTreeMap<L, B>, M> for BTreeMap<K, A> {
    fn convert_ref(&self) -> BTreeMap<L, B> {
        self.iter().map(|(key, value)| (key.convert_ref(), value.convert_ref())).collect()
    }
}

impl<K: Convert<L, M>, L: Eq + Hash, A: Convert<B, M>, B, S, T: BuildHasher + Default, M> Convert<HashMap<L, B, T>, M> for HashMap<K, A, S> {
    fn convert(self) -> HashMap<L, B, T> {
        self.into_iter().map(|(key, value)| (key.convert(), value.convert())).collect()
    }
}

impl<K: ConvertRef<L, M>, L: Eq + Hash, A: ConvertRef<B, M>, B, S, T: BuildHasher + Default, M> ConvertRef<HashMap<L, B, T>, M> for HashMap<K, A, S> {
    fn convert_ref(&self) -> HashMap<L, B, T> {
        self.iter().map(|(key, value)| (key.convert_ref(), value.convert_ref())).collect()
    }
}

impl<A: Convert<B, M>, B: Ord, M> Convert<BTreeSet<B>, M> for BTreeSet<A> {
    fn convert(self) -> BTreeSet<B> {
        self.into_iter().map(Convert::convert).collect()
    }
}

impl<A: ConvertRef<B, M>, B: Ord, M> ConvertRef<BTreeSet<B>, M> for BTreeSet<A> {
    fn convert_ref(&self) -> BTreeSet<B> {
        self.iter().map(ConvertRef::convert_ref).collect()
    }
}

impl<A: Convert<B, M>, B: Eq + Hash, S, T: BuildHasher + Default, M> Convert<HashSet<B, T>, M> for HashSet<A, S> {
    fn convert(self) -> HashSet<B, T> {
        self.into_iter().map(Convert::convert).collect()
    }
}

impl<A: ConvertRef<B, M>, B: Eq + Hash, S, T: BuildHasher + Default, M> ConvertRef<HashSet<B, T>, M> for HashSet<A, S> {
    fn convert_ref(&self) -> HashSet<B, T> {
        self.iter().map(ConvertRef::convert_ref).collect()
    }
}

impl<A: Convert<B, M>, B, M, const N: usize> Convert<[B; N], M> for [A; N] {
    fn convert(self) -> [B; N] {
        self.map(Convert::convert)
    }
}

impl<A: ConvertRef<B, M>, B, M, const N: usize> ConvertRef<[B; N], M> for [A; N] {
    fn convert_ref(&self) -> [B; N] {
        self.each_ref().map(ConvertRef::convert_ref)
    }
}

/// Tuples, member by member.
macro_rules! tuples {
    ($(($($a:ident $b:ident $i:tt),+))*) => {$(
        impl<$($a: Convert<$b, M>, $b,)+ M> Convert<($($b,)+), M> for ($($a,)+) {
            fn convert(self) -> ($($b,)+) {
                ($(self.$i.convert(),)+)
            }
        }

        impl<$($a: ConvertRef<$b, M>, $b,)+ M> ConvertRef<($($b,)+), M> for ($($a,)+) {
            fn convert_ref(&self) -> ($($b,)+) {
                ($(self.$i.convert_ref(),)+)
            }
        }
    )*};
}

tuples! {
    (A0 B0 0)
    (A0 B0 0, A1 B1 1)
    (A0 B0 0, A1 B1 1, A2 B2 2)
    (A0 B0 0, A1 B1 1, A2 B2 2, A3 B3 3)
    (A0 B0 0, A1 B1 1, A2 B2 2, A3 B3 3, A4 B4 4)
    (A0 B0 0, A1 B1 1, A2 B2 2, A3 B3 3, A4 B4 4, A5 B5 5)
    (A0 B0 0, A1 B1 1, A2 B2 2, A3 B3 3, A4 B4 4, A5 B5 5, A6 B6 6)
    (A0 B0 0, A1 B1 1, A2 B2 2, A3 B3 3, A4 B4 4, A5 B5 5, A6 B6 6, A7 B7 7)
}
