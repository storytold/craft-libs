//! The compiled graph (magic `SBAM`): header, root record, node table and the value
//! expressions the nodes carry. Offsets inside the file are relative to [`SECTION_BASE`]; [`Section`] reads
//! at such offsets.
//!
//! Layout (found by reading packages):
//! - header (0x34 bytes): magic, version, cooker build, asm uid, sizes, root offset at 0x2c;
//! - root: output uids, (type, uid) per graph input, the node table;
//! - nodes: one record each, [`NodeRecord`]; nodes name their image inputs by node index;
//! - a block of input default values just before the root.

pub mod atomic;
pub mod expr;
pub mod fxmap;
pub mod modes;
pub mod node;
mod section;

pub use atomic::{AtomicNode, Kind, Param, Value};
pub use expr::{Expr, Instr, NO_OPERAND, Operands, VALUE_TYPE_BITS, ValueType, swizzle_source};
pub use modes::{BlendAlpha, NormalOptions, RawLayout, SampleDepth, StoredChannels, StoredImage, TransformSampling};
pub use node::NodeRecord;
pub use section::Section;

use crate::decode::DecodeError;

const RECORD: &str = "compiled graph";
const MAGIC: &[u8; 4] = crate::format_names::COMPILED_GRAPH_MAGIC;
/// Offsets inside the file count from here.
pub const SECTION_BASE: usize = 0x34;
// Format data: header fields (byte offsets from the start of the file).
/// The format version, in the high half of this word.
const HEADER_VERSION: u32 = 0x04;
const HEADER_COOKER_BUILD: u32 = 0x08;
const HEADER_UID: u32 = 0x0c;
/// Section offset of the root record: u32 counts (outputs in the low half, inputs in the high half), then the entries.
const HEADER_ROOT: u32 = 0x2c;
/// An embedded JPEG starts with its start-of-image marker and the next marker's first byte.
const JPEG_START: [u8; 3] = [0xff, 0xd8, 0xff];

/// One graph input as the compiled code lists it: its type code and uid (as in the XML description).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsmInput {
    pub ty: u32,
    pub uid: u32,
}

#[derive(Debug, Clone)]
pub struct Assembly {
    pub version: u16,
    pub cooker_build: u32,
    pub asm_uid: u32,
    pub outputs: Vec<u32>,
    pub inputs: Vec<AsmInput>,
    pub nodes: Vec<NodeRecord>,
    /// Section offset of the input default values (the record before the root).
    pub defaults_offset: u32,
    /// Section offset of the root record.
    pub root_offset: u32,
    /// Section offset just past the node table (the output bindings follow).
    pub node_table_end: u32,
    data: Vec<u8>,
}

impl Assembly {
    pub fn parse(bytes: &[u8]) -> Result<Assembly, DecodeError> {
        if bytes.get(..4) != Some(MAGIC.as_slice()) {
            return Err(DecodeError::BadMagic { record: RECORD });
        }
        let header = Section::new(bytes, 0);
        let version = u16::try_from(header.u32(HEADER_VERSION)? >> 16).unwrap_or(0);
        let cooker_build = header.u32(HEADER_COOKER_BUILD)?;
        let asm_uid = header.u32(HEADER_UID)?;
        let root_offset = header.u32(HEADER_ROOT)?;
        let s = Section::new(bytes, SECTION_BASE);

        let counts = s.u32(root_offset)?;
        let (n_out, n_in) = (counts & 0xffff, counts >> 16);
        let mut at = root_offset + 4;
        let mut outputs = Vec::new();
        for _ in 0..n_out {
            outputs.push(s.u32(at)?);
            at += 4;
        }
        let mut inputs = Vec::new();
        for _ in 0..n_in {
            inputs.push(AsmInput { ty: s.u32(at)?, uid: s.u32(at + 4)? });
            at += 8;
        }
        let n_nodes = s.u32(at)?;
        let table = s.u32(at + 4)?;
        let node_table_end = s.u32(at + 8)?;
        let defaults_offset = s.u32(at + 12)?;
        if n_nodes > (bytes.len() / 4) as u32 {
            return Err(DecodeError::invalid(RECORD, format!("node count {n_nodes} larger than the file")));
        }
        let mut offsets = Vec::new();
        for i in 0..n_nodes {
            offsets.push(s.u32(table + 4 * i)?);
        }
        let nodes = node::split(&s, &offsets, defaults_offset)?;
        Ok(Assembly { version, cooker_build, asm_uid, outputs, inputs, nodes, defaults_offset, root_offset, node_table_end, data: bytes.to_vec() })
    }

    /// Reader over the file's section (offsets as stored in the file).
    pub fn section(&self) -> Section<'_> {
        Section::new(&self.data, SECTION_BASE)
    }

    /// Every node decoded into its atomic operation and parameters.
    pub fn atomic_nodes(&self) -> Vec<Result<AtomicNode, DecodeError>> {
        let s = self.section();
        self.nodes.iter().map(|n| atomic::decode(n, &s)).collect()
    }

    /// The JPEG stored at section offset `off`: an image node's body word points at `{u32 length, bytes}`.
    /// `None` when `off` holds anything else (other embedded forms are not understood yet).
    pub fn embedded_jpeg(&self, off: u32) -> Option<&[u8]> {
        let s = self.section();
        let len = s.u32(off).ok()?;
        s.bytes(off.checked_add(4)?, len).filter(|b| b.starts_with(&JPEG_START))
    }

    pub fn input(&self, uid: u32) -> Option<&AsmInput> {
        self.inputs.iter().find(|i| i.uid == uid)
    }
}

#[cfg(test)]
mod tests;
