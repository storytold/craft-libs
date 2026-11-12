//! Lists, tables and properties ([MS-PST] 2.3): a node's heap with its subnodes, the property
//! context (2.3.3) and the table context (2.3.4), whose row matrix may be spread over blocks and
//! is read a block at a time.

use std::io::{Read, Seek};

use crate::bth;
use crate::bytes::{slice, u8_at, u16_at, u32_at};
use crate::error::{Cap, Error, Result};
use crate::header::Format;
use crate::heap::{CLIENT_PC, CLIENT_TC, Heap, is_hid};
use crate::ndb::{Ndb, Subnodes, is_internal};
use crate::prop::{self, PropValue, Properties, Property, fixed_size};

/// A node's data as a heap, and its subnodes (where values too large for the heap are kept).
pub(crate) struct NodeData {
    pub heap: Heap,
    pub subnodes: Subnodes,
}

/// How much more one call may read and keep: every node it loads, every table block and
/// every value, charged as read (so a value named twice costs twice). Past it, `cap`.
pub(crate) struct Budget {
    pub left: u64,
    /// A value larger than this is not read but left as [`PropValue::Large`].
    pub inline: u64,
    pub cap: Cap,
}

impl Budget {
    pub(crate) fn new(left: u64, inline: u64, cap: Cap) -> Budget {
        Budget { left, inline, cap }
    }

    pub(crate) fn take(&mut self, n: u64) -> Result<()> {
        self.left = self.left.checked_sub(n).ok_or(Error::Cap(self.cap))?;
        Ok(())
    }

    /// Charges what is kept of a value or a structure: refused past what is left.
    pub(crate) fn keep(&mut self, n: usize) -> Result<()> {
        self.take(n as u64)
    }

    /// Charges what is kept of a note of something left out: never refused (each is one per
    /// part already charged, so they are bounded), but what is left goes down.
    pub(crate) fn note(&mut self, n: usize) {
        self.left = self.left.saturating_sub(n as u64);
    }
}

impl NodeData {
    /// A node's heap and subnodes, charged to `budget`.
    pub(crate) fn load<R: Read + Seek>(ndb: &mut Ndb<R>, data: u64, sub: u64, budget: &mut Budget) -> Result<NodeData> {
        let max = ndb.limits.max_item_bytes.min(budget.left);
        let pieces = ndb.read_pieces(data, max, budget.cap)?;
        budget.take(pieces.iter().map(|p| p.len() as u64).sum())?;
        let heap = Heap::new(pieces)?;
        let subnodes = ndb.subnodes(sub)?;
        Ok(NodeData { heap, subnodes })
    }

    /// The size of the value an HNID names, without reading it.
    pub(crate) fn hnid_len<R: Read + Seek>(&self, ndb: &mut Ndb<R>, hnid: u32) -> Result<u64> {
        if is_hid(hnid) {
            return Ok(self.heap.item(hnid)?.len() as u64);
        }
        let entry = self.subnodes.get(&hnid).ok_or(Error::NotFound("a property's subnode"))?;
        ndb.data_size(entry.data)
    }

    /// The subnode an HNID names, when it names one.
    pub(crate) fn subnode(&self, hnid: u32) -> Option<crate::ndb::SubEntry> {
        if is_hid(hnid) { None } else { self.subnodes.get(&hnid).copied() }
    }

    /// The value an HNID names, refused past `max` bytes.
    pub(crate) fn hnid_bytes<R: Read + Seek>(&self, ndb: &mut Ndb<R>, hnid: u32, max: u64, cap: Cap) -> Result<Vec<u8>> {
        if is_hid(hnid) {
            let item = self.heap.item(hnid)?;
            if item.len() as u64 > max {
                return Err(Error::Cap(cap));
            }
            return Ok(item.to_vec());
        }
        let entry = self.subnodes.get(&hnid).ok_or(Error::NotFound("a property's subnode"))?;
        ndb.read_data(entry.data, max, cap)
    }

    /// A value stored apart: read when small enough, else [`PropValue::Large`].
    fn apart<R: Read + Seek>(&self, ndb: &mut Ndb<R>, ty: u16, hnid: u32, budget: &mut Budget) -> Result<PropValue> {
        let inline = budget.inline;
        let size = self.hnid_len(ndb, hnid)?;
        if size > inline {
            return Ok(PropValue::Large { size });
        }
        budget.take(size)?;
        let b = self.hnid_bytes(ndb, hnid, inline, budget.cap)?;
        // What making the value holds must fit in what is left, before it is made.
        if prop::building_cost(ty, &b) as u64 > budget.left {
            return Err(Error::Cap(budget.cap));
        }
        prop::stored(ty, &b)
    }
}

/// A value that failed to read becomes [`PropValue::Damaged`] with its reason, so that one
/// damaged property doesn't cost the rest; a cap, or the reader under the file failing, is
/// passed on.
fn salvage(ty: u16, r: Result<PropValue>) -> Result<PropValue> {
    match r {
        Ok(v) => Ok(v),
        Err(e @ (Error::Cap(_) | Error::Io(_))) => Err(e),
        Err(reason) => Ok(PropValue::Damaged { ty, reason }),
    }
}

/// A property context's raw records: id, type and the 4-byte value or HNID.
pub(crate) fn pc_records(node: &NodeData) -> Result<Vec<(u16, u16, u32)>> {
    if node.heap.client != CLIENT_PC {
        return Err(Error::Corrupt("a property context's signature"));
    }
    // Property ids are 16 bits: more records than that are repeats.
    let tree = bth::read(&node.heap, node.heap.user_root, 1 << 16, Error::Corrupt("a property context with more records than ids"))?;
    if tree.key_size != 2 || tree.data_size != 6 {
        return Err(Error::Corrupt("a property context's record sizes"));
    }
    tree.records.iter().map(|r| Ok((u16_at(r, 0)?, u16_at(r, 2)?, u32_at(r, 4)?))).collect()
}

/// One property context value from its record.
pub(crate) fn pc_value<R: Read + Seek>(ndb: &mut Ndb<R>, node: &NodeData, ty: u16, raw: u32, budget: &mut Budget) -> Result<PropValue> {
    let r = match fixed_size(ty) {
        Some(n) if n <= 4 => prop::fixed(ty, &raw.to_le_bytes()),
        Some(_) => node.heap.item(raw).and_then(|b| prop::fixed(ty, b)),
        None if is_known_variable(ty) => node.apart(ndb, ty, raw, budget),
        None => Ok(PropValue::Unknown { ty, data: raw.to_le_bytes().to_vec() }),
    };
    salvage(ty, r)
}

fn is_known_variable(ty: u16) -> bool {
    use crate::prop::types::{BINARY, MULTIPLE, OBJECT, RESTRICTION, RULE_ACTION, SERVER_ID, STRING, STRING8};
    ty & MULTIPLE != 0 || matches!(ty, STRING | STRING8 | BINARY | OBJECT | SERVER_ID | RESTRICTION | RULE_ACTION)
}

/// All of a property context's properties; those in `defer` that are stored apart are left as
/// [`PropValue::Large`] (an attachment's data, read on its own).
pub(crate) fn pc_properties<R: Read + Seek>(ndb: &mut Ndb<R>, node: &NodeData, budget: &mut Budget, defer: &[u16]) -> Result<Properties> {
    let mut out = Vec::new();
    for (id, ty, raw) in pc_records(node)? {
        let value = if defer.contains(&id) && fixed_size(ty).is_none() && is_known_variable(ty) && ty != crate::prop::types::OBJECT {
            salvage(ty, node.hnid_len(ndb, raw).map(|size| PropValue::Large { size }))?
        } else {
            pc_value(ndb, node, ty, raw, budget)?
        };
        let p = Property { id, ty, value };
        budget.keep(p.kept() as usize)?;
        out.push(p);
    }
    out.sort_by_key(|p| p.id);
    Ok(Properties(out))
}

#[derive(Debug, Clone, Copy)]
struct Column {
    id: u16,
    ty: u16,
    offset: usize,
    size: usize,
    bit: usize,
}

enum Matrix {
    None,
    Heap(Vec<u8>),
    Blocks { ids: Vec<u64>, per_block: usize, cached: Option<(usize, Vec<u8>)> },
}

/// A table context: its columns, its rows (`(row id, row index)` from the row index), and the
/// row matrix, read a block at a time.
pub(crate) struct Table {
    node: NodeData,
    cols: Vec<Column>,
    row_size: usize,
    ceb_start: usize,
    pub rows: Vec<(u32, u32)>,
    matrix: Matrix,
}

impl Table {
    /// A table of at most `max_rows` rows; more are refused (`cap`) as the row index is read.
    pub(crate) fn open<R: Read + Seek>(ndb: &mut Ndb<R>, node: NodeData, max_rows: usize, cap: Cap) -> Result<Table> {
        let format = ndb.header.format;
        if node.heap.client != CLIENT_TC {
            return Err(Error::Corrupt("a table context's signature"));
        }
        let info = node.heap.item(node.heap.user_root)?;
        if u8_at(info, 0)? != CLIENT_TC {
            return Err(Error::Corrupt("a table context's header"));
        }
        let ncols = usize::from(u8_at(info, 1)?);
        let rgib = [u16_at(info, 2)?, u16_at(info, 4)?, u16_at(info, 6)?, u16_at(info, 8)?].map(usize::from);
        let [b4, b2, b1, bm] = rgib;
        if !(b4 <= b2 && b2 <= b1 && b1 <= bm) || bm < 4 {
            return Err(Error::Corrupt("a table context's row layout"));
        }
        let row_index_hid = u32_at(info, 10)?;
        let rows_hnid = u32_at(info, 14)?;
        let mut cols = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let c = slice(info, i.saturating_mul(8).saturating_add(22), 8)?;
            let tag = u32_at(c, 0)?;
            let col = Column {
                id: (tag >> 16) as u16,
                ty: (tag & 0xFFFF) as u16,
                offset: usize::from(u16_at(c, 4)?),
                size: usize::from(u8_at(c, 6)?),
                bit: usize::from(u8_at(c, 7)?),
            };
            if col.offset.saturating_add(col.size) > b1 {
                return Err(Error::Corrupt("a table column outside its row"));
            }
            cols.push(col);
        }
        let trailer = format.trailer_size();
        let block_capacity = 8192usize.saturating_sub(trailer);
        if bm > block_capacity {
            return Err(Error::Corrupt("a table row larger than a block"));
        }
        let tree = bth::read(&node.heap, row_index_hid, max_rows, Error::Cap(cap))?;
        let index_size = match format {
            Format::Unicode => 4,
            Format::Ansi => 2,
        };
        if tree.key_size != 4 || tree.data_size != index_size {
            return Err(Error::Corrupt("a table's row index"));
        }
        let mut rows = Vec::with_capacity(tree.records.len());
        for r in &tree.records {
            let index = match format {
                Format::Unicode => u32_at(r, 4)?,
                Format::Ansi => u32::from(u16_at(r, 4)?),
            };
            rows.push((u32_at(r, 0)?, index));
        }
        let matrix = if rows_hnid == 0 {
            Matrix::None
        } else if is_hid(rows_hnid) {
            Matrix::Heap(node.heap.item(rows_hnid)?.to_vec())
        } else {
            let entry = node.subnodes.get(&rows_hnid).ok_or(Error::NotFound("a table's rows"))?;
            let max = ndb.limits.max_item_bytes;
            let (ids, _) = ndb.data_blocks(entry.data, max, Cap::ItemSize)?;
            Matrix::Blocks { ids, per_block: block_capacity.checked_div(bm).unwrap_or(1), cached: None }
        };
        if matches!(matrix, Matrix::None) && !rows.is_empty() {
            return Err(Error::NotFound("a table's rows"));
        }
        Ok(Table { node, cols, row_size: bm, ceb_start: b1, rows, matrix })
    }

    /// The bytes of the row at `index` in the row matrix.
    fn row_bytes<R: Read + Seek>(&mut self, ndb: &mut Ndb<R>, index: u32, budget: &mut Budget) -> Result<Vec<u8>> {
        let index = index as usize;
        let size = self.row_size;
        match &mut self.matrix {
            Matrix::None => Err(Error::NotFound("a table's rows")),
            Matrix::Heap(b) => Ok(slice(b, index.checked_mul(size).ok_or(Error::OutOfBounds)?, size)?.to_vec()),
            Matrix::Blocks { ids, per_block, cached } => {
                let per = (*per_block).max(1);
                let block = index.checked_div(per).unwrap_or(0);
                let within = index.checked_rem(per).unwrap_or(0);
                if cached.as_ref().is_none_or(|(i, _)| *i != block) {
                    let bid = *ids.get(block).ok_or(Error::Corrupt("a table row past its blocks"))?;
                    if is_internal(bid) {
                        return Err(Error::Corrupt("a table row in an internal block"));
                    }
                    let b = ndb.block(bid)?;
                    budget.take(b.len() as u64)?;
                    *cached = Some((block, b));
                }
                let data = cached.as_ref().map(|(_, d)| d.as_slice()).unwrap_or(&[]);
                Ok(slice(data, within.saturating_mul(size), size)?.to_vec())
            }
        }
    }

    /// The row with row id `id` at `index`: its cells, those that exist. Values larger than the
    /// inline cap come back as [`PropValue::Large`].
    pub(crate) fn row<R: Read + Seek>(&mut self, ndb: &mut Ndb<R>, id: u32, index: u32, budget: &mut Budget) -> Result<Properties> {
        let row = self.row_bytes(ndb, index, budget)?;
        if u32_at(&row, 0)? != id {
            return Err(Error::Corrupt("a table row under another row's id"));
        }
        let mut out = Vec::new();
        for col in self.cols.clone() {
            let byte = row.get(self.ceb_start.saturating_add(col.bit / 8)).copied().unwrap_or(0);
            if byte & (0x80u8 >> (col.bit % 8)) == 0 {
                continue;
            }
            let cell = slice(&row, col.offset, col.size)?;
            let value = match fixed_size(col.ty) {
                Some(n) if n <= 8 => {
                    if col.size < n {
                        PropValue::Unknown { ty: col.ty, data: cell.to_vec() }
                    } else {
                        salvage(col.ty, prop::fixed(col.ty, cell))?
                    }
                }
                Some(_) => salvage(col.ty, u32_at(cell, 0).and_then(|h| self.node.heap.item(h).and_then(|b| prop::fixed(col.ty, b))))?,
                // A cell too short for its reference loses that property alone.
                None if is_known_variable(col.ty) => salvage(col.ty, u32_at(cell, 0).and_then(|hnid| self.node.apart(ndb, col.ty, hnid, budget)))?,
                None => PropValue::Unknown { ty: col.ty, data: cell.to_vec() },
            };
            let p = Property { id: col.id, ty: col.ty, value };
            budget.keep(p.kept() as usize)?;
            out.push(p);
        }
        out.sort_by_key(|p| p.id);
        Ok(Properties(out))
    }
}
