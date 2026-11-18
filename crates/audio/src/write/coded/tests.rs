use super::*;

use crate::Speaker;

fn map(mask: u32, order: Option<&[usize]>, layout: u32) -> Option<Vec<usize>> {
    channel_map(mask, order, &ChannelLayout::from_mask(layout))
}

/// Channels are matched by speaker, not by position: a layout that lists the same speakers in
/// another WAV order is reordered, and one with other speakers is refused.
#[test]
fn channels_are_mapped_by_speaker() {
    // L R C LFE Ls Rs onto itself, and onto an encoder order C L R Ls Rs LFE.
    assert_eq!(map(0x60F, None, 0x60F), Some(vec![0, 1, 2, 3, 4, 5]));
    assert_eq!(map(0x60F, Some(&[2, 0, 1, 4, 5, 3]), 0x60F), Some(vec![2, 0, 1, 4, 5, 3]));
    // Back surrounds feed a format that has one surround pair, and the other way round.
    assert_eq!(map(0x60F, None, 0x3F), Some(vec![0, 1, 2, 3, 4, 5]));
    assert_eq!(map(0x3F, None, 0x60F), Some(vec![0, 1, 2, 3, 4, 5]));
    // 6.1 with the back centre after the pair (L R C LFE Lb Rb Cb) onto L R C LFE Cb Ls Rs.
    assert_eq!(map(0x70F, None, 0x13F), Some(vec![0, 1, 2, 3, 6, 4, 5]));
    // Quad has no centre; 7.1 with sides and backs has no front pair left and right of centre.
    assert_eq!(map(0x107, None, 0x33), None);
    assert_eq!(map(0xFF, None, 0x63F), None);
    // With both pairs in the format, one pair cannot feed both.
    assert_eq!(map(0x63F, None, 0xFF), None);
    assert_eq!(channel_map(0x3, None, &ChannelLayout::unordered(2)), None);
    assert_eq!(map(0x3, None, 0x7), None);
}

#[cfg(feature = "aac-encode")]
#[test]
fn every_aac_channel_count_has_a_layout_and_an_order() {
    for channels in 1..=8 {
        let mask = EncodeFormat::Aac.layout_masks(channels as u16)[0];
        assert_eq!(mask.count_ones() as usize, channels);
        let mut order = map(mask, aac_order(channels), mask).unwrap();
        order.sort_unstable();
        assert_eq!(order, (0..channels).collect::<Vec<_>>());
    }
    assert!(EncodeFormat::Aac.layout_masks(9).is_empty() && EncodeFormat::Aac.layout_masks(0).is_empty());
    // The layouts this crate's decoder reports for what the encoder writes (`track::aac_layout`),
    // and configuration 7's speakers for 8 channels.
    let masks: Vec<u32> = (1..=8).map(|channels| EncodeFormat::Aac.layout_masks(channels)[0]).collect();
    assert_eq!(masks, [0x4, 0x3, 0x7, 0x107, 0x607, 0x60F, 0x70F, 0xFF]);
    assert_eq!(
        (EncodeFormat::Mp3.layout_masks(1), EncodeFormat::Mp3.layout_masks(2), EncodeFormat::Mp3.layout_masks(3)),
        (&[0x4][..], &[0x3][..], &[][..])
    );
    // Configuration 7: C, Lc Rc, L R, Ls Rs, LFE.
    let wide: Vec<Speaker> = ChannelLayout::from_mask(0xFF).speakers();
    let named: Vec<Speaker> = aac_order(8).unwrap().iter().map(|&c| wide[c]).collect();
    use Speaker::*;
    assert_eq!(named, [FrontCenter, FrontLeftOfCenter, FrontRightOfCenter, FrontLeft, FrontRight, BackLeft, BackRight, Lfe]);
}

/// The range the writer keeps a bit rate in is the one the encoder accepts.
#[cfg(feature = "aac-encode")]
#[test]
fn the_aac_bit_rate_range_is_the_encoders() {
    use crate::aac::{Encoder, EncoderConfig, SAMPLE_RATES};
    for &rate in &SAMPLE_RATES {
        for channels in 1..=8 {
            let (lo, hi) = crate::aac::cbr_range(rate, channels);
            assert!(lo <= hi, "{rate} Hz, {channels} channels");
            let accepts = |bps| Encoder::new(EncoderConfig::cbr(rate, channels, bps)).is_ok();
            assert!(accepts(lo) && accepts(hi), "{rate} Hz, {channels} channels: {lo}..={hi}");
            assert!(!accepts(lo - 1) && !accepts(hi + 1), "{rate} Hz, {channels} channels: {lo}..={hi}");
            for want in [None, Some(RateControl::Bitrate(0)), Some(RateControl::Bitrate(u32::MAX))] {
                let mode = aac_mode(want, rate, channels);
                assert!(
                    Encoder::new(EncoderConfig { mode, ..EncoderConfig::cbr(rate, channels, 0) }).is_ok(),
                    "{rate} Hz, {channels} channels, {want:?}"
                );
            }
        }
    }
}

/// Every layout the AC-3 encoders have is found from its speakers, at a bit rate the encoder
/// accepts whatever rate was asked for.
#[cfg(all(feature = "ac3-encode", feature = "eac3-encode"))]
#[test]
fn ac3_settings_follow_the_layout_and_validate() {
    use crate::ac3::encoder::{Codec, CodingMode};
    let layouts = [
        (0x4, CodingMode::Mono, false),
        (0x3, CodingMode::Stereo, false),
        (0xB, CodingMode::Stereo, true),
        (0x7, CodingMode::ThreeFront, false),
        (0x103, CodingMode::StereoSurround, false),
        (0x107, CodingMode::ThreeFrontSurround, false),
        (0x33, CodingMode::StereoTwoSurround, false),
        (0x603, CodingMode::StereoTwoSurround, false),
        (0x37, CodingMode::ThreeFrontTwoSurround, false),
        (0x3F, CodingMode::ThreeFrontTwoSurround, true),
        (0x60F, CodingMode::ThreeFrontTwoSurround, true),
    ];
    for codec in [Codec::Ac3, Codec::Eac3] {
        for sample_rate in [32_000, 44_100, 48_000] {
            for (mask, mode, lfe) in layouts {
                for want in [None, Some(RateControl::Bitrate(0)), Some(RateControl::Bitrate(200_000)), Some(RateControl::Bitrate(u32::MAX))] {
                    let layout = ChannelLayout::from_mask(mask);
                    let (config, map) = ac3_config(codec, sample_rate, &layout, want).unwrap();
                    assert_eq!((config.mode, config.lfe), (mode, lfe), "{mask:#x}");
                    assert_eq!(map, (0..usize::from(layout.channels())).collect::<Vec<_>>());
                    assert_eq!(config.validate(), Ok(()), "{codec:?} {sample_rate} {mask:#x} {want:?}: {} kbit/s", config.bitrate_kbps);
                }
            }
            let seven_one = ac3_config(codec, sample_rate, &ChannelLayout::SURROUND_7_1, Some(RateControl::Bitrate(u32::MAX)));
            match codec {
                Codec::Ac3 => assert!(matches!(seven_one, Err(AudioError::Unsupported(_)))),
                Codec::Eac3 => assert_eq!(seven_one.unwrap().0.validate(), Ok(())),
            }
        }
    }
    // The highest table rate not above the one asked for; the default is 96 kbit/s per channel.
    let rate = |codec, mask, want| ac3_config(codec, 48_000, &ChannelLayout::from_mask(mask), want).unwrap().0.bitrate_kbps;
    assert_eq!(rate(Codec::Ac3, 0x3, Some(RateControl::Bitrate(200_000))), 192);
    assert_eq!(rate(Codec::Ac3, 0x3, None), 192);
    assert_eq!(rate(Codec::Ac3, 0x60F, None), 448);
    assert_eq!(rate(Codec::Ac3, 0x60F, Some(RateControl::Bitrate(1))), 160);
    assert_eq!(rate(Codec::Eac3, 0x3, Some(RateControl::Bitrate(200_500))), 200);
    assert!(matches!(ac3_config(Codec::Ac3, 48_000, &ChannelLayout::STEREO, Some(RateControl::Quality(0.5))), Err(AudioError::Unsupported(_))));
    assert!(matches!(ac3_config(Codec::Ac3, 48_000, &ChannelLayout::unordered(2), None), Err(AudioError::Unsupported(_))));
}

/// The layouts `EncodeFormat` lists are the encoders' own: ALAC's layout per channel count, and
/// the speakers of every AC-3 and E-AC-3 coding mode the encoder has.
#[cfg(all(feature = "alac-encode", feature = "ac3-encode", feature = "eac3-encode"))]
#[test]
fn the_listed_layouts_are_the_encoders() {
    use crate::ac3::encoder::{Codec, CodingMode::*, Config};
    for channels in 0..=10u16 {
        let alac = crate::alac::channel_mask(channels as u8);
        assert_eq!(EncodeFormat::Alac.layout_masks(channels), if alac == 0 { &[][..] } else { std::slice::from_ref(&alac) }, "ALAC, {channels}");
        for (format, codec) in [(EncodeFormat::Ac3, Codec::Ac3), (EncodeFormat::Eac3, Codec::Eac3)] {
            let modes = [Mono, Stereo, ThreeFront, StereoSurround, ThreeFrontSurround, StereoTwoSurround, ThreeFrontTwoSurround, Surround71];
            let mut encoder: Vec<u32> = modes
                .iter()
                .flat_map(|&mode| [false, true].map(|lfe| Config { codec, sample_rate: 48_000, bitrate_kbps: 640, mode, lfe }))
                .filter(|config| config.validate().is_ok() && config.channels() == usize::from(channels))
                .filter_map(|config| config.channel_layout().mask())
                .collect();
            let mut listed = format.layout_masks(channels).to_vec();
            encoder.sort_unstable();
            listed.sort_unstable();
            assert_eq!(listed, encoder, "{format:?}, {channels} channels");
            for &mask in &listed {
                let layout = ChannelLayout::from_mask(mask);
                assert_eq!(ac3_config(codec, 48_000, &layout, None).unwrap().0.channel_layout(), layout);
            }
        }
    }
}

/// The sample rates `EncodeFormat` lists are the encoders'.
#[cfg(all(feature = "aac-encode", feature = "mp3-encode"))]
#[test]
fn the_listed_sample_rates_are_the_encoders() {
    let sorted = |rates: &[u32]| {
        let mut rates = rates.to_vec();
        rates.sort_unstable();
        rates
    };
    assert_eq!(EncodeFormat::Aac.sample_rates().unwrap(), sorted(&crate::aac::SAMPLE_RATES));
    assert_eq!(EncodeFormat::Mp3.sample_rates().unwrap(), sorted(&crate::mp3::SAMPLE_RATES));
}
