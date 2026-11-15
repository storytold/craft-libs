//! Editable PSD Type Tool Object Setting (`TySh`) envelope, extracted from PhotoCraft.
//! App-specific text styles, layout, units and editing policy stay with the consumer.
//! Parsing intentionally retains PhotoCraft's optional-warp and partial-bounds recovery.
//! Existing read-only [`super::TypeLayer`] behavior is unchanged.

use crate::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D, VersionedDescriptor};
use crate::error::{PsdError, Result};

/// Maximum encoded envelope size, bounding hostile input and serialized output.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Parsed `TySh` contents.
#[derive(Clone, Debug, PartialEq)]
pub struct TySh {
    /// Affine transform in PSD order: xx, xy, yx, yy, tx, ty.
    pub transform: [f64; 6],
    /// Text descriptor, including text content and styling data.
    pub text: Descriptor,
    /// Optional warp descriptor retained from the Type Tool block.
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
    if data.len() > MAX_BYTES {
        return None;
    }
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
    if bounds.iter().any(|x| !x.is_finite()) {
        return None;
    }
    Some(TySh { transform: m, text: text.descriptor, warp, bounds })
}

/// `tysh` moved by (`dx`, `dy`): only the transform's translation changes. The text, its styles,
/// the warp and the trailing bounds (which are in text space) stay byte for byte, so a moved
/// Photoshop type layer keeps everything Photoshop wrote. `None` when the block is too short.
pub fn translate_tysh(tysh: &[u8], dx: f64, dy: f64) -> Option<Vec<u8>> {
    if tysh.len() > MAX_BYTES || tysh.len() < 50 {
        return None;
    }
    let mut transform = [0.0; 6];
    for (i, x) in transform.iter_mut().enumerate() {
        *x = f64::from_be_bytes(tysh.get(2 + i * 8..10 + i * 8)?.try_into().ok()?);
    }
    transform[4] += dx;
    transform[5] += dy;
    if !transform_is_safe(&transform) {
        return None;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(tysh.len()).ok()?;
    out.extend_from_slice(tysh);
    // Version (2 bytes), then xx, xy, yx, yy, tx, ty as big-endian f64: tx at 34, ty at 42.
    for (at, d) in [(34, dx), (42, dy)] {
        let b = out.get_mut(at..at + 8)?;
        let v = f64::from_be_bytes(b.try_into().ok()?) + d;
        b.copy_from_slice(&v.to_be_bytes());
    }
    Some(out)
}

/// Serializes a `TySh` block.
pub fn write_tysh(t: &TySh) -> Result<Vec<u8>> {
    if !transform_is_safe(&t.transform) || t.bounds.iter().any(|x| !x.is_finite()) {
        return Err(PsdError::LimitExceeded("TySh geometry"));
    }
    let default;
    let warp = match &t.warp {
        Some(w) => w,
        None => {
            default = default_warp();
            &default
        }
    };
    let mut budget = Budget(78); // versions, transform and trailing bounds
    budget.descriptor(&t.text, 0)?;
    budget.descriptor(warp, 0)?;
    let mut v = Vec::new();
    v.try_reserve_exact(budget.0).map_err(|_| PsdError::OutOfMemory("TySh output"))?;
    v.extend_from_slice(&1u16.to_be_bytes());
    for x in t.transform {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(&50u16.to_be_bytes());
    v.extend_from_slice(&16u32.to_be_bytes());
    t.text.write(&mut v);
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&16u32.to_be_bytes());
    warp.write(&mut v);
    for b in t.bounds {
        v.extend_from_slice(&b.to_be_bytes());
    }
    Ok(v)
}

// Count before cloning or serializing: bound both recursive trees and total output.
struct Budget(usize);
impl Budget {
    fn add(&mut self, n: usize) -> Result<()> {
        self.0 = self.0.checked_add(n).filter(|n| *n <= MAX_BYTES).ok_or(PsdError::LimitExceeded("TySh bytes"))?;
        Ok(())
    }
    fn unicode(&mut self, s: &UnicodeString) -> Result<()> {
        self.add(4)?;
        self.add(s.0.len().checked_mul(2).ok_or(PsdError::LimitExceeded("TySh string"))?)
    }
    fn id(&mut self, id: &Id) -> Result<()> {
        self.add(4)?;
        self.add(id.as_bytes().len())
    }
    fn class(&mut self, c: &Class) -> Result<()> {
        self.unicode(&c.name)?;
        self.id(&c.class_id)
    }
    fn descriptor(&mut self, d: &Descriptor, depth: usize) -> Result<()> {
        if depth > 32 {
            return Err(PsdError::LimitExceeded("TySh depth"));
        }
        self.unicode(&d.name)?;
        self.id(&d.class_id)?;
        self.add(4)?;
        for (k, v) in &d.items {
            self.id(k)?;
            self.value(v, depth + 1)?;
        }
        Ok(())
    }
    fn value(&mut self, v: &D, depth: usize) -> Result<()> {
        if depth > 32 {
            return Err(PsdError::LimitExceeded("TySh depth"));
        }
        self.add(4)?;
        match v {
            D::Descriptor(d) | D::GlobalObject(d) => self.descriptor(d, depth),
            D::ObjectArray(o) => {
                self.add(4)?;
                self.descriptor(&o.body, depth)
            }
            D::List(xs) => {
                self.add(4)?;
                for x in xs {
                    self.value(x, depth + 1)?;
                }
                Ok(())
            }
            D::Reference(xs) => {
                self.add(4)?;
                for x in xs {
                    self.add(4)?;
                    match x {
                        ReferenceItem::Property { class, key } => {
                            self.class(class)?;
                            self.id(key)?;
                        }
                        ReferenceItem::Class(c) => self.class(c)?,
                        ReferenceItem::Enumerated { class, type_id, value } => {
                            self.class(class)?;
                            self.id(type_id)?;
                            self.id(value)?;
                        }
                        ReferenceItem::Offset { class, .. } | ReferenceItem::Identifier { class, .. } | ReferenceItem::Index { class, .. } => {
                            self.class(class)?;
                            self.add(4)?;
                        }
                        ReferenceItem::Name { class, name } => {
                            self.class(class)?;
                            self.unicode(name)?;
                        }
                    }
                }
                Ok(())
            }
            D::Text(x) => self.unicode(x),
            D::Class(c) | D::GlobalClass(c) => self.class(c),
            D::Enumerated { type_id, value } => {
                self.id(type_id)?;
                self.id(value)
            }
            D::UnitFloats { values, .. } => {
                self.add(8)?;
                self.add(values.len().checked_mul(8).ok_or(PsdError::LimitExceeded("TySh values"))?)
            }
            D::RawData(x) | D::Alias(x) | D::Path(x) => {
                self.add(4)?;
                self.add(x.len())
            }
            D::Double(_) | D::LargeInteger(_) => self.add(8),
            D::UnitFloat { .. } => self.add(12),
            D::Integer(_) => self.add(4),
            D::Boolean(_) => self.add(1),
        }
    }
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
        let b = write_tysh(&t).unwrap();
        assert_eq!(parse_tysh(&b), Some(t));
        let shifted = translate_tysh(&b, 7.0, -3.0).unwrap();
        assert_eq!(&shifted[..34], &b[..34]);
        assert_eq!(&shifted[50..], &b[50..]);
        assert_eq!(parse_tysh(&shifted).unwrap().transform, [1.0, 0.0, 0.0, 1.0, 32.0, 27.0]);
    }
    #[test]
    fn truncations_and_unsafe_transforms_preserve_recovery_contract() {
        let mut t = sample();
        let bytes = write_tysh(&t).unwrap();
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
                assert!(write_tysh(&t).is_err());
            }
        }
        t = sample();
        t.warp = None;
        assert!(parse_tysh(&write_tysh(&t).unwrap()).unwrap().warp.is_some());
        let partial = parse_tysh(&bytes[..bytes.len() - 4]).unwrap();
        assert_eq!(partial.bounds, [-1.0, 2.0, 30.0, 0.0]);
    }
    #[test]
    fn hostile_output_and_translations_are_refused_before_copying() {
        let bytes = write_tysh(&sample()).unwrap();
        for bad in [f64::NAN, f64::INFINITY, f64::MAX] {
            assert!(translate_tysh(&bytes, bad, 0.0).is_none());
        }
        let mut t = sample();
        t.bounds[0] = f32::NAN;
        assert!(write_tysh(&t).is_err());
        t = sample();
        t.text = Descriptor::new("TxLr").with("data", D::RawData(vec![0; MAX_BYTES]));
        assert!(write_tysh(&t).is_err());
        let oversized = vec![0; MAX_BYTES + 1];
        assert!(translate_tysh(&oversized, 0.0, 0.0).is_none());
        assert!(parse_tysh(&oversized).is_none());
        let mut nested = D::Integer(1);
        for _ in 0..34 {
            nested = D::List(vec![nested]);
        }
        t = sample();
        t.text = Descriptor::new("TxLr").with("nest", nested);
        assert!(write_tysh(&t).is_err());
    }
}
