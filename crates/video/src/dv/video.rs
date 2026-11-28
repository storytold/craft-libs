//! A DV frame's picture (ITU-R BT.1618-1 Annex 1 §1.7 and §2; SMPTE ST 314 §4.7 and §5): the 135 video DIF blocks
//! of each DIF sequence hold compressed macro blocks, five of them a video segment whose overflowing coefficients
//! are shared out in three passes (§2.6). Each block is weighted DCT coefficients, quantised by class, area and the
//! macro block's quantisation number (§2.3), and transformed as one 8 × 8 block or as the sum and difference of two
//! fields (2-4-8, §2.2).

use std::sync::OnceLock;

use super::vlc::{self, Code};
use super::{Error, Header, Result, Sampling};

/// A decoded picture: 720 pixels wide, 8-bit Y'CbCr (ITU-R BT.601 levels), in the frame's own colour sampling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// Pixels a line.
    pub width: usize,
    /// Lines: 480 (525/60) or 576 (625/50).
    pub height: usize,
    /// How the colour difference planes are sampled.
    pub layout: Layout,
    /// Luma, `width` × `height`.
    pub y: Vec<u8>,
    /// Cb: `width / 4` wide and `height` lines ([`Layout::Yuv411`]), `width / 2` and `height / 2`
    /// ([`Layout::Yuv420`]), or `width / 2` and `height` ([`Layout::Yuv422`]).
    pub cb: Vec<u8>,
    /// Cr, as `cb`.
    pub cr: Vec<u8>,
    /// The picture is 16:9 (squeezed into the 720 pixels), not 4:3.
    pub wide: bool,
    /// The picture is two interlaced fields, bottom first ([`Header::interlaced`]), not progressive.
    pub interlaced: bool,
    /// The sample aspect ratio ([`Header::sample_aspect`]).
    pub sample_aspect: (u32, u32),
}

impl Picture {
    /// The size of the Cb and Cr planes.
    pub fn chroma_size(&self) -> (usize, usize) {
        match self.layout {
            Layout::Yuv411 => (self.width / 4, self.height),
            Layout::Yuv420 => (self.width / 2, self.height / 2),
            Layout::Yuv422 => (self.width / 2, self.height),
        }
    }
}

/// How a [`Picture`]'s colour difference is sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// A quarter of the width, every line: 525/60, and SMPTE ST 314's 625/50 (DVCPRO).
    Yuv411,
    /// Half the width, every other line: the 4:2:0 of IEC 61834's 625/50.
    Yuv420,
    /// Half the width, every line: the 50 Mb/s structure (DVCPRO50).
    Yuv422,
}

/// Pixels a line.
const WIDTH: usize = 720;
/// Bytes of a DIF block, of its ID, and of a DIF sequence.
const BLOCK: usize = 80;
const ID: usize = 3;
const SEQUENCE: usize = 150 * BLOCK;
/// Bits of a compressed macro block (77 bytes after the ID).
const MACRO_BITS: usize = 77 * 8;
/// A run of bits in a compressed macro block: from the first bit to the one after the last.
type Span = (usize, usize);
/// The compressed-data areas of the six DCT blocks (four Y, Cr, Cb) in a compressed macro block, in bits from its
/// first byte (STA and QNO): Y 14 bytes each, Cr and Cb 10 (Fig. 30).
const AREAS: [Span; 6] = [(8, 120), (120, 232), (232, 344), (344, 456), (456, 536), (536, 616)];
/// The 50 Mb/s structure's compressed macro block (Fig. 29): Y0, Y1, Cr and Cb, and after each Y a 2-byte data area
/// (X0, X1, not decoded) and an additional AC area (E) that passes 2 and 3 fill after the Y block's own area.
const AREAS_422: [Span; 4] = [(8, 120), (232, 344), (456, 536), (536, 616)];
const EXTRA_422: [Span; 2] = [(136, 232), (360, 456)];

/// Bits of a block's DC coefficient, DCT mode and class number.
const DC_BITS: usize = 12;
/// The DC error code (§2.6, Video error code): a block whose DC is this was not reproduced.
const DC_ERROR: i32 = -256;
/// What a block that was not reproduced decodes to.
const GREY: [u8; 64] = [128; 64];

/// The order the coefficients are sent in (Fig. 27), as indices `8 × v + h` (`v` the row: vertical frequency, or
/// for 2-4-8 the sum (0–3) and difference (4–7) of the fields): for 8-8 and for 2-4-8.
const SCAN: [[u8; 64]; 2] = [
    scan_of(&[
        1, 2, 6, 7, 15, 16, 28, 29, //
        3, 5, 8, 14, 17, 27, 30, 43, //
        4, 9, 13, 18, 26, 31, 42, 44, //
        10, 12, 19, 25, 32, 41, 45, 54, //
        11, 20, 24, 33, 40, 46, 53, 55, //
        21, 23, 34, 39, 47, 52, 56, 61, //
        22, 35, 38, 48, 51, 57, 60, 62, //
        36, 37, 49, 50, 58, 59, 63, 64,
    ]),
    scan_of(&[
        1, 3, 7, 19, 21, 35, 37, 51, //
        5, 9, 17, 23, 33, 39, 49, 53, //
        11, 15, 25, 31, 41, 47, 55, 61, //
        13, 27, 29, 43, 45, 57, 59, 63, //
        2, 4, 8, 20, 22, 36, 38, 52, //
        6, 10, 18, 24, 34, 40, 50, 54, //
        12, 16, 26, 32, 42, 48, 56, 62, //
        14, 28, 30, 44, 46, 58, 60, 64,
    ]),
];

/// The figure's grid (the 1-based place in the order of each coefficient) as the order.
const fn scan_of(order: &[u8; 64]) -> [u8; 64] {
    let mut scan = [0u8; 64];
    let mut at = 0;
    while at < 64 {
        let place = order[at] as usize;
        if place >= 1 && place <= 64 {
            scan[place - 1] = at as u8;
        }
        at += 1;
    }
    scan
}

/// The area number of each coefficient (Fig. 28), by index `8 × v + h`, for 8-8 and for 2-4-8 (the DC's is unused).
const AREA: [[u8; 64]; 2] = [
    [
        0, 0, 0, 1, 1, 1, 2, 2, //
        0, 0, 1, 1, 1, 2, 2, 2, //
        0, 1, 1, 1, 2, 2, 2, 3, //
        1, 1, 1, 2, 2, 2, 3, 3, //
        1, 1, 2, 2, 2, 3, 3, 3, //
        1, 2, 2, 2, 3, 3, 3, 3, //
        2, 2, 2, 3, 3, 3, 3, 3, //
        2, 2, 3, 3, 3, 3, 3, 3,
    ],
    [
        0, 0, 1, 1, 1, 2, 2, 3, //
        0, 1, 1, 2, 2, 2, 3, 3, //
        1, 1, 2, 2, 2, 3, 3, 3, //
        1, 2, 2, 2, 3, 3, 3, 3, //
        0, 0, 1, 1, 2, 2, 2, 3, //
        0, 1, 1, 2, 2, 2, 3, 3, //
        1, 1, 2, 2, 2, 3, 3, 3, //
        1, 2, 2, 3, 3, 3, 3, 3,
    ],
];

/// The quantisation steps of the four areas (Table 23), by row: row 0 is class 0's QNO 15.
const STEPS: [[u8; 4]; 22] = [
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 2],
    [1, 1, 2, 2],
    [1, 1, 2, 2],
    [1, 2, 2, 4],
    [1, 2, 2, 4],
    [2, 2, 4, 4],
    [2, 2, 4, 4],
    [2, 4, 4, 8],
    [2, 4, 4, 8],
    [4, 4, 8, 8],
    [4, 4, 8, 8],
    [4, 8, 8, 16],
    [4, 8, 8, 16],
    [8, 8, 16, 16],
    [8, 8, 16, 16],
];

/// The row of [`STEPS`] where each class's QNO 15 is (Table 23): class 0 from the first row, 1 three rows down,
/// 2 six, 3 five.
const CLASS_ROW: [usize; 4] = [0, 3, 6, 5];

/// The tables computed once: the inverse of each coefficient's weight (§2.2.2) and the cosines of the inverse DCT.
struct Tables {
    /// 1 / W(h, v) by mode and index `8 × v + h`.
    unweight: [[f32; 64]; 2],
    /// C(k) cos(π k (2x + 1) / 16) by `8 × k + x`.
    cos8: [f32; 64],
    /// C(u) cos(π u (2z + 1) / 8) by `4 × u + z`.
    cos4: [f32; 16],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let pi = std::f64::consts::PI;
        let cs = |m: f64| (m * pi / 16.0).cos();
        // w(0) … w(7) of §2.2.2.
        let w = [
            1.0,
            cs(4.0) / (4.0 * cs(7.0) * cs(2.0)),
            cs(4.0) / (2.0 * cs(6.0)),
            1.0 / (2.0 * cs(5.0)),
            7.0 / 8.0,
            cs(4.0) / cs(3.0),
            cs(4.0) / cs(2.0),
            cs(4.0) / cs(1.0),
        ];
        let mut unweight = [[0.0f32; 64]; 2];
        for v in 0..8 {
            for h in 0..8 {
                let i = 8 * v + h;
                let (eight, two) = if i == 0 {
                    (0.25, 0.25)
                } else {
                    let field = if v < 4 { 2 * v } else { 2 * (v - 4) };
                    (w[h] * w[v] / 2.0, w[h] * w[field] / 2.0)
                };
                unweight[0][i] = (1.0 / eight) as f32;
                unweight[1][i] = (1.0 / two) as f32;
            }
        }
        let scale = |k: usize| if k == 0 { 0.5 / 2f64.sqrt() } else { 0.5 };
        let mut cos8 = [0.0f32; 64];
        for k in 0..8 {
            for x in 0..8 {
                cos8[8 * k + x] = (scale(k) * (pi * k as f64 * (2 * x + 1) as f64 / 16.0).cos()) as f32;
            }
        }
        let mut cos4 = [0.0f32; 16];
        for u in 0..4 {
            for z in 0..4 {
                cos4[4 * u + z] = (scale(u) * (pi * u as f64 * (2 * z + 1) as f64 / 8.0).cos()) as f32;
            }
        }
        Tables { unweight, cos8, cos4 }
    })
}

/// One DCT block while a video segment is read.
#[derive(Clone)]
struct Block {
    /// The quantised AC coefficients by index `8 × v + h` (the DC is in `dc`).
    coefficients: [i32; 64],
    dc: i32,
    /// 0: 8-8, 1: 2-4-8.
    mode: usize,
    class: usize,
    /// The place in the scan order of the next coefficient (1 to 64).
    next: usize,
    /// Its EOB was read (or its data is broken).
    done: bool,
    /// The start of a code that did not fit where the block's data broke off, value and length.
    pending: (u32, usize),
}

impl Block {
    fn new() -> Self {
        Block { coefficients: [0; 64], dc: 0, mode: 0, class: 0, next: 1, done: false, pending: (0, 0) }
    }
}

/// Bits read one after another: the start of a code left over (`head`), then `tail` (one bit a byte, 0 or 1).
struct Source<'a> {
    head: (u32, usize),
    tail: &'a [u8],
    at: usize,
}

impl Source<'_> {
    fn bit(&self, at: usize) -> u32 {
        let (value, length) = self.head;
        if at < length { (value >> (length - 1 - at)) & 1 } else { self.tail.get(at - length).map_or(0, |&b| u32::from(b)) }
    }

    fn available(&self) -> usize {
        (self.head.1 + self.tail.len()).saturating_sub(self.at)
    }

    /// The next 16 bits, zeros past the end.
    fn peek(&self) -> u32 {
        (0..vlc::LONGEST as usize).fold(0, |v, i| (v << 1) | self.bit(self.at + i))
    }

    /// The bits not read, as a value and a length (fewer than 16: the start of one code).
    fn rest(&self) -> (u32, usize) {
        let length = self.available().min(vlc::LONGEST as usize);
        let value = (0..length).fold(0, |v, i| (v << 1) | self.bit(self.at + i));
        (value, length)
    }

    /// Bits of `tail` read.
    fn tail_used(&self) -> usize {
        self.at.saturating_sub(self.head.1).min(self.tail.len())
    }
}

/// Reads `block`'s codes from `source` until its EOB, or until the bits run out in the middle of a code.
fn read_codes(block: &mut Block, source: &mut Source<'_>) {
    let table = vlc::table();
    while !block.done {
        let Some(entry) = table.get(source.peek() as usize) else {
            block.done = true;
            break;
        };
        let length = usize::from(entry.length);
        if length == 0 {
            // With fewer than 16 bits left, the zeros after them can make the start of a code look like none: it
            // goes on where the block's data goes on. With 16, no code starts here: broken data, the rest of the
            // block is left as zeros.
            if source.available() >= vlc::LONGEST as usize {
                block.done = true;
            }
            break;
        }
        if length > source.available() {
            break;
        }
        source.at += length;
        match entry.code {
            Code::Invalid => block.done = true,
            Code::Eob => block.done = true,
            Code::Pair { run, amplitude } => {
                let place = block.next + usize::from(run);
                if amplitude != 0
                    && let Some(&index) = SCAN[block.mode].get(place)
                    && let Some(slot) = block.coefficients.get_mut(usize::from(index))
                {
                    *slot = i32::from(amplitude);
                }
                block.next = place + 1;
                if block.next > 64 {
                    block.done = true;
                }
            }
        }
    }
}

/// Decodes the picture of a DV frame whose header is `header` (25 Mb/s: 4:1:1, or 625/50's 4:2:0; 50 Mb/s: 4:2:2 in
/// two channels of DIF sequences, the first coding the even super block rows and the second the odd ones).
pub(crate) fn decode(frame: &[u8], header: &Header) -> Result<Picture> {
    let sequences = header.system.sequences();
    let channels = header.channels();
    let height = header.system.height();
    if frame.len() < header.frame_bytes() {
        return Err(Error::Invalid("the frame is shorter than its DIF sequences".to_owned()));
    }
    let (chroma_width, chroma_height) = match header.sampling {
        Sampling::C411 => (WIDTH / 4, height),
        Sampling::C420 => (WIDTH / 2, height / 2),
        Sampling::C422 => (WIDTH / 2, height),
    };
    let mut planes = Planes {
        y: vec![128; WIDTH * height],
        cb: vec![128; chroma_width * chroma_height],
        cr: vec![128; chroma_width * chroma_height],
        chroma_width,
    };
    // The DIF sequences are independent: each decodes its segments into macro blocks of pixels, which are put on
    // the planes here. With the `threads` feature the sequences are shared out over rayon's workers.
    let units = channels * sequences;
    let unit = |unit: usize| -> Vec<Decoded> {
        let (channel, sequence) = (unit / sequences, unit % sequences);
        (0..27).filter_map(|k| decode_segment(frame, header, (channel, sequence), k)).flatten().collect()
    };
    #[cfg(feature = "threads")]
    let decoded: Vec<Vec<Decoded>> = {
        use rayon::prelude::*;
        (0..units).into_par_iter().map(unit).collect()
    };
    #[cfg(not(feature = "threads"))]
    let decoded: Vec<Vec<Decoded>> = (0..units).map(unit).collect();
    for (x, y, shape, pixels) in decoded.into_iter().flatten() {
        for (l, block) in pixels.iter().enumerate() {
            put(&mut planes, shape, x, y, l, block);
        }
    }
    let layout = match header.sampling {
        Sampling::C411 => Layout::Yuv411,
        Sampling::C420 => Layout::Yuv420,
        Sampling::C422 => Layout::Yuv422,
    };
    Ok(Picture {
        width: WIDTH,
        height,
        layout,
        y: planes.y,
        cb: planes.cb,
        cr: planes.cr,
        wide: header.wide,
        interlaced: header.interlaced,
        sample_aspect: header.sample_aspect(),
    })
}

/// The planes being filled: Y 720 wide, Cb and Cr `chroma_width`.
struct Planes {
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
    chroma_width: usize,
}

/// The shape of a macro block on the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// 4:1:1: four Y blocks side by side (32 × 8), Cr and Cb 8 × 8.
    Wide,
    /// 4:1:1's rightmost column: Y 2 × 2 (16 × 16), Cr and Cb 4 × 16 sent as 8 × 8 (Fig. 16).
    Right,
    /// 4:2:0: Y 2 × 2, Cr and Cb 8 × 8 over the same 16 × 16.
    Square,
    /// 4:2:2: Y 2 × 1 (16 × 8), Cr and Cb 8 × 8 over the same 16 × 8 (Fig. 19).
    Half,
}

/// Where macro block `k` of super block (`i`, `j`) is: its luma x and y and its shape (Figs 22, 24, 26 for 4:1:1;
/// 4:2:0 from US 5,585,930: a super block of 9 × 3 macro blocks, numbered down and up the columns).
fn place(sampling: Sampling, i: usize, j: usize, k: usize) -> (usize, usize, Shape) {
    // 4:2:0 and 4:2:2: 9 × 3 macro blocks, down the first column, up the second, … (Fig. 25 for 4:2:2).
    let column = k / 3;
    let row = if column.is_multiple_of(2) { k % 3 } else { 2 - k % 3 };
    match sampling {
        Sampling::C420 => ((9 * j + column) * 16, (3 * i + row) * 16, Shape::Square),
        Sampling::C422 => ((9 * j + column) * 16, (3 * i + row) * 8, Shape::Half),
        Sampling::C411 => {
            // Super block rows are 6 macro block rows (of 8 lines); macro block columns 32 pixels.
            let top = 6 * i;
            match j {
                1 | 3 => {
                    let first = if j == 1 { 4 } else { 13 };
                    if k < 3 {
                        // The lower half of the column shared with the super block on the left.
                        (first * 32, (top + 3 + k) * 8, Shape::Wide)
                    } else {
                        let k = k - 3;
                        let column = k / 6;
                        let row = if column.is_multiple_of(2) { 5 - k % 6 } else { k % 6 };
                        ((first + 1 + column) * 32, (top + row) * 8, Shape::Wide)
                    }
                }
                _ => {
                    let first = match j {
                        0 => 0,
                        2 => 9,
                        _ => 18,
                    };
                    if k < 24 {
                        let column = k / 6;
                        let row = if column.is_multiple_of(2) { k % 6 } else { 5 - k % 6 };
                        ((first + column) * 32, (top + row) * 8, Shape::Wide)
                    } else if j == 4 {
                        (704, (top + 2 * (k - 24)) * 8, Shape::Right)
                    } else {
                        ((first + 4) * 32, (top + k - 24) * 8, Shape::Wide)
                    }
                }
            }
        }
    }
}

/// A decoded macro block: its luma x and y, its shape, and the pixels of its six DCT blocks.
type Decoded = (usize, usize, Shape, [[u8; 64]; 6]);

/// Decodes video segment `k` of DIF sequence `sequence` of channel `channel` (0 but for 50 Mb/s): its five
/// compressed macro blocks, read in the three passes of §2.6, as pixels and where they go (`None` when the frame is
/// too short for it).
fn decode_segment(frame: &[u8], header: &Header, (channel, sequence): (usize, usize), k: usize) -> Option<[Decoded; 5]> {
    let n = header.system.sequences();
    let (areas, extra): (&[Span], &[Span]) = match header.sampling {
        Sampling::C422 => (&AREAS_422, &EXTRA_422),
        _ => (&AREAS, &[]),
    };
    // The macro blocks of the segment, in the order of its DIF blocks (§2.1.6, Tables 18 and 19): super block row
    // and column. At 50 Mb/s the first channel codes the even rows, the second the odd ones.
    let row = |offset: usize| match header.sampling {
        Sampling::C422 => 2 * ((sequence + offset) % n) + channel,
        _ => (sequence + offset) % n,
    };
    let rows = [row(2), row(6), row(8), row(0), row(4)];
    let columns = [2, 1, 3, 0, 4];
    // The segment's bits, one a byte, and each macro block's quantisation number and status (STA).
    let mut bits = [0u8; 5 * MACRO_BITS];
    let mut qno = [0usize; 5];
    let mut sta = [0u8; 5];
    for (m, chunk) in bits.as_chunks_mut::<MACRO_BITS>().0.iter_mut().enumerate() {
        // Video DIF block 5k + m of the sequence: after the header, subcode and VAUX blocks, 15 to an audio block.
        let v = 5 * k + m;
        let index = 6 + (v / 15) * 16 + 1 + v % 15;
        let start = (channel * n + sequence) * SEQUENCE + index * BLOCK + ID;
        let bytes = frame.get(start..start + MACRO_BITS / 8)?;
        for (i, bit) in chunk.iter_mut().enumerate() {
            *bit = (bytes[i / 8] >> (7 - i % 8)) & 1;
        }
        qno[m] = usize::from(bytes[0] & 0x0F);
        sta[m] = bytes[0] >> 4;
    }
    // STA (Table 26): 0111 and 1111 say the macro block has errors, so it is left grey. Only the macro blocks whose
    // STA has s3 = 0 and s0 = 0 keep the data continuous with each other (type a): pass 3 runs over those alone, and
    // the others neither give it their unused space nor take from it.
    let damaged = sta.map(|status| matches!(status, 0b0111 | 0b1111));
    let continuous = sta.map(|status| status & 0b1001 == 0);
    // Six DCT blocks a macro block, four at 50 Mb/s (the rest marked done, so no pass reads into them).
    let mut blocks: [[Block; 6]; 5] = std::array::from_fn(|_| {
        std::array::from_fn(|l| {
            let mut block = Block::new();
            block.done = l >= areas.len();
            block
        })
    });
    // The unused end of each block's area after pass 1: (start, end) in `bits`.
    let mut unused = [[(0usize, 0usize); 6]; 5];

    // Pass 1: each block from its own area.
    for (m, macro_blocks) in blocks.iter_mut().enumerate() {
        for (block, &(start, end)) in macro_blocks.iter_mut().zip(areas) {
            let l = (start, end);
            let base = m * MACRO_BITS;
            let area = &bits[base + start..base + end];
            let field = |from: usize, count: usize| area[from..from + count].iter().fold(0u32, |v, &b| (v << 1) | u32::from(b));
            let dc = field(0, 9);
            block.dc = if dc & 0x100 != 0 { dc as i32 - 512 } else { dc as i32 };
            block.mode = field(9, 1) as usize;
            block.class = field(10, 2) as usize;
            let mut source = Source { head: (0, 0), tail: &area[DC_BITS..], at: 0 };
            read_codes(block, &mut source);
            if block.done {
                let from = base + start + DC_BITS + source.tail_used();
                if let Some(index) = areas.iter().position(|&area| area == l) {
                    unused[m][index] = (from, base + end);
                }
            } else {
                block.pending = source.rest();
            }
        }
    }

    // Pass 2: what each macro block's blocks left over, from the unused space of the same macro block.
    let mut leftover: Vec<u8> = Vec::new();
    for (m, macro_blocks) in blocks.iter_mut().enumerate() {
        // Each block's unused end, and after a 50 Mb/s Y block its additional AC area.
        let base = m * MACRO_BITS;
        let space: Vec<u8> = unused[m]
            .iter()
            .enumerate()
            .flat_map(|(l, &(from, to))| {
                let more = extra.get(l).map_or((0, 0), |&(start, end)| (base + start, base + end));
                bits[from..to].iter().chain(&bits[more.0..more.1]).copied()
            })
            .collect();
        let mut used = 0;
        for block in macro_blocks.iter_mut().filter(|b| !b.done) {
            used = continue_block(block, &space, used);
        }
        if continuous[m] && macro_blocks.iter().all(|b| b.done) {
            leftover.extend_from_slice(&space[used.min(space.len())..]);
        }
    }

    // Pass 3: what is still left over, from the space no macro block used, in order.
    let mut used = 0;
    for (macro_blocks, _) in blocks.iter_mut().zip(continuous).filter(|&(_, continuous)| continuous) {
        for block in macro_blocks.iter_mut().filter(|b| !b.done) {
            used = continue_block(block, &leftover, used);
        }
    }

    // The pixels.
    let tables = tables();
    Some(std::array::from_fn(|m| {
        let (x, y, shape) = place(header.sampling, rows[m], columns[m], k);
        let pixels = std::array::from_fn(|l| if damaged[m] { GREY } else { reconstruct(&blocks[m][l], qno[m], tables) });
        (x, y, shape, pixels)
    }))
}

/// Continues `block` from `stream`, of which `used` bits were taken already; how many are taken after it.
fn continue_block(block: &mut Block, stream: &[u8], used: usize) -> usize {
    let tail = stream.get(used..).unwrap_or(&[]);
    let mut source = Source { head: block.pending, tail, at: 0 };
    read_codes(block, &mut source);
    if block.done {
        used + source.tail_used()
    } else {
        block.pending = source.rest();
        stream.len()
    }
}

/// The 8 × 8 pixels of `block` (rows of 8), from its quantised coefficients.
fn reconstruct(block: &Block, qno: usize, tables: &Tables) -> [u8; 64] {
    if block.dc == DC_ERROR {
        // Not reproduced: left grey rather than taken for a dark block.
        return GREY;
    }
    let mode = block.mode.min(1);
    let class = block.class.min(3);
    let row = (CLASS_ROW[class] + 15).saturating_sub(qno).min(STEPS.len() - 1);
    let steps = STEPS[row];
    // Class 3's coefficients were halved before they were quantised (§2.3.4).
    let scale = if class == 3 { 2 } else { 1 };
    let mut c = [0.0f32; 64];
    c[0] = block.dc as f32 * tables.unweight[mode][0];
    for (i, &q) in block.coefficients.iter().enumerate().skip(1) {
        if q != 0 {
            let step = i32::from(steps[usize::from(AREA[mode][i])]);
            c[i] = (q * step * scale) as f32 * tables.unweight[mode][i];
        }
    }
    let mut out = [0.0f32; 64];
    if mode == 0 {
        idct88(&c, &mut out, tables);
    } else {
        idct248(&c, &mut out, tables);
    }
    let mut pixels = [0u8; 64];
    for (pixel, value) in pixels.iter_mut().zip(out) {
        *pixel = (value + 128.0).round().clamp(0.0, 255.0) as u8;
    }
    pixels
}

/// The 8-8 inverse DCT: `c` by `8 × v + h`, `out` by `8 × y + x`.
fn idct88(c: &[f32; 64], out: &mut [f32; 64], tables: &Tables) {
    // Across: t[v][x] = Σ_h C(h) c[v][h] cos(π h (2x + 1) / 16).
    let mut t = [0.0f32; 64];
    for v in 0..8 {
        for x in 0..8 {
            t[8 * v + x] = (0..8).map(|h| c[8 * v + h] * tables.cos8[8 * h + x]).sum();
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            out[8 * y + x] = (0..8).map(|v| t[8 * v + x] * tables.cos8[8 * v + y]).sum();
        }
    }
}

/// The 2-4-8 inverse DCT: rows 0–3 of `c` are the sum of the fields, 4–7 their difference; the 4-point transforms
/// give the sum and difference of each pair of lines (2z, 2z + 1).
fn idct248(c: &[f32; 64], out: &mut [f32; 64], tables: &Tables) {
    let mut t = [0.0f32; 64];
    for v in 0..8 {
        for x in 0..8 {
            t[8 * v + x] = (0..8).map(|h| c[8 * v + h] * tables.cos8[8 * h + x]).sum();
        }
    }
    for z in 0..4 {
        for x in 0..8 {
            let sum: f32 = (0..4).map(|u| t[8 * u + x] * tables.cos4[4 * u + z]).sum();
            let difference: f32 = (0..4).map(|u| t[8 * (u + 4) + x] * tables.cos4[4 * u + z]).sum();
            out[8 * (2 * z) + x] = sum + difference;
            out[8 * (2 * z + 1) + x] = sum - difference;
        }
    }
}

/// Puts DCT block `l` (0–3 Y, 4 Cr, 5 Cb) of the macro block at luma (`x`, `y`) on `planes`.
fn put(planes: &mut Planes, shape: Shape, x: usize, y: usize, l: usize, pixels: &[u8; 64]) {
    if shape == Shape::Half {
        put_half(planes, x, y, l, pixels);
        return;
    }
    let chroma_width = planes.chroma_width;
    let write = |plane: &mut Vec<u8>, width: usize, at_x: usize, at_y: usize, row: &[u8]| {
        let start = at_y * width + at_x;
        if let Some(target) = plane.get_mut(start..start + row.len()) {
            target.copy_from_slice(row);
        }
    };
    match (shape, l) {
        (Shape::Wide, 0..=3) => {
            for (r, row) in pixels.as_chunks::<8>().0.iter().enumerate() {
                write(&mut planes.y, WIDTH, x + 8 * l, y + r, row);
            }
        }
        (Shape::Right | Shape::Square, 0..=3) => {
            let (dx, dy) = (8 * (l % 2), 8 * (l / 2));
            for (r, row) in pixels.as_chunks::<8>().0.iter().enumerate() {
                write(&mut planes.y, WIDTH, x + dx, y + dy + r, row);
            }
        }
        (_, 4 | 5) => {
            let plane = if l == 4 { &mut planes.cr } else { &mut planes.cb };
            match shape {
                Shape::Wide => {
                    for (r, row) in pixels.as_chunks::<8>().0.iter().enumerate() {
                        write(plane, chroma_width, x / 4, y + r, row);
                    }
                }
                Shape::Right => {
                    // 4 wide and 16 tall: the left half of the block is the upper 8 lines, the right half the lower.
                    for (r, row) in pixels.as_chunks::<8>().0.iter().enumerate() {
                        write(plane, chroma_width, x / 4, y + r, &row[..4]);
                        write(plane, chroma_width, x / 4, y + 8 + r, &row[4..]);
                    }
                }
                Shape::Square => {
                    for (r, row) in pixels.as_chunks::<8>().0.iter().enumerate() {
                        write(plane, chroma_width, x / 2, y / 2 + r, row);
                    }
                }
                // Put by `put_half`.
                Shape::Half => {}
            }
        }
        _ => {}
    }
}

/// [`put`] for a 4:2:2 macro block: blocks 0 and 1 are Y side by side, 2 Cr and 3 Cb.
fn put_half(planes: &mut Planes, x: usize, y: usize, l: usize, pixels: &[u8; 64]) {
    let chroma_width = planes.chroma_width;
    let (plane, width, at_x) = match l {
        0 | 1 => (&mut planes.y, WIDTH, x + 8 * l),
        2 => (&mut planes.cr, chroma_width, x / 2),
        3 => (&mut planes.cb, chroma_width, x / 2),
        _ => return,
    };
    for (r, row) in pixels.as_chunks::<8>().0.iter().enumerate() {
        let start = (y + r) * width + at_x;
        if let Some(target) = plane.get_mut(start..start + 8) {
            target.copy_from_slice(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scan_orders_visit_every_coefficient_once() {
        for scan in SCAN {
            let mut seen = [false; 64];
            for index in scan {
                assert!(!seen[usize::from(index)]);
                seen[usize::from(index)] = true;
            }
            assert_eq!(scan[0], 0, "the DC comes first");
        }
        // 8-8 goes across first, 2-4-8 to the difference of the fields first.
        assert_eq!(&SCAN[0][..3], &[0, 1, 8]);
        assert_eq!(&SCAN[1][..3], &[0, 32, 1]);
    }

    #[test]
    fn each_quantisation_number_of_each_class_has_a_row() {
        for (class, &first) in CLASS_ROW.iter().enumerate() {
            for qno in 0..16 {
                let row = first + 15 - qno;
                assert!(row < STEPS.len(), "class {class} QNO {qno}");
            }
        }
        // Table 23: class 0 QNO 0 is (2, 4, 4, 8), class 2 QNO 0 is (8, 8, 16, 16), class 3 QNO 15 is all 1.
        assert_eq!(STEPS[CLASS_ROW[0] + 15], [2, 4, 4, 8]);
        assert_eq!(STEPS[CLASS_ROW[2] + 15], [8, 8, 16, 16]);
        assert_eq!(STEPS[CLASS_ROW[3]], [1, 1, 1, 1]);
        assert_eq!(STEPS[CLASS_ROW[1] + 15], [4, 8, 8, 16]);
    }

    #[test]
    fn every_macro_block_has_a_place_of_its_own() {
        for (sampling, height) in [(Sampling::C411, 480), (Sampling::C411, 576), (Sampling::C420, 576), (Sampling::C422, 480), (Sampling::C422, 576)]
        {
            // Super block rows of 48 lines (24 at 50 Mb/s, as many again).
            let rows = if sampling == Sampling::C422 { height / 24 } else { height / 48 };
            // Each luma pixel is covered once.
            let mut covered = vec![0u8; WIDTH * height];
            for i in 0..rows {
                for j in 0..5 {
                    for k in 0..27 {
                        let (x, y, shape) = place(sampling, i, j, k);
                        let (w, h) = match shape {
                            Shape::Wide => (32, 8),
                            Shape::Right | Shape::Square => (16, 16),
                            Shape::Half => (16, 8),
                        };
                        for yy in y..y + h {
                            for xx in x..x + w {
                                covered[yy * WIDTH + xx] += 1;
                            }
                        }
                    }
                }
            }
            assert!(covered.iter().all(|&c| c == 1), "{sampling:?} {height}");
        }
    }

    #[test]
    fn a_dc_only_block_is_flat_at_half_its_dc() {
        let mut block = Block::new();
        block.dc = 40;
        block.done = true;
        let pixels = reconstruct(&block, 15, tables());
        assert!(pixels.iter().all(|&p| p == 148), "{pixels:?}");
        block.mode = 1;
        let pixels = reconstruct(&block, 15, tables());
        assert!(pixels.iter().all(|&p| p == 148), "{pixels:?}");
    }

    #[test]
    fn the_inverse_dct_undoes_the_forward_one_of_the_recommendation() {
        // §2.2.1's forward transforms of a picture, then the inverse: the same picture (8-8 and 2-4-8).
        let picture: Vec<f32> = (0..64).map(|i| ((i * 37) % 200) as f32 - 100.0).collect();
        let pi = std::f32::consts::PI;
        let scale = |k: usize| if k == 0 { 0.5 / 2f32.sqrt() } else { 0.5 };
        let mut c = [0.0f32; 64];
        for v in 0..8 {
            for h in 0..8 {
                let mut s = 0.0;
                for y in 0..8 {
                    for x in 0..8 {
                        s += picture[8 * y + x]
                            * (pi * v as f32 * (2 * y + 1) as f32 / 16.0).cos()
                            * (pi * h as f32 * (2 * x + 1) as f32 / 16.0).cos();
                    }
                }
                c[8 * v + h] = scale(v) * scale(h) * s;
            }
        }
        let mut out = [0.0f32; 64];
        idct88(&c, &mut out, tables());
        for (a, b) in out.iter().zip(&picture) {
            assert!((a - b).abs() < 0.01, "{a} {b}");
        }
        let mut c = [0.0f32; 64];
        for u in 0..4 {
            for h in 0..8 {
                let (mut sum, mut difference) = (0.0, 0.0);
                for z in 0..4 {
                    for x in 0..8 {
                        let kc = (pi * u as f32 * (2 * z + 1) as f32 / 8.0).cos() * (pi * h as f32 * (2 * x + 1) as f32 / 16.0).cos();
                        sum += (picture[8 * (2 * z) + x] + picture[8 * (2 * z + 1) + x]) * kc;
                        difference += (picture[8 * (2 * z) + x] - picture[8 * (2 * z + 1) + x]) * kc;
                    }
                }
                c[8 * u + h] = scale(u) * scale(h) * sum;
                c[8 * (u + 4) + h] = scale(u) * scale(h) * difference;
            }
        }
        idct248(&c, &mut out, tables());
        for (a, b) in out.iter().zip(&picture) {
            assert!((a - b).abs() < 0.01, "{a} {b}");
        }
    }

    #[test]
    fn a_segment_overflowing_into_its_neighbours_is_read_in_three_passes() {
        // One stream split as §2.6 splits it: block 0 of macro block 0 has more codes than fit in its area; they
        // go first to the unused space of the same macro block's blocks, then to that of the next macro blocks.
        let push = |bits: &mut Vec<u8>, code: &str| bits.extend(code.bytes().map(|b| b - b'0'));
        // 60 codes of (0, 1) +: 3 bits each, 180 bits: more than the 100 after the DC in a Y area.
        let mut long = Vec::new();
        for _ in 0..60 {
            push(&mut long, "000");
        }
        push(&mut long, "0110");
        let mut block = Block::new();
        // Pass 1 reads what fits in 100 bits: 33 codes and one bit of the 34th.
        let mut source = Source { head: (0, 0), tail: &long[..100], at: 0 };
        read_codes(&mut block, &mut source);
        block.pending = source.rest();
        assert!(!block.done);
        assert_eq!(block.next, 34);
        assert_eq!(block.pending, (0, 1));
        // Pass 2: 50 more bits.
        let used = continue_block(&mut block, &long[100..150], 0);
        assert_eq!(used, 50);
        assert!(!block.done);
        // Pass 3: the rest, then 10 bits that belong to someone else.
        let mut rest = long[150..].to_vec();
        push(&mut rest, "1111111111");
        let used = continue_block(&mut block, &rest, 0);
        assert!(block.done);
        assert_eq!(used, long.len() - 150);
        assert_eq!(block.next, 61);
        assert!(block.coefficients.iter().filter(|&&c| c == 1).count() == 60);
    }

    #[test]
    fn a_block_with_its_last_coefficient_still_ends_at_its_eob() {
        // Every place of the scan taken (63 codes of (0, 1) +), then the EOB that ends every block (§2.6), then
        // bits of the next block: the EOB is read, and nothing after it.
        let push = |bits: &mut Vec<u8>, code: &str| bits.extend(code.bytes().map(|b| b - b'0'));
        let mut stream = Vec::new();
        for _ in 0..63 {
            push(&mut stream, "000");
        }
        let last = stream.len();
        push(&mut stream, "0110");
        let full = stream.len();
        push(&mut stream, "1111");
        let mut block = Block::new();
        let mut source = Source { head: (0, 0), tail: &stream, at: 0 };
        read_codes(&mut block, &mut source);
        assert!(block.done);
        assert_eq!(source.tail_used(), full);
        // With the block's data broken off after its last coefficient, it waits for its EOB in the next pass.
        let mut block = Block::new();
        let mut source = Source { head: (0, 0), tail: &stream[..last], at: 0 };
        read_codes(&mut block, &mut source);
        assert!(!block.done);
        block.pending = source.rest();
        assert_eq!(continue_block(&mut block, &stream[last..], 0), full - last);
        assert!(block.done);
    }

    #[test]
    fn a_macro_block_whose_status_says_error_is_grey() {
        // A segment of five macro blocks whose blocks are each a DC of 40 and an EOB: flat at 148, unless the macro
        // block's STA says it has errors (Table 26: 0111, 1111). One concealed without errors (1010) still decodes.
        let header = Header { system: crate::dv::System::Lines525, sampling: Sampling::C411, wide: false, interlaced: true };
        let segment = |sta: [u8; 5]| {
            let mut frame = vec![0u8; header.frame_bytes()];
            for (m, &status) in sta.iter().enumerate() {
                // Video DIF block m of the first segment of the first DIF sequence.
                let start = (7 + m) * BLOCK + ID;
                frame[start] = (status << 4) | 15;
                for &(area, _) in &AREAS {
                    // DC 000101000, 8-8, class 00, EOB 0110.
                    for (i, bit) in "0001010000000110".bytes().enumerate() {
                        let at = area + i;
                        frame[start + at / 8] |= (bit - b'0') << (7 - at % 8);
                    }
                }
            }
            decode_segment(&frame, &header, (0, 0), 0).expect("a whole segment")
        };
        let flat = |decoded: &Decoded, value: u8| decoded.3.iter().flatten().all(|&p| p == value);
        let clean = segment([0; 5]);
        assert!(clean.iter().all(|decoded| flat(decoded, 148)));
        let marked = segment([0, 0b0111, 0b1111, 0b1010, 0]);
        assert!(flat(&marked[1], 128) && flat(&marked[2], 128));
        assert!(flat(&marked[0], 148) && flat(&marked[3], 148) && flat(&marked[4], 148));
    }
}
