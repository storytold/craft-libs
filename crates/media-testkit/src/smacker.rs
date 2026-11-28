//! Builds synthetic Smacker data for tests: an LSB-first bit writer, the two kinds of Huffman
//! tree with their codes, and a file writer. Real Smacker files come from commercial games and
//! cannot be committed, so the tests of the demuxer and the decoders build their own streams.
//!
//! Self-contained (it does not use craft-media), so the tests check the readers against an
//! independent writer.

/// LSB-first bit writer: the first bit written is bit 0 of the first byte.
#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    /// An empty writer.
    pub fn new() -> BitWriter {
        BitWriter::default()
    }

    /// Writes one bit.
    pub fn bit(&mut self, b: bool) {
        if self.bits.is_multiple_of(8) {
            self.bytes.push(0);
        }
        if let Some(last) = self.bytes.last_mut() {
            *last |= u8::from(b) << (self.bits % 8);
        }
        self.bits += 1;
    }

    /// Writes the low `n` bits of `v`, least significant first.
    pub fn bits(&mut self, v: u32, n: u32) {
        for i in 0..n {
            self.bit(v >> i & 1 == 1);
        }
    }

    /// Bits written so far.
    pub fn len(&self) -> usize {
        self.bits
    }

    /// Whether nothing was written.
    pub fn is_empty(&self) -> bool {
        self.bits == 0
    }

    /// The bytes, the last one padded with zero bits.
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// A balanced binary tree over distinct symbols, in pre-order, with the code of each symbol.
#[derive(Clone)]
pub struct Tree<T> {
    /// The nodes in pre-order: `None` for a branch, `Some` for a leaf.
    pub nodes: Vec<Option<T>>,
    /// Each symbol with its code (`false` for the left child).
    pub codes: Vec<(T, Vec<bool>)>,
}

impl<T: Copy + PartialEq> Tree<T> {
    /// The tree over `symbols` (at least one, all distinct). One symbol gets the empty code.
    pub fn new(symbols: &[T]) -> Tree<T> {
        fn build<T: Copy>(symbols: &[T], prefix: &mut Vec<bool>, tree: &mut Tree<T>) {
            if let [one] = symbols {
                tree.nodes.push(Some(*one));
                tree.codes.push((*one, prefix.clone()));
                return;
            }
            tree.nodes.push(None);
            let (left, right) = symbols.split_at(symbols.len() / 2);
            for (side, bit) in [(left, false), (right, true)] {
                prefix.push(bit);
                build(side, prefix, tree);
                prefix.pop();
            }
        }
        assert!(!symbols.is_empty(), "a tree needs a symbol");
        let mut tree = Tree { nodes: Vec::new(), codes: Vec::new() };
        build(symbols, &mut Vec::new(), &mut tree);
        tree
    }

    /// Writes the code of `symbol`. Panics for a symbol the tree does not have.
    #[allow(clippy::panic)]
    pub fn put(&self, w: &mut BitWriter, symbol: T) {
        let code = self.codes.iter().find(|(s, _)| *s == symbol);
        let (_, code) = code.unwrap_or_else(|| panic!("symbol is not in the tree"));
        for &b in code {
            w.bit(b);
        }
    }
}

impl Tree<u8> {
    /// Writes the tree as Smacker stores a byte tree: 1 for a branch, 0 and the byte for a leaf.
    pub fn write(&self, w: &mut BitWriter) {
        for n in &self.nodes {
            match n {
                None => w.bit(true),
                Some(v) => {
                    w.bit(false);
                    w.bits(u32::from(*v), 8);
                }
            }
        }
    }

    /// Writes the tree where it is optional: a set flag, the tree, a closing bit.
    pub fn write_optional(&self, w: &mut BitWriter) {
        w.bit(true);
        self.write(w);
        w.bit(false);
    }
}

/// One of the four trees of Smacker video: 16-bit leaves coded with a byte tree for each half.
#[derive(Clone)]
pub struct BigTree {
    low: Tree<u8>,
    high: Tree<u8>,
    tree: Tree<u16>,
    escapes: [u16; 3],
}

impl BigTree {
    /// The tree over `symbols` (at least one, all distinct). A symbol equal to `escapes[n]` is a
    /// leaf that decodes to the `n`th most recent value.
    pub fn new(symbols: &[u16], escapes: [u16; 3]) -> BigTree {
        let halves = |f: fn(u16) -> u8| {
            let mut v: Vec<u8> = symbols.iter().map(|&s| f(s)).collect();
            v.sort_unstable();
            v.dedup();
            Tree::new(&v)
        };
        BigTree { low: halves(|s| s as u8), high: halves(|s| (s >> 8) as u8), tree: Tree::new(symbols), escapes }
    }

    /// The table size the file header declares for this tree, in bytes.
    pub fn size(&self) -> u32 {
        self.tree.nodes.len() as u32 * 4
    }

    /// Writes the tree as it is stored in the tree data of a file.
    pub fn write(&self, w: &mut BitWriter) {
        w.bit(true);
        self.low.write_optional(w);
        self.high.write_optional(w);
        for e in self.escapes {
            w.bits(u32::from(e), 16);
        }
        for n in &self.tree.nodes {
            match n {
                None => w.bit(true),
                Some(v) => {
                    w.bit(false);
                    self.low.put(w, *v as u8);
                    self.high.put(w, (*v >> 8) as u8);
                }
            }
        }
        w.bit(false);
    }

    /// Writes the code of `symbol`.
    pub fn put(&self, w: &mut BitWriter, symbol: u16) {
        self.tree.put(w, symbol);
    }
}

/// What [`file()`] writes.
pub struct FileSpec<'a> {
    /// `SMK2` or `SMK4`.
    pub signature: [u8; 4],
    /// Picture width.
    pub width: u32,
    /// Picture height.
    pub height: u32,
    /// The frame rate field as stored.
    pub frame_rate: i32,
    /// Header flags; bit 0 makes the last of `frames` the ring frame.
    pub flags: u32,
    /// The rate field of each audio track (flags in the top byte).
    pub audio_rates: [u32; 7],
    /// The table sizes of the four video trees.
    pub tree_sizes: [u32; 4],
    /// The Huffman tree data.
    pub trees: &'a [u8],
    /// Each frame: its flags byte (bit 0 palette, bits 1 to 7 audio tracks) and its bytes.
    pub frames: &'a [(u8, Vec<u8>)],
}

/// A Smacker file. Frame bytes are padded to a multiple of four.
pub fn file(spec: &FileSpec<'_>) -> Vec<u8> {
    let frames: Vec<Vec<u8>> = spec
        .frames
        .iter()
        .map(|(_, d)| {
            let mut d = d.clone();
            d.resize(d.len().next_multiple_of(4), 0);
            d
        })
        .collect();
    let mut out = spec.signature.to_vec();
    let ring = spec.flags & 1;
    let count = spec.frames.len() as u32 - ring.min(spec.frames.len() as u32);
    for v in [spec.width, spec.height, count, spec.frame_rate as u32, spec.flags] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0; 28]);
    out.extend_from_slice(&(spec.trees.len() as u32).to_le_bytes());
    for v in spec.tree_sizes.iter().chain(&spec.audio_rates).chain(&[0]) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for f in &frames {
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
    }
    out.extend(spec.frames.iter().map(|(flags, _)| *flags));
    out.extend_from_slice(spec.trees);
    for f in &frames {
        out.extend_from_slice(f);
    }
    out
}

/// The bytes of a frame: a palette chunk (padded to a multiple of four with its length byte),
/// the audio chunks (each behind its length), and the video data.
pub fn frame(palette: Option<&[u8]>, audio: &[&[u8]], video: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(p) = palette {
        let len = (p.len() + 1).next_multiple_of(4);
        out.push((len / 4) as u8);
        out.extend_from_slice(p);
        out.resize(len, 0);
    }
    for a in audio {
        out.extend_from_slice(&(a.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(a);
    }
    out.extend_from_slice(video);
    out
}
