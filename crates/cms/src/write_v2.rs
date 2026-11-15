//! Bounded ICC v2.1 matrix/TRC display-profile writer, extracted from TextureCraft.
//! Colour-space selection and transfer-curve sampling remain caller policy.

const D50_XYZ: [f64; 3] = [0.9642, 1.0, 0.8249];

/// One common red/green/blue transfer curve.
#[derive(Clone, Copy)]
pub enum ToneCurve<'a> {
    /// ICC gamma 1.0 (`curv` with one u8Fixed8 entry).
    Linear,
    /// Already quantized samples, inclusive endpoints, between 2 and 4096 entries.
    Samples(&'a [u16]),
}

/// Parameters for an ICC v2.1 RGB display profile.
pub struct MatrixProfile<'a> {
    /// RGB-to-XYZ matrix already adapted to PCS D50, in row-major order.
    pub matrix: [[f64; 3]; 3],
    /// Common RGB transfer curve; its storage is shared by all three tags.
    pub curve: ToneCurve<'a>,
    /// ASCII description, at most 1024 bytes with no embedded NUL.
    pub description: &'a str,
    /// ASCII copyright text, at most 1024 bytes with no embedded NUL.
    pub copyright: &'a str,
    /// Raw ICC date fields: year, month, day, hour, minute, second.
    pub date: [u16; 6],
}

fn s15f16(v: f64) -> [u8; 4] {
    ((v * 65536.0).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32).to_be_bytes()
}

fn xyz_tag(v: [f64; 3]) -> Vec<u8> {
    let mut b = b"XYZ \0\0\0\0".to_vec();
    for c in v {
        b.extend_from_slice(&s15f16(c));
    }
    b
}

fn text_tag(text: &str) -> Vec<u8> {
    let mut b = b"text\0\0\0\0".to_vec();
    b.extend_from_slice(text.as_bytes());
    b.push(0);
    b
}

/// `textDescriptionType` with the ASCII description only.
fn desc_tag(text: &str) -> Vec<u8> {
    let mut b = b"desc\0\0\0\0".to_vec();
    b.extend_from_slice(&(text.len() as u32 + 1).to_be_bytes());
    b.extend_from_slice(text.as_bytes());
    b.push(0);
    // No Unicode text and no Macintosh ScriptCode text: empty fields and the 67-byte filler.
    b.extend_from_slice(&[0; 4 + 4 + 2 + 1 + 67]);
    b
}

fn curve_tag(curve: ToneCurve<'_>) -> Vec<u8> {
    let mut b = b"curv\0\0\0\0".to_vec();
    match curve {
        ToneCurve::Linear => {
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&0x0100u16.to_be_bytes());
        }
        ToneCurve::Samples(samples) => {
            b.extend_from_slice(&(samples.len() as u32).to_be_bytes());
            for sample in samples {
                b.extend_from_slice(&sample.to_be_bytes());
            }
        }
    }
    b
}

/// Write a deterministic ICC v2.1 RGB display profile with one shared RGB TRC.
///
/// Returns `None` for nonfinite matrix values, descriptions longer than 1024 ASCII
/// bytes or containing NUL, or sampled curves outside 2..=4096 entries. These caps
/// bound every allocation independently of caller input. Matrix values outside the
/// signed 15.16 range saturate, matching the legacy exporter. The caller owns colour
/// adaptation, transfer sampling, names and timestamp policy.
pub fn matrix_profile(params: &MatrixProfile<'_>) -> Option<Vec<u8>> {
    let valid_text = |s: &str| s.len() <= 1024 && s.is_ascii() && !s.contains('\0');
    if !valid_text(params.description)
        || !valid_text(params.copyright)
        || !params.matrix.iter().flatten().all(|v| v.is_finite())
        || matches!(params.curve, ToneCurve::Samples(s) if !(2..=4096).contains(&s.len()))
    {
        return None;
    }
    let m = params.matrix;
    let column = |c: usize| [m[0][c], m[1][c], m[2][c]];
    let curve = curve_tag(params.curve);
    // The green and blue curves are the red one's entry.
    let tags: [(&[u8; 4], Vec<u8>); 9] = [
        (b"desc", desc_tag(params.description)),
        (b"cprt", text_tag(params.copyright)),
        (b"wtpt", xyz_tag(D50_XYZ)),
        (b"rXYZ", xyz_tag(column(0))),
        (b"gXYZ", xyz_tag(column(1))),
        (b"bXYZ", xyz_tag(column(2))),
        (b"rTRC", curve.clone()),
        (b"gTRC", curve.clone()),
        (b"bTRC", curve),
    ];
    // Header (128) + tag count (4) + 12 per tag, then the tag data, each tag padded to 4 bytes.
    let table_end = 128 + 4 + 12 * tags.len();
    let mut offsets = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    for (i, (_, bytes)) in tags.iter().enumerate() {
        let shared = i >= 7;
        let at = if shared { offsets.get(6).copied().unwrap_or(0) } else { table_end + data.len() };
        offsets.push(at);
        if !shared {
            data.extend_from_slice(bytes);
            while !data.len().is_multiple_of(4) {
                data.push(0);
            }
        }
    }
    let total = table_end + data.len();
    let mut out = vec![0u8; 128];
    out.get_mut(0..4)?.copy_from_slice(&(total as u32).to_be_bytes());
    out.get_mut(8..12)?.copy_from_slice(&[2, 0x10, 0, 0]); // version 2.1.0
    out.get_mut(12..16)?.copy_from_slice(b"mntr");
    out.get_mut(16..20)?.copy_from_slice(b"RGB ");
    out.get_mut(20..24)?.copy_from_slice(b"XYZ ");
    // Serialize caller-supplied ICC date fields; no calendar arithmetic.
    for (i, value) in params.date.iter().enumerate() {
        out.get_mut(24 + i * 2..26 + i * 2)?.copy_from_slice(&value.to_be_bytes());
    }
    out.get_mut(36..40)?.copy_from_slice(b"acsp");
    for (k, v) in D50_XYZ.iter().enumerate() {
        out.get_mut(68 + 4 * k..72 + 4 * k)?.copy_from_slice(&s15f16(*v));
    }
    out.extend_from_slice(&(tags.len() as u32).to_be_bytes());
    for ((sig, bytes), at) in tags.iter().zip(&offsets) {
        out.extend_from_slice(*sig);
        out.extend_from_slice(&(*at as u32).to_be_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    }
    out.extend_from_slice(&data);
    Some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn params() -> MatrixProfile<'static> {
        MatrixProfile {
            matrix: [[0.4361, 0.3851, 0.1431], [0.2225, 0.7169, 0.0606], [0.0139, 0.0971, 0.7141]],
            curve: ToneCurve::Linear,
            description: "v2",
            copyright: "free",
            date: [2026, 1, 1, 0, 0, 0],
        }
    }

    #[test]
    fn valid_profile_parses_and_aliases_curves() {
        for curve in [ToneCurve::Linear, ToneCurve::Samples(&[0, 32768, 65535])] {
            let bytes = matrix_profile(&MatrixProfile { curve, ..params() }).unwrap();
            let p = crate::Profile::parse(&bytes).unwrap();
            assert_eq!(p.version, (2, 0x10));
            assert_eq!(p.description, "v2");
            assert_eq!(&bytes[24..28], &[7, 234, 0, 1]);
            assert_eq!(&bytes[208..216], &bytes[220..228]);
            assert_eq!(&bytes[208..216], &bytes[232..240]);
            assert_eq!(bytes, matrix_profile(&MatrixProfile { curve, ..params() }).unwrap());
        }
    }

    #[test]
    fn bounded_inputs_reject_before_serializing() {
        let long = "x".repeat(1025);
        for description in [long.as_str(), "bad\0name", "non-ascii é"] {
            assert!(matrix_profile(&MatrixProfile { description, ..params() }).is_none());
        }
        for samples in [&[][..], &[0][..], &vec![0; 4097][..]] {
            assert!(matrix_profile(&MatrixProfile { curve: ToneCurve::Samples(samples), ..params() }).is_none());
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut p = params();
            p.matrix[0][0] = value;
            assert!(matrix_profile(&p).is_none());
        }
        let samples = vec![0; 4096];
        let name = "x".repeat(1024);
        let bytes = matrix_profile(&MatrixProfile { description: &name, copyright: &name, curve: ToneCurve::Samples(&samples), ..params() }).unwrap();
        assert!(bytes.len() < 12 * 1024);
    }
}
