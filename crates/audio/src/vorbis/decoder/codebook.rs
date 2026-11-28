//! Codebooks (Vorbis I §3): a Huffman code over entry numbers and an optional table of vectors.

use super::Error;
use super::bits::{BitReader, Eop, ilog};

const NONE: u32 = u32::MAX;
const LEAF: u32 = 1 << 31;
/// Entries a codebook may declare.
const MAX_ENTRIES: u32 = 1 << 20;
/// Nodes of one codeword tree.
const MAX_NODES: usize = 1 << 22;

#[derive(Clone, Copy)]
struct Node {
    child: [u32; 2],
    full: bool,
}

pub(super) struct Codebook {
    pub(super) dimensions: usize,
    pub(super) entries: usize,
    tree: Vec<Node>,
    /// The only used entry of a single-entry book.
    single: Option<u32>,
    /// `entries * dimensions` values when the book has a lookup table.
    vectors: Vec<f32>,
}

/// `float32_unpack` (§9.2.2).
fn float32_unpack(x: u32) -> f64 {
    let mantissa = f64::from(x & 0x001F_FFFF);
    let exponent = ((x & 0x7FE0_0000) >> 21) as i32;
    let m = if x & 0x8000_0000 != 0 { -mantissa } else { mantissa };
    m * 2f64.powi(exponent - 788)
}

/// `lookup1_values` (§9.2.3): the greatest `v` with `v^dimensions <= entries`.
fn lookup1_values(entries: u32, dimensions: u32) -> u32 {
    let mut v = 1u32;
    loop {
        let next = u128::from(v) + 1;
        let mut p = 1u128;
        for _ in 0..dimensions {
            p *= next;
            if p > u128::from(entries) {
                return v;
            }
        }
        v += 1;
    }
}

fn insert(tree: &mut Vec<Node>, at: usize, depth: u32, len: u32, entry: u32) -> bool {
    for b in 0..2 {
        let child = tree[at].child[b];
        if depth + 1 == len {
            if child == NONE {
                tree[at].child[b] = LEAF | entry;
                refresh(tree, at);
                return true;
            }
            continue;
        }
        if child == NONE {
            if tree.len() >= MAX_NODES {
                return false;
            }
            tree.push(Node { child: [NONE; 2], full: false });
            let new = tree.len() - 1;
            tree[at].child[b] = new as u32;
            if insert(tree, new, depth + 1, len, entry) {
                refresh(tree, at);
                return true;
            }
            return false;
        }
        if child & LEAF != 0 || tree[child as usize].full {
            continue;
        }
        if insert(tree, child as usize, depth + 1, len, entry) {
            refresh(tree, at);
            return true;
        }
    }
    false
}

fn refresh(tree: &mut [Node], at: usize) {
    let done = |c: u32, t: &[Node]| c != NONE && (c & LEAF != 0 || t[c as usize].full);
    let full = done(tree[at].child[0], tree) && done(tree[at].child[1], tree);
    tree[at].full = full;
}

impl Codebook {
    /// Read one codebook from a setup header. `budget` is the bytes the setup's codebooks may
    /// still take in all.
    pub(super) fn read(r: &mut BitReader<'_>, budget: &mut usize) -> Result<Self, Error> {
        let bad = |m: &'static str| Error::Setup(m);
        let eop = |_: Eop| Error::Setup("setup header ends inside a codebook");
        if r.read(24).map_err(eop)? != 0x0056_4342 {
            return Err(bad("codebook sync pattern missing"));
        }
        let dimensions = r.read(16).map_err(eop)?;
        let entries = r.read(24).map_err(eop)?;
        if dimensions == 0 || entries == 0 || entries > MAX_ENTRIES {
            return Err(bad("codebook with no dimensions, no entries or too many"));
        }
        let mut lengths: Vec<u8> = Vec::with_capacity(entries as usize);
        if r.bit().map_err(eop)? {
            // Ordered: the number of codewords of each length.
            let mut current = 0u32;
            let mut len = r.read(5).map_err(eop)? + 1;
            while current < entries {
                let number = r.read(ilog(i64::from(entries - current))).map_err(eop)?;
                if number > entries - current || len > 32 {
                    return Err(bad("ordered codebook lengths overrun the entries"));
                }
                lengths.extend(std::iter::repeat_n(len as u8, number as usize));
                current += number;
                len += 1;
            }
        } else {
            let sparse = r.bit().map_err(eop)?;
            for _ in 0..entries {
                if sparse && !r.bit().map_err(eop)? {
                    lengths.push(0);
                } else {
                    lengths.push((r.read(5).map_err(eop)? + 1) as u8);
                }
            }
        }

        // The codeword tree.
        let used = lengths.iter().filter(|&&l| l != 0).count();
        let mut tree = vec![Node { child: [NONE; 2], full: false }];
        let mut single = None;
        if used == 1 {
            let entry = lengths.iter().position(|&l| l != 0).unwrap_or(0);
            if lengths[entry] != 1 {
                return Err(bad("a codebook with one used entry must give it length 1"));
            }
            single = Some(entry as u32);
        } else if used > 1 {
            for (entry, &len) in lengths.iter().enumerate() {
                if len != 0 && !insert(&mut tree, 0, 0, u32::from(len), entry as u32) {
                    return Err(bad("overspecified codebook"));
                }
            }
            if !tree[0].full {
                return Err(bad("underspecified codebook"));
            }
        }
        let tree_bytes = tree.len() * std::mem::size_of::<Node>();
        if tree_bytes > *budget {
            return Err(Error::Limit("the codebooks are too large"));
        }
        *budget -= tree_bytes;

        // The lookup table.
        let lookup_type = r.read(4).map_err(eop)?;
        let mut vectors = Vec::new();
        if lookup_type > 2 {
            return Err(bad("reserved codebook lookup type"));
        }
        if lookup_type != 0 {
            let minimum = float32_unpack(r.read(32).map_err(eop)?);
            let delta = float32_unpack(r.read(32).map_err(eop)?);
            let value_bits = r.read(4).map_err(eop)? + 1;
            let sequence = r.bit().map_err(eop)?;
            // Check before multiplying or reading multiplicands: the product can overflow
            // usize on 32-bit targets, and a type-2 table otherwise allocates before admission.
            let total = (entries as usize)
                .checked_mul(dimensions as usize)
                .filter(|&n| n <= *budget / std::mem::size_of::<f32>())
                .ok_or(Error::Limit("the codebooks' vector tables are too large"))?;
            let values = if lookup_type == 1 { lookup1_values(entries, dimensions) as usize } else { total };
            if !minimum.is_finite() || !delta.is_finite() || values > r.remaining() / value_bits as usize {
                return Err(bad("codebook lookup table is damaged or truncated"));
            }
            let multiplicands: Vec<u32> = (0..values).map(|_| r.read(value_bits).unwrap_or(0)).collect();
            *budget -= total * std::mem::size_of::<f32>();
            vectors = vec![0f32; total];
            for e in 0..entries as usize {
                let out = &mut vectors[e * dimensions as usize..(e + 1) * dimensions as usize];
                let mut last = 0f64;
                let mut divisor = 1usize;
                for (i, slot) in out.iter_mut().enumerate() {
                    let m = if lookup_type == 1 {
                        let at = (e / divisor) % values;
                        divisor = divisor.saturating_mul(values);
                        multiplicands[at]
                    } else {
                        multiplicands[e * dimensions as usize + i]
                    };
                    let v = f64::from(m) * delta + minimum + last;
                    if sequence {
                        last = v;
                    }
                    *slot = v as f32;
                }
            }
        }
        Ok(Self { dimensions: dimensions as usize, entries: entries as usize, tree, single, vectors })
    }

    /// Whether the book has a lookup table.
    pub(super) fn has_vectors(&self) -> bool {
        !self.vectors.is_empty()
    }

    /// Read a codeword; the entry number (scalar context).
    pub(super) fn scalar(&self, r: &mut BitReader<'_>) -> Result<u32, Eop> {
        if let Some(e) = self.single {
            r.read(1)?;
            return Ok(e);
        }
        let mut at = 0usize;
        loop {
            let c = self.tree[at].child[usize::from(r.bit()?)];
            if c == NONE {
                return Err(Eop);
            }
            if c & LEAF != 0 {
                return Ok(c & !LEAF);
            }
            at = c as usize;
        }
    }

    /// Read a codeword and return its vector (VQ context); `None` for a book without a table.
    pub(super) fn vector(&self, r: &mut BitReader<'_>) -> Result<Option<&[f32]>, Eop> {
        let e = self.scalar(r)? as usize;
        Ok(self.vectors.get(e * self.dimensions..(e + 1) * self.dimensions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack fields least significant bit first.
    struct W(Vec<u8>, usize);
    impl W {
        fn put(&mut self, v: u64, n: u32) {
            for i in 0..n {
                if self.1.is_multiple_of(8) {
                    self.0.push(0);
                }
                let last = self.0.len() - 1;
                self.0[last] |= (((v >> i) & 1) as u8) << (self.1 % 8);
                self.1 += 1;
            }
        }
    }

    fn book_bytes(lengths: &[u32], lookup: Option<(u32, u32, &[u32])>) -> Vec<u8> {
        let mut w = W(Vec::new(), 0);
        w.put(0x56_4342, 24);
        w.put(1, 16);
        w.put(lengths.len() as u64, 24);
        w.put(0, 1); // not ordered
        w.put(0, 1); // not sparse
        for &l in lengths {
            w.put(u64::from(l - 1), 5);
        }
        match lookup {
            None => w.put(0, 4),
            Some((ty, bits, mult)) => {
                w.put(u64::from(ty), 4);
                w.put(0, 32); // minimum: 0.0
                w.put(0, 32);
                w.put(u64::from(bits - 1), 4);
                w.put(0, 1);
                for &m in mult {
                    w.put(u64::from(m), bits);
                }
            }
        }
        w.0
    }

    #[test]
    fn the_specification_example_assigns_its_codewords() {
        // §3.2.1: lengths 2 4 4 4 4 2 3 3 -> 00, 0100, 0101, 0110, 0111, 10, 110, 111.
        let bytes = book_bytes(&[2, 4, 4, 4, 4, 2, 3, 3], None);
        let mut budget = 1 << 26;
        let book = Codebook::read(&mut BitReader::new(&bytes), &mut budget).expect("book");
        let code = |bits: &[u8]| {
            let mut w = W(Vec::new(), 0);
            for &b in bits {
                w.put(u64::from(b), 1);
            }
            let mut r = BitReader::new(&w.0);
            book.scalar(&mut r).expect("entry")
        };
        assert_eq!(code(&[0, 0]), 0);
        assert_eq!(code(&[0, 1, 0, 0]), 1);
        assert_eq!(code(&[0, 1, 1, 1]), 4);
        assert_eq!(code(&[1, 0]), 5);
        assert_eq!(code(&[1, 1, 0]), 6);
        assert_eq!(code(&[1, 1, 1]), 7);
    }

    #[test]
    fn over_and_underspecified_books_are_errors() {
        let mut budget = 1 << 26;
        let over = book_bytes(&[1, 1, 1], None);
        assert!(Codebook::read(&mut BitReader::new(&over), &mut budget).is_err());
        let under = book_bytes(&[2, 2, 2], None);
        assert!(Codebook::read(&mut BitReader::new(&under), &mut budget).is_err());
    }

    #[test]
    fn a_single_entry_book_sinks_one_bit_of_either_value() {
        let mut budget = 1 << 26;
        let bytes = book_bytes(&[1], None);
        let book = Codebook::read(&mut BitReader::new(&bytes), &mut budget).expect("book");
        for byte in [0u8, 1] {
            let data = [byte];
            let mut r = BitReader::new(&data);
            assert_eq!(book.scalar(&mut r), Ok(0));
            assert_eq!(r.remaining(), 7);
        }
        let bad = book_bytes(&[2], None);
        assert!(Codebook::read(&mut BitReader::new(&bad), &mut budget).is_err());
    }

    #[test]
    fn lookup_type_one_builds_a_lattice() {
        // 4 entries, 2 dimensions, 2 values per dimension: minimum 0 and delta 1 in the packed
        // float format are written below by hand (mantissa 1, exponent 788 -> 1.0).
        let mut w = W(Vec::new(), 0);
        w.put(0x56_4342, 24);
        w.put(2, 16);
        w.put(4, 24);
        w.put(0, 1);
        w.put(0, 1);
        for _ in 0..4 {
            w.put(1, 5); // length 2 each
        }
        w.put(1, 4); // lookup type 1
        w.put(0, 32); // minimum 0
        w.put((788u64 << 21) | 1, 32); // delta 1.0
        w.put(0, 4); // 1 bit per value
        w.put(0, 1);
        w.put(0b10, 2); // multiplicands 0, 1
        let mut budget = 1 << 26;
        let book = Codebook::read(&mut BitReader::new(&w.0), &mut budget).expect("book");
        assert_eq!(lookup1_values(4, 2), 2);
        // Entry e is (m[e % 2], m[(e / 2) % 2]).
        assert_eq!(book.vectors, vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn vector_budget_is_checked_before_reading_lookup_values() {
        // A valid single-entry tree, but two values exceed the remaining budget.
        // No multiplicands follow: admission must reject it before reading them.
        for lookup in [1, 2] {
            let mut w = W(Vec::new(), 0);
            w.put(0x56_4342, 24);
            w.put(2, 16);
            w.put(1, 24);
            w.put(0, 1);
            w.put(0, 1);
            w.put(0, 5);
            w.put(lookup, 4);
            w.put(0, 32);
            w.put(0, 32);
            w.put(15, 4);
            w.put(0, 1);
            let mut budget = std::mem::size_of::<Node>() + 4;
            assert!(matches!(Codebook::read(&mut BitReader::new(&w.0), &mut budget), Err(Error::Limit(_))));
        }
    }

    #[test]
    fn vector_size_that_overflows_32_bits_is_refused() {
        for lookup in [1, 2] {
            let mut w = W(Vec::new(), 0);
            w.put(0x56_4342, 24);
            w.put(65535, 16);
            w.put(65538, 24); // product is above u32::MAX
            w.put(0, 1); // unordered
            w.put(1, 1); // sparse: only one entry is used
            w.put(1, 1);
            w.put(0, 5); // length 1
            for _ in 1..65538 {
                w.put(0, 1);
            }
            w.put(lookup, 4);
            w.put(0, 32);
            w.put(0, 32);
            w.put(0, 4);
            w.put(0, 1);
            w.put(0, 1); // lookup type 1's single multiplicand
            let mut budget = 256 << 20;
            assert!(matches!(Codebook::read(&mut BitReader::new(&w.0), &mut budget), Err(Error::Limit(_))));
        }
    }

    #[test]
    fn helper_functions() {
        assert_eq!(float32_unpack((788u32 << 21) | 5), 5.0);
        assert_eq!(float32_unpack(0x8000_0000 | (789u32 << 21) | 3), -6.0);
        for (entries, dims, want) in [(8, 3, 2), (9, 2, 3), (1, 5, 1), (10, 1, 10), (1000, 3, 10), (4_000_000, 2, 2000)] {
            assert_eq!(lookup1_values(entries, dims), want, "{entries} entries, {dims} dims");
        }
    }
}
