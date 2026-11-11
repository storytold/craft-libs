//! AAC-LC decoder: raw access units (`raw_data_block`, ISO/IEC 14496-3 §4.4.2.1) → planar f32.

use craft_media::bitstream::BitReader;

use crate::aac::config::{AudioSpecificConfig, ElementType, ProgramConfig};
use crate::aac::huffman::{self, INTENSITY_HCB, INTENSITY_HCB2, NOISE_HCB, ZERO_HCB};
use crate::aac::ics::{IcsInfo, WindowSequence};
use crate::aac::mdct::{MdctScratch, WindowShape, mdct_long, mdct_short, window};
use crate::aac::tables::{pow43, pow43_table, sf_gain};
use crate::aac::tns::{self, TnsData};
use crate::aac::{Error, Result};

const ID_SCE: u32 = 0;
const ID_CPE: u32 = 1;
const ID_CCE: u32 = 2;
const ID_LFE: u32 = 3;
const ID_DSE: u32 = 4;
const ID_PCE: u32 = 5;
const ID_FIL: u32 = 6;
#[cfg(test)]
const ID_END: u32 = 7;

/// `extension_type` of a fill element carrying SBR data (ISO/IEC 14496-3 §4.5.2.8.2, Table 4.121).
const EXT_SBR_DATA: u32 = 13;
const EXT_SBR_DATA_CRC: u32 = 14;

/// The noise generator's state at the start of a stream and after [`Decoder::reset`].
const NOISE_SEED: u32 = 0x2545_F491;

/// One decoded `individual_channel_stream` before stereo processing and synthesis.
struct Ics {
    info: IcsInfo,
    /// Band type per (group, sfb).
    band_type: [[u8; 64]; 8],
    /// Scalefactor / noise energy / intensity position per (group, sfb).
    sf: [[i32; 64]; 8],
    tns: Option<TnsData>,
    /// Dequantised spectrum (short windows at stride 128).
    spec: Vec<f32>,
}

#[derive(Clone)]
struct ChannelState {
    overlap: Vec<f32>,
    prev_shape: WindowShape,
}

/// Buffers reused by every frame so the steady state does not allocate per channel.
#[derive(Clone)]
struct Scratch {
    /// Quantised values of the channel being parsed.
    q: Vec<i32>,
    /// Windowed output of the current block (2048) and the raw inverse transform (2048 / 256).
    buf: Vec<f32>,
    y: Vec<f32>,
    long: MdctScratch,
    short: MdctScratch,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch { q: vec![0; 1024], buf: vec![0.0; 2048], y: vec![0.0; 2048], long: mdct_long().scratch(), short: mdct_short().scratch() }
    }
}

/// AAC-LC decoder.
#[derive(Clone)]
pub struct Decoder {
    asc: AudioSpecificConfig,
    layout: Vec<(ElementType, u8)>,
    /// First output channel of each layout entry.
    offsets: Vec<usize>,
    channels: Vec<ChannelState>,
    noise_state: u32,
    scratch: Scratch,
    /// A fill element carried SBR data (implicitly signalled HE-AAC).
    sbr_seen: bool,
}

impl Decoder {
    /// Create a decoder from an `AudioSpecificConfig`.
    pub fn new(asc: &[u8]) -> Result<Decoder> {
        Self::from_config(AudioSpecificConfig::parse(asc)?)
    }

    /// Create a decoder for an ADTS stream from its first header. With channel configuration 0 the
    /// channel layout is taken from the program config element carried in the first frame that
    /// decodes with it.
    pub fn from_adts(header: &crate::aac::config::AdtsHeader) -> Result<Decoder> {
        Self::from_config(header.to_asc())
    }

    /// Create a decoder from a parsed `AudioSpecificConfig`. Only audio object type 2 (AAC LC) is accepted.
    pub fn from_config(asc: AudioSpecificConfig) -> Result<Decoder> {
        if asc.object_type != 2 {
            return Err(Error::Unsupported("audio object type other than AAC LC"));
        }
        if usize::from(asc.sf_index) >= crate::aac::SAMPLE_RATES.len() || asc.sample_rate == 0 {
            return Err(Error::InvalidConfig("a sampling frequency index with no rate"));
        }
        let layout = asc.layout();
        if layout.is_empty() && asc.channel_config != 0 {
            return Err(Error::InvalidConfig("unsupported channel configuration"));
        }
        let mut d = Decoder {
            asc,
            layout: Vec::new(),
            offsets: Vec::new(),
            channels: Vec::new(),
            noise_state: NOISE_SEED,
            scratch: Scratch::new(),
            sbr_seen: false,
        };
        d.set_layout(layout);
        Ok(d)
    }

    fn set_layout(&mut self, layout: Vec<(ElementType, u8)>) {
        let mut offsets = Vec::with_capacity(layout.len());
        let mut n = 0;
        for e in &layout {
            offsets.push(n);
            n += e.0.channels();
        }
        self.layout = layout;
        self.offsets = offsets;
        self.channels = vec![ChannelState { overlap: vec![0.0; 1024], prev_shape: WindowShape::Sine }; n];
    }

    /// The `AudioSpecificConfig` the decoder was created with.
    pub fn config(&self) -> &AudioSpecificConfig {
        &self.asc
    }
    /// Output sampling rate in Hz: the AAC-LC core rate.
    pub fn sample_rate(&self) -> u32 {
        self.asc.sample_rate
    }
    /// Whether the stream is HE-AAC: SBR signalled in the `AudioSpecificConfig` (explicit) or SBR
    /// data seen in a fill element of a decoded access unit (implicit). Only the AAC-LC core is
    /// decoded, at [`Self::sample_rate`]; the SBR high band is not reconstructed.
    pub fn sbr(&self) -> bool {
        self.asc.sbr || self.sbr_seen
    }
    /// Output channels of the current element layout.
    pub fn channels(&self) -> usize {
        self.channels.len()
    }
    /// Element layout in output channel order (C, L/R, surround pairs, LFE for configurations 1–7).
    pub fn layout(&self) -> &[(ElementType, u8)] {
        &self.layout
    }

    /// Clear the overlap and noise state (e.g. after a seek), so a seek to the same place always
    /// gives the same samples.
    pub fn reset(&mut self) {
        self.noise_state = NOISE_SEED;
        for c in &mut self.channels {
            c.overlap.fill(0.0);
            c.prev_shape = WindowShape::Sine;
        }
    }

    fn sf_index(&self) -> u8 {
        self.asc.sf_index
    }

    /// Decode one access unit (one `raw_data_block`) into 1024 samples per channel. An access unit
    /// that fails leaves the decoder as it was, but for [`Self::sbr`]: its damage, including a damaged
    /// program config element, does not reach the access units after it.
    pub fn decode(&mut self, au: &[u8]) -> Result<Vec<Vec<f32>>> {
        let before = (self.asc.pce.clone(), self.layout.clone(), self.offsets.clone(), self.channels.clone(), self.noise_state);
        let decoded = self.decode_block(au, true);
        if decoded.is_err() {
            (self.asc.pce, self.layout, self.offsets, self.channels, self.noise_state) = before;
        }
        decoded
    }

    fn decode_block(&mut self, au: &[u8], allow_reconfig: bool) -> Result<Vec<Vec<f32>>> {
        let mut br = BitReader::new(au);
        let nch = self.channels.len();
        let mut saw_element = false;
        let mut out: Vec<Option<Vec<f32>>> = vec![None; nch];
        let mut used = vec![false; self.layout.len()];
        loop {
            let id = br.read_bits(3)?;
            saw_element |= matches!(id, ID_SCE | ID_CPE | ID_LFE);
            match id {
                ID_SCE | ID_LFE => {
                    let tag = br.read_bits(4)? as u8;
                    let ty = if id == ID_SCE { ElementType::Sce } else { ElementType::Lfe };
                    let slot = self.find_slot(ty, tag, &used)?;
                    used[slot] = true;
                    let mut ics = self.decode_ics(&mut br, None)?;
                    let ch = self.offsets[slot];
                    self.fill_noise(&mut ics, None);
                    out[ch] = Some(self.finish_channel(ch, &mut ics));
                }
                ID_CPE => {
                    let tag = br.read_bits(4)? as u8;
                    let slot = self.find_slot(ElementType::Cpe, tag, &used)?;
                    used[slot] = true;
                    let ch = self.offsets[slot];
                    let [a, b] = self.decode_cpe(&mut br, ch)?;
                    out[ch] = Some(a);
                    out[ch + 1] = Some(b);
                }
                ID_CCE => return Err(Error::Unsupported("coupling channel element")),
                ID_DSE => {
                    br.skip(4)?;
                    let align = br.read_bits(1)? == 1;
                    let mut count = br.read_bits(8)? as usize;
                    if count == 255 {
                        count += br.read_bits(8)? as usize;
                    }
                    if align {
                        br.byte_align();
                    }
                    br.skip(count * 8)?;
                }
                ID_PCE => {
                    let pce = ProgramConfig::parse(&mut br)?;
                    if allow_reconfig && !saw_element && self.asc.channel_config == 0 && self.asc.pce.as_ref() != Some(&pce) {
                        let layout = pce.layout();
                        if !layout.is_empty() && layout.iter().map(|e| e.0.channels()).sum::<usize>() <= 64 {
                            self.asc.pce = Some(pce);
                            self.set_layout(layout);
                            return self.decode_block(au, false);
                        }
                    }
                }
                ID_FIL => {
                    let mut count = br.read_bits(4)? as usize;
                    if count == 15 {
                        count += br.read_bits(8)? as usize;
                        count -= 1;
                    }
                    if count > 0 {
                        let ext = br.read_bits(4)?;
                        self.sbr_seen |= matches!(ext, EXT_SBR_DATA | EXT_SBR_DATA_CRC);
                        br.skip(count * 8 - 4)?;
                    }
                }
                _ => break, // ID_END
            }
        }
        if nch == 0 && !saw_element {
            return Err(Error::InvalidConfig("channel configuration 0 without a program config element"));
        }
        let mut res = Vec::with_capacity(nch);
        for o in out {
            res.push(o.ok_or(Error::Bitstream("access unit is missing a channel element"))?);
        }
        Ok(res)
    }

    fn find_slot(&self, ty: ElementType, tag: u8, used: &[bool]) -> Result<usize> {
        let free = |want: &dyn Fn(ElementType, u8) -> bool| self.layout.iter().enumerate().position(|(i, &(t, g))| want(t, g) && !used[i]);
        let slot = if self.asc.channel_config == 0 {
            // A program config names its elements' tags.
            free(&|t, g| t == ty && g == tag)
        } else {
            // An indexed configuration does not: its elements are placed by the order they arrive
            // in, whatever their tags. Some encoders code an LFE as an SCE, as ffmpeg allows.
            free(&|t, _| t == ty).or_else(|| free(&|t, _| ty == ElementType::Sce && t == ElementType::Lfe))
        };
        slot.ok_or(Error::Bitstream("unexpected channel element for the channel configuration"))
    }

    fn decode_cpe(&mut self, br: &mut BitReader, ch: usize) -> Result<[Vec<f32>; 2]> {
        let common_window = br.read_bits(1)? == 1;
        let mut ms_mask_present = 0;
        let mut ms_used = [[false; 64]; 8];
        let mut info = None;
        if common_window {
            let i = IcsInfo::parse(br, self.sf_index())?;
            ms_mask_present = br.read_bits(2)?;
            if ms_mask_present == 3 {
                return Err(Error::Bitstream("reserved ms_mask_present"));
            }
            if ms_mask_present == 1 {
                for g in 0..i.num_groups {
                    for sfb in 0..i.max_sfb {
                        ms_used[g][sfb] = br.read_bits(1)? == 1;
                    }
                }
            } else if ms_mask_present == 2 {
                ms_used = [[true; 64]; 8];
            }
            info = Some(i);
        }
        let mut l = self.decode_ics(br, info)?;
        let mut r = self.decode_ics(br, info)?;
        let ms = if ms_mask_present > 0 { Some(&ms_used) } else { None };
        self.fill_noise(&mut l, None);
        self.fill_noise(&mut r, if common_window { Some((&l, ms)) } else { None });
        if common_window {
            let swb = l.info.swb_offsets(self.sf_index());
            for g in 0..l.info.num_groups {
                let w0 = l.info.group_start(g);
                for sfb in 0..l.info.max_sfb {
                    let (bl, br_) = (l.band_type[g][sfb], r.band_type[g][sfb]);
                    let range = swb[sfb] as usize..swb[sfb + 1] as usize;
                    if br_ == INTENSITY_HCB || br_ == INTENSITY_HCB2 {
                        let mut c: f32 = if br_ == INTENSITY_HCB { 1.0 } else { -1.0 };
                        if ms_mask_present > 0 && ms_used[g][sfb] {
                            c = -c;
                        }
                        let scale = c * (-0.25 * r.sf[g][sfb].clamp(-400, 400) as f64).exp2() as f32;
                        for w in w0..w0 + l.info.group_len[g] as usize {
                            for k in range.clone() {
                                r.spec[w * 128 + k] = l.spec[w * 128 + k] * scale;
                            }
                        }
                    } else if ms_mask_present > 0 && ms_used[g][sfb] && bl < NOISE_HCB && br_ < NOISE_HCB {
                        for w in w0..w0 + l.info.group_len[g] as usize {
                            for k in range.clone() {
                                let (m, s) = (l.spec[w * 128 + k], r.spec[w * 128 + k]);
                                l.spec[w * 128 + k] = m + s;
                                r.spec[w * 128 + k] = m - s;
                            }
                        }
                    }
                }
            }
        }
        Ok([self.finish_channel(ch, &mut l), self.finish_channel(ch + 1, &mut r)])
    }

    fn decode_ics(&mut self, br: &mut BitReader, common: Option<IcsInfo>) -> Result<Ics> {
        let sfi = self.sf_index();
        let global_gain = br.read_bits(8)? as i32;
        let info = match common {
            Some(i) => i,
            None => IcsInfo::parse(br, sfi)?,
        };
        let swb = info.swb_offsets(sfi);
        let num_swb = swb.len() - 1;
        let mut ics = Ics { info, band_type: [[0; 64]; 8], sf: [[0; 64]; 8], tns: None, spec: vec![0.0; 1024] };

        // section_data
        let (sect_bits, esc) = if info.is_short() { (3, 7) } else { (5, 31) };
        for g in 0..info.num_groups {
            let mut k = 0;
            while k < info.max_sfb {
                let cb = br.read_bits(4)? as u8;
                if cb == 12 {
                    return Err(Error::Bitstream("reserved codebook 12"));
                }
                let mut len = 0usize;
                loop {
                    let inc = br.read_bits(sect_bits)?;
                    len += inc as usize;
                    if inc != esc {
                        break;
                    }
                }
                if k + len > info.max_sfb {
                    return Err(Error::Bitstream("section exceeds max_sfb"));
                }
                if len == 0 {
                    // zero-length sections are legal but useless; guard against endless loops
                    if br.bits_left() == 0 {
                        return Err(Error::Eof);
                    }
                }
                for sfb in k..k + len {
                    ics.band_type[g][sfb] = cb;
                }
                k += len;
            }
        }

        // scale_factor_data
        let mut sf = global_gain;
        let mut noise = global_gain - 90;
        let mut is_pos = 0i32;
        let mut noise_first = true;
        for g in 0..info.num_groups {
            for sfb in 0..info.max_sfb {
                match ics.band_type[g][sfb] {
                    ZERO_HCB => ics.sf[g][sfb] = 0,
                    INTENSITY_HCB | INTENSITY_HCB2 => {
                        is_pos += huffman::decode_sf(br)?;
                        ics.sf[g][sfb] = is_pos;
                    }
                    NOISE_HCB => {
                        if noise_first {
                            noise += br.read_bits(9)? as i32 - 256;
                            noise_first = false;
                        } else {
                            noise += huffman::decode_sf(br)?;
                        }
                        ics.sf[g][sfb] = noise;
                    }
                    _ => {
                        sf += huffman::decode_sf(br)?;
                        if !(0..=255).contains(&sf) {
                            return Err(Error::Bitstream("scalefactor out of range"));
                        }
                        ics.sf[g][sfb] = sf;
                    }
                }
            }
        }

        // pulse_data
        let mut pulses: Vec<(usize, i32)> = Vec::new();
        if br.read_bits(1)? == 1 {
            if info.is_short() {
                return Err(Error::Bitstream("pulse data in short window"));
            }
            let n = br.read_bits(2)? as usize + 1;
            let start = br.read_bits(6)? as usize;
            if start >= num_swb {
                return Err(Error::Bitstream("pulse_start_sfb out of range"));
            }
            let mut k = swb[start] as usize;
            for _ in 0..n {
                k += br.read_bits(5)? as usize;
                let amp = br.read_bits(4)? as i32;
                if k >= 1024 {
                    return Err(Error::Bitstream("pulse position out of range"));
                }
                pulses.push((k, amp));
            }
        }
        if br.read_bits(1)? == 1 {
            ics.tns = Some(TnsData::parse(br, &info)?);
        }
        if br.read_bits(1)? == 1 {
            return Err(Error::Unsupported("gain control data (AAC SSR)"));
        }

        // spectral_data
        let mut q = std::mem::take(&mut self.scratch.q);
        q.clear();
        q.resize(1024, 0);
        let mut vals = [0i32; 4];
        for g in 0..info.num_groups {
            let w0 = info.group_start(g);
            let glen = info.group_len[g] as usize;
            for sfb in 0..info.max_sfb {
                let cb = ics.band_type[g][sfb];
                if cb == ZERO_HCB || cb >= NOISE_HCB {
                    continue;
                }
                let dim = huffman::book(cb).dim;
                let (lo, hi) = (swb[sfb] as usize, swb[sfb + 1] as usize);
                for w in w0..w0 + glen {
                    let base = w * 128;
                    let mut k = lo;
                    while k < hi {
                        huffman::decode_spectral(br, cb, &mut vals)?;
                        q[base + k..base + k + dim].copy_from_slice(&vals[..dim]);
                        k += dim;
                    }
                }
            }
        }
        for (k, amp) in pulses {
            if q[k] > 0 {
                q[k] += amp;
            } else {
                q[k] -= amp;
            }
        }

        // dequantise
        let t43 = pow43_table();
        for g in 0..info.num_groups {
            let w0 = info.group_start(g);
            for sfb in 0..info.max_sfb {
                let cb = ics.band_type[g][sfb];
                if cb == ZERO_HCB || cb >= NOISE_HCB {
                    continue;
                }
                let gain = sf_gain(ics.sf[g][sfb]);
                for w in w0..w0 + info.group_len[g] as usize {
                    let r = w * 128 + swb[sfb] as usize..w * 128 + swb[sfb + 1] as usize;
                    for (o, &v) in ics.spec[r.clone()].iter_mut().zip(&q[r]) {
                        let a = v.unsigned_abs();
                        let m = t43.get(a as usize).copied().unwrap_or_else(|| pow43(a)) * gain;
                        *o = if v < 0 { -m } else { m };
                    }
                }
            }
        }
        self.scratch.q = q;
        Ok(ics)
    }

    fn next_random(&mut self) -> f32 {
        self.noise_state = self.noise_state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.noise_state as i32) as f32
    }

    /// Perceptual noise substitution. `pair` = (left channel, ms mask) for the right channel of a CPE:
    /// bands that are noise in both channels with `ms_used` reuse the left channel's noise vector.
    fn fill_noise(&mut self, ics: &mut Ics, pair: Option<(&Ics, Option<&[[bool; 64]; 8]>)>) {
        let info = ics.info;
        let swb = info.swb_offsets(self.sf_index());
        for g in 0..info.num_groups {
            let w0 = info.group_start(g);
            for sfb in 0..info.max_sfb {
                if ics.band_type[g][sfb] != NOISE_HCB {
                    continue;
                }
                let (lo, hi) = (swb[sfb] as usize, swb[sfb + 1] as usize);
                let target = (0.25 * ics.sf[g][sfb].clamp(-400, 400) as f64).exp2();
                let correlated = match pair {
                    Some((l, Some(ms))) => ms[g][sfb] && l.band_type[g][sfb] == NOISE_HCB,
                    _ => false,
                };
                for w in w0..w0 + info.group_len[g] as usize {
                    let band = w * 128 + lo..w * 128 + hi;
                    if correlated && let Some((l, _)) = pair {
                        ics.spec[band.clone()].copy_from_slice(&l.spec[band.clone()]);
                    } else {
                        for k in band.clone() {
                            ics.spec[k] = self.next_random();
                        }
                    }
                    let energy: f64 = ics.spec[band.clone()].iter().map(|&v| v as f64 * v as f64).sum();
                    let scale = if energy > 0.0 { (target / energy.sqrt()) as f32 } else { 0.0 };
                    for k in band {
                        ics.spec[k] *= scale;
                    }
                }
            }
        }
    }

    /// TNS + filterbank for a single channel element; returns output samples.
    fn finish_channel(&mut self, ch: usize, ics: &mut Ics) -> Vec<f32> {
        if let Some(t) = &ics.tns {
            tns::apply_decoder(&mut ics.spec, &ics.info, self.sf_index(), t);
        }
        synth(&mut self.channels[ch], &mut self.scratch, &ics.info, &ics.spec)
    }
}

/// Inverse MDCT, windowing and overlap-add (§4.6.18), output scaled to ±1.0 full scale.
fn synth(state: &mut ChannelState, sc: &mut Scratch, info: &IcsInfo, spec: &[f32]) -> Vec<f32> {
    let Scratch { buf, y, long, short, .. } = sc;
    buf.fill(0.0);
    let prev = state.prev_shape;
    let cur = info.window_shape;
    let norm = 1.0 / 32768.0;
    match info.window_sequence {
        WindowSequence::EightShort => {
            let t = mdct_short();
            let y = &mut y[..256];
            let wc = window(cur, false);
            for w in 0..8 {
                t.inverse_with(&spec[w * 128..(w + 1) * 128], y, short);
                let wl = window(if w == 0 { prev } else { cur }, false);
                let off = 448 + 128 * w;
                let s = norm / 128.0;
                for n in 0..128 {
                    buf[off + n] += y[n] * wl[n] * s;
                    buf[off + 128 + n] += y[128 + n] * wc[127 - n] * s;
                }
            }
        }
        seq => {
            mdct_long().inverse_with(&spec[..1024], y, long);
            let s = norm / 1024.0;
            match seq {
                WindowSequence::LongStop => {
                    let ws = window(prev, false);
                    for n in 448..576 {
                        buf[n] = y[n] * ws[n - 448] * s;
                    }
                    for n in 576..1024 {
                        buf[n] = y[n] * s;
                    }
                }
                _ => {
                    let wl = window(prev, true);
                    for n in 0..1024 {
                        buf[n] = y[n] * wl[n] * s;
                    }
                }
            }
            match seq {
                WindowSequence::LongStart => {
                    let ws = window(cur, false);
                    for n in 1024..1472 {
                        buf[n] = y[n] * s;
                    }
                    for n in 1472..1600 {
                        buf[n] = y[n] * ws[127 - (n - 1472)] * s;
                    }
                }
                _ => {
                    let wl = window(cur, true);
                    for n in 1024..2048 {
                        buf[n] = y[n] * wl[2047 - n] * s;
                    }
                }
            }
        }
    }
    // A hostile stream can overflow f32 (PNS at full energy through a resonant TNS filter); that must
    // not reach the output or the next frame's overlap.
    for s in buf.iter_mut().filter(|s| !s.is_finite()) {
        *s = 0.0;
    }
    let out: Vec<f32> = (0..1024).map(|n| state.overlap[n] + buf[n]).collect();
    state.overlap.copy_from_slice(&buf[1024..]);
    state.prev_shape = cur;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_media::bitstream::BitWriter;

    /// Implicitly signalled HE-AAC (an AAC-LC config) is recognised from the SBR fill element.
    #[test]
    fn sbr_fill_element_marks_implicit_he_aac() {
        // AAC-LC, 22.05 kHz, stereo
        let mut dec = Decoder::new(&[0x13, 0x90]).unwrap();
        assert!(!dec.sbr());
        // ID_FIL, count 1, extension_type EXT_SBR_DATA (+ 4 bits of payload), ID_END
        let _ = dec.decode(&[0b1100_0011, 0b1010_0001, 0b1100_0000]);
        assert!(dec.sbr());
        // other fill payloads (EXT_FILL) don't
        let mut dec = Decoder::new(&[0x13, 0x90]).unwrap();
        let _ = dec.decode(&[0b1100_0010, 0b0000_0001, 0b1100_0000]);
        assert!(!dec.sbr());
        // explicit signalling: AOT 5, core 22.05 kHz, stereo, extension 44.1 kHz, core AOT 2
        let dec = Decoder::new(&[0x2B, 0x92, 0x08, 0x00]).unwrap();
        assert!(dec.sbr());
        assert_eq!(dec.sample_rate(), 22_050);
    }

    /// An access unit whose elements are all empty: `elements` are (id, tag), then ID_END.
    fn silent_au(pce: Option<&ProgramConfig>, elements: &[(u32, u32)]) -> Vec<u8> {
        let mut bw = BitWriter::new();
        if let Some(pce) = pce {
            bw.write_bits(ID_PCE, 3);
            pce.write(&mut bw).unwrap();
        }
        for &(id, tag) in elements {
            bw.write_bits(id, 3);
            bw.write_bits(tag, 4);
            if id == ID_CPE {
                bw.write_bits(0, 1); // common_window
            }
            for _ in 0..if id == ID_CPE { 2 } else { 1 } {
                bw.write_bits(0, 8); // global_gain
                bw.write_bits(0, 11); // ics_info: long window, max_sfb 0, no predictor
                bw.write_bits(0, 3); // no pulse, TNS or gain control data
            }
        }
        bw.write_bits(ID_END, 3);
        bw.finish()
    }

    /// Indexed configurations place elements by arrival order, whatever their tags (as ffmpeg does).
    #[test]
    fn indexed_configurations_ignore_element_tags() {
        let dec = Decoder::from_config(AudioSpecificConfig::lc(4, 5)).unwrap();
        // SCE, CPE, CPE: the first CPE to arrive is L/R even when tagged 1
        assert_eq!(dec.find_slot(ElementType::Cpe, 1, &[true, false, false]), Ok(1));
        assert_eq!(dec.find_slot(ElementType::Cpe, 0, &[true, true, false]), Ok(2));
    }

    #[test]
    fn program_configs_place_elements_by_tag() {
        let pce = ProgramConfig { front: vec![(true, 3), (true, 5)], ..Default::default() };
        let asc = AudioSpecificConfig { pce: Some(pce), ..AudioSpecificConfig::lc(4, 0) };
        let dec = Decoder::from_config(asc).unwrap();
        assert_eq!(dec.find_slot(ElementType::Cpe, 5, &[false, false]), Ok(1));
    }

    /// A program config lists its elements: one it does not list has no place.
    #[test]
    fn program_configs_refuse_unlisted_tags() {
        let pce = ProgramConfig { front: vec![(true, 3), (true, 5)], ..Default::default() };
        let asc = AudioSpecificConfig { pce: Some(pce), ..AudioSpecificConfig::lc(4, 0) };
        let dec = Decoder::from_config(asc).unwrap();
        assert!(dec.find_slot(ElementType::Cpe, 9, &[false, false]).is_err());
        assert!(dec.find_slot(ElementType::Sce, 3, &[false, false]).is_err());
    }

    /// Some encoders code 5.1's LFE as an SCE, as ffmpeg allows. Nothing else changes type: an LFE
    /// is not written to the centre channel.
    #[test]
    fn indexed_configurations_refuse_elements_of_another_type() {
        let dec = Decoder::from_config(AudioSpecificConfig::lc(4, 6)).unwrap();
        // SCE, CPE, CPE, LFE
        assert_eq!(dec.find_slot(ElementType::Sce, 1, &[true, false, false, false]), Ok(3));
        assert!(dec.find_slot(ElementType::Lfe, 1, &[false, false, false, true]).is_err());
        assert!(dec.find_slot(ElementType::Cpe, 2, &[false, true, true, false]).is_err());
    }

    /// A PCE in an access unit that then fails leaves the layout as it was.
    #[test]
    fn a_damaged_in_band_pce_does_not_change_the_layout() {
        let two = ProgramConfig { front: vec![(true, 0)], ..Default::default() };
        let six = ProgramConfig { front: vec![(false, 0), (true, 0)], back: vec![(true, 1)], lfe: vec![0], ..Default::default() };
        let mut dec = Decoder::from_config(AudioSpecificConfig { pce: Some(two.clone()), ..AudioSpecificConfig::lc(4, 0) }).unwrap();
        let good = silent_au(None, &[(ID_CPE, 0)]);
        assert_eq!(dec.decode(&good).unwrap().len(), 2);
        // the new layout's elements are missing from this access unit
        assert!(dec.decode(&silent_au(Some(&six), &[(ID_CPE, 0)])).is_err());
        assert_eq!((dec.channels(), dec.config().pce.as_ref()), (2, Some(&two)));
        assert_eq!(dec.decode(&good).unwrap().len(), 2);
        // one that decodes does change it
        let full = silent_au(Some(&six), &[(ID_SCE, 0), (ID_CPE, 0), (ID_CPE, 1), (ID_LFE, 0)]);
        assert_eq!(dec.decode(&full).unwrap().len(), 6);
        assert_eq!(dec.channels(), 6);
    }

    /// A failed access unit leaves no trace: the next one decodes as if it had not been there.
    #[test]
    fn a_failed_access_unit_leaves_the_state_as_it_was() {
        let mut dec = Decoder::from_config(AudioSpecificConfig::lc(4, 1)).unwrap();
        dec.channels[0].overlap.fill(0.5);
        dec.channels[0].prev_shape = WindowShape::Kbd;
        let noise = dec.noise_state;
        // a complete channel element, then the access unit ends without ID_END
        let au = silent_au(None, &[(ID_SCE, 0)]);
        assert!(dec.decode(&au[..au.len() - 1]).is_err());
        assert!(dec.channels[0].overlap.iter().all(|&s| s == 0.5));
        assert_eq!((dec.channels[0].prev_shape, dec.noise_state), (WindowShape::Kbd, noise));
    }

    #[test]
    fn a_reset_restarts_the_noise_generator() {
        let mut dec = Decoder::from_config(AudioSpecificConfig::lc(4, 1)).unwrap();
        let first = dec.next_random();
        dec.next_random();
        dec.reset();
        assert_eq!(dec.next_random(), first);
    }

    #[test]
    fn non_finite_spectra_do_not_reach_the_output_or_the_overlap() {
        let mut state = ChannelState { overlap: vec![0.0; 1024], prev_shape: WindowShape::Sine };
        let info = IcsInfo::parse(&mut BitReader::new(&[0; 2]), 4).unwrap();
        let out = synth(&mut state, &mut Scratch::new(), &info, &vec![f32::INFINITY; 1024]);
        assert!(out.iter().chain(&state.overlap).all(|s| s.is_finite()));
    }
}
