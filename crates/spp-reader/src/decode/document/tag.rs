//! Value type tags of the reference app's object serialisation and how much data each one carries.
//! Meanings are inferred from field names and values in sample files; sizes are verified on
//! files from 2019 to 2023 and, for the compact layout, on files saved by version 12.2.
//! Tags 0x07 and 0x08 (likely int vectors) were never seen and stay unknown.

/// One value type. The discriminant is the tag byte in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ValueTag {
    Float = 0x01,
    Vec2 = 0x02,
    Vec3 = 0x03,
    Vec4 = 0x04,
    Int = 0x05,
    IVec2 = 0x06,
    /// 8 bytes (4 in the compact layout); enum-like values (`usage`, `format`, `alphaType`).
    Enum = 0x09,
    /// 1 byte, `0x00` or `0xff`.
    Bool = 0x0a,
    /// 4 bytes (`randomSeed`, `tags`).
    UInt = 0x0b,
    /// 8 bytes; the object's own id (`uid`).
    Uid = 0x0c,
    Mat3 = 0x0d,
    Mat4 = 0x0e,
    /// 8 bytes; the `uid` of another object (`source`, `selection`).
    UidRef = 0x0f,
    /// `u32` byte length + bytes.
    String = 0x10,
    /// `u32` end offset + `u32` count + tagged values (in practice [`ValueTag::Value`] objects).
    ValueList = 0x11,
    /// An object that may be absent: flag `0xff` = none, `0x00` = object follows.
    Pointer = 0x12,
    /// `u32` end offset + `u32` count + tagged values (in practice [`ValueTag::Pointer`]s).
    PointerList = 0x13,
    /// An object stored inline; same layout as [`ValueTag::Pointer`].
    Value = 0x14,
    /// 8 bytes; channel type values (`type`, `channelType`).
    ChannelType = 0x15,
    /// 16 bytes; the wide channel mask of the compact layout (`channelTypes`).
    WideMask = 0x16,
}

/// How the bytes after a tag are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    Fixed(usize),
    String,
    List,
    Object,
}

impl ValueTag {
    /// A tag as stored in a compact layout's field table. `ChannelType` was not seen there (`type` is an enum);
    /// its size is unknown, so it stays an unknown tag.
    pub fn from_compact(tag: u32) -> Option<Self> {
        match tag {
            0x15 => None,
            0x16 => Some(ValueTag::WideMask),
            _ => Self::from_byte(u8::try_from(tag).ok()?),
        }
    }

    /// The bytes after a value in the compact layout.
    pub fn compact_payload(self) -> Payload {
        match self {
            ValueTag::Enum => Payload::Fixed(4),
            _ => self.payload(),
        }
    }

    pub fn from_byte(byte: u8) -> Option<Self> {
        use ValueTag::*;
        Some(match byte {
            0x01 => Float,
            0x02 => Vec2,
            0x03 => Vec3,
            0x04 => Vec4,
            0x05 => Int,
            0x06 => IVec2,
            0x09 => Enum,
            0x0a => Bool,
            0x0b => UInt,
            0x0c => Uid,
            0x0d => Mat3,
            0x0e => Mat4,
            0x0f => UidRef,
            0x10 => String,
            0x11 => ValueList,
            0x12 => Pointer,
            0x13 => PointerList,
            0x14 => Value,
            0x15 => ChannelType,
            _ => return None,
        })
    }

    pub fn payload(self) -> Payload {
        use ValueTag::*;
        match self {
            Bool => Payload::Fixed(1),
            Float | Int | UInt => Payload::Fixed(4),
            Vec2 | IVec2 | Enum | Uid | UidRef | ChannelType => Payload::Fixed(8),
            Vec3 => Payload::Fixed(12),
            Vec4 => Payload::Fixed(16),
            Mat3 => Payload::Fixed(36),
            Mat4 => Payload::Fixed(64),
            WideMask => Payload::Fixed(16),
            String => Payload::String,
            ValueList | PointerList => Payload::List,
            Pointer | Value => Payload::Object,
        }
    }
}
