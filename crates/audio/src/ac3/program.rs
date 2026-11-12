//! Complete six-block E-AC-3 programme access units, including the conventional 7.1 topology.
//!
//! The existing syncframe decoder deliberately ignores dependent substreams. This API instead
//! accepts one complete 1536-sample access unit: independent programme zero, optionally followed
//! by its four-channel dependent replacement/extension. It returns full-range PCM (no dialnorm,
//! RF compression or dynamic-range metadata gain), so an independent downmix's gain is never
//! incorrectly applied to the assembled programme. It does not claim support for every E-AC-3
//! dependent channel map or shorter-frame grouping.

use super::{Bits, Decoder, Error, Header, Result, eac3, parse_header};
use crate::ChannelLayout;

/// Decoded complete programme, in WAVE speaker-mask order.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProgrammeFrame {
    /// Samples per second, shared by every substream.
    pub sample_rate: u32,
    /// Speaker assignments; dual mono remains unordered.
    pub layout: ChannelLayout,
    /// Exactly 1536 samples in each channel.
    pub channels: Vec<Vec<f32>>,
}

/// Stateful decoder for complete six-block E-AC-3 programme-zero access units.
///
/// For 7.1 the independent 3/2+LFE downmix is followed by one four-channel dependent stream whose
/// explicit map is Ls, Rs, Lrs, Rrs (0x1A00). The first pair replaces the independent surrounds;
/// the second pair adds rear speakers. Output order is L, R, C, LFE, Lrs, Rrs, Ls, Rs.
/// Unsupported topologies fail explicitly. After any error all overlap state is reset.
pub struct ProgrammeDecoder {
    independent: Decoder,
    dependent: Decoder,
    topology: Option<(u32, u8, bool, bool)>,
    expected_layout: ChannelLayout,
}

impl ProgrammeDecoder {
    /// Start with empty overlap state and full-range output.
    ///
    /// The expected layout comes from the container or the caller's programme description.
    /// Requiring it prevents a missing dependent frame from silently downgrading 7.1 to 5.1.
    pub fn new(expected_layout: ChannelLayout) -> Self {
        let mut independent = Decoder::new();
        independent.apply_dynamic_range = false;
        let mut dependent = Decoder::new();
        dependent.apply_dynamic_range = false;
        Self { independent, dependent, topology: None, expected_layout }
    }

    /// Clear the programme's overlap state before seeking or switching streams.
    pub fn reset(&mut self) {
        self.independent.reset();
        self.dependent.reset();
        self.topology = None;
    }

    /// Decode one complete access unit, never publishing a partial programme.
    ///
    /// Input is bounded to two syncframes of at most 4096 bytes each. Both CRCs, frame bounds,
    /// programme IDs, rates, block counts and the dependent channel map are checked first.
    pub fn decode(&mut self, access_unit: &[u8]) -> Result<ProgrammeFrame> {
        let result = self.decode_inner(access_unit);
        if result.is_err() {
            self.reset();
        }
        result
    }

    fn decode_inner(&mut self, au: &[u8]) -> Result<ProgrammeFrame> {
        let (primary, dependent) = validate(au)?;
        let layout = if dependent.is_some() { ChannelLayout::SURROUND_7_1 } else { primary_layout(primary) };
        if layout != self.expected_layout {
            return Err(Error::Invalid("programme does not match the expected speaker layout"));
        }
        let topology = (primary.sample_rate, primary.acmod, primary.lfeon, dependent.is_some());
        if self.topology != Some(topology) {
            self.reset();
        }
        let mut channels = self.independent.decode_eac3_substream(&au[..primary.frame_bytes], primary)?.channels;
        if let Some(header) = dependent {
            let more = self.dependent.decode_eac3_substream(&au[primary.frame_bytes..], header)?.channels;
            channels = assemble_7_1(channels, more)?;
        }
        self.topology = Some(topology);
        Ok(ProgrammeFrame { sample_rate: primary.sample_rate, layout, channels })
    }
}

fn primary_layout(h: Header) -> ChannelLayout {
    let mask = match h.acmod {
        0 => return ChannelLayout::unordered(if h.lfeon { 3 } else { 2 }),
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x103,
        5 => 0x107,
        6 => 0x603,
        _ => 0x607,
    };
    ChannelLayout::from_mask(mask | if h.lfeon { 8 } else { 0 })
}

fn validate(au: &[u8]) -> Result<(Header, Option<Header>)> {
    if au.len() > 8192 {
        return Err(Error::Invalid("programme access unit exceeds two syncframes"));
    }
    let first = checked_frame(au)?;
    if first.strmtyp != 0 || first.substreamid != 0 {
        return Err(Error::Unsupported("programme must start with independent substream zero".into()));
    }
    if first.frame_bytes == au.len() {
        return Ok((first, None));
    }
    let remainder = &au[first.frame_bytes..];
    let second = checked_frame(remainder)?;
    if second.frame_bytes != remainder.len() {
        return Err(Error::Unsupported("more than one dependent substream".into()));
    }
    if second.strmtyp != 1 || second.substreamid != 0 {
        return Err(Error::Invalid("expected dependent substream zero"));
    }
    if second.sample_rate != first.sample_rate || second.blocks != first.blocks {
        return Err(Error::Invalid("dependent rate or block count differs from independent"));
    }
    if first.acmod != 7 || !first.lfeon || !matches!(second.acmod, 5 | 6) || second.lfeon {
        return Err(Error::Unsupported("dependent topology is not the supported 7.1 programme".into()));
    }
    let mut bits = Bits::new(remainder);
    let bsi = eac3::read_bsi(&mut bits)?;
    if bits.overrun() {
        return Err(Error::Invalid("truncated dependent bit stream information"));
    }
    if !bsi.compre {
        return Err(Error::Invalid("last dependent substream must signal compre"));
    }
    if bsi.chanmap != Some(0x1A00) {
        return Err(Error::Unsupported("dependent channel map must be Ls Rs Lrs Rrs".into()));
    }
    Ok((first, Some(second)))
}

fn checked_frame(bytes: &[u8]) -> Result<Header> {
    let h = parse_header(bytes)?;
    if !h.is_eac3() || h.bsid != 16 || h.blocks != 6 || !matches!(h.sample_rate, 32_000 | 44_100 | 48_000) {
        return Err(Error::Unsupported("programme decoder requires full-rate six-block E-AC-3 bsid 16".into()));
    }
    let frame = bytes.get(..h.frame_bytes).ok_or(Error::Invalid("truncated programme syncframe"))?;
    if crc16(&frame[2..]) != 0 {
        return Err(Error::Invalid("programme syncframe CRC mismatch"));
    }
    Ok(h)
}

// ETSI TS 102 366 E.1.6: polynomial x^16+x^15+x^2+1; callers exclude only the syncword.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
        }
    }
    crc
}

fn assemble_7_1(mut primary: Vec<Vec<f32>>, mut dependent: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>> {
    if primary.len() != 6 || dependent.len() != 4 || primary.iter().chain(&dependent).any(|c| c.len() != 1536) {
        return Err(Error::Invalid("decoded programme dimensions differ from signalled channels"));
    }
    // Keep L R C LFE; discard downmixed Ls/Rs. Dependent coded order is Ls Rs Lrs Rrs.
    primary.truncate(4);
    let rear = dependent.split_off(2);
    primary.extend(rear);
    primary.extend(dependent);
    Ok(primary)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original independent-spec oracle envelopes: valid BSI/CRC, deliberately no audio blocks.
    const PRIMARY: [u8; 32] = [
        0x0b, 0x77, 0x00, 0x0f, 0x3f, 0x87, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x71, 0x87,
    ];
    const DEPENDENT: [u8; 32] = [
        0x0b, 0x77, 0x40, 0x0f, 0x3c, 0x87, 0xe0, 0x11, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x39, 0x93,
    ];

    #[test]
    fn seven_one_rejects_valid_independent_prefix_before_audio_decode() -> Result<()> {
        assert!(validate(&PRIMARY)?.1.is_none());
        let mut decoder = ProgrammeDecoder::new(ChannelLayout::SURROUND_7_1);
        assert_eq!(decoder.decode(&PRIMARY).err(), Some(Error::Invalid("programme does not match the expected speaker layout")));
        assert!(decoder.topology.is_none());
        Ok(())
    }

    #[test]
    fn validates_dependency_order_frame_bounds_and_crc() -> Result<()> {
        let mut au = PRIMARY.to_vec();
        au.extend_from_slice(&DEPENDENT);
        assert!(validate(&au)?.1.is_some());
        assert!(validate(&DEPENDENT).is_err());
        assert!(validate(&au[..63]).is_err());
        au.push(0);
        assert!(validate(&au).is_err());
        au.pop();
        au[55] ^= 1;
        assert!(validate(&au).is_err());
        let mut duplicate = PRIMARY.to_vec();
        duplicate.extend_from_slice(&PRIMARY);
        assert!(validate(&duplicate).is_err());
        Ok(())
    }

    fn changed_dependent(bit: usize, width: usize, value: u16) -> Vec<u8> {
        let mut frame = DEPENDENT;
        for i in 0..width {
            let at = bit + i;
            let mask = 1 << (7 - at % 8);
            frame[at / 8] = (frame[at / 8] & !mask) | if value & (1 << (width - 1 - i)) != 0 { mask } else { 0 };
        }
        let crc = crc16(&frame[2..30]);
        frame[30..].copy_from_slice(&crc.to_be_bytes());
        let mut au = PRIMARY.to_vec();
        au.extend_from_slice(&frame);
        au
    }

    #[test]
    fn rejects_crc_valid_but_inconsistent_programme_descriptions() {
        // Fields are normative bit offsets in the independent structural BSI fixture.
        let wrong_map = changed_dependent(60, 16, 0x1800);
        assert_eq!(validate(&wrong_map).err(), Some(Error::Unsupported("dependent channel map must be Ls Rs Lrs Rrs".into())));
        let missing_last_marker = changed_dependent(50, 1, 0);
        assert_eq!(validate(&missing_last_marker).err(), Some(Error::Invalid("last dependent substream must signal compre")));
        let wrong_rate = changed_dependent(32, 2, 2);
        assert_eq!(validate(&wrong_rate).err(), Some(Error::Invalid("dependent rate or block count differs from independent")));
        let short_frame = changed_dependent(34, 2, 0);
        assert_eq!(validate(&short_frame).err(), Some(Error::Unsupported("programme decoder requires full-rate six-block E-AC-3 bsid 16".into())));
        let mut decoder = ProgrammeDecoder::new(ChannelLayout::SURROUND_7_1);
        decoder.reset();
        assert_eq!(decoder.expected_layout, ChannelLayout::SURROUND_7_1);
    }

    #[cfg(feature = "eac3-encode")]
    #[test]
    fn explicit_map_overrides_dependent_acmod_speaker_order() -> std::result::Result<(), Box<dyn std::error::Error>> {
        use super::super::encoder::{Codec, CodingMode, Config, Encoder};
        let config = Config { codec: Codec::Eac3, mode: CodingMode::Surround71, lfe: true, sample_rate: 48_000, bitrate_kbps: 640 };
        let mut encoder = Encoder::new(config)?;
        let source: Vec<Vec<f32>> = (0..8).map(|c| (0..1536).map(|n| ((n as f32) * (0.01 + c as f32 * 0.017)).sin() * 0.2).collect()).collect();
        let packets = encoder.push(&source.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
        let packet = packets.first().ok_or("encoder did not emit the complete frame")?;
        let expected = ProgrammeDecoder::new(ChannelLayout::SURROUND_7_1).decode(&packet.data)?;
        let mut alternate = packet.data.clone();
        let offset = parse_header(&alternate)?.frame_bytes;
        let dependent = &mut alternate[offset..];
        // Both acmod5 and6 code four FBW channels. The explicit map overrides their default
        // positions, so changing only acmod (with no coupling/rematrix/mix metadata) is valid.
        dependent[4] = (dependent[4] & 0xf1) | (5 << 1);
        let end = dependent.len() - 2;
        let crc = crc16(&dependent[2..end]);
        dependent[end..].copy_from_slice(&crc.to_be_bytes());
        let actual = ProgrammeDecoder::new(ChannelLayout::SURROUND_7_1).decode(&alternate)?;
        assert_eq!(expected.layout, actual.layout);
        assert_eq!(expected.channels, actual.channels);
        Ok(())
    }

    #[test]
    fn corrupt_envelopes_and_truncations_never_panic_or_publish_partial_pcm() {
        let mut original = PRIMARY.to_vec();
        original.extend_from_slice(&DEPENDENT);
        for length in 0..original.len() {
            let result = std::panic::catch_unwind(|| ProgrammeDecoder::new(ChannelLayout::SURROUND_7_1).decode(&original[..length]));
            assert!(matches!(result, Ok(Err(_))), "truncated length {length}");
        }
        for bit in 0..original.len() * 8 {
            let mut corrupt = original.clone();
            corrupt[bit / 8] ^= 1 << (bit % 8);
            let result = std::panic::catch_unwind(|| ProgrammeDecoder::new(ChannelLayout::SURROUND_7_1).decode(&corrupt));
            assert!(matches!(result, Ok(Err(_))), "corrupt bit {bit}");
        }
    }

    #[test]
    fn dependent_replaces_downmix_and_labels_rears() -> Result<()> {
        let primary = (0..6).map(|c| vec![c as f32; 1536]).collect();
        let dependent = (10..14).map(|c| vec![c as f32; 1536]).collect();
        let result = assemble_7_1(primary, dependent)?;
        assert_eq!(result.iter().map(|c| c[0]).collect::<Vec<_>>(), [0., 1., 2., 3., 12., 13., 10., 11.]);
        assert_eq!(ChannelLayout::SURROUND_7_1.mask(), Some(0x63f));
        Ok(())
    }

    #[test]
    fn incomplete_programme_never_produces_partial_channels() {
        assert!(assemble_7_1(vec![vec![0.; 1536]; 6], vec![vec![0.; 1535]; 4]).is_err());
        let mut decoder = ProgrammeDecoder::new(ChannelLayout::SURROUND_7_1);
        decoder.topology = Some((48_000, 7, true, true));
        assert!(decoder.decode(&[0; 8193]).is_err());
        assert!(decoder.topology.is_none());
    }
}
