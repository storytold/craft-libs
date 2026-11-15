//! Editable PSD Type Tool Object Setting (`TySh`) envelope, extracted from PhotoCraft.
//! App-specific text styles, layout, units and editing policy stay with the consumer.
//! Parsing intentionally retains PhotoCraft's optional-warp and partial-bounds recovery.
//! Existing read-only [`super::TypeLayer`] behavior is unchanged.

use crate::descriptor::{Descriptor, Id, Value as D, VersionedDescriptor};

/// Parsed `TySh` contents.
#[derive(Clone, Debug, PartialEq)]
pub struct TySh {
    pub transform: [f64; 6],
    pub text: Descriptor,
    pub warp: Option<Descriptor>,
    /// The four trailing big-endian f32 values (left, top, right, bottom) after the warp
    /// descriptor. Photoshop writes zeros for unwarped type and the text-space bounds (the
    /// `bounds` descriptor's rectangle) for warped type.
    pub bounds: [f32; 4],
}

/// Whether a type transform can reach rendering. Pixel coordinates use i32 in the originating PhotoCraft consumer, so non-finite and unrepresentable affine terms are rejected (#4758).
pub fn transform_is_safe(t: &[f64; 6]) -> bool {
    t.iter().all(|v| v.is_finite() && v.abs() <= f64::from(i32::MAX))
}

/// Parses a `TySh` block.
pub fn parse_tysh(data: &[u8]) -> Option<TySh> {
    let f = |at: usize| data.get(at..at + 8).map(|b| f64::from_be_bytes(b.try_into().unwrap_or([0; 8])));
    let mut m = [0.0; 6];
    for (i, v) in m.iter_mut().enumerate() {
        *v = f(2 + i * 8)?;
    }
    if !transform_is_safe(&m) {
        return None;
    }
    let (text, used) = VersionedDescriptor::parse_prefix(data.get(52..)?).ok()?;
    let rest = data.get(52 + used..).unwrap_or(&[]);
    let (warp, wused) = match rest.get(2..).and_then(|r| VersionedDescriptor::parse_prefix(r).ok()) {
        Some((w, n)) => (Some(w.descriptor), n + 2),
        None => (None, 0),
    };
    let tail = rest.get(wused..).unwrap_or(&[]);
    let mut bounds = [0f32; 4];
    for (i, b) in bounds.iter_mut().enumerate() {
        if let Some(x) = tail.get(i * 4..i * 4 + 4) {
            *b = f32::from_be_bytes(x.try_into().unwrap_or([0; 4]));
        }
    }
    Some(TySh { transform: m, text: text.descriptor, warp, bounds })
}

/// `tysh` moved by (`dx`, `dy`): only the transform's translation changes. The text, its styles,
/// the warp and the trailing bounds (which are in text space) stay byte for byte, so a moved
/// Photoshop type layer keeps everything Photoshop wrote. `None` when the block is too short.
pub fn translate_tysh(tysh: &[u8], dx: f64, dy: f64) -> Option<Vec<u8>> {
    let mut out = tysh.to_vec();
    // Version (2 bytes), then xx, xy, yx, yy, tx, ty as big-endian f64: tx at 34, ty at 42.
    for (at, d) in [(34, dx), (42, dy)] {
        let b = out.get_mut(at..at + 8)?;
        let v = f64::from_be_bytes(b.try_into().ok()?) + d;
        b.copy_from_slice(&v.to_be_bytes());
    }
    Some(out)
}

/// Serializes a `TySh` block.
pub fn write_tysh(t: &TySh) -> Vec<u8> {
    let mut v = 1u16.to_be_bytes().to_vec();
    for x in t.transform {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(&50u16.to_be_bytes());
    v.extend(VersionedDescriptor::new(t.text.clone()).to_bytes());
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend(VersionedDescriptor::new(t.warp.clone().unwrap_or_else(default_warp)).to_bytes());
    for b in t.bounds {
        v.extend_from_slice(&b.to_be_bytes());
    }
    v
}

fn default_warp() -> Descriptor {
    Descriptor::new("warp")
        .with("warpStyle", enumv("warpStyle", "warpNone"))
        .with("warpValue", D::Double(0.0))
        .with("warpPerspective", D::Double(0.0))
        .with("warpPerspectiveOther", D::Double(0.0))
        .with("warpRotate", enumv("Ornt", "Hrzn"))
}

fn enumv(t: &str, v: &str) -> D {
    D::Enumerated { type_id: Id::new(t), value: Id::new(v) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> TySh {
        TySh {
            transform: [1.0, 0.0, 0.0, 1.0, 25.0, 30.0],
            text: Descriptor::new("TxLr").with("EngineData", D::RawData(b"preserved".to_vec())),
            warp: Some(default_warp()),
            bounds: [-1.0, 2.0, 30.0, 40.0],
        }
    }
    #[test]
    fn roundtrip_preserves_descriptors_transform_and_bounds() {
        let t = sample();
        let b = write_tysh(&t);
        assert_eq!(parse_tysh(&b), Some(t));
        let shifted = translate_tysh(&b, 7.0, -3.0).unwrap();
        assert_eq!(&shifted[..34], &b[..34]);
        assert_eq!(&shifted[50..], &b[50..]);
        assert_eq!(parse_tysh(&shifted).unwrap().transform, [1.0, 0.0, 0.0, 1.0, 32.0, 27.0]);
    }
    #[test]
    fn truncations_and_unsafe_transforms_preserve_recovery_contract() {
        let mut t = sample();
        let bytes = write_tysh(&t);
        let (_, n) = VersionedDescriptor::parse_prefix(&bytes[52..]).unwrap();
        for cut in 0..52 + n {
            assert!(parse_tysh(&bytes[..cut]).is_none(), "{cut}");
        }
        let short = parse_tysh(&bytes[..52 + n]).unwrap();
        assert!(short.warp.is_none());
        assert_eq!(short.bounds, [0.0; 4]);
        for cut in 0..50 {
            assert!(translate_tysh(&bytes[..cut], 1.0, 2.0).is_none());
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::from(i32::MAX) + 1.0] {
            for i in 0..6 {
                t = sample();
                t.transform[i] = bad;
                assert!(parse_tysh(&write_tysh(&t)).is_none());
            }
        }
        t = sample();
        t.warp = None;
        assert!(parse_tysh(&write_tysh(&t)).unwrap().warp.is_some());
        let partial = parse_tysh(&bytes[..bytes.len() - 4]).unwrap();
        assert_eq!(partial.bounds, [-1.0, 2.0, 30.0, 0.0]);
    }
}
