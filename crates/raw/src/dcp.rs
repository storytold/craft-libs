//! DNG Camera Profiles (`.dcp`): the user's own camera colour profiles, read at run time from
//! files they supply (DNG specification, chapter 6 "Camera Profiles"). A `.dcp` is a TIFF-structured
//! file whose header carries the magic `RC` (0x4352) in place of 42, with the profile tags in IFD 0:
//! `UniqueCameraModel`, `ProfileName`, the colour/forward matrices and calibration illuminants, the
//! hue/saturation map, look table and tone curve. Nothing here ships a profile.

use craft_tiff::{Ifd, ParseOptions, Tiff, tags as t};

use crate::ColorData;

/// The magic number of a DNG camera profile file (`IIRC` / `MMCR`).
pub const DCP_MAGIC: u16 = 0x4352;

/// A parsed camera profile.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DcpProfile {
    /// `ProfileName` (empty when the file has none).
    pub name: String,
    /// `UniqueCameraModel`: the camera the profile was made for (empty when absent).
    pub camera_model: String,
    /// The colour model: illuminants, matrices and the profile look (hue/sat map, look table, tone curve).
    pub color: ColorData,
}

/// Errors reading a `.dcp`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DcpError {
    #[error("not a DNG camera profile (.dcp)")]
    NotDcp,
    #[error("damaged camera profile: {0}")]
    Damaged(String),
    #[error("the camera profile has no usable colour matrix")]
    NoMatrix,
}

/// Is `bytes` a DCP file (by its header)?
pub fn is_dcp(bytes: &[u8]) -> bool {
    matches!(bytes.get(0..4), Some([b'I', b'I', b'R', b'C'] | [b'M', b'M', b'C', b'R']))
}

/// The largest `.dcp` file read or parsed (real profiles are kilobytes to a few MB).
pub const MAX_DCP_BYTES: u64 = 64 << 20;

/// Read the `.dcp` file at `path`, reading at most [`MAX_DCP_BYTES`] + 1 bytes: a larger file is refused
/// before it is loaded into memory.
pub fn read_dcp(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX_DCP_BYTES.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DCP_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the camera profile is larger than 64 MiB"));
    }
    Ok(bytes)
}

/// Parse a `.dcp` file. Files above 64 MiB ([`MAX_DCP_BYTES`]) are refused.
pub fn parse_dcp(bytes: &[u8]) -> Result<DcpProfile, DcpError> {
    if !is_dcp(bytes) {
        return Err(DcpError::NotDcp);
    }
    if bytes.len() as u64 > MAX_DCP_BYTES {
        return Err(DcpError::Damaged("larger than 64 MiB".into()));
    }
    // the TIFF reader only knows 42 / BigTIFF / raw magics: give it a header with 42 in place of RC
    let mut data = bytes.to_vec();
    let forty_two: [u8; 2] = if bytes.first() == Some(&b'I') { [42, 0] } else { [0, 42] };
    if let Some(m) = data.get_mut(2..4) {
        m.copy_from_slice(&forty_two);
    }
    let opts = ParseOptions { follow_children: false, max_ifds: 4, value_budget: Some(64 << 20), ..Default::default() };
    let tiff = Tiff::parse_with(&data, &opts).map_err(|e| DcpError::Damaged(e.to_string()))?;
    let ifd: &Ifd = tiff.ifds.first().ok_or_else(|| DcpError::Damaged("no IFD".into()))?;
    let mut color = crate::dng::color_data(ifd, ifd, tiff.order);
    // a profile carries no white balance or calibration of a particular shot
    color.as_shot_neutral = None;
    color.as_shot_white_xy = None;
    color.analog_balance = None;
    color.camera_calibration = [None, None];
    color.baseline_sharpness = None;
    color.profile.gain_table_map = None;
    // only BaselineExposureOffset belongs to a profile (BaselineExposure is the camera's)
    color.baseline_exposure =
        ifd.f64s(t::BASELINE_EXPOSURE_OFFSET).and_then(|v| v.first().copied()).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-4.0, 4.0);
    // a single-illuminant profile names one illuminant; its second matrix slot stays empty
    if color.color_matrix[0].is_none() {
        // the first slot is the one every profile must fill (DNG: ColorMatrix1 is required)
        return Err(DcpError::NoMatrix);
    }
    let text = |tag: u16| ifd.string(tag).map(|s| s.trim_matches(char::from(0)).trim().to_string()).unwrap_or_default();
    Ok(DcpProfile { name: text(t::PROFILE_NAME), camera_model: text(t::UNIQUE_CAMERA_MODEL), color })
}

/// Write `p` as a `.dcp` file (little-endian): its name, camera, illuminants, matrices, hue/saturation
/// map, look table, tone curve and baseline exposure offset.
pub fn write_dcp(p: &DcpProfile) -> Result<Vec<u8>, DcpError> {
    use craft_tiff::writer::srational;
    use craft_tiff::{ByteOrder, IfdBuilder, TiffWriter, Value};
    let c = &p.color;
    // ColorMatrix1 must remain usable after TIFF rational rounding, or our reader rejects it.
    let first = c.color_matrix[0].as_ref().ok_or(DcpError::NoMatrix)?;
    let serialized = first
        .iter()
        .flatten()
        .map(|v| {
            let (n, d) = srational(*v);
            f64::from(n) / f64::from(d)
        })
        .collect();
    if first.iter().flatten().any(|v| !v.is_finite()) || crate::dng::mat3(Some(serialized)).is_none() {
        return Err(DcpError::NoMatrix);
    }
    let mut ifd = IfdBuilder::new();
    ifd.set(t::UNIQUE_CAMERA_MODEL, Value::Ascii(p.camera_model.clone()));
    ifd.set(t::PROFILE_NAME, Value::Ascii(p.name.clone()));
    let mat = |m: &crate::Matrix3| Value::SRational(m.iter().flatten().map(|v| srational(*v)).collect());
    for i in 0..2 {
        if let Some(m) = &c.color_matrix[i] {
            ifd.set([t::CALIBRATION_ILLUMINANT_1, t::CALIBRATION_ILLUMINANT_2][i], Value::Short(vec![c.illuminant[i]]));
            ifd.set([t::COLOR_MATRIX_1, t::COLOR_MATRIX_2][i], mat(m));
        }
        if let Some(m) = &c.forward_matrix[i] {
            ifd.set([t::FORWARD_MATRIX_1, t::FORWARD_MATRIX_2][i], mat(m));
        }
    }
    let flat = |table: &crate::profile::HsvTable| Value::Float(table.data.iter().flatten().copied().collect());
    let dims =
        |table: &crate::profile::HsvTable| Value::Long(vec![table.hue_divisions as u32, table.sat_divisions as u32, table.val_divisions as u32]);
    if let Some(a) = c.profile.hue_sat_map[0].as_ref() {
        ifd.set(t::PROFILE_HUE_SAT_MAP_DIMS, dims(a));
        ifd.set(t::PROFILE_HUE_SAT_MAP_DATA_1, flat(a));
        if a.srgb_value {
            ifd.set(t::PROFILE_HUE_SAT_MAP_ENCODING, Value::Long(vec![1]));
        }
        if let Some(b) = c.profile.hue_sat_map[1]
            .as_ref()
            .filter(|b| (b.hue_divisions, b.sat_divisions, b.val_divisions) == (a.hue_divisions, a.sat_divisions, a.val_divisions))
        {
            ifd.set(t::PROFILE_HUE_SAT_MAP_DATA_2, flat(b));
        }
    }
    if let Some(l) = &c.profile.look_table {
        ifd.set(t::PROFILE_LOOK_TABLE_DIMS, dims(l));
        ifd.set(t::PROFILE_LOOK_TABLE_DATA, flat(l));
        if l.srgb_value {
            ifd.set(t::PROFILE_LOOK_TABLE_ENCODING, Value::Long(vec![1]));
        }
    }
    if let Some(tc) = &c.profile.tone_curve {
        ifd.set(t::PROFILE_TONE_CURVE, Value::Float(tc.points.iter().flatten().copied().collect()));
    }
    if c.baseline_exposure != 0.0 {
        ifd.set(t::BASELINE_EXPOSURE_OFFSET, Value::SRational(vec![srational(c.baseline_exposure)]));
    }
    let mut out = TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).map_err(|e| DcpError::Damaged(e.to_string()))?;
    // the DCP magic in place of 42
    if let Some(m) = out.get_mut(2..4) {
        m.copy_from_slice(&DCP_MAGIC.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn writing_requires_the_first_color_matrix() {
        let mut p = DcpProfile { name: "test".into(), camera_model: "test".into(), color: ColorData::default() };
        assert_eq!(write_dcp(&p), Err(DcpError::NoMatrix));
        let matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        p.color.color_matrix[1] = Some(matrix);
        assert_eq!(write_dcp(&p), Err(DcpError::NoMatrix));
        p.color.color_matrix[0] = Some(matrix);
        let back = parse_dcp(&write_dcp(&p).unwrap()).unwrap();
        assert_eq!(back.color.color_matrix, p.color.color_matrix);
    }

    #[test]
    fn writing_rejects_unusable_or_rounding_singular_first_matrices() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for matrix in [
            [[0.0; 3]; 3],
            [[1.0, 1.0, 0.0], [1.0, 1.0 + 1e-8, 0.0], [0.0, 0.0, 1.0]],
            [[f64::NAN, 0.0, 0.0], identity[1], identity[2]],
            [[f64::INFINITY, 0.0, 0.0], identity[1], identity[2]],
        ] {
            let mut p = DcpProfile { name: "test".into(), camera_model: "test".into(), color: ColorData::default() };
            p.color.color_matrix[0] = Some(matrix);
            assert_eq!(write_dcp(&p), Err(DcpError::NoMatrix));
        }
    }

    #[test]
    fn reading_a_profile_stops_past_the_size_limit() {
        let dir = std::env::temp_dir().join(format!("lc-read-dcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (fits, big) = (dir.join("fits.dcp"), dir.join("big.dcp"));
        std::fs::File::create(&fits).unwrap().set_len(MAX_DCP_BYTES).unwrap();
        std::fs::File::create(&big).unwrap().set_len(MAX_DCP_BYTES + 1).unwrap();
        assert_eq!(read_dcp(&fits).unwrap().len() as u64, MAX_DCP_BYTES);
        assert_eq!(read_dcp(&big).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A minimal single-illuminant profile (our own numbers) built in memory: no profile file is
    /// committed to the repository.
    pub(crate) fn synthetic_dcp(little: bool, with_hue_sat: bool) -> Vec<u8> {
        // entries: (tag, type, count, payload bytes)
        let mut entries: Vec<(u16, u16, u32, Vec<u8>)> = Vec::new();
        let u16b = |v: u16| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
        let u32b = |v: u32| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
        let srat = |v: f64| {
            let (n, d) = ((v * 10000.0).round() as i32, 10000i32);
            let mut b = if little { n.to_le_bytes().to_vec() } else { n.to_be_bytes().to_vec() };
            b.extend(if little { d.to_le_bytes() } else { d.to_be_bytes() });
            b
        };
        let ascii = |s: &str| {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            b
        };
        entries.push((t::UNIQUE_CAMERA_MODEL, 2, 13, ascii("Test Camera1")));
        let m = [0.9, -0.3, -0.05, -0.4, 1.2, 0.2, -0.05, 0.15, 0.7];
        entries.push((t::COLOR_MATRIX_1, 10, 9, m.iter().flat_map(|v| srat(*v)).collect()));
        entries.push((t::CALIBRATION_ILLUMINANT_1, 3, 1, [u16b(21), vec![0, 0]].concat()));
        entries.push((t::PROFILE_NAME, 2, 9, ascii("My Look")));
        if with_hue_sat {
            // 2 hues × 2 sats × 1 value: hue shift, sat scale, value scale per entry (FLOAT)
            entries.push((t::PROFILE_HUE_SAT_MAP_DIMS, 4, 3, [u32b(2), u32b(2), u32b(1)].concat()));
            let f = |v: f32| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
            entries.push((
                t::PROFILE_HUE_SAT_MAP_DATA_1,
                11,
                12,
                [f(0.0), f(1.0), f(1.0), f(5.0), f(1.1), f(1.0), f(0.0), f(1.0), f(1.0), f(-5.0), f(0.9), f(1.0)].concat(),
            ));
        }
        entries.sort_by_key(|e| e.0);
        let mut out = if little { b"IIRC".to_vec() } else { b"MMCR".to_vec() };
        out.extend(u32b(8));
        let n = entries.len();
        let mut data_off = 8 + 2 + n * 12 + 4;
        let mut ifd = u16b(n as u16);
        let mut tail = Vec::new();
        for (tag, ty, count, payload) in &entries {
            ifd.extend(u16b(*tag));
            ifd.extend(u16b(*ty));
            ifd.extend(u32b(*count));
            if payload.len() <= 4 {
                let mut p = payload.clone();
                p.resize(4, 0);
                ifd.extend(p);
            } else {
                ifd.extend(u32b(data_off as u32));
                tail.extend(payload);
                data_off += payload.len();
                if data_off % 2 == 1 {
                    tail.push(0);
                    data_off += 1;
                }
            }
        }
        ifd.extend(u32b(0));
        out.extend(ifd);
        out.extend(tail);
        out
    }

    #[test]
    fn reads_a_single_illuminant_profile_in_both_byte_orders() {
        for little in [true, false] {
            let p = parse_dcp(&synthetic_dcp(little, true)).unwrap();
            assert_eq!(p.name, "My Look");
            assert_eq!(p.camera_model, "Test Camera1");
            assert_eq!(p.color.illuminant, [21, 0]);
            let m = p.color.color_matrix[0].unwrap();
            assert!((m[1][1] - 1.2).abs() < 1e-4, "{m:?}");
            assert!(p.color.color_matrix[1].is_none());
            assert!(p.color.profile.hue_sat_map[0].is_some());
            assert!(p.color.as_shot_neutral.is_none());
        }
        assert!(parse_dcp(&synthetic_dcp(true, false)).unwrap().color.profile.hue_sat_map[0].is_none());
    }

    #[test]
    fn rejects_other_files_and_damage_without_panicking() {
        assert_eq!(parse_dcp(b"II*\0\x08\0\0\0"), Err(DcpError::NotDcp));
        assert_eq!(parse_dcp(b""), Err(DcpError::NotDcp));
        let good = synthetic_dcp(true, true);
        for cut in [4, 8, 10, 30, good.len() / 2, good.len() - 1] {
            let _ = parse_dcp(&good[..cut]);
        }
        let mut bad = good.clone();
        bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_dcp(&bad).is_err());
    }

    /// The hue/saturation map's header is the user's file too: dimensions far past the data, an entry count
    /// claiming 4 billion values, or a data offset past the end drop the map (the matrix still reads) instead of
    /// allocating for the claim or reading past the file.
    #[test]
    fn a_hostile_hue_sat_map_header_drops_the_map() {
        let good = synthetic_dcp(true, true);
        let at = |needle: &[u8]| good.windows(needle.len()).position(|w| w == needle).unwrap();
        let le = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let dims = at(&le(&[2, 2, 1]));
        // the map data's directory entry: tag, FLOAT, count 12, offset
        let entry = at(&[t::PROFILE_HUE_SAT_MAP_DATA_1.to_le_bytes(), 11u16.to_le_bytes()].concat());
        let patched = |at: usize, bytes: &[u8]| {
            let mut b = good.clone();
            b[at..at + bytes.len()].copy_from_slice(bytes);
            b
        };
        for (what, bytes) in [
            ("4096³ dimensions", patched(dims, &le(&[4096, 4096, 4096]))),
            ("dimensions past u32 products", patched(dims, &le(&[u32::MAX, u32::MAX, u32::MAX]))),
            ("a zero dimension", patched(dims, &le(&[0, 2, 1]))),
            ("4 billion values", patched(entry + 4, &le(&[u32::MAX]))),
            ("data past the end", patched(entry + 8, &le(&[u32::MAX - 16]))),
        ] {
            let p = parse_dcp(&bytes).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert!(p.color.profile.hue_sat_map[0].is_none(), "{what}: the map is dropped");
            assert!(p.color.color_matrix[0].is_some(), "{what}: the matrix still reads");
        }
    }

    #[test]
    fn written_profiles_read_back() {
        let p = parse_dcp(&synthetic_dcp(false, true)).unwrap();
        let back = parse_dcp(&write_dcp(&p).unwrap()).unwrap();
        assert_eq!(back.name, p.name);
        assert_eq!(back.camera_model, p.camera_model);
        assert_eq!(back.color.illuminant, p.color.illuminant);
        assert_eq!(back.color.profile.hue_sat_map, p.color.profile.hue_sat_map);
        let (a, b) = (back.color.color_matrix[0].unwrap(), p.color.color_matrix[0].unwrap());
        assert!(a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-6));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod boundary_tests {
    use super::*;
    use craft_tiff::{ByteOrder, IfdBuilder, TiffWriter, Value};

    fn profile_bytes(offset: f64, matrix: bool) -> Vec<u8> {
        let mut ifd = IfdBuilder::new();
        if matrix {
            ifd.set(t::COLOR_MATRIX_1, Value::SRational(vec![(1, 1), (0, 1), (0, 1), (0, 1), (1, 1), (0, 1), (0, 1), (0, 1), (1, 1)]));
        }
        ifd.set(t::BASELINE_EXPOSURE_OFFSET, Value::Double(vec![offset]));
        ifd.set(t::BASELINE_EXPOSURE, Value::Double(vec![10.0]));
        ifd.set(t::AS_SHOT_NEUTRAL, Value::Double(vec![0.5, 1.0, 0.7]));
        let mut bytes = TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap();
        bytes[2..4].copy_from_slice(&DCP_MAGIC.to_le_bytes());
        bytes
    }

    #[test]
    fn profile_requires_matrix_and_never_keeps_shot_exposure_or_white_balance() {
        assert_eq!(parse_dcp(&profile_bytes(0.0, false)), Err(DcpError::NoMatrix));
        for (offset, expected) in [(9.0, 4.0), (-9.0, -4.0), (f64::INFINITY, 0.0), (f64::NAN, 0.0)] {
            let profile = parse_dcp(&profile_bytes(offset, true)).unwrap();
            assert_eq!(profile.color.baseline_exposure, expected);
            assert_eq!(profile.color.as_shot_neutral, None);
            assert_eq!(profile.color.camera_calibration, [None, None]);
            assert_eq!(profile.color.profile.gain_table_map, None);
        }
    }
}
