//! `AudioSpecificConfig` (ISO/IEC 14496-3 §1.6.2.1), `program_config_element` (§4.4.1.1) and ADTS
//! framing (§1.A.2.2).

use craft_media::bitstream::{BitReader, BitWriter};

use crate::aac::tables::{SAMPLE_RATES, table_index_for_rate};
use crate::aac::{Error, Result};

/// Syntactic element types (`id_syn_ele`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementType {
    /// single_channel_element
    Sce,
    /// channel_pair_element
    Cpe,
    /// lfe_channel_element
    Lfe,
}

impl ElementType {
    /// Channels the element carries: 2 for a CPE, 1 otherwise.
    pub fn channels(self) -> usize {
        if self == ElementType::Cpe { 2 } else { 1 }
    }
}

/// A `program_config_element` (only the parts that affect channel mapping are interpreted).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProgramConfig {
    /// `element_instance_tag` (4 bits).
    pub element_instance_tag: u8,
    /// `object_type` (2 bits): the profile, audio object type minus 1.
    pub object_type: u8,
    /// `sampling_frequency_index` (4 bits).
    pub sf_index: u8,
    /// (is_cpe, tag) for each front channel element.
    pub front: Vec<(bool, u8)>,
    /// (is_cpe, tag) for side channel elements.
    pub side: Vec<(bool, u8)>,
    /// (is_cpe, tag) for back channel elements.
    pub back: Vec<(bool, u8)>,
    /// Tags of the LFE channel elements.
    pub lfe: Vec<u8>,
    /// Tags of the associated data elements.
    pub assoc_data: Vec<u8>,
    /// (is_ind_sw, tag)
    pub cc: Vec<(bool, u8)>,
    /// Comment field bytes (at most 255 are written).
    pub comment: Vec<u8>,
}

impl ProgramConfig {
    // Public for FilmCraft's re-export only; it takes craft-media's bit reader.
    #[doc(hidden)]
    pub fn parse(br: &mut BitReader) -> Result<ProgramConfig> {
        let mut p = ProgramConfig {
            element_instance_tag: br.read_bits(4)? as u8,
            object_type: br.read_bits(2)? as u8,
            sf_index: br.read_bits(4)? as u8,
            ..Default::default()
        };
        let nf = br.read_bits(4)?;
        let ns = br.read_bits(4)?;
        let nb = br.read_bits(4)?;
        let nl = br.read_bits(2)?;
        let na = br.read_bits(3)?;
        let nc = br.read_bits(4)?;
        if br.read_bits(1)? == 1 {
            br.skip(4)?;
        }
        if br.read_bits(1)? == 1 {
            br.skip(4)?;
        }
        if br.read_bits(1)? == 1 {
            br.skip(3)?;
        }
        for (n, v) in [(nf, &mut p.front), (ns, &mut p.side), (nb, &mut p.back)] {
            for _ in 0..n {
                let cpe = br.read_bits(1)? == 1;
                v.push((cpe, br.read_bits(4)? as u8));
            }
        }
        for _ in 0..nl {
            p.lfe.push(br.read_bits(4)? as u8);
        }
        for _ in 0..na {
            p.assoc_data.push(br.read_bits(4)? as u8);
        }
        for _ in 0..nc {
            let ind = br.read_bits(1)? == 1;
            p.cc.push((ind, br.read_bits(4)? as u8));
        }
        br.byte_align();
        let len = br.read_bits(8)?;
        for _ in 0..len {
            p.comment.push(br.read_bits(8)? as u8);
        }
        Ok(p)
    }

    /// Serialise the element (no mixdown signalling; comment truncated to 255 bytes). More elements
    /// than its count fields hold are refused, with nothing written.
    pub fn write(&self, bw: &mut BitWriter) -> Result<()> {
        if [self.front.len(), self.side.len(), self.back.len(), self.cc.len()].iter().any(|&n| n > 15)
            || self.lfe.len() > 3
            || self.assoc_data.len() > 7
        {
            return Err(Error::InvalidConfig("more elements than a program config element can list"));
        }
        bw.write_bits(self.element_instance_tag as u32, 4);
        bw.write_bits(self.object_type as u32, 2);
        bw.write_bits(self.sf_index as u32, 4);
        bw.write_bits(self.front.len() as u32, 4);
        bw.write_bits(self.side.len() as u32, 4);
        bw.write_bits(self.back.len() as u32, 4);
        bw.write_bits(self.lfe.len() as u32, 2);
        bw.write_bits(self.assoc_data.len() as u32, 3);
        bw.write_bits(self.cc.len() as u32, 4);
        bw.write_bits(0, 3); // no mixdowns
        for v in [&self.front, &self.side, &self.back] {
            for &(cpe, tag) in v {
                bw.write_bits(cpe as u32, 1);
                bw.write_bits(tag as u32, 4);
            }
        }
        for &t in &self.lfe {
            bw.write_bits(t as u32, 4);
        }
        for &t in &self.assoc_data {
            bw.write_bits(t as u32, 4);
        }
        for &(ind, t) in &self.cc {
            bw.write_bits(ind as u32, 1);
            bw.write_bits(t as u32, 4);
        }
        bw.align_zero();
        bw.write_bits(self.comment.len().min(255) as u32, 8);
        for &b in self.comment.iter().take(255) {
            bw.write_bits(b as u32, 8);
        }
        Ok(())
    }

    /// Elements in output channel order (front, side, back, LFE).
    pub fn layout(&self) -> Vec<(ElementType, u8)> {
        let mut out = Vec::new();
        for v in [&self.front, &self.side, &self.back] {
            for &(cpe, tag) in v {
                out.push((if cpe { ElementType::Cpe } else { ElementType::Sce }, tag));
            }
        }
        for &t in &self.lfe {
            out.push((ElementType::Lfe, t));
        }
        out
    }
}

/// Element layout of `channelConfiguration` 1..=7, 11 and 12 (tags as conventionally assigned; 11 and
/// 12 from ISO/IEC 14496-3 Amd 4: 6.1 with a back centre, 7.1 with back surrounds).
// Public for FilmCraft's re-export only.
#[doc(hidden)]
pub fn config_layout(channel_config: u8) -> Option<Vec<(ElementType, u8)>> {
    use ElementType::*;
    Some(match channel_config {
        1 => vec![(Sce, 0)],
        2 => vec![(Cpe, 0)],
        3 => vec![(Sce, 0), (Cpe, 0)],
        4 => vec![(Sce, 0), (Cpe, 0), (Sce, 1)],
        5 => vec![(Sce, 0), (Cpe, 0), (Cpe, 1)],
        6 => vec![(Sce, 0), (Cpe, 0), (Cpe, 1), (Lfe, 0)],
        7 | 12 => vec![(Sce, 0), (Cpe, 0), (Cpe, 1), (Cpe, 2), (Lfe, 0)],
        11 => vec![(Sce, 0), (Cpe, 0), (Cpe, 1), (Sce, 1), (Lfe, 0)],
        _ => return None,
    })
}

/// Parsed `AudioSpecificConfig`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSpecificConfig {
    /// Audio object type of the core (2 = AAC LC).
    pub object_type: u8,
    /// Core sampling rate.
    pub sample_rate: u32,
    /// Index into the scalefactor-band tables (the signalled index, or derived from an explicit rate).
    pub sf_index: u8,
    /// `channelConfiguration` (0 = channel layout from the program config element).
    pub channel_config: u8,
    /// `frameLengthFlag`: 960-sample frames when set (refused by [`Self::parse`]).
    pub frame_length_flag: bool,
    /// Explicitly signalled SBR/PS (HE-AAC v1/v2); only the AAC-LC core is decoded.
    pub sbr: bool,
    /// Explicitly signalled PS (audio object type 29, HE-AAC v2).
    pub ps: bool,
    /// Output (SBR) rate when `sbr` is set.
    pub extension_sample_rate: Option<u32>,
    /// The program config element, present when `channel_config` is 0.
    pub pce: Option<ProgramConfig>,
}

fn read_aot(br: &mut BitReader) -> Result<u8> {
    let a = br.read_bits(5)? as u8;
    Ok(if a == 31 { 32 + br.read_bits(6)? as u8 } else { a })
}

fn read_rate(br: &mut BitReader) -> Result<(u8, u32)> {
    let i = br.read_bits(4)? as u8;
    if i == 15 {
        let r = br.read_bits(24)?;
        if r == 0 {
            return Err(Error::InvalidConfig("sampling rate of 0"));
        }
        return Ok((table_index_for_rate(r), r));
    }
    let r = *SAMPLE_RATES.get(i as usize).ok_or(Error::InvalidConfig("reserved sampling frequency index"))?;
    Ok((i, r))
}

/// Backward-compatible explicit SBR signalling (ISO/IEC 14496-3 §1.6.6.2): after an AAC-LC
/// config, a sync extension `0x2b7` with extension object type 5 names SBR and its output rate,
/// and `0x548` after it names PS. Returns `(sbr, extension rate, ps)`, or `None` when the bits
/// that follow are not that extension (they are then ignored, as decoders that don't know it do).
fn sync_extension(br: &mut BitReader) -> Option<(bool, Option<u32>, bool)> {
    if br.bits_left() < 16 || br.read_bits(11).ok()? != 0x2b7 || read_aot(br).ok()? != 5 {
        return None;
    }
    if br.read_bits(1).ok()? != 1 {
        return Some((false, None, false));
    }
    let (_, rate) = read_rate(br).ok()?;
    let ps = br.bits_left() >= 12 && br.read_bits(11).ok()? == 0x548 && br.read_bits(1).ok()? == 1;
    Some((true, Some(rate), ps))
}

impl AudioSpecificConfig {
    /// A plain AAC-LC configuration. A sampling frequency index with no rate (13 and above) gives a
    /// sample rate of 0, which [`crate::aac::Decoder`] refuses.
    pub fn lc(sf_index: u8, channel_config: u8) -> AudioSpecificConfig {
        AudioSpecificConfig {
            object_type: 2,
            sample_rate: SAMPLE_RATES.get(usize::from(sf_index)).copied().unwrap_or(0),
            sf_index,
            channel_config,
            frame_length_flag: false,
            sbr: false,
            ps: false,
            extension_sample_rate: None,
            pce: None,
        }
    }

    /// Parse an `AudioSpecificConfig` (§1.6.2.1). Only AAC-LC cores are accepted; explicit SBR/PS
    /// signalling is recorded, both hierarchical (audio object types 5 and 29) and backward
    /// compatible (a `0x2b7` sync extension after the config).
    pub fn parse(data: &[u8]) -> Result<AudioSpecificConfig> {
        let mut br = BitReader::new(data);
        let mut aot = read_aot(&mut br)?;
        let (sf_index, sample_rate) = read_rate(&mut br)?;
        let channel_config = br.read_bits(4)? as u8;
        let (mut sbr, mut ps, mut extension_sample_rate) = (false, false, None);
        if aot == 5 || aot == 29 {
            sbr = true;
            ps = aot == 29;
            let (_, ext) = read_rate(&mut br)?;
            extension_sample_rate = Some(ext);
            aot = read_aot(&mut br)?;
        }
        if aot != 2 {
            return Err(Error::Unsupported(match aot {
                1 => "AAC Main profile",
                3 => "AAC SSR profile",
                4 => "AAC LTP profile",
                23 => "AAC-LD",
                39 => "AAC-ELD",
                _ => "audio object type other than AAC LC",
            }));
        }
        let frame_length_flag = br.read_bits(1)? == 1;
        if br.read_bits(1)? == 1 {
            br.skip(14)?; // coreCoderDelay
        }
        let extension_flag = br.read_bits(1)? == 1;
        let pce = if channel_config == 0 { Some(ProgramConfig::parse(&mut br)?) } else { None };
        // GASpecificConfig ends with extensionFlag3 when extensionFlag is set (AAC-LC has no other
        // extension fields); what follows it is the sync extension.
        if extension_flag && br.bits_left() > 0 {
            br.skip(1)?;
        }
        if frame_length_flag {
            return Err(Error::Unsupported("960-sample frames"));
        }
        if !sbr && let Some((s, rate, p)) = sync_extension(&mut br) {
            (sbr, extension_sample_rate, ps) = (s, rate, p);
        }
        let cfg =
            AudioSpecificConfig { object_type: aot, sample_rate, sf_index, channel_config, frame_length_flag, sbr, ps, extension_sample_rate, pce };
        if cfg.channels() == 0 {
            return Err(Error::InvalidConfig("no channels"));
        }
        Ok(cfg)
    }

    /// Serialise (AAC-LC, no SBR signalling). Channel configuration 0 needs its program config
    /// element, and the sampling rate must not be 0: otherwise the bytes would not parse back.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if self.channel_config == 0 && self.pce.is_none() {
            return Err(Error::InvalidConfig("channel configuration 0 without a program config element"));
        }
        if self.sample_rate == 0 {
            return Err(Error::InvalidConfig("a sampling rate of 0"));
        }
        let mut bw = BitWriter::new();
        bw.write_bits(self.object_type as u32, 5);
        match crate::aac::tables::sample_rate_index(self.sample_rate) {
            Some(i) => bw.write_bits(i as u32, 4),
            None => {
                bw.write_bits(15, 4);
                bw.write_bits(self.sample_rate, 24);
            }
        }
        bw.write_bits(self.channel_config as u32, 4);
        bw.write_bits(0, 3); // frameLengthFlag, dependsOnCoreCoder, extensionFlag
        if self.channel_config == 0
            && let Some(p) = &self.pce
        {
            p.write(&mut bw)?;
        }
        Ok(bw.finish())
    }

    /// Elements in output channel order.
    pub fn layout(&self) -> Vec<(ElementType, u8)> {
        match &self.pce {
            Some(p) if self.channel_config == 0 => p.layout(),
            _ => config_layout(self.channel_config).unwrap_or_default(),
        }
    }

    /// Total output channels of [`Self::layout`].
    pub fn channels(&self) -> usize {
        self.layout().iter().map(|e| e.0.channels()).sum()
    }
}

/// A parsed ADTS header.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdtsHeader {
    /// Audio object type (`profile + 1`).
    pub object_type: u8,
    /// `sampling_frequency_index` (4 bits).
    pub sf_index: u8,
    /// `channel_configuration` (3 bits; 0 = program config element in the stream).
    pub channel_config: u8,
    /// Whole frame length including the header.
    pub frame_length: usize,
    /// `adts_buffer_fullness` (11 bits; 0x7FF signals variable bitrate).
    pub buffer_fullness: u16,
    /// Header length (7, or 9 with CRC).
    pub header_length: usize,
    /// Raw data blocks in the frame (`number_of_raw_data_blocks_in_frame` + 1, so 1..=4).
    pub raw_data_blocks: u8,
}

impl AdtsHeader {
    /// Parse the ADTS fixed and variable header at the start of `data` (§1.A.2.2).
    pub fn parse(data: &[u8]) -> Result<AdtsHeader> {
        let mut br = BitReader::new(data);
        if br.read_bits(12)? != 0xFFF {
            return Err(Error::Bitstream("missing ADTS syncword"));
        }
        br.skip(1)?; // ID
        if br.read_bits(2)? != 0 {
            return Err(Error::Bitstream("ADTS layer must be 0"));
        }
        let protection_absent = br.read_bits(1)? == 1;
        let object_type = br.read_bits(2)? as u8 + 1;
        let sf_index = br.read_bits(4)? as u8;
        if sf_index as usize >= SAMPLE_RATES.len() {
            return Err(Error::Bitstream("invalid ADTS sampling frequency index"));
        }
        br.skip(1)?;
        let channel_config = br.read_bits(3)? as u8;
        br.skip(4)?;
        let frame_length = br.read_bits(13)? as usize;
        let buffer_fullness = br.read_bits(11)? as u16;
        let raw_data_blocks = br.read_bits(2)? as u8 + 1;
        let header_length = if protection_absent { 7 } else { 9 };
        if frame_length < header_length {
            return Err(Error::Bitstream("ADTS frame shorter than its header"));
        }
        Ok(AdtsHeader { object_type, sf_index, channel_config, frame_length, buffer_fullness, header_length, raw_data_blocks })
    }

    /// The equivalent `AudioSpecificConfig`. With channel configuration 0 it has no layout: the
    /// decoder takes that from the program config element in the stream.
    pub fn to_asc(&self) -> AudioSpecificConfig {
        AudioSpecificConfig { object_type: self.object_type, ..AudioSpecificConfig::lc(self.sf_index, self.channel_config) }
    }

    /// Write a 7-byte header (no CRC) for a frame whose raw payload is `payload_len` bytes. What the
    /// header's fields cannot hold is refused: a frame too long for its 13-bit length, an object
    /// type outside its 2-bit profile (1–4), a sampling frequency index with no rate, and channel
    /// configurations above 7 (11 and 12 need a program config element in ADTS).
    pub fn write(object_type: u8, sf_index: u8, channel_config: u8, payload_len: usize, buffer_fullness: u16) -> Result<[u8; 7]> {
        if payload_len > 0x1FFF - 7 {
            return Err(Error::InvalidConfig("an ADTS frame longer than its 13-bit length"));
        }
        if !(1..=4).contains(&object_type) {
            return Err(Error::InvalidConfig("an ADTS profile holds audio object types 1 to 4"));
        }
        if usize::from(sf_index) >= SAMPLE_RATES.len() {
            return Err(Error::InvalidConfig("a sampling frequency index with no rate"));
        }
        if channel_config > 7 {
            return Err(Error::InvalidConfig("an ADTS channel configuration above 7"));
        }
        let len = (payload_len + 7) as u32;
        let mut bw = BitWriter::new();
        bw.write_bits(0xFFF, 12);
        bw.write_bits(0, 1); // MPEG-4
        bw.write_bits(0, 2);
        bw.write_bits(1, 1); // protection absent
        bw.write_bits(u32::from(object_type - 1), 2);
        bw.write_bits(sf_index as u32, 4);
        bw.write_bits(0, 1);
        bw.write_bits(u32::from(channel_config), 3);
        bw.write_bits(0, 4);
        bw.write_bits(len & 0x1FFF, 13);
        bw.write_bits(buffer_fullness as u32 & 0x7FF, 11);
        bw.write_bits(0, 2);
        let v = bw.finish();
        let mut out = [0u8; 7];
        out.copy_from_slice(&v);
        Ok(out)
    }
}

/// Split an ADTS stream into `(header, raw payload)` frames. Stops at the first malformed or
/// truncated frame (returning what was parsed so far); errors only if nothing parses.
pub fn split_adts(data: &[u8]) -> Result<Vec<(AdtsHeader, &[u8])>> {
    split_adts_upto(data, usize::MAX)
}

/// [`split_adts`], stopping after `max` frames.
pub(crate) fn split_adts_upto(mut data: &[u8], max: usize) -> Result<Vec<(AdtsHeader, &[u8])>> {
    let mut out = Vec::new();
    while data.len() >= 7 && out.len() < max {
        let h = match AdtsHeader::parse(data) {
            Ok(h) => h,
            Err(e) => {
                if out.is_empty() {
                    return Err(e);
                }
                break;
            }
        };
        if h.frame_length > data.len() {
            break;
        }
        out.push((h, &data[h.header_length..h.frame_length]));
        data = &data[h.frame_length..];
    }
    if out.is_empty() {
        return Err(Error::Bitstream("no ADTS frames"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Backward-compatible signalling, as craft-media's Matroska demuxer and some MP4 muxers
    /// write it: an AAC-LC config, then `0x2b7`, object type 5 and the output rate.
    #[test]
    fn a_sync_extension_after_the_config_signals_sbr_and_ps() {
        // AAC-LC, 22.05 kHz, stereo; 0x2b7, AOT 5, SBR, 44.1 kHz
        let asc = AudioSpecificConfig::parse(&[0x13, 0x90, 0x56, 0xE5, 0xA0]).unwrap();
        assert_eq!((asc.object_type, asc.sample_rate, asc.sbr, asc.extension_sample_rate, asc.ps), (2, 22_050, true, Some(44_100), false));
        // ... then 0x548 and PS
        let mut bw = BitWriter::new();
        for (value, bits) in [(2, 5), (7, 4), (1, 4), (0, 3), (0x2b7, 11), (5, 5), (1, 1), (4, 4), (0x548, 11), (1, 1)] {
            bw.write_bits(value, bits);
        }
        let asc = AudioSpecificConfig::parse(&bw.finish()).unwrap();
        assert_eq!((asc.sbr, asc.extension_sample_rate, asc.ps), (true, Some(44_100), true));
        // with extensionFlag set, extensionFlag3 comes first
        let mut bw = BitWriter::new();
        for (value, bits) in [(2, 5), (7, 4), (1, 4), (0b001, 3), (0, 1), (0x2b7, 11), (5, 5), (1, 1), (4, 4)] {
            bw.write_bits(value, bits);
        }
        let asc = AudioSpecificConfig::parse(&bw.finish()).unwrap();
        assert_eq!((asc.sbr, asc.extension_sample_rate), (true, Some(44_100)));
        // SBR flagged off, a different extension, or too few bits: plain AAC-LC
        for bytes in [&[0x13, 0x90, 0x56, 0xE5, 0x00][..], &[0x13, 0x90, 0x12, 0x34], &[0x13, 0x90, 0x56]] {
            let asc = AudioSpecificConfig::parse(bytes).unwrap();
            assert_eq!((asc.sbr, asc.extension_sample_rate), (false, None), "{bytes:02x?}");
        }
    }

    #[test]
    fn an_index_with_no_rate_does_not_panic_and_is_refused() {
        for sf_index in 13..=15 {
            let asc = AudioSpecificConfig::lc(sf_index, 2);
            assert_eq!(asc.sample_rate, 0);
            #[cfg(feature = "aac-decode")]
            assert!(crate::aac::Decoder::from_config(asc).is_err());
        }
    }

    #[test]
    fn configs_that_would_not_parse_back_are_not_written() {
        assert!(AudioSpecificConfig::lc(4, 0).to_bytes().is_err());
        assert!(AudioSpecificConfig::lc(13, 2).to_bytes().is_err(), "a sampling rate of 0");
        let pce = ProgramConfig { front: vec![(true, 0)], ..Default::default() };
        let asc = AudioSpecificConfig { pce: Some(pce), ..AudioSpecificConfig::lc(4, 0) };
        assert_eq!(AudioSpecificConfig::parse(&asc.to_bytes().unwrap()).unwrap(), asc);
    }

    #[test]
    fn adts_headers_refuse_fields_they_cannot_hold() {
        for (object_type, sf_index, channel_config) in [(0, 4, 2), (5, 4, 2), (29, 4, 2), (2, 13, 2), (2, 15, 2), (2, 4, 8), (2, 4, 11), (2, 4, 12)] {
            assert!(AdtsHeader::write(object_type, sf_index, channel_config, 100, 0x7FF).is_err(), "{object_type} {sf_index} {channel_config}");
        }
        for object_type in 1..=4 {
            let h = AdtsHeader::parse(&AdtsHeader::write(object_type, 12, 7, 100, 0x7FF).unwrap()).unwrap();
            assert_eq!((h.object_type, h.sf_index, h.channel_config, h.frame_length), (object_type, 12, 7, 107));
        }
    }

    #[test]
    fn program_configs_refuse_more_elements_than_their_counts_hold() {
        let fits = ProgramConfig { front: vec![(false, 0); 15], lfe: vec![0; 3], assoc_data: vec![0; 7], ..Default::default() };
        assert!(fits.write(&mut BitWriter::new()).is_ok());
        for pce in [
            ProgramConfig { front: vec![(false, 0); 16], ..Default::default() },
            ProgramConfig { side: vec![(false, 0); 16], ..Default::default() },
            ProgramConfig { back: vec![(false, 0); 16], ..Default::default() },
            ProgramConfig { lfe: vec![0; 4], ..Default::default() },
            ProgramConfig { assoc_data: vec![0; 8], ..Default::default() },
            ProgramConfig { cc: vec![(false, 0); 16], ..Default::default() },
        ] {
            assert!(pce.write(&mut BitWriter::new()).is_err(), "{pce:?}");
            let asc = AudioSpecificConfig { pce: Some(pce), ..AudioSpecificConfig::lc(4, 0) };
            assert!(asc.to_bytes().is_err());
        }
    }

    #[test]
    fn adts_headers_refuse_frames_too_long_for_their_length() {
        let longest = AdtsHeader::write(2, 4, 2, 0x1FFF - 7, 0x7FF).unwrap();
        assert_eq!(AdtsHeader::parse(&longest).unwrap().frame_length, 0x1FFF);
        for payload in [0x1FFF - 6, 0x1FFF, usize::MAX] {
            assert!(AdtsHeader::write(2, 4, 2, payload, 0x7FF).is_err(), "{payload}");
        }
    }

    /// An explicit sampling rate of 0 is not a rate.
    #[test]
    fn a_zero_explicit_rate_is_refused() {
        let mut bw = BitWriter::new();
        bw.write_bits(2, 5);
        bw.write_bits(15, 4);
        bw.write_bits(0, 24);
        bw.write_bits(2, 4);
        bw.write_bits(0, 3);
        assert!(AudioSpecificConfig::parse(&bw.finish()).is_err());
    }

    #[test]
    fn asc_roundtrip() {
        let a = AudioSpecificConfig::lc(4, 2);
        let b = a.to_bytes().unwrap();
        assert_eq!(b, vec![0x12, 0x10]);
        assert_eq!(AudioSpecificConfig::parse(&b).unwrap(), a);
        // 48 kHz 5.1
        let a = AudioSpecificConfig::lc(3, 6);
        assert_eq!(AudioSpecificConfig::parse(&a.to_bytes().unwrap()).unwrap().channels(), 6);
    }

    #[test]
    fn amd4_channel_configurations() {
        // 6.1 and 7.1 back (#922); 22.2 and 7.1 with heights stay unsupported
        let channels = |c| AudioSpecificConfig::parse(&AudioSpecificConfig::lc(3, c).to_bytes().unwrap()).map(|a| a.channels()).ok();
        assert_eq!([7, 11, 12].map(channels), [Some(8), Some(7), Some(8)]);
        assert_eq!([13, 14].map(config_layout), [None, None]);
    }

    #[test]
    fn asc_with_pce() {
        let pce = ProgramConfig {
            sf_index: 3,
            object_type: 1,
            front: vec![(false, 0), (true, 0)],
            side: vec![(true, 1)],
            back: vec![(false, 1)],
            lfe: vec![0],
            ..Default::default()
        };
        let a = AudioSpecificConfig { pce: Some(pce), ..AudioSpecificConfig::lc(3, 0) };
        let p = AudioSpecificConfig::parse(&a.to_bytes().unwrap()).unwrap();
        assert_eq!(p.channels(), 7);
        assert_eq!(p, a);
    }

    #[test]
    fn he_aac_asc_parses_core() {
        // AOT 5, 24 kHz core, stereo, ext 48 kHz, core AOT 2
        let mut bw = BitWriter::new();
        bw.write_bits(5, 5);
        bw.write_bits(6, 4);
        bw.write_bits(2, 4);
        bw.write_bits(3, 4);
        bw.write_bits(2, 5);
        bw.write_bits(0, 3);
        let a = AudioSpecificConfig::parse(&bw.finish()).unwrap();
        assert!(a.sbr);
        assert_eq!(a.sample_rate, 24000);
        assert_eq!(a.extension_sample_rate, Some(48000));
    }

    #[test]
    fn adts_roundtrip() {
        let h = AdtsHeader::write(2, 4, 2, 100, 0x7FF).unwrap();
        let mut frame = h.to_vec();
        frame.extend(std::iter::repeat_n(0u8, 100));
        let p = AdtsHeader::parse(&frame).unwrap();
        assert_eq!(p.frame_length, 107);
        assert_eq!((p.object_type, p.sf_index, p.channel_config), (2, 4, 2));
        let frames = split_adts(&frame).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].1.len(), 100);
    }
}
