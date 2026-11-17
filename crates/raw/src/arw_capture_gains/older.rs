//! The older Q12 record has its own layout. Do not reuse newer Q16 radius/center offsets.
use crate::lens_metadata::{SonyCameraSetting, SonyCaptureGainsQ12, SonyCaptureGainsQ12Layout};
use craft_tiff::{Ifd, Value};

fn encoded_record(note: &Ifd) -> Option<&[u8]> {
    let mut fields = note.entries.iter().filter(|e| e.tag == 0x9405);
    let Value::Undefined(encoded) = &fields.next()?.value else { return None };
    fields.next().is_none().then_some(encoded)
}

pub(crate) fn read(note: &Ifd) -> Option<SonyCaptureGainsQ12> {
    let encoded = encoded_record(note)?;
    if matches!(encoded.len(), 1770 | 1834 | 1836 | 1840) {
        return read_single_sequence(encoded);
    }
    if !matches!(encoded.len(), 1992 | 2048 | 2300) {
        return None;
    }
    let plain: Vec<u8> = encoded[..907].iter().map(|&b| crate::arw::DECIPHER[b as usize]).collect();
    let word = |p| u16::from_le_bytes([plain[p], plain[p + 1]]);
    // The RX10 shares the 1992 layout's gains and parameters. Its discriminator
    // is zero because this record carries no native attenuation table.
    let (layout, gain, params, native) = match (encoded.len(), plain[0]) {
        (2300, 0xa3) if plain[696..708] == [255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 255] => {
            (SonyCaptureGainsQ12Layout::Standard2300, 440, 708, 860)
        }
        (1992, 0x88) if plain[840..842] == [1, 0] => (SonyCaptureGainsQ12Layout::Legacy1992, 452, 720, 842),
        (1992, 0x88) if plain[840..842] == [0, 0] && (0..32).map(|i| word(720 + i * 2)).eq(RX10_PARAMETERS.into_iter().flatten()) => {
            (SonyCaptureGainsQ12Layout::Legacy1992, 452, 720, 842)
        }
        (2048, revision @ (0x89 | 0x8a | 0x8b | 0x8e | 0x90 | 0x91)) => {
            let (gain, native) = match revision {
                0x89 | 0x8a => (452, 842),
                0x8b => (452, 848),
                0x8e => (452, 872),
                _ => (440, 860),
            };
            // The ten-byte sentinel is stable; the following bytes vary by capture.
            // Do not treat the opaque parameter groups as known centers or radius scales.
            if plain[gain + 256..gain + 266] != [255; 10] || !matches!(plain[native + 32], 0 | 1) {
                return None;
            }
            (SonyCaptureGainsQ12Layout::Legacy2048 { revision }, gain, gain + 268, native)
        }
        _ => return None,
    };
    let gains_q12 = std::array::from_fn(|plane| std::array::from_fn(|knot| word(gain + plane * 64 + knot * 2)));
    if gains_q12.iter().any(|g| g[0] != 4096 || g.iter().any(|&v| !(4096..=32768).contains(&v))) {
        return None;
    }
    let native_present = match layout {
        SonyCaptureGainsQ12Layout::Legacy1992 => plain[840] == 1,
        SonyCaptureGainsQ12Layout::Legacy2048 { .. } => plain[native + 32] == 1,
        _ => true,
    };
    Some(SonyCaptureGainsQ12 {
        layout,
        source_bits: None,
        gains_q12,
        parameters: std::array::from_fn(|group| std::array::from_fn(|plane| word(params + group * 8 + plane * 2))),
        // On the measured 2048-byte layouts, marker 0 means that these bytes are not
        // an attenuation table. Preserve the gains without manufacturing a native copy.
        native_shading: native_present.then(|| std::array::from_fn(|i| word(native + 2 * i) as i16)),
        native_flags: [plain[native + 32], plain[native + 33], plain[native + 34]],
    })
}

/// On the measured RX1 pair, the Off and Auto files store the same
/// sequence. The descriptor and the independent camera setting agree
/// on whether it was used. RX1R revision 4 Auto carries either a unity
/// sequence or the applied non-unity sequence.
pub(crate) fn read_early_rx1(note: &Ifd, model: &str, setting: SonyCameraSetting) -> Option<SonyCaptureGainsQ12> {
    if !matches!(model, "DSC-RX1" | "DSC-RX1R") {
        return None;
    }
    let encoded = encoded_record(note)?;
    let lookup = &crate::arw::DECIPHER;
    let revision = lookup[*encoded.first()? as usize];
    let plain: Vec<u8> = encoded.get(1526..1610)?.iter().map(|&v| lookup[v as usize]).collect();
    let auto = match (model, encoded.len(), revision, &plain[..16], setting) {
        ("DSC-RX1", 1834, 3, [1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0], SonyCameraSetting::Off) => false,
        ("DSC-RX1", 1834, 3, [1, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0], SonyCameraSetting::Auto) => true,
        ("DSC-RX1R", 1838, 4, [1, 1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0], SonyCameraSetting::Auto) => true,
        _ => return None,
    };
    let nominal: [u16; 32] = std::array::from_fn(|i| u16::from_le_bytes([plain[20 + i * 2], plain[21 + i * 2]]));
    if nominal[0] != 4096 || nominal.windows(2).any(|w| w[0] > w[1]) || nominal[31] > 8000 {
        return None;
    }
    let gains = if auto { nominal } else { [4096; 32] };
    Some(SonyCaptureGainsQ12 {
        layout: SonyCaptureGainsQ12Layout::LegacySingle { bytes: encoded.len() as u16 },
        source_bits: None,
        gains_q12: [gains; 4],
        parameters: [[0; 4]; 8],
        native_shading: None,
        native_flags: [plain[1], plain[2], plain[11]],
    })
}

const RX10_PARAMETERS: [[u16; 4]; 8] = [[2748; 4], [1836; 4], [12336; 4], [48; 4], [48; 4], [0; 4], [0; 4], [0; 4]];

/// The A58 stores the nominal gain with either menu setting. Paired mosaics
/// show that Auto applies it and Off leaves it out. The A57 Off original
/// keeps the optical words and records a unity sequence.
pub(crate) fn read_slt_nominal(note: &Ifd, model: &str, setting: SonyCameraSetting) -> Option<SonyCaptureGainsQ12> {
    let bytes = match (model, setting) {
        ("SLT-A57", SonyCameraSetting::Off) => 1834,
        ("SLT-A58", _) => 1836,
        _ => return None,
    };
    let encoded = encoded_record(note)?;
    if encoded.len() != usize::from(bytes) {
        return None;
    }
    let lookup = &crate::arw::DECIPHER;
    if (lookup[encoded[0] as usize], lookup[encoded[1] as usize]) != (3, 0) {
        return None;
    }
    let plain: Vec<u8> = encoded[1526..1642].iter().map(|&v| lookup[v as usize]).collect();
    if !matches!(&plain[4..20], [1, 1, _, _, 0, 0, 1, 0 | 1, 1, 1, 1, 0, 0, _, 55, 0]) || !matches!(&plain[6..8], [1, 0] | [0, 1]) {
        return None;
    }
    let auto = match (&plain[..4], setting) {
        ([1, 1, 1, 1 | 2 | 4], SonyCameraSetting::Auto) => true,
        ([1, 0, 0, 1 | 2 | 3 | 5], SonyCameraSetting::Off) => false,
        _ => return None,
    };
    let word = |at| u16::from_le_bytes([plain[at], plain[at + 1]]);
    let nominal: [u16; 32] = std::array::from_fn(|i| word(20 + 2 * i));
    if nominal[0] != 4096 || nominal.windows(2).any(|w| w[0] > w[1]) || nominal[31] > 8192 || bytes == 1834 && nominal != [4096; 32] {
        return None;
    }
    let gain = if auto { nominal } else { [4096; 32] };
    Some(SonyCaptureGainsQ12 {
        layout: SonyCaptureGainsQ12Layout::LegacySingle { bytes },
        source_bits: None,
        gains_q12: [gain; 4],
        parameters: [[0; 4]; 8],
        native_shading: Some(std::array::from_fn(|i| word(84 + 2 * i) as i16)),
        native_flags: [plain[3], plain[9], plain[11]],
    })
}

fn read_single_sequence(encoded: &[u8]) -> Option<SonyCaptureGainsQ12> {
    let lookup = &crate::arw::DECIPHER;
    let signature = (encoded.len(), lookup[encoded[0] as usize], lookup[encoded[1] as usize]);
    if !matches!(signature, (1770 | 1834 | 1836, 3, 0) | (1840, 5, 0)) {
        return None;
    }
    let mut plain = [0; 116];
    for (dst, &src) in plain.iter_mut().zip(encoded.get(1526..1642)?) {
        *dst = lookup[src as usize];
    }
    let lens_descriptor = signature.1 == 3 && matches!(&plain[..16], [1, 1, 1, 1..=3, 1, 1, 1, 0, 0, 0, 1, 0 | 1, 1..=2, 1..=2, 1..=2, _]);
    // The A65/A77 chart pairs use the same gain and optical offsets.
    // The descriptors change when Shading or CA is disabled.
    let slt_descriptor = encoded.len() == 1770
        && matches!(&plain[12..20], [1, 1, 1, 0, 0, 253, 55, 0] | [1, 1, 1, 0, 0, 255, 63, 0])
        && matches!(
            &plain[..12],
            [1, 1, 1, 1..=4, 1, 1, 1, 0, 0, 0, 1, 0 | 1]
                | [1, 1, 1, 1..=4, 1, 1, 0, 1, 0, 0, 1, 0 | 1]
                | [1, 0, 0, 3 | 5 | 6, 1, 1, 1, 0, 0, 0, 1, 0 | 1]
        );
    // Fixed-lens descriptors carry a gain sequence here and store their
    // optical table in the TIFF lens fields.
    let fixed_lens_descriptor = matches!(
        (encoded.len(), &plain[..16]),
        (1834, [1, 1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0 | 1, 0, 0, 0, 0]) | (1840, [1, 1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 0, 0])
    );
    if !lens_descriptor && !slt_descriptor && !fixed_lens_descriptor {
        return None;
    }
    let word = |at| u16::from_le_bytes([plain[at], plain[at + 1]]);
    let gain = std::array::from_fn(|i| word(20 + 2 * i));
    if gain[0] != 4096 || gain.iter().any(|v| !(4096..=32768).contains(v)) || (slt_descriptor && plain[1] == 0 && gain != [4096; 32]) {
        return None;
    }
    Some(SonyCaptureGainsQ12 {
        layout: SonyCaptureGainsQ12Layout::LegacySingle { bytes: encoded.len() as u16 },
        source_bits: None,
        gains_q12: [gain; 4],
        parameters: [[0; 4]; 8],
        native_shading: (lens_descriptor || slt_descriptor).then(|| std::array::from_fn(|i| word(84 + 2 * i) as i16)),
        native_flags: [plain[3], plain[9], plain[11]],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::Entry;

    #[test]
    fn a58_needs_shading_auto_to_apply_the_nominal_gain() {
        let mut bytes = vec![0; 1836];
        bytes[0] = 3;
        let descriptor = [1, 1, 1, 4, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 1, 0, 0, 0xd9, 55, 0];
        bytes[1526..1546].copy_from_slice(&descriptor);
        for (block, value) in bytes[1546..1610].chunks_exact_mut(2).zip([4096u16; 31].into_iter().chain([5272])) {
            block.copy_from_slice(&value.to_le_bytes());
        }
        bytes[1630..1632].copy_from_slice(&8759u16.to_le_bytes());
        let auto = read_slt_nominal(&note(&bytes), "SLT-A58", SonyCameraSetting::Auto).unwrap();
        assert_eq!(auto.gains_q12[0][31], 5272);
        assert!(read_slt_nominal(&note(&bytes), "SLT-A58", SonyCameraSetting::Off).is_none());
        bytes[1527..1530].copy_from_slice(&[0, 0, 5]);
        let off = read_slt_nominal(&note(&bytes), "SLT-A58", SonyCameraSetting::Off).unwrap();
        assert_eq!(off.gains_q12, [[4096; 32]; 4]);
        assert_eq!(off.native_shading, auto.native_shading);

        bytes.truncate(1834);
        assert!(read_slt_nominal(&note(&bytes), "SLT-A57", SonyCameraSetting::Off).is_none());
        bytes[1608..1610].copy_from_slice(&4096u16.to_le_bytes());
        let a57 = read_slt_nominal(&note(&bytes), "SLT-A57", SonyCameraSetting::Off).unwrap();
        assert_eq!(a57.gains_q12, [[4096; 32]; 4]);
        assert_eq!(a57.native_shading, auto.native_shading);
    }

    #[test]
    fn slt_kit_descriptor_keeps_lens_words_through_ca_and_shading_switches() {
        let mut bytes = vec![0; 1770];
        bytes[0] = 3;
        let attenuation: [i16; 16] = [0, 20, 192, 532, 1044, 1792, 2708, 3708, 5012, 6704, 8960, 0, 0, 0, 0, 0];
        for (block, value) in bytes[1610..1642].chunks_exact_mut(2).zip(attenuation) {
            block.copy_from_slice(&value.to_le_bytes());
        }
        for (descriptor, last) in [
            ([1, 1, 1, 4, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 1, 0, 0, 253, 55, 0], 5415u16),
            ([1, 0, 0, 5, 1, 1, 1, 0, 0, 0, 1, 0, 1, 1, 1, 0, 0, 253, 55, 0], 4096),
        ] {
            bytes[1526..1546].copy_from_slice(&descriptor);
            for (block, value) in bytes[1546..1610].chunks_exact_mut(2).zip([4096u16; 31].into_iter().chain([last])) {
                block.copy_from_slice(&value.to_le_bytes());
            }
            let record = read(&note(&bytes)).unwrap();
            assert_eq!(record.gains_q12[0][31], last);
            assert_eq!(record.native_shading, Some(attenuation));
        }
        bytes[1608..1610].copy_from_slice(&5415u16.to_le_bytes());
        assert!(read(&note(&bytes)).is_none(), "Shading Off requires unity gain");
    }

    #[test]
    fn rx1_off_and_auto_store_the_same_nominal_sequence() {
        let mut plain = vec![0; 1834];
        plain[0] = 3;
        plain[1526..1542].copy_from_slice(&[1, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0]);
        for i in 0..32 {
            plain[1546 + i * 2..1548 + i * 2].copy_from_slice(&(4096 + i as u16 * 60).to_le_bytes());
        }
        let note = |plain: &[u8]| {
            let encoded = plain.iter().map(|b| crate::arw::DECIPHER.iter().position(|v| v == b).unwrap() as u8).collect();
            Ifd { entries: vec![Entry { tag: 0x9405, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() }
        };
        let auto = read_early_rx1(&note(&plain), "DSC-RX1", SonyCameraSetting::Auto).unwrap();
        assert_eq!(auto.gains_q12[0][31], 4096 + 31 * 60);
        assert!(read_early_rx1(&note(&plain), "DSC-RX1", SonyCameraSetting::Off).is_none());
        plain[1527..1529].fill(0);
        let off = read_early_rx1(&note(&plain), "DSC-RX1", SonyCameraSetting::Off).unwrap();
        assert_eq!(off.gains_q12, [[4096; 32]; 4]);
    }

    #[test]
    fn rx1r_auto_keeps_both_unity_and_non_unity_capture_sequences() {
        let mut plain = vec![0; 1838];
        plain[0] = 4;
        plain[1526..1542].copy_from_slice(&[1, 1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        for (block, gain) in plain[1546..1610].chunks_exact_mut(2).zip([4096u16; 32]) {
            block.copy_from_slice(&gain.to_le_bytes());
        }
        let unity = read_early_rx1(&note(&plain), "DSC-RX1R", SonyCameraSetting::Auto).unwrap();
        assert_eq!(unity.gains_q12, [[4096; 32]; 4]);
        for (block, i) in plain[1546..1610].chunks_exact_mut(2).zip(0..32u16) {
            block.copy_from_slice(&(4096 + 70 * i).to_le_bytes());
        }
        let applied = read_early_rx1(&note(&plain), "DSC-RX1R", SonyCameraSetting::Auto).unwrap();
        assert_eq!(applied.gains_q12[0][31], 6266);
        assert!(read_early_rx1(&note(&plain), "DSC-RX1R", SonyCameraSetting::Off).is_none());
    }

    fn plain() -> Vec<u8> {
        let mut bytes = vec![0; 2300];
        bytes[0] = 0xa3;
        bytes[696..708].copy_from_slice(&[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 255]);
        for plane in 0..4 {
            for knot in 0..32 {
                let at = 440 + plane * 64 + knot * 2;
                bytes[at..at + 2].copy_from_slice(&(4096u16 + (plane as u16 + 1) * knot as u16 * 12).to_le_bytes());
            }
        }
        for i in 0..32 {
            bytes[708 + i * 2..710 + i * 2].copy_from_slice(&(100 + i as u16).to_le_bytes());
        }
        for i in 0..16 {
            bytes[860 + i * 2..862 + i * 2].copy_from_slice(&(i as i16 * 400 - 1).to_le_bytes());
        }
        bytes[892..895].copy_from_slice(&[1, 1, 1]);
        bytes
    }

    fn note(bytes: &[u8]) -> Ifd {
        let encoded = bytes.iter().map(|&v| if v < 249 { (u32::from(v).pow(3) % 249) as u8 } else { v }).collect();
        Ifd { entries: vec![Entry { tag: 0x9405, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() }
    }

    #[test]
    fn single_sequence_retains_shared_gain_and_only_the_present_native_words() {
        let gains: [u16; 32] = std::array::from_fn(|i| 4096 + i as u16 * 12);
        let mut bytes = vec![0; 1834];
        bytes[0] = 3;
        bytes[1526..1542].copy_from_slice(&[1, 1, 1, 3, 1, 1, 1, 0, 0, 0, 1, 1, 2, 1, 2, 0]);
        for (block, gain) in bytes[1546..1610].chunks_exact_mut(2).zip(gains) {
            block.copy_from_slice(&gain.to_le_bytes());
        }
        bytes[1612..1614].copy_from_slice(&64_i16.to_le_bytes());
        let capture = read(&note(&bytes)).unwrap();
        assert_eq!(capture.layout, SonyCaptureGainsQ12Layout::LegacySingle { bytes: 1834 });
        assert_eq!(capture.gains_q12, [gains; 4]);
        assert_eq!(capture.native_shading.unwrap()[1], 64);

        bytes[1526..1542].copy_from_slice(&[1, 1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        let fixed = read(&note(&bytes)).unwrap();
        assert_eq!(fixed.gains_q12, [gains; 4]);
        assert_eq!(fixed.native_shading, None);
        bytes[1537] = 1;
        let rx100 = read(&note(&bytes)).unwrap();
        assert_eq!(rx100.gains_q12, [gains; 4]);
        assert_eq!(rx100.native_flags, [0, 1, 1]);
        assert!(rx100.native_shading.is_none());
        let mut rx100_ii = bytes.clone();
        rx100_ii.resize(1840, 0);
        assert!(read(&note(&rx100_ii)).is_none());
        rx100_ii[0] = 5;
        let next = read(&note(&rx100_ii)).unwrap();
        assert_eq!(next.layout, SonyCaptureGainsQ12Layout::LegacySingle { bytes: 1840 });
        assert_eq!(next.gains_q12, rx100.gains_q12);
        assert!(next.native_shading.is_none());
        rx100_ii[1537] = 0;
        assert!(read(&note(&rx100_ii)).is_none());
        bytes[1546] = 1;
        assert!(read(&note(&bytes)).is_none());
        bytes[1546] = 0;
        bytes[1535] = 0;
        assert!(read(&note(&bytes)).is_none());
    }

    fn legacy_plain() -> Vec<u8> {
        let standard = plain();
        let mut bytes = vec![0; 1992];
        bytes[0] = 0x88;
        bytes[452..708].copy_from_slice(&standard[440..696]);
        bytes[720..784].copy_from_slice(&standard[708..772]);
        bytes[840..842].copy_from_slice(&[1, 0]);
        bytes[842..877].copy_from_slice(&standard[860..895]);
        bytes
    }

    #[test]
    fn rx10_keeps_gains_when_legacy_1992_has_no_native_attenuation() {
        let mut bytes = legacy_plain();
        bytes[840] = 0;
        for (dst, value) in bytes[720..784].chunks_exact_mut(2).zip(RX10_PARAMETERS.into_iter().flatten()) {
            dst.copy_from_slice(&value.to_le_bytes());
        }
        let capture = read(&note(&bytes)).unwrap();
        assert_eq!(capture.layout, SonyCaptureGainsQ12Layout::Legacy1992);
        assert_eq!(capture.gains_q12, read(&note(&plain())).unwrap().gains_q12);
        assert_eq!(capture.parameters, RX10_PARAMETERS);
        assert!(capture.native_shading.is_none());
        bytes[720] ^= 1;
        assert!(read(&note(&bytes)).is_none());
    }

    fn revision_plain(revision: u8, native_present: bool) -> Vec<u8> {
        let standard = plain();
        let (gain, native) = match revision {
            0x89 | 0x8a => (452, 842),
            0x8b => (452, 848),
            0x8e => (452, 872),
            0x90 | 0x91 => (440, 860),
            _ => panic!("test revision"),
        };
        let mut bytes = vec![0; 2048];
        bytes[0] = revision;
        bytes[gain..gain + 266].copy_from_slice(&standard[440..706]);
        bytes[gain + 268..gain + 332].copy_from_slice(&standard[708..772]);
        bytes[native..native + 35].copy_from_slice(&standard[860..895]);
        if !native_present {
            bytes[native..native + 32].fill(231); // Absent slot is unrelated data, not zero attenuation.
            bytes[native + 32] = 0;
        }
        bytes
    }

    #[test]
    fn legacy_2048_rejects_unknown_layouts_and_bounds_without_scanning() {
        for revision in [0x89, 0x8a, 0x8b, 0x8e, 0x90, 0x91] {
            let bytes = revision_plain(revision, true);
            let (gain, native) = match revision {
                0x89 | 0x8a => (452, 842),
                0x8b => (452, 848),
                0x8e => (452, 872),
                _ => (440, 860),
            };
            let got = read(&note(&bytes)).unwrap();
            assert_eq!(got.gains_q12, read(&note(&plain())).unwrap().gains_q12);
            assert_eq!(got.native_shading, read(&note(&plain())).unwrap().native_shading);
            let absent = read(&note(&revision_plain(revision, false))).unwrap();
            assert!(absent.native_shading.is_none());
            assert_eq!(absent.gains_q12, got.gains_q12);
            assert!(!serde_json::to_value(&absent).unwrap().as_object().unwrap().contains_key("native_shading"));
            for len in [0, 906, 1992, 2047, 2049, 2300] {
                let mut bad = bytes.clone();
                bad.resize(len, 0);
                assert!(read(&note(&bad)).is_none());
            }
            for (offset, value) in [(0, 0x8c), (gain + 256, 0), (gain + 265, 0), (native + 32, 2)] {
                let mut bad = bytes.clone();
                bad[offset] = value;
                assert!(read(&note(&bad)).is_none(), "revision {revision}, offset {offset}");
            }
            for (offset, value) in [(gain, 4097u16), (gain + 64, 0), (gain + 254, 32769)] {
                let mut bad = bytes.clone();
                bad[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
                assert!(read(&note(&bad)).is_none());
            }
            let mut duplicate = note(&bytes);
            duplicate.entries.push(duplicate.entries[0].clone());
            assert!(read(&duplicate).is_none());
            duplicate.entries.truncate(1);
            duplicate.entries[0].value = Value::Byte(vec![0; 2048]);
            assert!(read(&duplicate).is_none());
        }
    }

    #[test]
    fn legacy_layout_rejects_other_payloads_and_keeps_serde_identity() {
        let b = legacy_plain();
        let got = read(&note(&b)).unwrap();
        assert_eq!(got.layout, SonyCaptureGainsQ12Layout::Legacy1992);
        assert_eq!(got.gains_q12, read(&note(&plain())).unwrap().gains_q12);
        assert_eq!(got.parameters, read(&note(&plain())).unwrap().parameters);
        assert_eq!(serde_json::from_str::<SonyCaptureGainsQ12>(&serde_json::to_string(&got).unwrap()).unwrap(), got);
        for len in 0..b.len() {
            assert!(read(&note(&b[..len])).is_none());
        }
        for len in [1993, 2048, 2300] {
            let mut bad = b.clone();
            bad.resize(len, 0);
            assert!(read(&note(&bad)).is_none());
        }
        for at in [0, 840, 841] {
            let mut bad = b.clone();
            bad[at] ^= 1;
            assert!(read(&note(&bad)).is_none());
        }
        for (at, value) in [(452, 4097u16), (516, 0), (704, 4095), (706, 32769)] {
            let mut bad = b.clone();
            bad[at..at + 2].copy_from_slice(&value.to_le_bytes());
            assert!(read(&note(&bad)).is_none());
        }
        let mut n = note(&b);
        n.entries.push(n.entries[0].clone());
        assert!(read(&n).is_none());
    }

    #[test]
    fn keeps_q12_planes_native_words_and_uninterpreted_parameters() {
        let mut b = plain();
        b[708..710].copy_from_slice(&u16::MAX.to_le_bytes());
        b[893] = 9; // Unknown flags remain diagnostic rather than becoming an inferred mode.
        let got = read(&note(&b)).unwrap();
        for plane in 0..4 {
            for knot in 0..32 {
                assert_eq!(got.gains_q12[plane][knot], 4096 + (plane as u16 + 1) * knot as u16 * 12);
            }
        }
        assert_eq!(got.parameters[0][0], u16::MAX);
        assert_eq!(got.parameters[7], [128, 129, 130, 131]);
        assert_eq!(got.native_shading, Some(std::array::from_fn(|i| i as i16 * 400 - 1)));
        assert_eq!(got.native_flags, [1, 9, 1]);
        assert_eq!(serde_json::from_str::<SonyCaptureGainsQ12>(&serde_json::to_string(&got).unwrap()).unwrap(), got);
    }

    #[test]
    fn original_cc0_fields_match_independent_q12_records() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-older-q12-layout-2026-11.json")))
                .unwrap();
        let records = fixture["records"].as_array().unwrap();
        assert_eq!(records.len(), 9);
        for sample in records {
            let mut encoded = vec![0; 2300];
            let fields = sample["encoded_ranges"].as_array().unwrap();
            assert_eq!(fields.len(), 3);
            for (field, (offset, count)) in fields.iter().zip([(0, 1), (440, 332), (860, 35)]) {
                assert_eq!(field["offset"].as_u64().unwrap() as usize, offset);
                let hex = field["hex"].as_str().unwrap();
                assert_eq!(hex.len(), count * 2);
                for i in 0..count {
                    encoded[offset + i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
                }
            }
            let n = Ifd { entries: vec![Entry { tag: 0x9405, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() };
            let expected: SonyCaptureGainsQ12 = serde_json::from_value(sample["record"].clone()).unwrap();
            assert_eq!(read(&n), Some(expected), "{}", sample["id"]);
        }
    }

    #[test]
    fn legacy_2048_original_fields_keep_revision_and_optional_native_table() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-q12-2048-layout-2026-11.json")))
                .unwrap();
        let rows = fixture["records"].as_array().unwrap();
        assert_eq!(rows.len(), 30);
        for row in rows {
            let mut encoded = vec![0; 2048];
            for field in row["encoded_ranges"].as_array().unwrap() {
                let offset = field["offset"].as_u64().unwrap() as usize;
                let hex = field["hex"].as_str().unwrap();
                for i in 0..hex.len() / 2 {
                    encoded[offset + i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
                }
            }
            let n = Ifd { entries: vec![Entry { tag: 0x9405, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() };
            let got = read(&n).expect("measured 2048-byte capture record");
            assert_eq!(serde_json::to_value(got).unwrap(), row["expected"], "{}", row["sample"]);
        }
    }

    #[test]
    fn legacy_1992_original_fields_keep_the_distinct_layout() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-q12-1992-layout-2026-11.json")))
                .unwrap();
        let rows = fixtures["records"].as_array().unwrap();
        assert_eq!(rows.len(), 5);
        for row in rows {
            let mut encoded = vec![0; 1992];
            for field in row["encoded_ranges"].as_array().unwrap() {
                let offset = field["offset"].as_u64().unwrap() as usize;
                let hex = field["hex"].as_str().unwrap();
                for i in 0..hex.len() / 2 {
                    encoded[offset + i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
                }
            }
            let n = Ifd { entries: vec![Entry { tag: 0x9405, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() };
            let got = read(&n).expect("measured legacy capture record");
            assert_eq!(serde_json::to_value(got).unwrap(), row["expected"], "{}", row["sample"]);
        }
    }

    #[test]
    fn shared_q12_layout_matches_original_fields_across_bodies() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-q12-shared-layout-2026-11.json")))
                .unwrap();
        let rows = fixtures["records"].as_array().unwrap();
        assert_eq!(rows.len(), 23);
        for row in rows {
            let mut encoded = vec![0; 2300];
            for field in row["encoded_ranges"].as_array().unwrap() {
                let offset = field["offset"].as_u64().unwrap() as usize;
                let hex = field["hex"].as_str().unwrap();
                for i in 0..hex.len() / 2 {
                    encoded[offset + i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
                }
            }
            let n = Ifd { entries: vec![Entry { tag: 0x9405, value: Value::Undefined(encoded), offset: 0 }], ..Default::default() };
            let expected: SonyCaptureGainsQ12 = serde_json::from_value(row["expected"].clone()).unwrap();
            assert_eq!(read(&n), Some(expected), "{}", row["sample"]);
        }
    }

    #[test]
    fn wrong_type_signature_and_duplicate_are_absent() {
        let b = plain();
        for at in [0, 696, 706, 707] {
            let mut bad = b.clone();
            bad[at] ^= 1;
            assert!(read(&note(&bad)).is_none());
        }
        let mut n = note(&b);
        n.entries.push(n.entries[0].clone());
        assert!(read(&n).is_none());
        let mut n = note(&b);
        n.entries[0].value = Value::Byte(vec![0; 2300]);
        assert!(read(&n).is_none());
    }

    #[test]
    fn truncation_extension_and_invalid_gain_are_absent() {
        let b = plain();
        for len in 0..b.len() {
            assert!(read(&note(&b[..len])).is_none());
        }
        let mut long = b.clone();
        long.push(0);
        assert!(read(&note(&long)).is_none());
        for (at, value) in [(440, 4097u16), (504, 0), (692, 4095), (694, 32769)] {
            let mut bad = b.clone();
            bad[at..at + 2].copy_from_slice(&value.to_le_bytes());
            assert!(read(&note(&bad)).is_none());
        }
        let old: crate::LensMetadata = serde_json::from_str("{}").unwrap();
        assert!(old.sony_capture_gains_q12.is_none());
    }

    fn fixture(order: craft_tiff::ByteOrder, include: bool, legacy: bool) -> Vec<u8> {
        fixture_payload(order, include, &if legacy { legacy_plain() } else { plain() })
    }

    fn fixture_payload(order: craft_tiff::ByteOrder, include: bool, plain: &[u8]) -> Vec<u8> {
        use craft_tiff::{IfdBuilder, ImageData, Tiff, TiffWriter, tags as t};
        let mut raw = IfdBuilder::new();
        raw.set(t::MAKE, Value::Ascii("SONY".into()));
        raw.set(t::MODEL, Value::Ascii("ILCE-7RM3".into()));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![32]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![24]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::COMPRESSION, Value::Short(vec![1]));
        let mut maker = Vec::new();
        order.put_u16(&mut maker, 1);
        order.put_u16(&mut maker, if include { 0x9405 } else { 0xffff });
        order.put_u16(&mut maker, 7);
        order.put_u32(&mut maker, plain.len() as u32);
        order.put_u32(&mut maker, 0);
        order.put_u32(&mut maker, 0);
        let n = note(plain);
        let Value::Undefined(payload) = &n.entries[0].value else { panic!("test payload") };
        maker.extend(payload);
        raw.set_child(t::EXIF_IFD, IfdBuilder::new().with(t::MAKER_NOTE, Value::Undefined(maker)));
        raw.set_image(ImageData::Strips { rows_per_strip: 24, strips: vec![vec![0; 32 * 24 * 2]] });
        let mut bytes = TiffWriter::new(order, false).write(&[raw]).unwrap();
        let tiff = Tiff::parse(&bytes).unwrap();
        let start = tiff.exif().unwrap().get(t::MAKER_NOTE).unwrap().offset as usize;
        let mut address = Vec::new();
        order.put_u32(&mut address, (start + 18) as u32);
        bytes[start + 10..start + 14].copy_from_slice(&address);
        bytes
    }

    #[test]
    fn legacy_2048_optional_native_tables_decode_in_both_tiff_orders() {
        for order in [craft_tiff::ByteOrder::Little, craft_tiff::ByteOrder::Big] {
            for revision in [0x89, 0x8a, 0x8b, 0x8e, 0x90, 0x91] {
                for present in [false, true] {
                    let plain = revision_plain(revision, present);
                    let bytes = fixture_payload(order, true, &plain);
                    let raw = crate::decode(&bytes).unwrap();
                    let header = crate::probe_info(&bytes).unwrap();
                    assert_eq!(header.lens_metadata, raw.lens_metadata);
                    assert_eq!(
                        header.lens_metadata.sony_capture_gains_q12,
                        read(&note(&plain)).map(|g| SonyCaptureGainsQ12 { source_bits: Some(16), ..g })
                    );
                }
            }
        }
    }

    #[test]
    fn header_and_full_decode_keep_q12_without_applying_it() {
        for order in [craft_tiff::ByteOrder::Little, craft_tiff::ByteOrder::Big] {
            for legacy in [false, true] {
                let bytes = fixture(order, true, legacy);
                let full = crate::decode(&bytes).unwrap();
                let header = crate::probe_info(&bytes).unwrap();
                assert_eq!(header.lens_metadata, full.lens_metadata);
                let capture = read(&note(&if legacy { legacy_plain() } else { plain() }));
                assert_eq!(header.lens_metadata.sony_capture_gains_q12, capture.map(|g| SonyCaptureGainsQ12 { source_bits: Some(16), ..g }));
                assert!(header.lens_metadata.sony_capture_gains.is_none());
                assert_eq!(full.data, crate::decode(&fixture(order, false, legacy)).unwrap().data);
                assert!(full.opcodes.list1.is_empty() && full.opcodes.list2.is_empty() && full.opcodes.list3.is_empty());
            }
        }
    }

    #[test]
    #[ignore = "set LIGHTCRAFT_SONY_Q12_INVENTORY to the local original-file survey"]
    fn original_q12_survey_matches_independent_records() {
        use sha2::{Digest, Sha256};
        let file = std::env::var("LIGHTCRAFT_SONY_Q12_INVENTORY").expect("local survey path");
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert!(!rows.is_empty());
        let mut samples = 0usize;
        for row in &rows {
            let path = row["path"].as_str().unwrap();
            let bytes = std::fs::read(path).unwrap();
            let hash = Sha256::digest(&bytes);
            let header = crate::probe_info(&bytes).unwrap();
            let raw = crate::decode(&bytes).unwrap();
            assert_eq!(header.lens_metadata, raw.lens_metadata, "{path}");
            let got = header.lens_metadata.sony_capture_gains_q12.as_ref().expect("older capture record");
            assert_eq!(serde_json::to_value(got.gains_q12).unwrap(), row["q12_gains"], "{path}");
            assert_eq!(serde_json::to_value(got.parameters).unwrap(), row["tail_words"], "{path}");
            assert_eq!(serde_json::to_value(got.native_shading).unwrap(), row["lens_words"], "{path}");
            assert!(header.lens_metadata.sony_capture_gains.is_none());
            samples += raw.width * raw.height * raw.cpp;
            assert_eq!(Sha256::digest(std::fs::read(path).unwrap()), hash, "original changed: {path}");
            eprintln!("Q12 original/header/full passed: {}", std::path::Path::new(path).file_name().unwrap().to_string_lossy());
        }
        eprintln!("Q12 survey: {} originals, {samples} samples, all hashes unchanged", rows.len());
    }
}
