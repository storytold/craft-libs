//! A clean-room Vorbis I decoder (feature `vorbis-decode`), written from the Vorbis I
//! specification (Xiph.Org, 2020).
//!
//! [`Decoder`] works on packets: build it from the three header packets (identification, comment
//! and setup), then give it audio packets in order. Each call returns the PCM the packet
//! completes, planar `f32` in the nominal range -1.0 to 1.0 (not clipped), channels in the order of
//! the stream. The first packet only primes the overlap and returns nothing. [`decode_ogg`] reads a
//! whole Ogg Vorbis file.
//!
//! Codebooks, floors 0 and 1, residues 0, 1 and 2, channel coupling and both block sizes with their
//! hybrid windows are implemented. Damaged input is an error, never a panic: setup sizes are
//! bounded before anything is allocated, and the codeword trees are checked for completeness.

mod bits;
mod codebook;
mod floor;
mod mdct;
mod residue;
mod tables;

use bits::{BitReader, Eop, ilog};
use codebook::Codebook;
use floor::{Fail, Floor, FloorData};
use mdct::{Imdct, slope};
use residue::Residue;

/// Errors of the Vorbis decoder.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The identification or comment header is not valid Vorbis.
    #[error("invalid Vorbis header: {0}")]
    Header(&'static str),
    /// The setup header is damaged or uses something Vorbis I does not allow.
    #[error("invalid Vorbis setup: {0}")]
    Setup(&'static str),
    /// The setup or decoded PCM asks for more memory than the decoder allows.
    #[error("Vorbis decode above the limits: {0}")]
    Limit(&'static str),
    /// The packet is not an audio packet (a header, or a packet with its first bit set).
    #[error("not an audio packet")]
    NotAudio,
    /// The packet is damaged: it ends before its mode and window flags are read, or names a mode
    /// that does not exist.
    #[error("damaged audio packet: {0}")]
    Packet(&'static str),
    /// An Ogg file with no Vorbis stream, or one that cannot be read.
    #[error("Ogg Vorbis file: {0}")]
    Ogg(String),
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, Error>;

/// The identification header (§4.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identification {
    /// The channel count, at least 1.
    pub channels: u8,
    /// Samples per second per channel.
    pub sample_rate: u32,
    /// The maximum bit rate in bits per second, 0 or negative when not set.
    pub bitrate_maximum: i32,
    /// The nominal bit rate in bits per second, 0 or negative when not set.
    pub bitrate_nominal: i32,
    /// The minimum bit rate in bits per second, 0 or negative when not set.
    pub bitrate_minimum: i32,
    /// The short block size in samples.
    pub blocksize_0: u32,
    /// The long block size in samples.
    pub blocksize_1: u32,
}

/// The comment header (§5): a vendor string and `NAME=value` comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comments {
    /// The vendor string of the encoder.
    pub vendor: String,
    /// The user comments, each `NAME=value`.
    pub comments: Vec<String>,
}

impl Comments {
    /// The first comment named `name` (case-insensitive), its value.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.comments.iter().find_map(|c| {
            let (k, v) = c.split_once('=')?;
            k.eq_ignore_ascii_case(name).then_some(v)
        })
    }
}

fn header_start(packet: &[u8], kind: u8) -> Result<BitReader<'_>> {
    if packet.len() < 7 || packet[0] != kind || &packet[1..7] != b"vorbis" {
        return Err(Error::Header("wrong packet type or missing \"vorbis\" tag"));
    }
    let mut r = BitReader::new(packet);
    let _ = r.read(8);
    for _ in 0..6 {
        let _ = r.read(8);
    }
    Ok(r)
}

fn read_identification(packet: &[u8]) -> Result<Identification> {
    let mut r = header_start(packet, 1)?;
    let eop = |_: Eop| Error::Header("identification header is truncated");
    if r.read(32).map_err(eop)? != 0 {
        return Err(Error::Header("unsupported Vorbis version"));
    }
    let channels = r.read(8).map_err(eop)? as u8;
    let sample_rate = r.read(32).map_err(eop)?;
    let bitrate_maximum = r.read(32).map_err(eop)? as i32;
    let bitrate_nominal = r.read(32).map_err(eop)? as i32;
    let bitrate_minimum = r.read(32).map_err(eop)? as i32;
    let e0 = r.read(4).map_err(eop)?;
    let e1 = r.read(4).map_err(eop)?;
    if !r.bit().map_err(eop)? {
        return Err(Error::Header("identification header framing bit is not set"));
    }
    if channels == 0 || sample_rate == 0 {
        return Err(Error::Header("no channels or a zero sample rate"));
    }
    if !(6..=13).contains(&e0) || !(6..=13).contains(&e1) || e0 > e1 {
        return Err(Error::Header("block sizes must be powers of two from 64 to 8192, the short one not above the long one"));
    }
    Ok(Identification { channels, sample_rate, bitrate_maximum, bitrate_nominal, bitrate_minimum, blocksize_0: 1 << e0, blocksize_1: 1 << e1 })
}

/// Comments are best effort: damage here is not fatal (§4.2.1).
fn read_comments(packet: &[u8]) -> Result<Comments> {
    header_start(packet, 3)?;
    let mut at = 7usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = packet.get(at..at.checked_add(n)?)?;
        at += n;
        Some(s)
    };
    let word = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let mut out = Comments::default();
    let Some(n) = take(4).map(word) else { return Ok(out) };
    let Some(vendor) = take(n) else { return Ok(out) };
    out.vendor = String::from_utf8_lossy(vendor).into_owned();
    let Some(count) = take(4).map(word) else { return Ok(out) };
    for _ in 0..count {
        let Some(n) = take(4).map(word) else { break };
        let Some(c) = take(n) else { break };
        out.comments.push(String::from_utf8_lossy(c).into_owned());
    }
    Ok(out)
}

struct Mapping {
    coupling: Vec<(usize, usize)>,
    mux: Vec<usize>,
    /// `(floor, residue)` of each submap.
    submaps: Vec<(usize, usize)>,
}

struct Mode {
    long: bool,
    mapping: usize,
}

struct Setup {
    books: Vec<Codebook>,
    floors: Vec<Floor>,
    residues: Vec<Residue>,
    mappings: Vec<Mapping>,
    modes: Vec<Mode>,
}

/// Bytes the codebooks of a setup may take in all (codeword trees and vector tables).
const CODEBOOK_BUDGET: usize = 256 << 20;

fn read_setup(packet: &[u8], channels: usize) -> Result<Setup> {
    let mut r = header_start(packet, 5)?;
    let eop = |_: Eop| Error::Setup("setup header is truncated");
    let count = r.read(8).map_err(eop)? as usize + 1;
    let mut budget = CODEBOOK_BUDGET;
    let mut books = Vec::with_capacity(count);
    for _ in 0..count {
        books.push(Codebook::read(&mut r, &mut budget)?);
    }
    // Time domain transforms: placeholders.
    for _ in 0..r.read(6).map_err(eop)? + 1 {
        if r.read(16).map_err(eop)? != 0 {
            return Err(Error::Setup("a time domain transform is not zero"));
        }
    }
    let count = r.read(6).map_err(eop)? as usize + 1;
    let mut floors = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = r.read(16).map_err(eop)?;
        floors.push(Floor::read(kind, &mut r, &books)?);
    }
    let count = r.read(6).map_err(eop)? as usize + 1;
    let mut residues = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = r.read(16).map_err(eop)?;
        if kind > 2 {
            return Err(Error::Setup("unknown residue type"));
        }
        residues.push(Residue::read(kind, &mut r, &books)?);
    }
    let count = r.read(6).map_err(eop)? as usize + 1;
    let mut mappings = Vec::with_capacity(count);
    for _ in 0..count {
        if r.read(16).map_err(eop)? != 0 {
            return Err(Error::Setup("unknown mapping type"));
        }
        let submaps = if r.bit().map_err(eop)? { r.read(4).map_err(eop)? as usize + 1 } else { 1 };
        let mut coupling = Vec::new();
        if r.bit().map_err(eop)? {
            let steps = r.read(8).map_err(eop)? as usize + 1;
            let bits = ilog(channels as i64 - 1);
            for _ in 0..steps {
                let magnitude = r.read(bits).map_err(eop)? as usize;
                let angle = r.read(bits).map_err(eop)? as usize;
                if magnitude == angle || magnitude >= channels || angle >= channels {
                    return Err(Error::Setup("channel coupling names an invalid channel pair"));
                }
                coupling.push((magnitude, angle));
            }
        }
        if r.read(2).map_err(eop)? != 0 {
            return Err(Error::Setup("reserved mapping bits are set"));
        }
        let mut mux = vec![0usize; channels];
        if submaps > 1 {
            for m in &mut mux {
                *m = r.read(4).map_err(eop)? as usize;
                if *m >= submaps {
                    return Err(Error::Setup("channel multiplexed to a submap that does not exist"));
                }
            }
        }
        let mut list = Vec::with_capacity(submaps);
        for _ in 0..submaps {
            r.read(8).map_err(eop)?;
            let floor = r.read(8).map_err(eop)? as usize;
            let residue = r.read(8).map_err(eop)? as usize;
            if floor >= floors.len() || residue >= residues.len() {
                return Err(Error::Setup("submap names a floor or residue that does not exist"));
            }
            list.push((floor, residue));
        }
        mappings.push(Mapping { coupling, mux, submaps: list });
    }
    let count = r.read(6).map_err(eop)? as usize + 1;
    let mut modes = Vec::with_capacity(count);
    for _ in 0..count {
        let long = r.bit().map_err(eop)?;
        let window = r.read(16).map_err(eop)?;
        let transform = r.read(16).map_err(eop)?;
        let mapping = r.read(8).map_err(eop)? as usize;
        if window != 0 || transform != 0 || mapping >= mappings.len() {
            return Err(Error::Setup("a mode has a nonzero window or transform type or an unknown mapping"));
        }
        modes.push(Mode { long, mapping });
    }
    if !r.bit().map_err(eop)? {
        return Err(Error::Setup("setup header framing bit is not set"));
    }
    Ok(Setup { books, floors, residues, mappings, modes })
}

/// The previous block, windowed, kept for overlap-add.
struct Previous {
    n: usize,
    data: Vec<Vec<f32>>,
}

/// A Vorbis I packet decoder.
pub struct Decoder {
    id: Identification,
    comments: Comments,
    setup: Setup,
    imdct: [Imdct; 2],
    slopes: [Vec<f32>; 2],
    previous: Option<Previous>,
}

impl Decoder {
    /// A decoder from the three header packets, in order. A damaged comment header is tolerated
    /// (its comments are then empty); the other two must be valid.
    pub fn new(identification: &[u8], comment: &[u8], setup: &[u8]) -> Result<Self> {
        let id = read_identification(identification)?;
        let comments = read_comments(comment)?;
        let setup = read_setup(setup, usize::from(id.channels))?;
        let (b0, b1) = (id.blocksize_0 as usize, id.blocksize_1 as usize);
        Ok(Self { id, comments, setup, imdct: [Imdct::new(b0), Imdct::new(b1)], slopes: [slope(b0 / 2), slope(b1 / 2)], previous: None })
    }

    /// The identification header.
    #[must_use]
    pub fn identification(&self) -> &Identification {
        &self.id
    }

    /// The comment header.
    #[must_use]
    pub fn comments(&self) -> &Comments {
        &self.comments
    }

    /// Forget the previous block, as after a seek: the next packet only primes the overlap again.
    pub fn reset(&mut self) {
        self.previous = None;
    }

    /// The window of a block (§4.3.1).
    fn window(&self, n: usize, long: bool, prev_long: bool, next_long: bool) -> Vec<f32> {
        let half = n / 2;
        let b0 = self.id.blocksize_0 as usize;
        let mut w = vec![0f32; n];
        // Left half.
        if long && !prev_long {
            let (start, end) = (n / 4 - b0 / 4, n / 4 + b0 / 4);
            w[start..end].copy_from_slice(&self.slopes[0]);
            w[end..half].fill(1.0);
        } else {
            w[..half].copy_from_slice(&self.slopes[usize::from(long)]);
        }
        // Right half.
        if long && !next_long {
            let start = 3 * n / 4 - b0 / 4;
            w[half..start].fill(1.0);
            for (i, s) in self.slopes[0].iter().rev().enumerate() {
                w[start + i] = *s;
            }
        } else {
            for (i, s) in self.slopes[usize::from(long)].iter().rev().enumerate() {
                w[half + i] = *s;
            }
        }
        w
    }

    /// Decode one audio packet. The result holds one `Vec` per channel, all of the same length:
    /// the samples this packet completes (none for the first packet after a start or reset).
    pub fn decode_packet(&mut self, packet: &[u8]) -> Result<Vec<Vec<f32>>> {
        let channels = usize::from(self.id.channels);
        let mut r = BitReader::new(packet);
        match r.bit() {
            Ok(false) => {}
            Ok(true) => return Err(Error::NotAudio),
            Err(_) => return Err(Error::Packet("empty packet")),
        }
        let mode_bits = ilog(self.setup.modes.len() as i64 - 1);
        let mode_number = r.read(mode_bits).map_err(|_| Error::Packet("packet ends inside the mode number"))? as usize;
        let Some(mode) = self.setup.modes.get(mode_number) else { return Err(Error::Packet("mode number out of range")) };
        let long = mode.long;
        let n = if long { self.id.blocksize_1 } else { self.id.blocksize_0 } as usize;
        let (mut prev_long, mut next_long) = (true, true);
        if long {
            prev_long = r.bit().map_err(|_| Error::Packet("packet ends inside the window flags"))?;
            next_long = r.bit().map_err(|_| Error::Packet("packet ends inside the window flags"))?;
        }
        let window = self.window(n, long, prev_long, next_long);
        let half = n / 2;
        let mapping = &self.setup.mappings[mode.mapping];

        let spectrum = self.spectrum(&mut r, mapping, half, channels)?;
        // Inverse transform and window.
        let which = usize::from(long);
        let mut blocks: Vec<Vec<f32>> = Vec::with_capacity(channels);
        for s in &spectrum {
            let mut y = vec![0f32; n];
            self.imdct[which].inverse(s, &mut y);
            for (v, w) in y.iter_mut().zip(&window) {
                *v *= *w;
            }
            blocks.push(y);
        }

        // Overlap-add with the previous block (§4.3.8).
        let out = match &self.previous {
            None => vec![Vec::new(); channels],
            Some(prev) => {
                let count = prev.n / 4 + n / 4;
                blocks
                    .iter()
                    .zip(&prev.data)
                    .map(|(cur, old)| {
                        (0..count)
                            .map(|k| {
                                let a = old.get(prev.n / 2 + k).copied().unwrap_or(0.0);
                                let b = (n / 4 + k).checked_sub(prev.n / 4).and_then(|i| cur.get(i)).copied().unwrap_or(0.0);
                                a + b
                            })
                            .collect()
                    })
                    .collect()
            }
        };
        self.previous = Some(Previous { n, data: blocks });
        Ok(out)
    }

    /// Floors, residues, coupling and the dot product: the spectrum of each channel (§4.3.2 to 4.3.6).
    fn spectrum(&self, r: &mut BitReader<'_>, mapping: &Mapping, half: usize, channels: usize) -> Result<Vec<Vec<f32>>> {
        let zero = || vec![vec![0f32; half]; channels];
        let setup = &self.setup;
        // Floors, in channel order.
        let mut floors: Vec<Option<FloorData>> = Vec::with_capacity(channels);
        for ch in 0..channels {
            let (floor, _) = mapping.submaps[mapping.mux[ch]];
            match setup.floors[floor].decode(r, &setup.books) {
                Ok(f) => floors.push(f),
                Err(Fail::Eop) => return Ok(zero()),
                Err(Fail::Invalid) => return Err(Error::Packet("a floor names a codebook that does not exist")),
            }
        }
        let mut no_residue: Vec<bool> = floors.iter().map(Option::is_none).collect();
        for &(m, a) in &mapping.coupling {
            if !no_residue[m] || !no_residue[a] {
                no_residue[m] = false;
                no_residue[a] = false;
            }
        }
        // Residues, in submap order.
        let mut residue = zero();
        for (i, &(_, res)) in mapping.submaps.iter().enumerate() {
            let members: Vec<usize> = (0..channels).filter(|&c| mapping.mux[c] == i).collect();
            if members.is_empty() {
                continue;
            }
            let skip: Vec<bool> = members.iter().map(|&c| no_residue[c]).collect();
            let mut vectors = vec![vec![0f32; half]; members.len()];
            setup.residues[res].decode(r, &setup.books, half, &skip, &mut vectors);
            for (v, &c) in vectors.into_iter().zip(&members) {
                residue[c] = v;
            }
        }
        // Inverse channel coupling (§4.3.5), last step first.
        for &(m, a) in mapping.coupling.iter().rev() {
            let mut mag = std::mem::take(&mut residue[m]);
            let mut ang = std::mem::take(&mut residue[a]);
            for (mv, av) in mag.iter_mut().zip(ang.iter_mut()) {
                let (big, small) = (*mv, *av);
                let (new_m, new_a);
                if big > 0.0 {
                    if small > 0.0 {
                        new_m = big;
                        new_a = big - small;
                    } else {
                        new_a = big;
                        new_m = big + small;
                    }
                } else if small > 0.0 {
                    new_m = big;
                    new_a = big + small;
                } else {
                    new_a = big;
                    new_m = big - small;
                }
                *mv = new_m;
                *av = new_a;
            }
            residue[m] = mag;
            residue[a] = ang;
        }
        // The floor times the residue.
        for ch in 0..channels {
            match &floors[ch] {
                Some(data) => {
                    let (floor, _) = mapping.submaps[mapping.mux[ch]];
                    let curve = setup.floors[floor].curve(data, half);
                    for (v, c) in residue[ch].iter_mut().zip(curve) {
                        *v *= c;
                    }
                }
                None => residue[ch].fill(0.0),
            }
        }
        Ok(residue)
    }
}

/// A decoded Ogg Vorbis file.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    /// Samples per second per channel.
    pub sample_rate: u32,
    /// One `Vec` of samples per channel.
    pub channels: Vec<Vec<f32>>,
    /// The stream's comment header.
    pub comments: Comments,
    /// Audio packets that could not be decoded and were skipped.
    pub damaged_packets: usize,
}

/// Decode the first Vorbis stream of an Ogg file. The result is trimmed to the stream's final
/// granule position, so an encoder's padding at the end is dropped. Damaged packets are skipped
/// and counted, not fatal. Collected PCM is limited to 256 MiB; use the packet
/// decoder for longer streams without retaining the entire output.
pub fn decode_ogg(data: &[u8]) -> Result<Decoded> {
    let ogg = craft_media::ogg::open(data).map_err(|e| Error::Ogg(e.to_string()))?;
    let index = ogg.stream_of(craft_media::ogg::Codec::Vorbis).ok_or_else(|| Error::Ogg("no Vorbis stream".into()))?;
    let stream = &ogg.streams[index];
    let [id, comment, setup, ..] = stream.headers.as_slice() else { return Err(Error::Ogg("the stream has fewer than three header packets".into())) };
    let mut decoder = Decoder::new(id, comment, setup)?;
    let channel_count = usize::from(decoder.id.channels);
    let mut channels = vec![Vec::new(); channel_count];
    let mut damaged = 0;
    for i in 0..stream.packets.len() {
        let packet = ogg.read_packet(data, index, i).map_err(|e| Error::Ogg(e.to_string()))?;
        match decoder.decode_packet(&packet) {
            Ok(pcm) => {
                append_pcm(&mut channels, pcm, 256 << 20)?;
            }
            Err(_) => damaged += 1,
        }
    }
    if let Some(total) = stream.last_granule.and_then(|g| usize::try_from(g).ok()) {
        for c in &mut channels {
            c.truncate(total);
        }
    }
    Ok(Decoded { sample_rate: decoder.id.sample_rate, channels, comments: decoder.comments.clone(), damaged_packets: damaged })
}

// Admit every channel together before extending any of them. Limit retained PCM,
// not the compressed input, setup tables or the decoder's bounded packet scratch.
fn append_pcm(channels: &mut [Vec<f32>], pcm: Vec<Vec<f32>>, budget: usize) -> Result<()> {
    let samples = channels.iter().chain(&pcm).try_fold(0usize, |n, c| n.checked_add(c.len()));
    if samples.is_none_or(|n| n > budget / std::mem::size_of::<f32>()) {
        return Err(Error::Limit("decoded PCM exceeds the whole-file budget"));
    }
    for (all, new) in channels.iter_mut().zip(pcm) {
        all.try_reserve_exact(new.len()).map_err(|_| Error::Limit("cannot allocate decoded PCM"))?;
        all.extend(new);
    }
    Ok(())
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn pcm_budget_counts_all_channels_before_extending() {
        let mut channels = vec![vec![1.0], vec![2.0]];
        append_pcm(&mut channels, vec![vec![3.0], vec![4.0]], 16).expect("exact budget");
        let before = channels.clone();
        assert!(matches!(append_pcm(&mut channels, vec![vec![5.0], vec![6.0]], 23), Err(Error::Limit(_))));
        assert_eq!(channels, before);
        append_pcm(&mut channels, vec![Vec::new(), Vec::new()], 16).expect("empty packet");
    }
}
