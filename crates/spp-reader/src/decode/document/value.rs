//! [`Scalar`]: a typed reading of one fixed-size or string value from the object serialisation.

use super::tag::ValueTag;
use std::fmt;

/// One decoded value. Vectors and matrices are kept as their float components.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Float(f32),
    /// vec2/vec3/vec4 and 3×3/4×4 matrices (row order as stored).
    Floats(Vec<f32>),
    Int(i32),
    IVec2([i32; 2]),
    Enum(u64),
    Bool(bool),
    UInt(u32),
    /// The object's own id.
    Uid(u64),
    /// Another object's id.
    UidRef(u64),
    String(String),
    ChannelType(u64),
    /// The 128-bit channel mask of the compact layout.
    WideMask(u128),
}

/// File-format data: the compact layout numbers channels by index (bit `i` of a 128-bit mask). Indices 0-13 and
/// 22-48 are the tagged layout's bits; indices 14-21 hold channels the tagged layout never had, while its user
/// channels 0-7 sat at bits 14-21 and moved to indices 64-71. Measured: one fresh project with every channel type.
const INDEX_ONLY_FIRST: u32 = 14;
const INDEX_ONLY_COUNT: u32 = 8;
/// Where indices 14-21 go in the tagged numbering: bits 49-56, which no channel uses in either layout.
const INDEX_ONLY_NARROW: u32 = 49;
const USER_WIDE: u32 = 64;
const USER_COUNT: u32 = 8;
const USER_NARROW: u32 = 14;

/// A compact-layout mask in the tagged numbering (see [`INDEX_ONLY_FIRST`]). User channels 8 and up have no bit there
/// and are dropped.
fn bit_of_wide(mask: u128) -> u64 {
    let field = |m: u64, at: u32, n: u32| (m >> at) & ((1 << n) - 1);
    let low = mask as u64;
    let index_only = field(low, INDEX_ONLY_FIRST, INDEX_ONLY_COUNT);
    let users = field((mask >> USER_WIDE) as u64, 0, USER_COUNT);
    let cleared = low & !(((1 << INDEX_ONLY_COUNT) - 1) << INDEX_ONLY_FIRST) & !(((1 << INDEX_ONLY_COUNT) - 1) << INDEX_ONLY_NARROW);
    cleared | index_only << INDEX_ONLY_NARROW | users << USER_NARROW
}

impl Scalar {
    /// Decodes a payload as delivered by [`super::Visitor::scalar`]; `None` if the size does not fit the tag.
    pub fn decode(tag: ValueTag, payload: &[u8]) -> Option<Scalar> {
        let u64_le = || payload.try_into().ok().map(u64::from_le_bytes);
        let u32_le = || payload.try_into().ok().map(u32::from_le_bytes);
        Some(match tag {
            ValueTag::Float => Scalar::Float(f32::from_bits(u32_le()?)),
            ValueTag::Vec2 | ValueTag::Vec3 | ValueTag::Vec4 | ValueTag::Mat3 | ValueTag::Mat4 => {
                if !payload.len().is_multiple_of(4) {
                    return None;
                }
                Scalar::Floats(payload.as_chunks::<4>().0.iter().map(|&c| f32::from_le_bytes(c)).collect())
            }
            ValueTag::Int => Scalar::Int(u32_le()? as i32),
            ValueTag::IVec2 => {
                let v = u64_le()?;
                Scalar::IVec2([v as u32 as i32, (v >> 32) as u32 as i32])
            }
            ValueTag::Enum => Scalar::Enum(u64_le()?),
            ValueTag::Bool => Scalar::Bool(*payload.first()? != 0),
            ValueTag::UInt => Scalar::UInt(u32_le()?),
            ValueTag::Uid => Scalar::Uid(u64_le()?),
            ValueTag::UidRef => Scalar::UidRef(u64_le()?),
            ValueTag::ChannelType => Scalar::ChannelType(u64_le()?),
            ValueTag::WideMask => Scalar::WideMask(u128::from_le_bytes(payload.try_into().ok()?)),
            ValueTag::String => Scalar::String(String::from_utf8_lossy(payload).into_owned()),
            ValueTag::ValueList | ValueTag::Pointer | ValueTag::PointerList | ValueTag::Value => return None,
        })
    }

    /// One channel's bit in a channel mask. Files up to the tagged layout store the bit itself (tag 0x15); the compact
    /// layout stores its index in the mask as an enum, see [`Scalar::channel_mask`] for where user channels go.
    pub fn channel_bit(&self) -> Option<u64> {
        match self {
            Scalar::Enum(index) => Some(bit_of_wide(u128::from(1u8).checked_shl(u32::try_from(*index).ok()?)?)).filter(|b| *b != 0),
            other => other.as_u64(),
        }
    }

    /// A channel mask in the numbering of the tagged layout. Wide masks keep the shared channels' bits, move their
    /// indices 14-21 to bits 49-56 and the eight user channels, which they keep at bits 64-71, back to bits 14-21.
    pub fn channel_mask(&self) -> Option<u64> {
        match self {
            Scalar::WideMask(mask) => Some(bit_of_wide(*mask)),
            other => other.as_u64(),
        }
    }

    /// The value as an id (uid, reference, enum or integer), if it is one. A wide mask gives its low 64 bits.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Scalar::WideMask(v) => Some(*v as u64),
            Scalar::Uid(v) | Scalar::UidRef(v) | Scalar::Enum(v) | Scalar::ChannelType(v) => Some(*v),
            Scalar::UInt(v) => Some(*v as u64),
            Scalar::Int(v) => u64::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scalar::Float(v) => write!(f, "{v}"),
            Scalar::Floats(v) => write!(f, "{v:?}"),
            Scalar::Int(v) => write!(f, "{v}"),
            Scalar::IVec2(v) => write!(f, "{v:?}"),
            Scalar::Enum(v) => write!(f, "enum {v}"),
            Scalar::Bool(v) => write!(f, "{v}"),
            Scalar::UInt(v) => write!(f, "{v}u"),
            Scalar::Uid(v) => write!(f, "uid {v}"),
            Scalar::UidRef(v) => write!(f, "-> {v}"),
            Scalar::String(v) => write!(f, "{v:?}"),
            Scalar::ChannelType(v) => write!(f, "channel {v}"),
            Scalar::WideMask(v) => write!(f, "mask {v:#x}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_by_tag() {
        assert_eq!(Scalar::decode(ValueTag::IVec2, &[11, 0, 0, 0, 10, 0, 0, 0]), Some(Scalar::IVec2([11, 10])));
        assert_eq!(Scalar::decode(ValueTag::Uid, &7u64.to_le_bytes()).and_then(|s| s.as_u64()), Some(7));
        assert_eq!(Scalar::decode(ValueTag::Float, &[0, 0]), None);
        assert_eq!(Scalar::decode(ValueTag::String, b"ab"), Some(Scalar::String("ab".into())));
        assert_eq!(Scalar::decode(ValueTag::Bool, &[0xff]), Some(Scalar::Bool(true)));
        assert_eq!(Scalar::decode(ValueTag::Enum, &[4, 0, 0, 0, 0, 0, 0, 0]), Some(Scalar::Enum(4)));
        let wide = Scalar::decode(ValueTag::WideMask, &[0x80; 16]);
        assert_eq!(wide.and_then(|s| s.as_u64()), Some(0x8080_8080_8080_8080));
    }

    #[test]
    fn channels_are_numbered_as_in_the_tagged_layout() {
        assert_eq!(Scalar::Enum(13).channel_bit(), Some(8192));
        assert_eq!(Scalar::Enum(64).channel_bit(), Some(1 << 14));
        assert_eq!(Scalar::Enum(200).channel_bit(), None);
        assert_eq!(Scalar::Enum(14).channel_bit(), Some(1 << 49), "index-only channels leave the user bits free");
        assert_eq!(Scalar::Enum(21).channel_bit(), Some(1 << 56));
        assert_eq!(Scalar::Enum(71).channel_bit(), Some(1 << 21));
        assert_eq!(Scalar::Enum(72).channel_bit(), None, "user channel 8 has no bit in the tagged numbering");
        assert_eq!(Scalar::Enum(48).channel_bit(), Some(1 << 48));
        assert_eq!(Scalar::ChannelType(8192).channel_bit(), Some(8192));
        assert_eq!(Scalar::WideMask(128).channel_mask(), Some(128));
        assert_eq!(Scalar::WideMask(u128::MAX).channel_mask(), Some(u64::MAX));
        assert_eq!(Scalar::WideMask(0xff << 64 | 0x1f3a).channel_mask(), Some(0x3fc000 | 0x1f3a));
        assert_eq!(Scalar::WideMask(1 << 17 | 1 << 66).channel_mask(), Some(1 << 52 | 1 << 16));
        assert_eq!(Scalar::Enum(5).channel_mask(), Some(5));
    }
}
