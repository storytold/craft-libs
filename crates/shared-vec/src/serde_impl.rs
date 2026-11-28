//! A list reads and writes as a plain sequence, the same as `Vec<T>`.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::SharedVec;
use crate::node::{self, MAX};

impl<T: Serialize> Serialize for SharedVec<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<'de, T: Deserialize<'de> + Clone> Deserialize<'de> for SharedVec<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(SeqVisitor(PhantomData))
    }
}

struct SeqVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de> + Clone> Visitor<'de> for SeqVisitor<T> {
    type Value = SharedVec<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a sequence")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<SharedVec<T>, A::Error> {
        // The length the input announces is not used for an allocation: the elements go into
        // blocks as they arrive.
        let mut leaves = Vec::new();
        let mut leaf = Vec::with_capacity(MAX);
        while let Some(value) = seq.next_element()? {
            leaf.push(value);
            if leaf.len() == MAX {
                leaves.push(Arc::new(std::mem::replace(&mut leaf, Vec::with_capacity(MAX))));
            }
        }
        if !leaf.is_empty() {
            leaves.push(Arc::new(leaf));
        }
        Ok(SharedVec { root: node::build(leaves) })
    }
}
