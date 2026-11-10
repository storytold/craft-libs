//! Splits the scene into its blocks without decoding them.
//!
//! Layout: `u64` size, `u32` flags, root node block, `u32` mesh count + mesh blocks,
//! `u32` material count + material blocks, then animations, textures, lights and cameras (the `tail`, read by
//! [`super::AssbinCamera`]).

use super::RECORD;
use super::block::{read_block, read_count, read_u32};
use super::header::AssbinHeader;
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;
use std::borrow::Cow;

/// Smallest possible mesh or material block: its size field and one `u32`.
const MIN_ITEM_LEN: usize = 12;

/// The scene bytes behind the header, decompressed if needed. Owns what the blocks borrow.
pub fn scene_bytes(data: &[u8]) -> Result<Cow<'_, [u8]>, DecodeError> {
    let header = AssbinHeader::parse(data)?;
    if header.shortened {
        return Err(DecodeError::invalid(RECORD, "shortened dump carries no mesh data"));
    }
    header.scene_bytes(data)
}

/// The content of each top-level block, borrowed from the scene bytes.
pub struct SceneBlocks<'a> {
    pub root_node: &'a [u8],
    pub meshes: Vec<&'a [u8]>,
    pub materials: Vec<&'a [u8]>,
    /// Everything after the materials: the animation, texture, light and camera lists.
    pub tail: &'a [u8],
}

impl<'a> SceneBlocks<'a> {
    pub fn split(scene: &'a [u8]) -> Result<Self, DecodeError> {
        let mut outer = ByteReader::new(scene);
        let mut r = ByteReader::new(read_block(&mut outer)?);
        let _flags = read_u32(&mut r)?;
        let root_node = read_block(&mut r)?;
        let meshes = read_blocks(&mut r)?;
        let materials = read_blocks(&mut r)?;
        let tail = r.bytes(r.remaining()).map_err(DecodeError::truncated(super::RECORD))?;
        Ok(SceneBlocks { root_node, meshes, materials, tail })
    }
}

/// Reads a `u32` count followed by that many blocks.
fn read_blocks<'a>(r: &mut ByteReader<'a>) -> Result<Vec<&'a [u8]>, DecodeError> {
    let count = read_count(r, MIN_ITEM_LEN)?;
    (0..count).map(|_| read_block(r)).collect()
}
