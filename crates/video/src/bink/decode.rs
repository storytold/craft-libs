//! Plane decoding of Bink video: tree headers, bundles, block types, the two list coders and
//! the inverse transform. See the module README for how each rule was established.

use super::bits::Bits;
use super::tables::{INTER_QUANT, INTRA_QUANT, RLE_RUNS, RUN_SCANS, TREES};
use super::{Error, Result};

fn invalid<T>(msg: &str) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// A Huffman tree as read from a plane header: for the next 7 bits, the symbol and code length.
#[derive(Clone)]
struct Tree {
    lookup: [(u8, u8); 128],
}

impl Tree {
    /// Tree `number` (0 to 15) with `order[rank]` as the symbol of each rank.
    fn new(number: usize, order: &[u8; 16]) -> Tree {
        let mut lookup = [(0u8, 0u8); 128];
        let codes = TREES.get(number).unwrap_or(&TREES[0]);
        for (rank, &(code, len)) in codes.iter().enumerate() {
            let mask = (1u32 << len) - 1;
            for (next, slot) in lookup.iter_mut().enumerate() {
                if next as u32 & mask == u32::from(code) {
                    *slot = (order[rank], len);
                }
            }
        }
        Tree { lookup }
    }

    /// Reads a tree header: the tree number, then the order of its symbols.
    fn read(bits: &mut Bits) -> Result<Tree> {
        let number = bits.get(4)? as usize;
        let mut order: [u8; 16] = std::array::from_fn(|i| i as u8);
        if number == 0 {
            return Ok(Tree::new(0, &order));
        }
        if bits.bit()? {
            // a list of the first symbols; the others follow in ascending order
            let listed = bits.get(3)? as usize + 1;
            let mut used = 0u16;
            for slot in order.iter_mut().take(listed) {
                let s = bits.get(4)? as u8;
                if used >> s & 1 != 0 {
                    return invalid("a tree header lists a symbol twice");
                }
                used |= 1 << s;
                *slot = s;
            }
            let mut rest = (0..16u8).filter(|s| used >> s & 1 == 0);
            for slot in order.iter_mut().skip(listed) {
                *slot = rest.next().unwrap_or(0);
            }
        } else {
            // merge passes: runs of 1, 2, 4, 8 symbols are merged pairwise, one bit per choice
            let depth = bits.get(2)?;
            for pass in 0..=depth {
                let size = 1usize << pass;
                let mut merged = [0u8; 16];
                for start in (0..16).step_by(size * 2) {
                    let (mut a, mut b) = (start, start + size);
                    for slot in merged.iter_mut().skip(start).take(size * 2) {
                        let from_b = if a == start + size {
                            true
                        } else if b == start + size * 2 {
                            false
                        } else {
                            bits.bit()?
                        };
                        let at = if from_b { &mut b } else { &mut a };
                        *slot = order[*at];
                        *at += 1;
                    }
                }
                order = merged;
            }
        }
        Ok(Tree::new(number, &order))
    }

    fn symbol(&self, bits: &mut Bits) -> Result<u8> {
        let (symbol, len) = self.lookup[bits.peek(7) as usize & 127];
        bits.skip(u32::from(len))?;
        Ok(symbol)
    }
}

/// The 23 trees of a plane, in header order.
struct Trees {
    block_type: Tree,
    sub_type: Tree,
    /// High nibble of a colour, chosen by the previous colour's high nibble.
    colour_high: Vec<Tree>,
    colour_low: Tree,
    pattern: Tree,
    x_off: Tree,
    y_off: Tree,
    run: Tree,
}

impl Trees {
    fn read(bits: &mut Bits) -> Result<Trees> {
        Ok(Trees {
            block_type: Tree::read(bits)?,
            sub_type: Tree::read(bits)?,
            colour_high: (0..16).map(|_| Tree::read(bits)).collect::<Result<Vec<_>>>()?,
            colour_low: Tree::read(bits)?,
            pattern: Tree::read(bits)?,
            x_off: Tree::read(bits)?,
            y_off: Tree::read(bits)?,
            run: Tree::read(bits)?,
        })
    }
}

/// One stream of values, refilled in chunks at the start of a row of blocks.
struct Bundle {
    values: Vec<i32>,
    next: usize,
    /// A zero length was read: the bundle has no more chunks in this plane.
    finished: bool,
    /// Width in bits of a chunk's length field.
    length_bits: u32,
}

impl Bundle {
    /// A bundle whose rows need at most `per_row` values: its length field has as many bits as
    /// `per_row + 511` does.
    fn new(per_row: usize) -> Bundle {
        let length_bits = usize::BITS - per_row.saturating_add(511).leading_zeros();
        Bundle { values: Vec::new(), next: 0, finished: false, length_bits }
    }

    /// At a row start: the length of a new chunk if this bundle is used up and not finished.
    fn begin_chunk(&mut self, bits: &mut Bits) -> Result<Option<usize>> {
        if self.finished || self.next < self.values.len() {
            return Ok(None);
        }
        self.values.clear();
        self.next = 0;
        let len = bits.get(self.length_bits)? as usize;
        self.finished = len == 0;
        Ok((len != 0).then_some(len))
    }

    fn take(&mut self) -> Result<i32> {
        let v = self.values.get(self.next).copied();
        self.next += 1;
        v.ok_or_else(|| Error::Invalid("a block needs a value its bundle does not hold".into()))
    }
}

/// The nine bundles of a plane, in the order their chunks are stored.
struct Bundles {
    block_type: Bundle,
    sub_type: Bundle,
    colour: Bundle,
    pattern: Bundle,
    x_off: Bundle,
    y_off: Bundle,
    intra_dc: Bundle,
    inter_dc: Bundle,
    run: Bundle,
    /// High nibble of the last colour read, which selects the tree of the next one.
    colour_state: usize,
    /// Revision `f`: a colour is a sign bit and a 7-bit magnitude around 128.
    signed_colours: bool,
}

impl Bundles {
    /// Bundles for a plane `blocks_wide` blocks wide whose rows hold at most `sub_types` 16x16 blocks.
    fn new(blocks_wide: usize, sub_types: usize, signed_colours: bool) -> Bundles {
        let bw = blocks_wide;
        Bundles {
            block_type: Bundle::new(bw),
            sub_type: Bundle::new(sub_types),
            colour: Bundle::new(bw.saturating_mul(64)),
            pattern: Bundle::new(bw.saturating_mul(8)),
            x_off: Bundle::new(bw),
            y_off: Bundle::new(bw),
            intra_dc: Bundle::new(bw),
            inter_dc: Bundle::new(bw),
            run: Bundle::new(bw.saturating_mul(48)),
            colour_state: 0,
            signed_colours,
        }
    }

    /// Reads the chunk of every bundle that is used up, at the start of a row of blocks.
    fn refill(&mut self, bits: &mut Bits, trees: &Trees) -> Result<()> {
        read_types(&mut self.block_type, bits, &trees.block_type)?;
        read_types(&mut self.sub_type, bits, &trees.sub_type)?;
        if let Some(len) = self.colour.begin_chunk(bits)? {
            let state = &mut self.colour_state;
            let signed = self.signed_colours;
            let mut colour = |bits: &mut Bits| -> Result<i32> {
                let high = trees.colour_high.get(*state).ok_or_else(|| Error::Invalid("colour tree".into()))?.symbol(bits)?;
                let low = trees.colour_low.symbol(bits)?;
                *state = usize::from(high);
                let v = i32::from(high) << 4 | i32::from(low);
                Ok(match (signed, v & 0x80 != 0) {
                    (false, _) => v,
                    (true, true) => 128 - (v & 0x7f),
                    (true, false) => (128 + v) & 0xff,
                })
            };
            if bits.bit()? {
                let c = colour(bits)?;
                self.colour.values.resize(len, c);
            } else {
                for _ in 0..len {
                    let c = colour(bits)?;
                    self.colour.values.push(c);
                }
            }
        }
        if let Some(len) = self.pattern.begin_chunk(bits)? {
            for _ in 0..len {
                let low = trees.pattern.symbol(bits)?;
                let high = trees.pattern.symbol(bits)?;
                self.pattern.values.push(i32::from(high) << 4 | i32::from(low));
            }
        }
        read_offsets(&mut self.x_off, bits, &trees.x_off)?;
        read_offsets(&mut self.y_off, bits, &trees.y_off)?;
        read_dc(&mut self.intra_dc, bits, false)?;
        read_dc(&mut self.inter_dc, bits, true)?;
        if let Some(len) = self.run.begin_chunk(bits)? {
            if bits.bit()? {
                let v = bits.get(4)? as i32;
                self.run.values.resize(len, v);
            } else {
                for _ in 0..len {
                    self.run.values.push(i32::from(trees.run.symbol(bits)?));
                }
            }
        }
        Ok(())
    }
}

/// A chunk of block types or sub-block types: symbols 12 to 15 repeat the previous value, which
/// is 0 at the start of a chunk.
fn read_types(bundle: &mut Bundle, bits: &mut Bits, tree: &Tree) -> Result<()> {
    let Some(len) = bundle.begin_chunk(bits)? else { return Ok(()) };
    if bits.bit()? {
        let v = bits.get(4)? as i32;
        bundle.values.resize(len, v);
        return Ok(());
    }
    let mut last = 0;
    while bundle.values.len() < len {
        let s = tree.symbol(bits)?;
        match RLE_RUNS.get(usize::from(s).wrapping_sub(12)) {
            None => {
                last = i32::from(s);
                bundle.values.push(last);
            }
            Some(&run) => {
                if bundle.values.len() + run > len {
                    return invalid("a run of block types passes the end of its chunk");
                }
                let filled = bundle.values.len() + run;
                bundle.values.resize(filled, last);
            }
        }
    }
    Ok(())
}

/// A chunk of motion offsets: a 4-bit magnitude and, when it is not zero, a sign bit.
fn read_offsets(bundle: &mut Bundle, bits: &mut Bits, tree: &Tree) -> Result<()> {
    let Some(len) = bundle.begin_chunk(bits)? else { return Ok(()) };
    let signed = |v: i32, bits: &mut Bits| -> Result<i32> { Ok(if v != 0 && bits.bit()? { -v } else { v }) };
    if bits.bit()? {
        let v = bits.get(4)? as i32;
        let v = signed(v, bits)?;
        bundle.values.resize(len, v);
    } else {
        for _ in 0..len {
            let v = i32::from(tree.symbol(bits)?);
            let v = signed(v, bits)?;
            bundle.values.push(v);
        }
    }
    Ok(())
}

/// The range a DC value may take: 16 bits, signed. A real stream stays far inside it.
const DC_RANGE: std::ops::RangeInclusive<i32> = -32768..=32767;

/// A chunk of DC values: a start value, then groups of 8 deltas that share a bit count.
fn read_dc(bundle: &mut Bundle, bits: &mut Bits, inter: bool) -> Result<()> {
    let Some(len) = bundle.begin_chunk(bits)? else { return Ok(()) };
    let mut value = if inter {
        let v = bits.get(10)? as i32;
        if v != 0 && bits.bit()? { -v } else { v }
    } else {
        bits.get(11)? as i32
    };
    bundle.values.push(value);
    while bundle.values.len() < len {
        let group = (len - bundle.values.len()).min(8);
        let delta_bits = bits.get(4)?;
        for _ in 0..group {
            if delta_bits != 0 {
                let d = bits.get(delta_bits)? as i32;
                value += if d != 0 && bits.bit()? { -d } else { d };
                if !DC_RANGE.contains(&value) {
                    return invalid("a DC value is out of range");
                }
            }
            bundle.values.push(value);
        }
    }
    Ok(())
}

/// An entry of the list the coefficient and residue coders walk.
#[derive(Clone, Copy)]
enum Entry {
    /// A 2x2 group at (row, column) whose 4x4 child region has not been offered yet.
    Parent(u8, u8),
    /// A 4x4 region at (row, column), to be split into four 2x2 groups.
    Region(u8, u8),
    /// A 2x2 group at (row, column) with no child.
    Group(u8, u8),
    /// One position.
    Single(u8, u8),
}

/// The 4x4 region below a parent group.
fn child_of(r: u8, c: u8) -> (u8, u8) {
    (if r == 0 { 0 } else { 4 }, if c == 0 { 0 } else { 4 })
}

/// Which of the two list coders runs.
#[derive(Clone, Copy, PartialEq)]
enum ListCoder {
    /// DCT coefficients: a value's low bits follow it at once, and position (0, 0) is the DC.
    Coefficients,
    /// Pixel residue: values are refined one plane at a time, and a budget ends the block.
    Residue,
}

/// Reads the values of one block with the list coder: `top_plane + 1` planes for residue,
/// `planes` planes for coefficients. `out` is indexed by row * 8 + column.
fn read_list(bits: &mut Bits, coder: ListCoder, planes: u32, mut budget: u32, out: &mut [i32; 64]) -> Result<()> {
    if planes == 0 {
        return Ok(());
    }
    let mut list: Vec<Entry> = Vec::with_capacity(80);
    list.extend([Entry::Parent(0, 2), Entry::Parent(2, 0), Entry::Parent(2, 2)]);
    match coder {
        ListCoder::Coefficients => list.extend([Entry::Single(0, 1), Entry::Single(1, 0), Entry::Single(1, 1)]),
        ListCoder::Residue => list.push(Entry::Group(0, 0)),
    }
    // positions in the order they became non-zero, for the residue refinement passes
    let mut order: Vec<u8> = Vec::with_capacity(if coder == ListCoder::Residue { 64 } else { 0 });
    let at = |r: u8, c: u8| usize::from(r) * 8 + usize::from(c);
    for plane in (0..planes).rev() {
        if coder == ListCoder::Residue {
            for &p in &order {
                if bits.bit()? {
                    let v = &mut out[usize::from(p) & 63];
                    *v += if *v < 0 { -(1 << plane) } else { 1 << plane };
                    budget -= 1;
                    if budget == 0 {
                        return Ok(());
                    }
                }
            }
        }
        // a new non-zero value at (r, c); `Ok(true)` when the residue budget is used up
        let mut value = |bits: &mut Bits, r: u8, c: u8, order: &mut Vec<u8>, budget: &mut u32| -> Result<bool> {
            let mut magnitude = 1i32 << plane;
            if coder == ListCoder::Coefficients {
                magnitude |= bits.get(plane)? as i32;
            }
            let p = at(r, c) & 63;
            out[p] = if bits.bit()? { -magnitude } else { magnitude };
            if coder == ListCoder::Residue {
                order.push(p as u8);
                *budget -= 1;
                return Ok(*budget == 0);
            }
            Ok(false)
        };
        let mut i = 0;
        while let Some(&entry) = list.get(i) {
            if !bits.bit()? {
                i += 1;
                continue;
            }
            match entry {
                Entry::Parent(r, c) | Entry::Group(r, c) => {
                    match entry {
                        Entry::Parent(..) => {
                            let (cr, cc) = child_of(r, c);
                            list[i] = Entry::Region(cr, cc);
                        }
                        _ => {
                            list.remove(i);
                        }
                    }
                    for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                        if bits.bit()? {
                            // stays zero for now: looked at again from the next plane on
                            list.insert(0, Entry::Single(r + dr, c + dc));
                            i += 1;
                        } else if value(bits, r + dr, c + dc, &mut order, &mut budget)? {
                            return Ok(());
                        }
                    }
                }
                Entry::Region(r, c) => {
                    list[i] = Entry::Group(r, c);
                    list.extend([Entry::Group(r, c + 2), Entry::Group(r + 2, c), Entry::Group(r + 2, c + 2)]);
                }
                Entry::Single(r, c) => {
                    list.remove(i);
                    if value(bits, r, c, &mut order, &mut budget)? {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

/// The inverse transform of a dequantised block, in place: columns, then rows, then the final
/// rounding. Every operation wraps at 32 bits, so no input can overflow; a real stream never
/// comes near that.
fn inverse_dct(block: &mut [i32; 64]) {
    const A1: i32 = 2896;
    const A2: i32 = 3784;
    const A3: i32 = 2217;
    const A4: i32 = 5352;
    fn mul(x: i32, a: i32) -> i32 {
        x.wrapping_mul(a) >> 11
    }
    fn pass(i: [i32; 8]) -> [i32; 8] {
        let (a, s) = (i32::wrapping_add, i32::wrapping_sub);
        let t10 = a(i[0], i[4]);
        let t11 = s(i[0], i[4]);
        let t13 = a(i[2], i[6]);
        let t12 = s(mul(s(i[2], i[6]), A1), t13);
        let (t0, t3, t1, t2) = (a(t10, t13), s(t10, t13), a(t11, t12), s(t11, t12));
        let z13 = a(i[5], i[3]);
        let z10 = s(i[5], i[3]);
        let z11 = a(i[1], i[7]);
        let z12 = s(i[1], i[7]);
        let t7 = a(z11, z13);
        let u11 = mul(s(z11, z13), A1);
        let z5 = mul(a(z10, z12), A2);
        let u10 = s(mul(z12, A3), z5);
        let u12 = a(mul(z10, -A4), z5);
        let t6 = s(u12, t7);
        let t5 = s(u11, t6);
        let t4 = a(u10, t5);
        [a(t0, t7), a(t1, t6), a(t2, t5), s(t3, t4), a(t3, t4), s(t2, t5), s(t1, t6), s(t0, t7)]
    }
    for c in 0..8 {
        let col = pass(std::array::from_fn(|r| block[r * 8 + c]));
        for (r, v) in col.into_iter().enumerate() {
            block[r * 8 + c] = v;
        }
    }
    for r in 0..8 {
        let row = pass(std::array::from_fn(|c| block[r * 8 + c]));
        for (c, v) in row.into_iter().enumerate() {
            block[r * 8 + c] = v.wrapping_add(0x7f) >> 8;
        }
    }
}

/// Reads the inline data of a DCT block and returns its 64 values after the inverse transform.
fn read_dct(bits: &mut Bits, dc: i32, tables: &[[u32; 64]; 16]) -> Result<[i32; 64]> {
    let planes = bits.get(4)?;
    let mut block = [0i32; 64];
    read_list(bits, ListCoder::Coefficients, planes, 0, &mut block)?;
    block[0] = dc;
    let table = &tables[bits.get(4)? as usize & 15];
    for (v, &m) in block.iter_mut().zip(table) {
        // 32-bit wrapping, like the transform
        *v = v.wrapping_mul(m as i32) >> 11;
    }
    inverse_dct(&mut block);
    Ok(block)
}

/// One plane being decoded: `cur` and `prev` hold `blocks_high * 8` rows of `stride` samples.
pub(super) struct Plane<'a> {
    pub(super) cur: &'a mut [u8],
    /// The previous picture, or `None` before the first: references then read the picture being
    /// decoded.
    pub(super) prev: Option<&'a [u8]>,
    pub(super) blocks_wide: usize,
    pub(super) blocks_high: usize,
    /// The most 16x16 blocks a row can hold, which sets the width of the sub-type length field.
    pub(super) sub_types: usize,
}

impl Plane<'_> {
    fn stride(&self) -> usize {
        self.blocks_wide * 8
    }

    /// Writes an 8x8 block of samples at block (bx, by).
    fn put(&mut self, bx: usize, by: usize, px: &[u8; 64]) -> Result<()> {
        let stride = self.stride();
        for (y, row) in px.as_chunks::<8>().0.iter().enumerate() {
            let at = (by * 8 + y) * stride + bx * 8;
            self.cur.get_mut(at..at + 8).ok_or_else(|| Error::Invalid("block outside the plane".into()))?.copy_from_slice(row);
        }
        Ok(())
    }

    /// Writes an 8x8 block scaled to 16x16 at block (bx, by), clipped to the plane: in the last
    /// column of an odd number of columns, or the last row of an odd number of rows, only the
    /// part inside the plane is drawn.
    fn put_scaled(&mut self, bx: usize, by: usize, px: &[u8; 64]) -> Result<()> {
        let stride = self.stride();
        let width = 16.min(stride.saturating_sub(bx * 8));
        let rows = 16.min((self.blocks_high * 8).saturating_sub(by * 8));
        for y in 0..rows {
            let at = (by * 8 + y) * stride + bx * 8;
            let row = self.cur.get_mut(at..at + width).ok_or_else(|| Error::Invalid("block outside the plane".into()))?;
            for (x, v) in row.iter_mut().enumerate() {
                *v = px[(y / 2) * 8 + x / 2];
            }
        }
        Ok(())
    }

    /// The 8x8 block of the previous picture at block (bx, by) moved by (dx, dy). With no
    /// previous picture it is read from the picture being decoded, as far as that is drawn.
    fn reference(&self, bx: usize, by: usize, dx: i32, dy: i32) -> Result<[u8; 64]> {
        let source = self.prev.unwrap_or(&*self.cur);
        let stride = self.stride();
        let x = (bx * 8) as i64 + i64::from(dx);
        let y = (by * 8) as i64 + i64::from(dy);
        if x < 0 || y < 0 || x + 8 > stride as i64 || y + 8 > (self.blocks_high * 8) as i64 {
            return invalid("a motion vector points outside the plane");
        }
        let (x, y) = (x as usize, y as usize);
        let mut px = [0u8; 64];
        for (r, row) in px.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let at = (y + r) * stride + x;
            row.copy_from_slice(source.get(at..at + 8).ok_or_else(|| Error::Invalid("reference outside the plane".into()))?);
        }
        Ok(px)
    }
}

/// Decodes a block of one of the types that do not use the previous picture (run fill, intra
/// DCT, fill, pattern fill, raw), which are also the sub-types of a 16x16 block.
fn intra_block(kind: i32, bits: &mut Bits, b: &mut Bundles) -> Result<[u8; 64]> {
    let mut px = [0u8; 64];
    match kind {
        3 => {
            let scan = &RUN_SCANS[bits.get(4)? as usize & 15];
            let mut i = 0usize;
            while i < 63 {
                let run = b.run.take()? as usize + 1;
                if i + run > 64 {
                    return invalid("a run passes the end of its block");
                }
                if bits.bit()? {
                    let c = b.colour.take()? as u8;
                    for &p in &scan[i..i + run] {
                        px[usize::from(p) & 63] = c;
                    }
                } else {
                    for &p in &scan[i..i + run] {
                        px[usize::from(p) & 63] = b.colour.take()? as u8;
                    }
                }
                i += run;
            }
            if i == 63 {
                px[usize::from(scan[63]) & 63] = b.colour.take()? as u8;
            }
        }
        5 => {
            let dc = b.intra_dc.take()?;
            let block = read_dct(bits, dc, &INTRA_QUANT)?;
            for (p, v) in px.iter_mut().zip(block) {
                *p = v as u8;
            }
        }
        6 => px = [b.colour.take()? as u8; 64],
        8 => {
            let colours = [b.colour.take()? as u8, b.colour.take()? as u8];
            for row in px.as_chunks_mut::<8>().0 {
                let mask = b.pattern.take()?;
                for (x, p) in row.iter_mut().enumerate() {
                    *p = colours[(mask >> x & 1) as usize];
                }
            }
        }
        9 => {
            for p in px.iter_mut() {
                *p = b.colour.take()? as u8;
            }
        }
        _ => return invalid("unknown block type"),
    }
    Ok(px)
}

/// Decodes one plane from `bits`, which stands at the plane's tree headers.
pub(super) fn decode_plane(bits: &mut Bits, plane: &mut Plane, signed_colours: bool) -> Result<()> {
    let trees = Trees::read(bits)?;
    let mut b = Bundles::new(plane.blocks_wide, plane.sub_types, signed_colours);
    for by in 0..plane.blocks_high {
        b.refill(bits, &trees)?;
        let mut bx = 0;
        while bx < plane.blocks_wide {
            let kind = b.block_type.take()?;
            match kind {
                0 => {
                    let px = plane.reference(bx, by, 0, 0)?;
                    plane.put(bx, by, &px)?;
                }
                1 => {
                    // a 16x16 block is listed on both of its rows and decoded on the first, which
                    // is an even row; it starts at an even column. Anywhere else the entry only
                    // steps over two blocks.
                    if by % 2 == 0 && bx % 2 == 0 {
                        let sub = b.sub_type.take()?;
                        if !matches!(sub, 3 | 5 | 6 | 8 | 9) {
                            return invalid("unknown 16x16 block type");
                        }
                        let px = intra_block(sub, bits, &mut b)?;
                        plane.put_scaled(bx, by, &px)?;
                    }
                    bx += 1;
                }
                2 | 4 | 7 => {
                    let dx = b.x_off.take()?;
                    let dy = b.y_off.take()?;
                    let mut px = plane.reference(bx, by, dx, dy)?;
                    let mut add = [0i32; 64];
                    if kind == 7 {
                        let dc = b.inter_dc.take()?;
                        add = read_dct(bits, dc, &INTER_QUANT)?;
                    } else if kind == 4 {
                        let budget = bits.get(7)? + 1;
                        let top_plane = bits.get(3)?;
                        read_list(bits, ListCoder::Residue, top_plane + 1, budget, &mut add)?;
                    }
                    for (p, v) in px.iter_mut().zip(add) {
                        *p = (i32::from(*p).wrapping_add(v)) as u8;
                    }
                    plane.put(bx, by, &px)?;
                }
                _ => {
                    let px = intra_block(kind, bits, &mut b)?;
                    plane.put(bx, by, &px)?;
                }
            }
            bx += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packs `(value, bits)` fields least significant bit first.
    fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
        let bits: Vec<bool> = fields.iter().flat_map(|&(v, n)| (0..n).map(move |k| v >> k & 1 != 0)).collect();
        bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (k, &v)| b | u8::from(v) << k)).collect()
    }

    /// Reads an inter DC chunk of two values: -1023, then a 15-bit delta subtracted from it.
    fn inter_dc_after_delta(delta: u32) -> Result<Vec<i32>> {
        let data = pack(&[(2, 10), (1023, 10), (1, 1), (15, 4), (delta, 15), (1, 1)]);
        let mut bundle = Bundle::new(1);
        read_dc(&mut bundle, &mut Bits::new(&data), true)?;
        Ok(bundle.values)
    }

    /// Packs a string of bits in read order.
    fn bit_string(bits: &str) -> Vec<u8> {
        pack(&bits.bytes().map(|b| (u32::from(b == b'1'), 1)).collect::<Vec<_>>())
    }

    /// The symbol order a tree holds: each rank's code is fed back through the tree.
    fn order_of(tree: &Tree, number: usize) -> Vec<u8> {
        TREES[number].iter().map(|&(code, len)| tree.symbol(&mut Bits::new(&pack(&[(u32::from(code), u32::from(len))]))).unwrap()).collect()
    }

    #[test]
    fn every_predefined_tree_is_a_complete_prefix_code() {
        for (number, codes) in TREES.iter().enumerate() {
            let identity: [u8; 16] = std::array::from_fn(|i| i as u8);
            let tree = Tree::new(number, &identity);
            assert!(tree.lookup.iter().all(|&(_, len)| (1..=7).contains(&len)), "tree {number} leaves a 7-bit pattern uncoded");
            assert_eq!(order_of(&tree, number), identity, "tree {number}");
            let kraft: u32 = codes.iter().map(|&(_, len)| 128 >> len).sum();
            assert_eq!(kraft, 128, "tree {number}");
        }
    }

    #[test]
    fn tree_headers_in_both_forms_as_real_frames_carry_them() {
        // list form: tree 12, six symbols listed, the rest ascending
        let data = bit_string("00111101100111000001101100110110");
        let tree = Tree::read(&mut Bits::new(&data)).unwrap();
        assert_eq!(order_of(&tree, 12), [9, 3, 8, 13, 12, 6, 0, 1, 2, 4, 5, 7, 10, 11, 14, 15]);
        // merge form: tree 10, three passes
        let data = bit_string("01010010000100100000011000001000");
        let mut bits = Bits::new(&data);
        let tree = Tree::read(&mut bits).unwrap();
        assert_eq!(order_of(&tree, 10), [0, 1, 2, 3, 4, 5, 6, 7, 9, 15, 8, 10, 11, 14, 12, 13]);
        assert!(bits.get(1).is_err(), "the header is the whole 32 bits");
        // tree 0 has no order to read
        let mut bits = Bits::new(&[0xf0]);
        Tree::read(&mut bits).unwrap();
        assert_eq!(bits.get(4).unwrap(), 15);
        // a symbol listed twice is refused: tree 1, two symbols, both 5
        assert!(Tree::read(&mut Bits::new(&pack(&[(1, 4), (1, 1), (1, 3), (5, 4), (5, 4)]))).is_err());
    }

    /// Reads one chunk of block types coded with the raw tree.
    fn types(fields: &[(u32, u32)]) -> Result<Vec<i32>> {
        let mut bundle = Bundle::new(1);
        let identity: [u8; 16] = std::array::from_fn(|i| i as u8);
        read_types(&mut bundle, &mut Bits::new(&pack(fields)), &Tree::new(0, &identity))?;
        Ok(bundle.values)
    }

    #[test]
    fn type_chunks_repeat_with_run_symbols_and_start_from_zero() {
        // 13 values: a run of 8 first (repeating 0, as at every chunk start), then 5, then a run of 4
        let coded = [(13, 10), (0, 1), (13, 4), (5, 4), (12, 4)];
        assert_eq!(types(&coded).unwrap(), [0, 0, 0, 0, 0, 0, 0, 0, 5, 5, 5, 5, 5]);
        // the same symbols in a chunk of 12 pass its end
        assert!(types(&[(12, 10), (0, 1), (13, 4), (5, 4), (12, 4)]).is_err());
        // all values equal
        assert_eq!(types(&[(5, 10), (1, 1), (7, 4)]).unwrap(), [7; 5]);
        // a zero length finishes the bundle and reads nothing more
        let mut bundle = Bundle::new(1);
        let identity: [u8; 16] = std::array::from_fn(|i| i as u8);
        let data = pack(&[(0, 10), (0x3ff, 10)]);
        let mut bits = Bits::new(&data);
        read_types(&mut bundle, &mut bits, &Tree::new(0, &identity)).unwrap();
        read_types(&mut bundle, &mut bits, &Tree::new(0, &identity)).unwrap();
        assert!(bundle.finished && bundle.values.is_empty());
        assert_eq!(bits.get(10).unwrap(), 0x3ff);
    }

    #[test]
    fn offsets_and_runs_have_an_all_equal_form() {
        let identity: [u8; 16] = std::array::from_fn(|i| i as u8);
        let raw = Tree::new(0, &identity);
        let mut bundle = Bundle::new(1);
        read_offsets(&mut bundle, &mut Bits::new(&pack(&[(3, 10), (1, 1), (9, 4), (1, 1)])), &raw).unwrap();
        assert_eq!(bundle.values, [-9, -9, -9]);
        let mut bundle = Bundle::new(1);
        read_offsets(&mut bundle, &mut Bits::new(&pack(&[(3, 10), (0, 1), (2, 4), (0, 1), (0, 4), (15, 4), (1, 1)])), &raw).unwrap();
        assert_eq!(bundle.values, [2, 0, -15]);
    }

    #[test]
    fn residue_refines_earlier_samples_before_each_plane_and_stops_at_its_budget() {
        // two planes. Plane 1: the first group is flagged; (0,2) becomes +2, (0,3) stays zero,
        // (1,2) becomes -2, (1,3) stays zero; nothing else is flagged. Plane 0: refinement bits
        // for (0,2) and (1,2) in that order (1, 0), then (1,3) is flagged and becomes +1.
        let stream = "1 00 1 01 1 0000  10  10 0 0000";
        let data = bit_string(&stream.replace(' ', ""));
        let mut out = [0i32; 64];
        read_list(&mut Bits::new(&data), ListCoder::Residue, 2, 100, &mut out).unwrap();
        let mut want = [0i32; 64];
        (want[2], want[10], want[11]) = (3, -2, 1);
        assert_eq!(out, want);
        // a budget of three is spent by the two new samples and the refinement bit
        let mut out = [0i32; 64];
        let mut bits = Bits::new(&data);
        read_list(&mut bits, ListCoder::Residue, 2, 3, &mut out).unwrap();
        (want[2], want[10], want[11]) = (3, -2, 0);
        assert_eq!(out, want);
        // and the block ends there: the next bit read is the second refinement bit
        assert_eq!(bits.get(2).unwrap(), 0b10);
    }

    #[test]
    fn dc_values_span_sixteen_signed_bits() {
        assert_eq!(inter_dc_after_delta(31745).unwrap(), [-1023, -32768]);
        assert!(inter_dc_after_delta(31746).is_err());
        // and the upper end: 2047, then +30720 reaches 32767; one more is out of range
        let up = |delta: u32| {
            let data = pack(&[(2, 10), (2047, 11), (15, 4), (delta, 15), (0, 1)]);
            let mut bundle = Bundle::new(1);
            read_dc(&mut bundle, &mut Bits::new(&data), false).map(|()| bundle.values)
        };
        assert_eq!(up(30720).unwrap(), [2047, 32767]);
        assert!(up(30721).is_err());
    }
}
