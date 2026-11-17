//! Sierra VMD fixture writer. ffmpeg reads VMD but cannot write it, so the VMD tests build their
//! files here and use ffmpeg only to decode them. Written from the public format description
//! (<https://wiki.multimedia.cx/index.php/VMD>); the packers favour coverage of every coding
//! mode over compression.

use crate::hostile::Rng;

/// Bytes of the file header.
pub const HEADER_LEN: usize = 816;

/// The audio fields of the header.
#[derive(Clone, Copy, Debug)]
pub struct Audio {
    /// Samples per second.
    pub sample_rate: u16,
    /// The frame length field as stored: negative for 16-bit sound.
    pub frame_length: i16,
    /// Number of sound buffers.
    pub buffers: u16,
    /// Audio flags (0x8000 old stereo, 0x200 new stereo).
    pub flags: u16,
}

/// What the header says.
#[derive(Clone, Debug)]
pub struct Spec {
    /// Picture width in pixels.
    pub width: u16,
    /// Picture height in pixels.
    pub height: u16,
    /// Left coordinate of the picture.
    pub left: u16,
    /// Top coordinate of the picture.
    pub top: u16,
    /// 256 R-G-B triplets of 6-bit components.
    pub palette: [u8; 768],
    /// Size of the unpack buffer for LZ-packed frames.
    pub unpack: u32,
    /// Video codec version (header bytes 4-5).
    pub codec_version: u16,
    /// The sound, if the file has any.
    pub audio: Option<Audio>,
}

impl Spec {
    /// A silent file of `width` x `height` with a grey ramp palette.
    pub fn new(width: u16, height: u16) -> Spec {
        let mut palette = [0; 768];
        for (i, p) in palette.iter_mut().enumerate() {
            *p = (i / 3 / 4) as u8;
        }
        Spec { width, height, left: 0, top: 0, palette, unpack: 1 << 20, codec_version: 0, audio: None }
    }
}

/// One frame of a block: its 16-byte record and its data.
#[derive(Clone, Debug)]
pub struct Frame {
    /// Frame type: 1 audio, 2 video.
    pub kind: u8,
    /// Bytes 6 to 15 of the frame record.
    pub tail: [u8; 10],
    /// The frame data.
    pub data: Vec<u8>,
}

impl Frame {
    /// A video frame updating `rect` (left, top, right, bottom; inclusive).
    pub fn video(rect: [u16; 4], new_palette: bool, data: Vec<u8>) -> Frame {
        let mut tail = [0; 10];
        for (i, v) in rect.iter().enumerate() {
            tail[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        tail[9] = if new_palette { 2 } else { 0 };
        Frame { kind: 2, tail, data }
    }

    /// An audio frame with the given block flags (1 sound, 2 mask and blocks, 3 silence).
    pub fn audio(flags: u8, data: Vec<u8>) -> Frame {
        let mut tail = [0; 10];
        tail[0] = flags;
        Frame { kind: 1, tail, data }
    }
}

/// A whole file: header, frame data, then the table of contents. Every block must hold the same
/// number of frames.
#[allow(clippy::panic)]
pub fn build(spec: &Spec, blocks: &[Vec<Frame>]) -> Vec<u8> {
    let per_block = blocks.first().map_or(0, Vec::len);
    assert!(blocks.iter().all(|b| b.len() == per_block), "every block holds the same number of frames");
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    let mut records = Vec::new();
    for block in blocks {
        offsets.push((HEADER_LEN + data.len()) as u32);
        for f in block {
            records.push(f.kind);
            records.push(0);
            records.extend_from_slice(&(f.data.len() as u32).to_le_bytes());
            records.extend_from_slice(&f.tail);
            data.extend_from_slice(&f.data);
        }
    }
    let mut h = vec![0u8; HEADER_LEN];
    let put16 = |h: &mut [u8], at: usize, v: u16| h[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let put32 = |h: &mut [u8], at: usize, v: u32| h[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put16(&mut h, 0, (HEADER_LEN - 2) as u16);
    put16(&mut h, 4, spec.codec_version);
    put16(&mut h, 6, blocks.len() as u16);
    put16(&mut h, 8, spec.top);
    put16(&mut h, 10, spec.left);
    put16(&mut h, 12, spec.width);
    put16(&mut h, 14, spec.height);
    put16(&mut h, 16, if spec.audio.is_some() { 0x1000 } else { 0 });
    put16(&mut h, 18, per_block as u16);
    put32(&mut h, 20, HEADER_LEN as u32);
    h[28..796].copy_from_slice(&spec.palette);
    put32(&mut h, 800, spec.unpack);
    if let Some(a) = spec.audio {
        put16(&mut h, 804, a.sample_rate);
        put16(&mut h, 806, a.frame_length as u16);
        put16(&mut h, 808, a.buffers);
        put16(&mut h, 810, a.flags);
    }
    put32(&mut h, 812, (HEADER_LEN + data.len()) as u32);
    h.extend_from_slice(&data);
    for o in offsets {
        h.extend_from_slice(&[0, 0]);
        h.extend_from_slice(&o.to_le_bytes());
    }
    h.extend_from_slice(&records);
    h
}

/// The 770 bytes that open a video frame carrying a new palette.
pub fn palette_chunk(palette: &[u8; 768]) -> Vec<u8> {
    let mut v = vec![0, 255];
    v.extend_from_slice(palette);
    v
}

/// LZ-pack `data` (what follows the coding method byte): the length, the optional marker that
/// selects long chains, then tagged literals and chains. `rng` picks between a literal and a
/// chain where both are possible, so both paths get exercised.
pub fn lz_pack(data: &[u8], marker: bool, rng: &mut Rng) -> Vec<u8> {
    let mut out = (data.len() as u32).to_le_bytes().to_vec();
    let mut queue = [0x20u8; 4096];
    let mut qpos = if marker {
        out.extend_from_slice(&[0x34, 0x12, 0x78, 0x56]);
        0x111usize
    } else {
        0xFEE
    };
    let max_len = if marker { 18 + 255 } else { 18 };
    let mut pos = 0;
    while pos < data.len() {
        // a tag of 0xFF with more than 8 bytes left means 8 raw bytes
        let tag_at = out.len();
        out.push(0);
        let mut tag = 0u8;
        for bit in 0..8 {
            if pos >= data.len() {
                break;
            }
            let left = data.len() - pos;
            // the longest chain in the queue; a chain may run into the bytes it is writing
            let (mut best_len, mut best_off) = (0usize, 0usize);
            for off in 0..4096usize {
                let mut n = 0;
                while n < max_len.min(left) {
                    let behind = (off + n).wrapping_sub(qpos) & 0xFFF;
                    let have = if behind < n { data[pos + behind] } else { queue[(off + n) & 0xFFF] };
                    if have != data[pos + n] {
                        break;
                    }
                    n += 1;
                }
                if n > best_len {
                    (best_len, best_off) = (n, off);
                }
            }
            if best_len >= 3 && rng.below(4) != 0 {
                // sometimes a shorter chain than the longest
                let len = if rng.below(3) == 0 { 3 + rng.below(best_len - 2) } else { best_len };
                let nibble = if marker && len >= 18 { 15 } else { len - 3 };
                out.push(best_off as u8);
                out.push(((best_off >> 4) & 0xF0) as u8 | nibble as u8);
                if marker && len >= 18 {
                    out.push((len - 18) as u8);
                }
                for i in 0..len {
                    queue[(qpos + i) & 0xFFF] = data[pos + i];
                }
                qpos = (qpos + len) & 0xFFF;
                pos += len;
            } else {
                tag |= 1 << bit;
                out.push(data[pos]);
                queue[qpos] = data[pos];
                qpos = (qpos + 1) & 0xFFF;
                pos += 1;
            }
        }
        out[tag_at] = tag;
    }
    out
}

/// The body of a method 1 (`rle` false) or method 3 frame that turns the `rect` of `prev` into
/// that of `cur`; both are whole pictures of `stride` bytes a row. Unchanged runs are skipped.
/// With `rle`, changed runs are run-length packed when `rng` says so (always when they begin
/// with 0xFF, which a plain run cannot).
///
/// With `rle`, no changed run is a single pixel: the description and ffmpeg disagree on how a
/// run-length run of one pixel ends, and the game files seen do not have one. A lone changed
/// pixel takes its neighbour along, so the rectangle must be at least two pixels wide.
pub fn delta_lines(prev: &[u8], cur: &[u8], stride: usize, rect: [u16; 4], rle: bool, rng: &mut Rng) -> Vec<u8> {
    let [left, top, right, bottom] = rect.map(usize::from);
    let width = right - left + 1;
    let mut out = Vec::new();
    for y in top..=bottom {
        let row = y * stride + left;
        let mut changed: Vec<bool> = (0..width).map(|x| prev[row + x] != cur[row + x]).collect();
        if rle {
            for x in 0..width {
                let alone = changed[x] && (x == 0 || !changed[x - 1]) && (x + 1 == width || !changed[x + 1]);
                if alone && x + 1 < width {
                    changed[x + 1] = true;
                } else if alone && x > 0 {
                    changed[x - 1] = true;
                }
            }
        }
        let mut x = 0;
        while x < width {
            let mut end = x;
            while end < width && changed[end] == changed[x] {
                end += 1;
            }
            let mut n = (end - x).min(128);
            if rle && changed[x] && end - x - n == 1 {
                n -= 1;
            }
            if !changed[x] {
                out.push((n - 1) as u8);
            } else {
                let run = &cur[row + x..row + x + n];
                out.push(0x80 | (n - 1) as u8);
                if rle && (run[0] == 0xFF || rng.below(2) == 0) {
                    out.push(0xFF);
                    rle_pack(run, &mut out);
                } else {
                    out.extend_from_slice(run);
                }
            }
            x += n;
        }
    }
    out
}

fn rle_pack(run: &[u8], out: &mut Vec<u8>) {
    let mut run = run;
    if run.len() % 2 == 1 {
        out.push(run[0]);
        run = &run[1..];
    }
    let pairs = run.as_chunks::<2>().0;
    let mut i = 0;
    while i < pairs.len() {
        let mut n = 1;
        while i + n < pairs.len() && pairs[i + n] == pairs[i] && n < 127 {
            n += 1;
        }
        if n > 1 {
            out.push(n as u8);
            out.extend_from_slice(&pairs[i]);
            i += n;
            continue;
        }
        // pairs up to the next repeat go out as they are
        let start = i;
        while i < pairs.len() && i - start < 127 && !(i + 1 < pairs.len() && pairs[i + 1] == pairs[i]) {
            i += 1;
        }
        out.push(0x80 | (i - start) as u8);
        out.extend(pairs[start..i].iter().flatten());
    }
}

/// The step table of the 16-bit sound coding.
pub const DPCM_STEPS: [u16; 128] = {
    let mut t = [0u16; 128];
    let mut i = 1;
    while i < 128 {
        t[i] = match i {
            1 => 8,
            2..=33 => 16 * (i as u16 - 1),
            34..=97 => 512 + 8 * (i as u16 - 33),
            98..=113 => 1024 + 64 * (i as u16 - 97),
            114..=121 => 2048 + 256 * (i as u16 - 113),
            122..=125 => 4096 + 1024 * (i as u16 - 121),
            126 => 12288,
            _ => 16384,
        };
        i += 1;
    }
    t
};

/// Pack one block of 16-bit sound: `samples` are interleaved across `channels`. Returns the
/// block (each channel's first sample as it is, then one code a sample) and the samples a
/// decoder gets back from it.
pub fn dpcm_pack(samples: &[i16], channels: usize) -> (Vec<u8>, Vec<i16>) {
    let mut out = Vec::new();
    let mut decoded = Vec::new();
    let mut pred = [0i32; 2];
    for (i, &s) in samples.iter().enumerate() {
        let ch = i % channels;
        if i < channels {
            out.extend_from_slice(&s.to_le_bytes());
            pred[ch] = i32::from(s);
        } else {
            let diff = i32::from(s) - pred[ch];
            let mag = diff.unsigned_abs();
            let mut code = 0;
            for (c, &step) in DPCM_STEPS.iter().enumerate() {
                if u32::from(step).abs_diff(mag) < u32::from(DPCM_STEPS[code]).abs_diff(mag) {
                    code = c;
                }
            }
            let step = i32::from(DPCM_STEPS[code]);
            // stay inside 16 bits, so that a decoder's handling of overflow does not matter
            let (next, byte) = if diff < 0 { (pred[ch] - step, 0x80 | code as u8) } else { (pred[ch] + step, code as u8) };
            let (next, byte) = if (-32768..=32767).contains(&next) { (next, byte) } else { (pred[ch], 0) };
            pred[ch] = next;
            out.push(byte);
        }
        decoded.push(pred[ch] as i16);
    }
    (out, decoded)
}
