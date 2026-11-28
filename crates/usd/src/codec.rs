//! Shared USD Crate (`.usdc`) wire constants and type metadata.

/// Magic bytes at the start of every USD Crate file.
pub const MAGIC: &[u8; 8] = b"PXR-USDC";
/// Size of the bootstrap header: magic, version, TOC offset, reserved.
pub const BOOTSTRAP_LEN: usize = 88;
/// Arrays shorter than this are never stored compressed.
pub const MIN_COMPRESSED_ARRAY: usize = 16;

/// ValueRep array flag.
pub const IS_ARRAY: u64 = 1 << 63;
/// ValueRep inline flag.
pub const IS_INLINED: u64 = 1 << 62;
/// ValueRep compressed flag.
pub const IS_COMPRESSED: u64 = 1 << 61;
/// ValueRep array-edit flag.
pub const IS_ARRAY_EDIT: u64 = 1 << 60;
/// ValueRep 48-bit payload mask.
pub const PAYLOAD_MASK: u64 = (1 << 48) - 1;

/// Value type ids (Crate data types).
pub mod ty {
    pub const BOOL: u8 = 1;
    pub const UCHAR: u8 = 2;
    pub const INT: u8 = 3;
    pub const UINT: u8 = 4;
    pub const INT64: u8 = 5;
    pub const UINT64: u8 = 6;
    pub const HALF: u8 = 7;
    pub const FLOAT: u8 = 8;
    pub const DOUBLE: u8 = 9;
    pub const STRING: u8 = 10;
    pub const TOKEN: u8 = 11;
    pub const ASSET_PATH: u8 = 12;
    pub const MATRIX2D: u8 = 13;
    pub const MATRIX3D: u8 = 14;
    pub const MATRIX4D: u8 = 15;
    pub const QUATD: u8 = 16;
    pub const QUATF: u8 = 17;
    pub const QUATH: u8 = 18;
    pub const VEC2D: u8 = 19;
    pub const VEC2F: u8 = 20;
    pub const VEC3F: u8 = 24;
    pub const VEC4F: u8 = 28;
    pub const VEC4I: u8 = 30;
    pub const TOKEN_LIST_OP: u8 = 32;
    pub const PATH_LIST_OP: u8 = 34;
    pub const REFERENCE_LIST_OP: u8 = 35;
    pub const PATH_VECTOR: u8 = 40;
    pub const TOKEN_VECTOR: u8 = 41;
    pub const SPECIFIER: u8 = 42;
    pub const PERMISSION: u8 = 43;
    pub const VARIABILITY: u8 = 44;
    pub const TIME_SAMPLES: u8 = 46;
    pub const DOUBLE_VECTOR: u8 = 48;
}

/// Scalar kind of numeric Crate types.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scalar {
    U8,
    I32,
    U32,
    I64,
    U64,
    F16,
    F32,
    F64,
}

impl Scalar {
    /// Byte size of one scalar element.
    pub fn size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::F16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}

/// `(scalar, components)` of numeric type `t`, or `None` for non-numeric types.
pub fn numeric(t: u8) -> Option<(Scalar, usize)> {
    Some(match t {
        ty::BOOL | ty::UCHAR => (Scalar::U8, 1),
        ty::INT => (Scalar::I32, 1),
        ty::UINT => (Scalar::U32, 1),
        ty::INT64 => (Scalar::I64, 1),
        ty::UINT64 => (Scalar::U64, 1),
        ty::HALF => (Scalar::F16, 1),
        ty::FLOAT => (Scalar::F32, 1),
        ty::DOUBLE => (Scalar::F64, 1),
        ty::MATRIX2D => (Scalar::F64, 4),
        ty::MATRIX3D => (Scalar::F64, 9),
        ty::MATRIX4D => (Scalar::F64, 16),
        ty::QUATD => (Scalar::F64, 4),
        ty::QUATF => (Scalar::F32, 4),
        ty::QUATH => (Scalar::F16, 4),
        // Vec{2,3,4}{d,f,h,i}: ids 19..=30 in that order.
        ty::VEC2D..=ty::VEC4I => {
            let k = t - ty::VEC2D;
            let scalar = [Scalar::F64, Scalar::F32, Scalar::F16, Scalar::I32][usize::from(k % 4)];
            (scalar, usize::from(k / 4) + 2)
        }
        _ => return None,
    })
}

/// Upper bound of the encoded size of `n` integers of `width` bytes.
pub fn encoded_ints_len(n: usize, width: usize) -> usize {
    if n == 0 { 0 } else { width.saturating_add(n.saturating_mul(2).saturating_add(7) / 8).saturating_add(n.saturating_mul(width)) }
}
