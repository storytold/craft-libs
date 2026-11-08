// Test fixture construction and expected-success operations intentionally unwrap.
#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn reserved_bsid_versions_are_refused_and_clear_overlap() {
    for bsid in [9, 10] {
        let bytes = [0x0b, 0x77, 0, 0, 0, bsid << 3, 0x40, 0];
        assert!(matches!(parse_header(&bytes), Err(Error::Unsupported(_))));
        let mut decoder = Decoder::new();
        decoder.delay[0][0] = 1.0;
        assert!(decoder.decode(&bytes).is_err());
        assert!(decoder.delay.iter().flatten().all(|&x| x == 0.0));
    }
}

#[test]
fn inactive_fine_snr_offsets_do_not_change_zero_allocation() {
    let run = |stale: bool| {
        let mut frame = FrameState::new(1, 0);
        frame.endmant[0] = 73;
        frame.fgaincod[0] = 7;
        if stale {
            frame.fsnroffst[CPL] = 15;
            frame.fsnroffst[LFE] = 15;
            frame.fsnroffst[4] = 15;
        }
        let data = [0; 2048];
        let mut bits = Bits::new(&data);
        let mut coefs = [[0.0; 256]; 6];
        Decoder::new().audio_block(&mut bits, &mut frame, 1, false, 1, &mut coefs, None).unwrap();
        (frame.bap[0], bits.pos, coefs)
    };
    assert_eq!(run(false), run(true));
}

#[cfg(feature = "ac3-encode")]
#[test]
fn failed_later_block_clears_unpublished_overlap() {
    use super::encoder::{Codec, CodingMode, Config, Encoder};
    let mut encoder =
        Encoder::new(Config { codec: Codec::Ac3, sample_rate: 48_000, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false }).unwrap();
    let samples: Vec<f32> = (0..4096).map(|i| 0.3 * (i as f32 * 0.17).sin()).collect();
    let packets = encoder.push(&[&samples, &samples]).unwrap();
    let good = &packets[1].data;
    let mut decoder = Decoder::new();
    decoder.decode(good).unwrap();
    let mut bad = good.clone();
    let offset = decoder.block_bit_positions[1].1.div_ceil(8);
    bad[offset..].fill(0xff);
    assert!(decoder.decode(&bad).is_err());
    assert!(decoder.delay.iter().flatten().all(|&x| x == 0.0), "failed frame must not retain synthesized overlap");
    assert_eq!(decoder.decode(good).unwrap().channels, Decoder::new().decode(good).unwrap().channels);
}

#[test]
fn reserved_ac3_policy_preserves_normative_reduced_eac3_rates() {
    for (code, rate) in [(0, 24_000), (1, 22_050), (2, 16_000)] {
        // E.1.3.1.4 fscod=3 uses fscod2; this is distinct from reserved AC-3 bsid 9/10.
        let bytes = [0x0b, 0x77, 0, 3, 0xc4 | (code << 4), 16 << 3, 0, 0];
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.sample_rate, rate);
        assert_eq!(h.samples(), 1536);
        assert!(h.is_eac3());
    }
}
