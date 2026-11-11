//! Syntax writer and bounded rate control; ETSI TS 102 366 §§4, 6, 7 and Annex E/F.
use super::super::{AllocParams, Globals, bit_allocation};
use super::{Codec, CodingMode, Config, Error};

#[derive(Default, Clone)]
struct Writer {
    bytes: Vec<u8>,
    bits: usize,
}
impl Writer {
    fn put(&mut self, value: u32, count: u32) {
        for bit in (0..count).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if value & (1 << bit) != 0 {
                self.bytes[self.bits / 8] |= 1 << (7 - self.bits % 8);
            }
            self.bits += 1;
        }
    }
    fn append(&mut self, other: &Self) {
        for bit in 0..other.bits {
            self.put(u32::from((other.bytes[bit / 8] >> (7 - bit % 8)) & 1), 1);
        }
    }
    fn flag(&mut self, value: bool) {
        self.put(u32::from(value), 1);
    }
}

fn rate_code(rate: u32) -> usize {
    match rate {
        48_000 => 0,
        44_100 => 1,
        _ => 2,
    }
}

pub(super) fn configuration(config: Config) -> Vec<u8> {
    let mut w = Writer::default();
    let acmod = if config.mode == CodingMode::Surround71 { 7 } else { config.mode as u32 };
    match config.codec {
        Codec::Ac3 => {
            w.put(rate_code(config.sample_rate) as u32, 2);
            w.put(8, 5);
            w.put(0, 3);
            w.put(acmod, 3);
            w.flag(config.lfe);
            w.put(super::super::BITRATES.iter().position(|&b| b == config.bitrate_kbps).unwrap_or(0) as u32, 5);
            w.put(0, 5);
        }
        Codec::Eac3 => {
            // Ceiling frame rate describes the maximum rate when 44.1 kHz needs alternating sizes.
            let words = (u64::from(config.bitrate_kbps) * 96_000).div_ceil(u64::from(config.sample_rate));
            let max_rate = (words * 16 * u64::from(config.sample_rate)).div_ceil(1_536_000);
            w.put(max_rate as u32, 13);
            w.put(0, 3); // one independent substream
            w.put(rate_code(config.sample_rate) as u32, 2);
            w.put(16, 5);
            w.put(0, 2);
            w.put(0, 3);
            w.put(acmod, 3);
            w.flag(config.lfe);
            w.put(0, 3);
            let dependent = config.mode == CodingMode::Surround71;
            w.put(u32::from(dependent), 4);
            if dependent {
                w.put(2, 9);
            } else {
                w.put(0, 1);
            }
        }
    }
    w.bytes
}

pub(super) fn encode(config: Config, frame_index: u64, spectra: &[Vec<[f32; 256]>], short: &[bool; 6]) -> Result<Vec<u8>, Error> {
    if config.mode == CodingMode::Surround71 {
        // Input WAVE order: L R C LFE Lrs Rrs Ls Rs. Independent stream is a
        // compatible 5.1 downmix; the dependent stream restores original surrounds.
        // Averaging (−6 dB per input) keeps correlated full-scale sides/rears
        // bounded. This intentionally is not Annex H dmixtyp=1's −3 dB mix;
        // that optional metadata is not emitted.
        let mut primary =
            vec![spectra[0].clone(), spectra[2].clone(), spectra[1].clone(), spectra[6].clone(), spectra[7].clone(), spectra[3].clone()];
        for block in 0..6 {
            for bin in 0..256 {
                primary[3][block][bin] = (spectra[6][block][bin] + spectra[4][block][bin]) * 0.5;
                primary[4][block][bin] = (spectra[7][block][bin] + spectra[5][block][bin]) * 0.5;
            }
        }
        let dependent = vec![spectra[6].clone(), spectra[7].clone(), spectra[4].clone(), spectra[5].clone()];
        let total_words = frame_words(config, frame_index);
        let primary_words = total_words * 3 / 5;
        let mut output = syncframe(config, 7, true, 0, primary_words, &primary, short)?;
        output.extend(syncframe(config, 6, false, 1, total_words - primary_words, &dependent, short)?);
        return Ok(output);
    }
    let nf = config.mode.channels();
    let fronts = match config.mode {
        CodingMode::Mono => 1,
        CodingMode::ThreeFront | CodingMode::ThreeFrontSurround | CodingMode::ThreeFrontTwoSurround => 3,
        _ => 2,
    };
    let mut order: Vec<usize> = (0..nf).map(|ch| if config.lfe && ch >= fronts { ch + 1 } else { ch }).collect();
    if fronts == 3 {
        order.swap(1, 2);
    }
    if config.lfe {
        order.push(fronts);
    }
    let coded: Vec<_> = order.iter().map(|&ch| spectra[ch].clone()).collect();
    syncframe(config, config.mode as u8, config.lfe, 0, frame_words(config, frame_index), &coded, short)
}

fn frame_words(config: Config, index: u64) -> usize {
    let numerator = u64::from(config.bitrate_kbps) * 96_000;
    let denominator = u64::from(config.sample_rate);
    // An accumulator without unbounded state drift or long-stream multiplication overflow.
    let floor = numerator / denominator;
    let remainder = numerator % denominator;
    let phase = (index % denominator) * remainder % denominator;
    (floor + u64::from(phase + remainder >= denominator)) as usize
}

struct Channel {
    coefficients: Vec<[f32; 256]>,
    exponents: [[u8; 256]; 6],
    codes: [Vec<u8>; 6],
    strategies: [u8; 6],
    end: usize,
}

fn channels(spectra: &[Vec<[f32; 256]>], nf: usize, bandwidth: usize, short: &[bool; 6]) -> Vec<Channel> {
    spectra
        .iter()
        .enumerate()
        .map(|(index, blocks)| {
            let end = if index == nf { 7 } else { 73 + 3 * bandwidth };
            let raw: Vec<[u8; 256]> = blocks
                .iter()
                .map(|block| {
                    std::array::from_fn(|bin| {
                        let peak = block[bin].abs();
                        if peak > 0.0 { (-peak.log2()).floor().clamp(0.0, 24.0) as u8 } else { 24 }
                    })
                })
                .collect();
            let mut starts = vec![0];
            let mut start = 0;
            for block in 1..6 {
                let difference: u32 = (0..end).map(|bin| u32::from(raw[block][bin].abs_diff(raw[start][bin]))).sum();
                if (index < nf && short[block] != short[block - 1]) || difference > end as u32 * 2 {
                    starts.push(block);
                    start = block;
                }
            }
            starts.push(6);
            let mut channel = Channel {
                coefficients: blocks.clone(),
                exponents: [[24; 256]; 6],
                codes: std::array::from_fn(|_| Vec::new()),
                strategies: [0; 6],
                end,
            };
            for range in starts.windows(2) {
                let (first, last) = (range[0], range[1]);
                let strategy = if index == nf || last - first >= 4 {
                    1
                } else if last - first >= 2 {
                    2
                } else {
                    3
                };
                let size = 1usize << (strategy - 1);
                let groups = (end - 1 + 3 * size - 3) / (3 * size);
                let mut envelope = [24u8; 256];
                for bin in 0..end {
                    envelope[bin] = (first..last).map(|block| raw[block][bin]).min().unwrap_or(24);
                }
                let mut codes = Vec::with_capacity(1 + groups * 3);
                codes.push(envelope[0].min(15));
                for group in 0..groups * 3 {
                    codes.push((0..size).map(|i| envelope[(1 + group * size + i).min(end - 1)]).min().unwrap_or(24));
                }
                for i in 1..codes.len() {
                    codes[i] = codes[i].min(codes[i - 1] + 2);
                }
                for i in (0..codes.len() - 1).rev() {
                    codes[i] = codes[i].min(codes[i + 1] + 2);
                }
                let reconstructed: [u8; 256] =
                    std::array::from_fn(|bin| if bin == 0 { codes[0] } else { codes.get(1 + (bin - 1) / size).copied().unwrap_or(24) });
                for block in first..last {
                    channel.exponents[block] = reconstructed;
                }
                channel.strategies[first] = strategy;
                channel.codes[first] = codes;
            }
            channel
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn syncframe(
    config: Config,
    acmod: u8,
    lfe: bool,
    stream_type: u32,
    words: usize,
    spectra: &[Vec<[f32; 256]>],
    short: &[bool; 6],
) -> Result<Vec<u8>, Error> {
    if !(64..=2048).contains(&words) && config.codec == Codec::Eac3 {
        return Err(Error::BitBudget);
    }
    let nf = super::super::NFCHANS[acmod as usize];
    let per_channel = config.bitrate_kbps / config.mode.channels() as u32;
    let cutoff = if per_channel < 48 {
        12_000
    } else if per_channel < 64 {
        16_000
    } else {
        20_000
    };
    let target_bin = (cutoff * 512 / config.sample_rate).min(253) as usize;
    let max_bw = target_bin.saturating_sub(73) / 3;
    for bandwidth in (0..=max_bw.min(60)).rev() {
        let channels = channels(spectra, nf, bandwidth, short);
        // Rate control uses exactly the reconstructed exponent envelope and the
        // decoder's standardized allocation math, not the preprocessed spectrum.
        let mut low = 0;
        let mut high = 1023;
        let mut best = None;
        while low <= high {
            let snr = (low + high) / 2;
            match serialize(config, acmod, lfe, stream_type, words, bandwidth, &channels, snr, short) {
                Some(frame) => {
                    best = Some(frame);
                    low = snr + 1;
                }
                None => {
                    if snr == 0 {
                        break;
                    }
                    high = snr - 1;
                }
            }
        }
        if let Some(frame) = best {
            return Ok(frame);
        }
    }
    Err(Error::BitBudget)
}

#[allow(clippy::too_many_arguments)]
fn serialize(
    config: Config,
    acmod: u8,
    lfe: bool,
    stream_type: u32,
    words: usize,
    bandwidth: usize,
    channels: &[Channel],
    snr: u32,
    short: &[bool; 6],
) -> Option<Vec<u8>> {
    let enhanced = config.codec == Codec::Eac3;
    let mut w = Writer::default();
    w.put(0x0b77, 16);
    if enhanced {
        eac3_header(&mut w, config, acmod, lfe, stream_type, words, channels, snr);
    } else {
        ac3_header(&mut w, config, acmod, lfe, words);
    }
    let globals = Globals { fscod: rate_code(config.sample_rate), sdcycod: 2, fdcycod: 1, sgaincod: 1, dbpbcod: 2, floorcod: 7 };
    let baps: Vec<Vec<[u8; 256]>> = (0..6)
        .map(|block| {
            channels
                .iter()
                .map(|channel| {
                    let mut bap = [0; 256];
                    if snr != 0 {
                        let params = AllocParams {
                            start: 0,
                            end: channel.end,
                            fgain: super::super::FASTGAIN[4],
                            snroffset: (snr as i32 - 240) * 4,
                            leak: None,
                            dba: None,
                        };
                        bit_allocation(&channel.exponents[block], &params, &globals, &mut bap);
                    }
                    bap
                })
                .collect()
        })
        .collect();
    let split = (words / 2 + words / 8) * 16;
    let mut headers: Vec<Writer> = Vec::with_capacity(6);
    let mut mantissas: Vec<Writer> = Vec::with_capacity(6);
    for block in 0..6 {
        let mut header = Writer::default();
        block_header(&mut header, enhanced, acmod, lfe, stream_type, bandwidth, channels, snr, block, short[block]);
        let mut mantissa = Writer::default();
        write_mantissas(&mut mantissa, channels, &baps[block], block);
        headers.push(header);
        mantissas.push(mantissa);
    }
    let skips = if enhanced { [None; 6] } else { plan_skips(w.bits, &headers, &mantissas, words)? };
    for block in 0..6 {
        w.append(&headers[block]);
        if !enhanced {
            w.flag(skips[block].is_some());
            if let Some(bytes) = skips[block] {
                w.put(bytes as u32, 9);
                for _ in 0..bytes {
                    w.put(0, 8);
                }
            }
            if block == 5 && w.bits < split {
                return None;
            }
        }
        w.append(&mantissas[block]);
        if !enhanced && block == 1 && w.bits > split {
            return None;
        }
        if w.bits + 18 > words * 16 {
            return None;
        }
    }
    w.bytes.resize(words * 2, 0); // auxiliary padding, auxdatae and crcrsv/encinfo are zero
    let size = w.bytes.len();
    if !enhanced {
        let split_bytes = split / 8;
        let residue = crc(&w.bytes[2..split_bytes]);
        let mut prefix = residue;
        for _ in 0..(split_bytes - 2) * 8 {
            prefix = reverse_zero_bit(prefix);
        }
        w.bytes[2..4].copy_from_slice(&prefix.to_be_bytes());
    }
    let mut tail = crc(&w.bytes[2..size - 2]);
    if !enhanced && tail == 0x0b77 {
        w.bytes[size - 3] ^= 1; // reserved bit prevents a false syncword in CRC2
        tail = crc(&w.bytes[2..size - 2]);
    }
    w.bytes[size - 2..].copy_from_slice(&tail.to_be_bytes());
    Some(w.bytes)
}

/// §4.5 limits the complete final region, including unused auxiliary padding.
/// Move excess padding into bounded skip fields before block 5's mantissas.
fn plan_skips(prefix: usize, headers: &[Writer], mantissas: &[Writer], words: usize) -> Option<[Option<usize>; 6]> {
    let total = words * 16;
    let split = (words / 2 + words / 8) * 16;
    let original = prefix + headers.iter().map(|h| h.bits + 1).sum::<usize>() + mantissas.iter().map(|m| m.bits).sum::<usize>() + 18;
    if original > total || mantissas[5].bits + 18 > total - split {
        return None;
    }
    let before_last = original - mantissas[5].bits - 18;
    let mut missing = split.saturating_sub(before_last);
    let mut skips = [None; 6];
    let mut added = 0;
    for block in (0..6).rev() {
        if missing == 0 {
            break;
        }
        let bytes = missing.saturating_sub(9).div_ceil(8).min(511);
        let bits = 9 + bytes * 8;
        skips[block] = Some(bytes);
        added += bits;
        missing = missing.saturating_sub(bits);
    }
    if missing != 0 || original + added > total {
        return None;
    }
    Some(skips)
}

#[allow(clippy::too_many_arguments)]
fn block_header(
    w: &mut Writer,
    enhanced: bool,
    acmod: u8,
    lfe: bool,
    stream_type: u32,
    bandwidth: usize,
    channels: &[Channel],
    snr: u32,
    block: usize,
    short: bool,
) {
    let nf = super::super::NFCHANS[acmod as usize];
    // Block switches are common to full-bandwidth channels; LFE always uses long blocks.
    for _ in 0..nf {
        w.flag(short);
    }
    for _ in 0..nf {
        w.flag(false);
    }
    for _ in 0..if acmod == 0 { 2 } else { 1 } {
        w.flag(false);
    } // dynrng
    if enhanced {
        if block == 0 {
            w.flag(false);
        }
    }
    // spxinu, later spxstre
    else {
        w.flag(block == 0);
        if block == 0 {
            w.flag(false);
        }
    }
    if enhanced && block > 0 {
        w.flag(false);
    } // spxstre
    if acmod == 2 {
        if !enhanced || block > 0 {
            w.flag(block == 0);
        }
        if block == 0 {
            w.put(0, 4);
        }
    }
    if !enhanced {
        for channel in &channels[..nf] {
            w.put(u32::from(channel.strategies[block]), 2);
        }
        if lfe {
            w.flag(channels[nf].strategies[block] != 0);
        }
    }
    for channel in &channels[..nf] {
        if channel.strategies[block] != 0 {
            w.put(bandwidth as u32, 6);
        }
    }
    for (ch, channel) in channels.iter().enumerate() {
        if channel.strategies[block] == 0 {
            continue;
        }
        let codes = &channel.codes[block];
        w.put(u32::from(codes[0]), 4);
        for first in (1..codes.len()).step_by(3) {
            let mut previous = i32::from(codes[first - 1]);
            let mut packed = 0;
            for &exponent in &codes[first..first + 3] {
                packed = packed * 5 + (i32::from(exponent) - previous + 2) as u32;
                previous = i32::from(exponent);
            }
            w.put(packed, 7);
        }
        if ch < nf {
            w.put(0, 2);
        }
    }
    if !enhanced {
        w.flag(block == 0);
        if block == 0 {
            w.put(2, 2);
            w.put(1, 2);
            w.put(1, 2);
            w.put(2, 2);
            w.put(7, 3);
        }
        w.flag(block == 0);
        if block == 0 {
            w.put(snr / 16, 6);
            for _ in channels {
                w.put(snr % 16, 4);
                w.put(4, 3);
            }
        }
        w.flag(false); // delta allocation
    } else if stream_type == 0 {
        w.flag(false);
    } // convsnroffste
}

fn ac3_header(w: &mut Writer, config: Config, acmod: u8, lfe: bool, words: usize) {
    w.put(0, 16); // CRC1, patched after allocation
    w.put(rate_code(config.sample_rate) as u32, 2);
    let index = super::super::BITRATES.iter().position(|&b| b == config.bitrate_kbps).unwrap_or(0);
    let base = (u64::from(config.bitrate_kbps) * 96_000 / u64::from(config.sample_rate)) as usize;
    w.put((index * 2 + usize::from(words > base)) as u32, 6);
    w.put(8, 5);
    w.put(0, 3);
    w.put(u32::from(acmod), 3);
    if acmod & 1 != 0 && acmod != 1 {
        w.put(1, 2);
    }
    if acmod & 4 != 0 {
        w.put(1, 2);
    }
    if acmod == 2 {
        w.put(0, 2);
    }
    w.flag(lfe);
    for _ in 0..if acmod == 0 { 2 } else { 1 } {
        w.put(31, 5);
        w.put(0, 3);
    }
    w.put(0, 1);
    w.put(1, 1);
    w.put(0, 3);
}

#[allow(clippy::too_many_arguments)]
fn eac3_header(w: &mut Writer, config: Config, acmod: u8, lfe: bool, stream_type: u32, words: usize, channels: &[Channel], snr: u32) {
    let nf = super::super::NFCHANS[acmod as usize];
    w.put(stream_type, 2);
    w.put(0, 3);
    w.put((words - 1) as u32, 11);
    w.put(rate_code(config.sample_rate) as u32, 2);
    w.put(3, 2);
    w.put(u32::from(acmod), 3);
    w.flag(lfe);
    w.put(16, 5);
    w.put(31, 5);
    w.flag(stream_type == 1);
    if stream_type == 1 {
        w.put(0, 8);
    }
    if acmod == 0 {
        w.put(31, 5);
        w.flag(false);
    }
    if stream_type == 1 {
        w.flag(true);
        w.put(0x1a00, 16);
    }
    w.flag(false);
    w.flag(false);
    w.flag(false); // mixmdate, infomdate, addbsie
    w.flag(true);
    w.flag(false); // per-block exponents, AHT disabled
    w.put(0, 2); // frame-level SNR
    w.flag(false);
    w.flag(true);
    w.flag(true); // transproce, blkswe, dithflage
    w.put(0, 5); // bamode, frmfgaincode, dbaflde, skipflde, spxattene
    if acmod > 1 {
        w.flag(false);
        for _ in 1..6 {
            w.flag(false);
        }
    }
    for block in 0..6 {
        for channel in &channels[..nf] {
            w.put(u32::from(channel.strategies[block]), 2);
        }
    }
    if lfe {
        for block in 0..6 {
            w.flag(channels[nf].strategies[block] != 0);
        }
    }
    if stream_type == 0 {
        for _ in 0..nf {
            w.put(0, 5);
        }
    } // converter exponents
    w.put(snr / 16, 6);
    w.put(snr % 16, 4);
    w.flag(false); // block-start information omitted
}

fn write_mantissas(w: &mut Writer, channels: &[Channel], baps: &[[u8; 256]], block: usize) {
    let values: Vec<(u8, f32)> = channels
        .iter()
        .zip(baps)
        .flat_map(|(channel, bap)| {
            (0..channel.end).map(move |bin| (bap[bin], channel.coefficients[block][bin] * 2f32.powi(i32::from(channel.exponents[block][bin]))))
        })
        .collect();
    let mut grouped = vec![None; values.len()];
    for (bap, levels, group_size, bits) in [(1, 3, 3, 5), (2, 5, 3, 7), (4, 11, 2, 7)] {
        let indexes: Vec<_> = values.iter().enumerate().filter_map(|(i, &(b, _))| (b == bap).then_some(i)).collect();
        for group in indexes.chunks(group_size) {
            let mut code = 0;
            for slot in 0..group_size {
                let value = group.get(slot).map_or(0.0, |&i| values[i].1);
                code = code * levels + quantize_symmetric(value, levels);
            }
            grouped[group[0]] = Some((code, bits));
        }
    }
    for (index, (bap, value)) in values.into_iter().enumerate() {
        if let Some((code, bits)) = grouped[index] {
            w.put(code, bits);
            continue;
        }
        match bap {
            0 | 1 | 2 | 4 => {}
            3 => w.put(quantize_symmetric(value, 7), 3),
            5 => w.put(quantize_symmetric(value, 15), 4),
            _ => {
                let bits = super::super::QNTZTAB[usize::from(bap)];
                let scale = 1i32 << (bits - 1);
                let code = (value * scale as f32).round().clamp(-scale as f32, (scale - 1) as f32) as i32;
                w.put(code as u32 & ((1 << bits) - 1), bits);
            }
        }
    }
}

fn quantize_symmetric(value: f32, levels: u32) -> u32 {
    ((value * levels as f32 + (levels - 1) as f32) * 0.5).round().clamp(0.0, (levels - 1) as f32) as u32
}

pub(super) fn crc(bytes: &[u8]) -> u16 {
    let mut value = 0u16;
    for &byte in bytes {
        value ^= u16::from(byte) << 8;
        for _ in 0..8 {
            value = if value & 0x8000 != 0 { (value << 1) ^ 0x8005 } else { value << 1 };
        }
    }
    value
}
fn reverse_zero_bit(value: u16) -> u16 {
    let high = value & 1;
    ((value ^ if high != 0 { 0x8005 } else { 0 }) >> 1) | (high << 15)
}
