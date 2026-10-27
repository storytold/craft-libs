//! Native WAV / BWF / RF64 writer (RIFF chunk layout per the Microsoft multimedia
//! specification, `bext` per EBU Tech 3285, `ds64` per EBU Tech 3306).

use super::pcm::{Quantizer, sample_format, validate_buffer, validate_layout, write_interleaved};
use crate::wav::{GUID_TAIL, WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IEEE_FLOAT, WAVE_FORMAT_PCM};
use crate::{AudioBuffer, AudioError, BitDepth, BwfInfo, Result};
use std::io::{Cursor, Seek, SeekFrom, Write};

const BEXT_MIN_LEN: usize = 602;

/// Default speaker mask for a channel count (callers that know the layout pass their own):
/// mono, stereo, LCR, quad, 5.0, 5.1, 6.1, 7.1 (sides + rears), 7.1.4.
fn channel_mask(channels: usize) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x33,
        5 => 0x37,
        6 => 0x3F,
        7 => 0x13F,
        8 => 0x63F,
        // L R C LFE Lrs Rrs Lss Rss Ltf Rtf Ltr Rtr (7.1.4).
        12 => 0x2_D63F,
        n if n <= 18 => (1u32 << n) - 1,
        _ => 0,
    }
}

fn put_fixed_str(out: &mut Vec<u8>, s: &str, len: usize) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(len);
    out.extend_from_slice(bytes.get(..n).unwrap_or_default());
    out.resize(out.len() + (len - n), 0);
}

fn bext_chunk(bwf: &BwfInfo) -> Vec<u8> {
    let mut c = Vec::with_capacity(BEXT_MIN_LEN);
    put_fixed_str(&mut c, &bwf.description, 256);
    put_fixed_str(&mut c, &bwf.originator, 32);
    put_fixed_str(&mut c, &bwf.originator_reference, 32);
    put_fixed_str(&mut c, &bwf.origination_date, 10);
    put_fixed_str(&mut c, &bwf.origination_time, 8);
    c.extend_from_slice(&((bwf.time_reference & 0xFFFF_FFFF) as u32).to_le_bytes());
    c.extend_from_slice(&((bwf.time_reference >> 32) as u32).to_le_bytes());
    c.extend_from_slice(&1u16.to_le_bytes()); // version 1
    c.resize(BEXT_MIN_LEN, 0); // UMID, loudness, reserved; no coding history
    c
}

fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

/// A WAV written block by block: a placeholder header first, then the real one (the same length,
/// RF64 above 4 GiB, or always when `force_rf64`) written over it by [`Writer::finish`].
pub(crate) struct Writer<W> {
    out: W,
    start: u64,
    channels: usize,
    sample_rate: u32,
    format: (u16, bool),
    bwf: Option<BwfInfo>,
    mask: Option<u32>,
    force_rf64: bool,
    quantizers: Vec<Quantizer>,
    frames: u64,
    bytes: Vec<u8>,
    /// Set when a block write has failed: the sink may hold a prefix of that block while
    /// `frames` does not count it, so the writer refuses every later call.
    failed: bool,
}

/// The optional header parts of a WAV file: BWF metadata, speaker mask, and whether to force RF64.
#[derive(Clone, Copy)]
pub(crate) struct HeaderExtras<'a> {
    pub(crate) bwf: Option<&'a BwfInfo>,
    pub(crate) mask: Option<u32>,
    pub(crate) force_rf64: bool,
}

impl<W: Write + Seek> Writer<W> {
    pub(crate) fn new(mut out: W, channels: usize, sample_rate: u32, depth: BitDepth, dither: bool, extras: HeaderExtras<'_>) -> Result<Self> {
        let HeaderExtras { bwf, mask, force_rf64 } = extras;
        validate_layout(channels, sample_rate)?;
        let format = sample_format(depth);
        let start = out.stream_position()?;
        out.write_all(&header(channels, sample_rate, format, bwf, mask, 0, force_rf64)?)?;
        let quantizers = Quantizer::per_channel(u32::from(format.0), dither, channels);
        Ok(Writer {
            out,
            start,
            channels,
            sample_rate,
            format,
            bwf: bwf.cloned(),
            mask,
            force_rf64,
            quantizers,
            frames: 0,
            bytes: Vec::new(),
            failed: false,
        })
    }

    fn header(&self, frames: u64) -> Result<Vec<u8>> {
        header(self.channels, self.sample_rate, self.format, self.bwf.as_ref(), self.mask, frames, self.force_rf64)
    }

    /// The frame count after `frames` more (RF64 has room for any bounce).
    pub(crate) fn check_room(&self, frames: u64) -> Result<u64> {
        let total = self.frames.saturating_add(frames);
        self.header(total)?;
        Ok(total)
    }

    /// Appends `frames` frames of planar audio (missing samples are silence).
    pub(crate) fn write(&mut self, block: &[Vec<f32>], frames: usize) -> Result<()> {
        if self.failed {
            return Err(super::failed_write_error());
        }
        let total = self.check_room(frames as u64)?;
        if let Err(e) = write_interleaved(&mut self.out, &mut self.bytes, block, frames, &mut self.quantizers, self.format, false) {
            self.failed = true;
            return Err(AudioError::from(e));
        }
        self.frames = total;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<W> {
        if self.failed {
            return Err(super::failed_write_error());
        }
        let data_len = u64::from(self.format.0 / 8).saturating_mul(self.channels as u64).saturating_mul(self.frames);
        if data_len % 2 == 1 {
            self.out.write_all(&[0])?;
        }
        let head = self.header(self.frames)?;
        self.out.seek(SeekFrom::Start(self.start))?;
        self.out.write_all(&head)?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Encode a WAV file. `force_rf64` writes the RF64 layout even for small files (for tests);
/// it is used automatically when the file would exceed 4 GiB.
/// `mask` overrides the speaker mask derived from the channel count.
pub(crate) fn encode(
    buf: &AudioBuffer,
    depth: BitDepth,
    dither: bool,
    bwf: Option<&BwfInfo>,
    force_rf64: bool,
    mask: Option<u32>,
) -> Result<Vec<u8>> {
    validate_buffer(buf)?;
    let channels = buf.channels.len();
    let frames = buf.frames();
    let format = sample_format(depth);
    let head = header(channels, buf.sample_rate, format, bwf, mask, frames as u64, force_rf64)?;
    let data_len = (usize::from(format.0 / 8) * channels).checked_mul(frames).ok_or_else(|| AudioError::TooLarge("data size overflow".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(head.len().saturating_add(data_len).saturating_add(1))
        .map_err(|_| AudioError::TooLarge(format!("cannot allocate {data_len} bytes for WAV output")))?;
    let mut w = Writer::new(Cursor::new(out), channels, buf.sample_rate, depth, dither, HeaderExtras { bwf, mask, force_rf64 })?;
    w.write(&buf.channels, frames)?;
    Ok(w.finish()?.into_inner())
}

/// Everything before the sample data of a WAV holding `frames` frames.
pub(crate) fn header(
    channels: usize,
    sample_rate: u32,
    (bits, is_float): (u16, bool),
    bwf: Option<&BwfInfo>,
    mask: Option<u32>,
    frames: u64,
    force_rf64: bool,
) -> Result<Vec<u8>> {
    let block = usize::from(bits / 8).checked_mul(channels).ok_or_else(|| AudioError::TooLarge("block size overflow".into()))?;
    let block_align = u16::try_from(block).map_err(|_| AudioError::Encode("too many channels for WAV".into()))?;
    let data_len = u64::from(block_align).checked_mul(frames).ok_or_else(|| AudioError::TooLarge("data size overflow".into()))?;
    let extensible = channels > 2 || bits > 16 || mask.is_some_and(|m| m != channel_mask(channels));
    let fmt_tag = if is_float { WAVE_FORMAT_IEEE_FLOAT } else { WAVE_FORMAT_PCM };

    let mut fmt = Vec::with_capacity(40);
    fmt.extend_from_slice(&(if extensible { WAVE_FORMAT_EXTENSIBLE } else { fmt_tag }).to_le_bytes());
    fmt.extend_from_slice(&(channels as u16).to_le_bytes());
    fmt.extend_from_slice(&sample_rate.to_le_bytes());
    fmt.extend_from_slice(&sample_rate.saturating_mul(u32::from(block_align)).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    if extensible {
        fmt.extend_from_slice(&22u16.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        fmt.extend_from_slice(&mask.unwrap_or_else(|| channel_mask(channels)).to_le_bytes());
        fmt.extend_from_slice(&fmt_tag.to_le_bytes());
        fmt.extend_from_slice(&GUID_TAIL);
    }

    let mut head = Vec::with_capacity(1024);
    let rf64_body_len = 28usize;
    // Reserve room for ds64 (as JUNK when not needed, so a later writer could upgrade in place).
    let mut meta = Vec::new();
    if let Some(bwf) = bwf {
        push_chunk(&mut meta, b"bext", &bext_chunk(bwf));
    }
    push_chunk(&mut meta, b"fmt ", &fmt);
    if is_float {
        push_chunk(&mut meta, b"fact", &u32::try_from(frames).unwrap_or(u32::MAX).to_le_bytes());
    }
    let total = ((4 + (8 + rf64_body_len) + meta.len() + 8) as u64)
        .checked_add(data_len)
        .and_then(|t| t.checked_add(data_len & 1))
        .ok_or_else(|| AudioError::TooLarge("WAV size overflow".into()))?;
    let rf64 = force_rf64 || u32::try_from(total).is_err();

    if rf64 {
        head.extend_from_slice(b"RF64");
        head.extend_from_slice(&u32::MAX.to_le_bytes());
        head.extend_from_slice(b"WAVE");
        let mut ds64 = Vec::with_capacity(rf64_body_len);
        ds64.extend_from_slice(&total.to_le_bytes());
        ds64.extend_from_slice(&data_len.to_le_bytes());
        ds64.extend_from_slice(&frames.to_le_bytes());
        ds64.extend_from_slice(&0u32.to_le_bytes());
        push_chunk(&mut head, b"ds64", &ds64);
    } else {
        head.extend_from_slice(b"RIFF");
        head.extend_from_slice(&(total as u32).to_le_bytes());
        head.extend_from_slice(b"WAVE");
        push_chunk(&mut head, b"JUNK", &[0u8; 28]);
    }
    head.extend_from_slice(&meta);
    head.extend_from_slice(b"data");
    head.extend_from_slice(&(if rf64 { u32::MAX } else { data_len as u32 }).to_le_bytes());
    Ok(head)
}
