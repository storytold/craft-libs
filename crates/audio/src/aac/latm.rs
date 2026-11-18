//! LOAS / LATM (ISO/IEC 14496-3 §1.7): AAC in an audio sync stream, as MPEG transport streams carry
//! it (stream type 0x11, ffmpeg's `-mpegts_flags latm`). The parts read here: one program, one
//! layer, one sub-frame, AAC-LC, variable frame length (`frameLengthType` 0), `audioMuxVersion` 0
//! or 1. Anything else is refused (`None`), never guessed.
//!
//! [`latm_config`] gives a frame's configuration as an `AudioSpecificConfig` for [`Decoder`], and
//! [`latm_split`] its payload. From FilmCraft's `filmcraft-codecs` (storytold/filmcraft@db60ee4).
//!
//! [`Decoder`]: crate::aac::Decoder

use craft_media::bitstream::BitReader;

use super::tables::SAMPLE_RATES;

/// The configuration a LATM `StreamMuxConfig` carries.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatmConfig {
    /// The `AudioSpecificConfig`, as [`Decoder::new`](crate::aac::Decoder::new) takes it.
    pub asc: Vec<u8>,
    /// Core sampling rate.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u32,
}

impl LatmConfig {
    /// A configuration from its parts (for a caller that keeps the configuration of earlier frames).
    pub fn new(asc: Vec<u8>, sample_rate: u32, channels: u32) -> LatmConfig {
        LatmConfig { asc, sample_rate, channels }
    }
}

/// `LatmGetValue()`: a 2-bit byte count less one, then that many bytes.
fn latm_value(r: &mut BitReader) -> Option<u32> {
    let n = r.read_bits(2).ok()? + 1;
    r.read_bits(8 * n).ok()
}

/// The program config element `r` has reached inside a `StreamMuxConfig`'s
/// `AudioSpecificConfig`, leaving the reader after it. The element aligns its comment to a byte of
/// the `AudioSpecificConfig`, which may start at any bit of a LATM frame, and it starts on such a
/// byte itself: so the bits are lined up from here to read it.
fn latm_program_config(r: &mut BitReader) -> Option<super::ProgramConfig> {
    let mut rest = r.clone();
    let mut lined_up = Vec::with_capacity(rest.bits_left() / 8);
    while let Ok(b) = rest.read_bits(8) {
        lined_up.push(b as u8);
    }
    let mut pce = BitReader::new(&lined_up);
    let config = super::ProgramConfig::parse(&mut pce).ok()?;
    r.skip(pce.position()).ok()?;
    Some(config)
}

/// `StreamMuxConfig()` after `useSameStreamMux`, leaving the reader after it.
fn stream_mux_config(r: &mut BitReader) -> Option<LatmConfig> {
    let version = r.read_bits(1).ok()?;
    let version_a = if version == 1 { r.read_bits(1).ok()? } else { 0 };
    if version_a != 0 {
        return None;
    }
    if version == 1 {
        latm_value(r)?; // taraBufferFullness
    }
    let _all_same_framing = r.read_bits(1).ok()?;
    let sub_frames = r.read_bits(6).ok()?;
    let programs = r.read_bits(4).ok()?;
    let layers = r.read_bits(3).ok()?;
    if sub_frames != 0 || programs != 0 || layers != 0 {
        return None;
    }
    let asc_len = if version == 1 { Some(latm_value(r)? as usize) } else { None };
    let asc_start = r.position();
    // AudioSpecificConfig
    let aot = r.read_bits(5).ok()?;
    let sf = r.read_bits(4).ok()?;
    if aot != 2 || sf == 15 {
        return None;
    }
    let ch = r.read_bits(4).ok()?;
    let frame_length_flag = r.read_bits(1).ok()?;
    let depends_on_core = r.read_bits(1).ok()?;
    let _extension = r.read_bits(1).ok()?;
    if frame_length_flag != 0 || depends_on_core != 0 {
        return None;
    }
    let sample_rate = *SAMPLE_RATES.get(sf as usize)?;
    let (asc, channels) = if ch == 0 {
        // the layout is in a program config element
        let pce = latm_program_config(r)?;
        let cfg = super::AudioSpecificConfig { pce: Some(pce), ..super::AudioSpecificConfig::lc(sf as u8, 0) };
        let channels = u32::try_from(cfg.channels()).ok().filter(|&n| n > 0)?;
        (cfg.to_bytes().ok()?, channels)
    } else {
        // `ch` is an index into Table 1.19, not a count: 7 and 12 are eight channels, 11 is seven,
        // and a reserved configuration has no layout
        (
            super::AudioSpecificConfig::lc(sf as u8, ch as u8).to_bytes().ok()?,
            super::config_layout(ch as u8)?.iter().map(|e| e.0.channels() as u32).sum(),
        )
    };
    // what ascLen counts past the fields read here is fill
    if let Some(len) = asc_len {
        r.skip(len.saturating_sub(r.position() - asc_start)).ok()?;
    }
    let frame_length_type = r.read_bits(3).ok()?;
    if frame_length_type != 0 {
        return None;
    }
    r.read_bits(8).ok()?; // latmBufferFullness
    if r.read_bits(1).ok()? == 1 {
        // otherData
        if version == 1 {
            latm_value(r)?;
        } else {
            loop {
                let esc = r.read_bits(1).ok()?;
                r.read_bits(8).ok()?;
                if esc == 0 {
                    break;
                }
            }
        }
    }
    if r.read_bits(1).ok()? == 1 {
        r.read_bits(8).ok()?; // crcCheckSum
    }
    Some(LatmConfig { asc, sample_rate, channels })
}

/// Whether `frame` starts with the LOAS `AudioSyncStream` sync word (0x2B7) and a length.
fn is_loas(frame: &[u8]) -> bool {
    frame.len() >= 4 && frame[0] == 0x56 && frame[1] & 0xE0 == 0xE0
}

/// The configuration a LOAS frame carries; `None` if it reuses an earlier one
/// (`useSameStreamMux`) or is not one this module reads.
pub fn latm_config(frame: &[u8]) -> Option<LatmConfig> {
    if !is_loas(frame) {
        return None;
    }
    let mut r = BitReader::new(frame.get(3..)?);
    if r.read_bits(1).ok()? == 1 {
        return None; // useSameStreamMux
    }
    stream_mux_config(&mut r)
}

/// A LOAS frame's `AudioSpecificConfig` and the payload of its single sub-frame. `last` is the
/// configuration of earlier frames: used by a frame that sets `useSameStreamMux`, replaced by one
/// that carries its own. `None` for a frame this module does not read, or a payload length past the
/// end of the frame.
pub fn latm_split(frame: &[u8], last: &mut Option<LatmConfig>) -> Option<(Vec<u8>, Vec<u8>)> {
    if !is_loas(frame) {
        return None;
    }
    let mut r = BitReader::new(frame.get(3..)?);
    let same = r.read_bits(1).ok()? == 1;
    let cfg = if same {
        last.clone()?
    } else {
        let c = stream_mux_config(&mut r)?;
        *last = Some(c.clone());
        c
    };
    // PayloadLengthInfo (frameLengthType 0), then PayloadMux, at any bit
    let mut len = 0usize;
    loop {
        let b = r.read_bits(8).ok()? as usize;
        len += b;
        if b != 255 {
            break;
        }
    }
    // each payload byte is a byte of this frame: a longer claim is damage, and must not size a buffer
    if len > frame.len() {
        return None;
    }
    let mut payload = Vec::with_capacity(len);
    for _ in 0..len {
        payload.push(r.read_bits(8).ok()? as u8);
    }
    Some((cfg.asc, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_media::bitstream::BitWriter;

    /// A LOAS frame: `fields` (value, bits) after the 3-byte header, then `payload` with its
    /// length, padded to a byte; the header's length is set to the body's.
    pub(super) fn loas(fields: &[(u32, u32)], payload: &[u8]) -> Vec<u8> {
        let mut w = BitWriter::new();
        for &(v, n) in fields {
            w.write_bits(v, n);
        }
        let mut len = payload.len();
        while len >= 255 {
            w.write_bits(255, 8);
            len -= 255;
        }
        w.write_bits(len as u32, 8);
        for &b in payload {
            w.write_bits(u32::from(b), 8);
        }
        w.align_zero();
        let body = w.finish();
        [&[0x56, 0xE0 | (body.len() >> 8) as u8, body.len() as u8][..], &body].concat()
    }

    /// `useSameStreamMux` 0 and a version-0 `StreamMuxConfig` for AAC-LC at `sf` with `ch`.
    pub(super) fn config_fields(sf: u32, ch: u32) -> Vec<(u32, u32)> {
        // useSameStreamMux, audioMuxVersion, allStreamsSameTimeFraming, numSubFrames, numProgram,
        // numLayer; object type, rate index, configuration, its three flags; frameLengthType,
        // latmBufferFullness, otherDataPresent, crcCheckPresent
        vec![(0, 1), (0, 1), (1, 1), (0, 6), (0, 4), (0, 3), (2, 5), (sf, 4), (ch, 4), (0, 3), (0, 3), (0xFF, 8), (0, 1), (0, 1)]
    }

    #[test]
    fn a_frame_gives_its_configuration_and_payload() {
        let payload: Vec<u8> = (0..300).map(|i| i as u8).collect();
        let frame = loas(&config_fields(3, 2), &payload);
        let cfg = latm_config(&frame).unwrap();
        assert_eq!((cfg.sample_rate, cfg.channels), (48_000, 2));
        assert_eq!(super::super::AudioSpecificConfig::parse(&cfg.asc).unwrap().channel_config, 2);
        let mut last = None;
        assert_eq!(latm_split(&frame, &mut last), Some((cfg.asc.clone(), payload.clone())));
        assert_eq!(last.as_ref(), Some(&cfg));
        // a frame that reuses the configuration
        let same = loas(&[(1, 1)], &payload[..10]);
        assert_eq!(latm_config(&same), None);
        assert_eq!(latm_split(&same, &mut last), Some((cfg.asc, payload[..10].to_vec())));
        assert_eq!(latm_split(&same, &mut None), None, "nothing to reuse");
    }

    #[test]
    fn a_channel_configuration_is_not_a_channel_count() {
        for (config, channels) in [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (7, 8), (11, 7), (12, 8)] {
            let got = latm_config(&loas(&config_fields(3, config), &[0; 4])).unwrap_or_else(|| panic!("configuration {config} refused"));
            assert_eq!((got.channels, got.sample_rate), (channels, 48_000), "configuration {config}");
        }
    }

    /// 5.1 as a program config element (centre, front pair, back pair, LFE) in the `StreamMuxConfig`,
    /// with `audioMuxVersion` 0 (the configuration inline) and 1 (`ascLen` before it, one fill
    /// byte after it): six channels, and the element is in the configuration the decoder gets.
    #[test]
    fn a_program_config_element_in_the_stream_mux_config_gives_the_layout() {
        use super::super::{AudioSpecificConfig, ProgramConfig};
        let pce = ProgramConfig { sf_index: 3, front: vec![(false, 0), (true, 0)], back: vec![(true, 1)], lfe: vec![0], ..Default::default() };
        // the AudioSpecificConfig on its own, to know its length and copy its bits
        let mut w = BitWriter::new();
        w.write_bits(2, 5);
        w.write_bits(3, 4);
        w.write_bits(0, 4);
        w.write_bits(0, 3);
        pce.write(&mut w).unwrap();
        let asc_bits = w.bit_len();
        let asc = w.finish();
        let asc_fields: Vec<(u32, u32)> = (0..asc_bits).map(|i| (u32::from(asc[i / 8] >> (7 - i % 8) & 1), 1)).collect();
        let tail = [(0, 3), (0xFF, 8), (0, 1), (0, 1)];
        for version in [0, 1] {
            let mut f = vec![(0, 1), (version, 1)];
            if version == 1 {
                f.extend([(0, 1), (0, 2), (0xFF, 8)]); // audioMuxVersionA, taraBufferFullness
            }
            f.extend([(1, 1), (0, 6), (0, 4), (0, 3)]);
            if version == 1 {
                f.extend([(0, 2), (asc_bits as u32 + 8, 8)]); // ascLen, with a byte of fill
            }
            f.extend(asc_fields.iter().copied());
            if version == 1 {
                f.push((0xA5, 8));
            }
            f.extend(tail);
            let frame = loas(&f, &[9; 12]);
            let cfg = latm_config(&frame).unwrap_or_else(|| panic!("version {version} refused"));
            assert_eq!((cfg.channels, cfg.sample_rate), (6, 48_000), "version {version}");
            let parsed = AudioSpecificConfig::parse(&cfg.asc).unwrap();
            assert_eq!((parsed.channel_config, parsed.pce.as_ref().map(|p| p.layout())), (0, Some(pce.layout())), "version {version}");
            // the payload is found after the configuration
            assert_eq!(latm_split(&frame, &mut None).map(|(_, p)| p), Some(vec![9; 12]), "version {version}");
        }
        // a configuration 0 with an element that does not parse is refused, not played as stereo
        let mut f = config_fields(3, 0);
        f.truncate(9);
        assert_eq!(latm_config(&loas(&f, &[])), None);
    }

    #[test]
    fn a_reserved_channel_configuration_is_refused() {
        for config in [8, 9, 10, 13, 14, 15] {
            assert_eq!(latm_config(&loas(&config_fields(3, config), &[0; 4])), None, "configuration {config}");
        }
    }

    #[test]
    fn what_this_module_does_not_read_is_refused() {
        let with = |k: usize, v: u32| {
            let mut f = config_fields(3, 2);
            f[k].0 = v;
            latm_config(&loas(&f, &[0; 4]))
        };
        assert!(with(0, 0).is_some());
        assert_eq!(with(3, 1), None, "two sub-frames");
        assert_eq!(with(4, 1), None, "two programs");
        assert_eq!(with(5, 1), None, "two layers");
        assert_eq!(with(6, 5), None, "HE-AAC object type");
        assert_eq!(with(7, 15), None, "an explicit rate");
        assert_eq!(with(7, 13), None, "a reserved rate index");
        assert_eq!(with(9, 4), None, "960-sample frames");
        assert_eq!(with(10, 1), None, "a fixed frame length type");
        assert_eq!(latm_config(&[0x47, 0, 0, 0]), None, "not LOAS");
        assert_eq!(latm_config(&[0x56, 0xE0]), None, "cut short");
    }

    #[test]
    fn a_latm_payload_length_beyond_the_frame_is_refused_before_it_is_reserved() {
        // useSameStreamMux, then a run of 255s that claims far more payload than the 24-byte frame holds
        let mut frame = vec![0x56, 0xE0, 0x00];
        frame.extend([0xFF; 20]);
        frame.push(0x7F);
        let mut last = Some(LatmConfig::new(vec![0x11, 0x90], 48_000, 2));
        assert_eq!(latm_split(&frame, &mut last), None);
    }

    #[test]
    fn mutated_frames_never_panic() {
        let base = loas(&config_fields(3, 2), &[7; 40]);
        let mut x = 0x2545_F491u32;
        for _ in 0..20_000 {
            let mut f = base.clone();
            for _ in 0..1 + x % 4 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let at = x as usize % f.len();
                f[at] = (x >> 8) as u8;
            }
            f.truncate(f.len() - (x >> 20) as usize % 8);
            let _ = latm_config(&f);
            let _ = latm_split(&f, &mut None);
            let mut last = latm_config(&base);
            let _ = latm_split(&f, &mut last);
        }
    }
}
