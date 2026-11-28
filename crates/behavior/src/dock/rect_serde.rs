//! Saved rects are `[x, y, width, height]`: the shape older files hold. `#[serde(with = "..")]` adapters and the
//! two conversions for readers that build the array themselves.

use emath::{Pos2, Rect, Vec2};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `[x, y, width, height]` of `rect`.
pub(crate) fn to_array(rect: Rect) -> [f32; 4] {
    [rect.min.x, rect.min.y, rect.width(), rect.height()]
}

/// The rect at `[x, y, width, height]`.
pub(crate) fn from_array([x, y, width, height]: [f32; 4]) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height))
}

pub(crate) fn serialize<S: Serializer>(rect: &Rect, serializer: S) -> Result<S::Ok, S::Error> {
    to_array(*rect).serialize(serializer)
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Rect, D::Error> {
    <[f32; 4]>::deserialize(deserializer).map(from_array)
}

/// The adapter for `Option<Rect>` fields. A field using it also needs `#[serde(default)]`.
pub(crate) mod option {
    use super::{Deserialize, Deserializer, Rect, Serialize, Serializer, from_array, to_array};

    pub(crate) fn serialize<S: Serializer>(rect: &Option<Rect>, serializer: S) -> Result<S::Ok, S::Error> {
        rect.map(to_array).serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Rect>, D::Error> {
        Option::<[f32; 4]>::deserialize(deserializer).map(|rect| rect.map(from_array))
    }
}
