//! Model segmentations stored with a mask ([`SegMask`]), so rendering never needs the model.

use std::cell::RefCell;
use std::collections::HashMap;

use serde::{Deserialize, Serialize, Serializer};

/// Logits are stored in steps of 1/`SCALE`, clamped to ±127/`SCALE` (≈ ±15.9: a sigmoid of
/// 1 − 1.2e-7, i.e. fully in or out).
const SCALE: f32 = 8.0;
/// Largest grid a stored segmentation may claim (hostile input caps the allocation).
pub const MAX_SIDE: usize = 1024;

/// A segmentation computed by a model (SAM 3 in `lightcraft-segment`): `side × side` logits
/// over the uncropped, oriented image stretched to a square (row `y`, column `x` covers
/// normalized `((x + 0.5) / side, (y + 0.5) / side)`), quantized to `i8`, deflated and base64
/// encoded. Coverage at a point is the sigmoid of the bilinearly sampled logit, so edges stay
/// smooth at any output size.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SegMask {
    pub side: u32,
    /// The grid's bytes (quantized, deflated, base64), unless this is a reference into the
    /// snapshot's shared grid table (`grid` is set and `data` is empty).
    #[serde(default)]
    pub data: String,
    /// The part of the image the grid covers, normalized `[x0, y0, x1, y1]` (`None` = all of
    /// it). A zoomed-in pass over one object covers just its surroundings, at a higher
    /// resolution than a pass over the whole photo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f64; 4]>,
    /// Index into the catalog snapshot's shared grid table (`Catalog::grids`) this mask refers
    /// to instead of carrying its own `data`. A serialization detail of snapshots only: the
    /// snapshot writer sets it (through the interner, into its own table), `Catalog::repair`
    /// resolves it when a snapshot is read, and no other serialization writes it. A `grid` that
    /// arrives from any other document (a command, a preset, XMP, a log record) is read but
    /// never written back, so it cannot point a snapshot at another mask's grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<u32>,
}

impl SegMask {
    /// From `side × side` logits over the whole image.
    pub fn from_logits(side: usize, logits: &[f32]) -> SegMask {
        let q: Vec<u8> = logits.iter().take(side * side).map(|l| ((l * SCALE).round().clamp(-127.0, 127.0) as i8) as u8).collect();
        let z = miniz_oxide::deflate::compress_to_vec(&q, 8);
        SegMask { side: side as u32, data: base64_encode(&z), rect: None, grid: None }
    }

    /// From `side × side` logits over the part `rect` of the image.
    pub fn from_logits_in(side: usize, logits: &[f32], rect: [f64; 4]) -> SegMask {
        SegMask { rect: Some(rect), ..SegMask::from_logits(side, logits) }
    }

    /// The covered part, normalized (the whole image without a `rect`); `None` when damaged.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let r = self.rect.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        (r.iter().all(|v| v.is_finite()) && r[2] > r[0] && r[3] > r[1]).then_some(r)
    }

    /// The logits, or `None` when the data is damaged or claims an absurd size.
    pub fn logits(&self) -> Option<Vec<f32>> {
        let side = self.side as usize;
        if side == 0 || side > MAX_SIDE {
            return None;
        }
        let z = base64_decode(&self.data)?;
        let q = miniz_oxide::inflate::decompress_to_vec_with_limit(&z, side * side).ok()?;
        (q.len() == side * side).then(|| q.iter().map(|b| f32::from(*b as i8) / SCALE).collect())
    }
}

// The grid table a snapshot serialization refers to: each distinct grid's data → its index.
thread_local! {
    static INTERNER: RefCell<Option<HashMap<String, u32>>> = const { RefCell::new(None) };
}

/// Serializes masks as references into a shared grid table (`grid` = index, `data` omitted)
/// while `f` runs, one thread at a time (snapshot writes). Nested calls are ignored (`None`):
/// the outermost writer owns the table.
pub fn with_grid_interner<T>(map: HashMap<String, u32>, f: impl FnOnce() -> T) -> Option<T> {
    INTERNER.with(|i| {
        if i.borrow().is_some() {
            return None;
        }
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                INTERNER.with(|j| *j.borrow_mut() = None);
            }
        }
        *i.borrow_mut() = Some(map);
        let _guard = Guard;
        Some(f())
    })
}

impl Serialize for SegMask {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        // While a snapshot is being written with a shared grid table, a mask whose exact bytes
        // are in the table refers to it by index instead of repeating the blob (issue storytold/lightcraft#2154).
        if !self.data.is_empty()
            && let Some(id) = INTERNER.with(|i| i.borrow().as_ref().and_then(|m| m.get(&self.data).copied()))
        {
            let len = if self.rect.is_some() { 3 } else { 2 };
            let mut map = serializer.serialize_map(Some(len))?;
            map.serialize_entry("side", &self.side)?;
            map.serialize_entry("grid", &id)?;
            if let Some(rect) = self.rect {
                map.serialize_entry("rect", &rect)?;
            }
            return map.end();
        }
        // Without a table hit: exactly the pre-v10 derived form (field order and skips included),
        // so every serialization that doesn't dedup keeps its bytes (`hash64`, cache keys). `grid`
        // is never written here: a value read from some document is dropped on save, and only the
        // interner above can write a reference (into the table of the snapshot being written).
        let len = 2 + usize::from(self.rect.is_some());
        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("side", &self.side)?;
        map.serialize_entry("data", &self.data)?;
        if let Some(rect) = self.rect {
            map.serialize_entry("rect", &rect)?;
        }
        map.end()
    }
}

/// Most zoomed-in detail patches one AI mask component keeps (the app makes one; each costs a
/// decode per render, so a hostile document can't make rendering explode).
pub const MAX_DETAIL: usize = 4;

/// Deserialize a component's detail patches, keeping at most [`MAX_DETAIL`] (the rest are
/// skipped without being decoded).
pub(crate) fn de_detail<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<SegMask>, D::Error> {
    struct Capped;
    impl<'de> serde::de::Visitor<'de> for Capped {
        type Value = Vec<SegMask>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list of segmentations")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<SegMask>, A::Error> {
            let mut out = Vec::new();
            while out.len() < MAX_DETAIL {
                match seq.next_element::<SegMask>()? {
                    Some(m) => out.push(m),
                    None => return Ok(out),
                }
            }
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(out)
        }
    }
    d.deserialize_seq(Capped)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut padded = false;
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            padded = true;
            continue;
        }
        if padded {
            return None;
        }
        let v = B64.iter().position(|b| *b == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits == 6 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for n in 0..10 {
            let v: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&v)).unwrap(), v);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert!(base64_decode("TW!u").is_none());
        assert!(base64_decode("Z").is_none());
        assert!(base64_decode("Zm9vY").is_none());
        assert!(base64_decode("Zg==!!!not-base64").is_none());
        assert_eq!(base64_decode("Zg==\n").unwrap(), b"f");
    }

    #[test]
    fn logits_round_trip_quantized() {
        let side = 16;
        let l: Vec<f32> = (0..side * side).map(|i| (i as f32 - 128.0) / 4.0).collect();
        let m = SegMask::from_logits(side, &l);
        let back = m.logits().unwrap();
        for (a, b) in l.iter().zip(&back) {
            assert!((a.clamp(-127.0 / SCALE, 127.0 / SCALE) - b).abs() <= 0.5 / SCALE + 1e-6);
        }
    }

    #[test]
    fn a_hostile_document_keeps_at_most_max_detail_patches() {
        let patch = serde_json::to_value(SegMask::from_logits(4, &[1.0; 16])).unwrap();
        let shape = serde_json::json!({"kind": "object", "hint": [{"x": 0.5, "y": 0.5}], "detail": vec![patch; 10_000]});
        let s: crate::MaskShape = serde_json::from_value(shape).unwrap();
        let crate::MaskShape::Object { detail, .. } = s else { panic!("an Object") };
        assert_eq!(detail.len(), MAX_DETAIL);
        // and a whole develop document with them still loads
        let mut d = crate::DevelopSettings::default();
        d.masks.push(crate::Mask {
            id: 1,
            components: vec![crate::MaskComponent {
                name: None,
                op: crate::MaskOp::Add,
                invert: false,
                shape: crate::MaskShape::Prompt { text: "x".into(), seg: None, detail: vec![SegMask::from_logits(4, &[1.0; 16]); 50], edge: 0.0 },
            }],
            ..Default::default()
        });
        let back: crate::DevelopSettings = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        let crate::MaskShape::Prompt { detail, .. } = &back.masks[0].components[0].shape else { panic!("a Prompt") };
        assert_eq!(detail.len(), MAX_DETAIL);
    }

    #[test]
    fn damaged_data_is_none() {
        let mut m = SegMask::from_logits(8, &[1.0; 64]);
        m.side = 9;
        assert!(m.logits().is_none(), "size mismatch");
        m.side = 100_000;
        assert!(m.logits().is_none(), "absurd size");
        assert!(SegMask { side: 8, data: "not base64 !".into(), rect: None, grid: None }.logits().is_none());
        assert!(SegMask { rect: Some([0.5, 0.0, 0.2, 1.0]), ..SegMask::from_logits(2, &[0.0; 4]) }.bounds().is_none());
    }

    /// A `grid` read from a document is never written outside the snapshot interner, with or
    /// without data: the bytes stay the pre-v10 form, so `hash64` and cache keys don't depend on
    /// a stray value (issue storytold/lightcraft#2154).
    #[test]
    fn stray_grid_is_never_written_outside_the_interner() {
        let empty = SegMask { side: 4, data: String::new(), rect: None, grid: None };
        let stray = SegMask { grid: Some(3), ..empty.clone() };
        assert_eq!(serde_json::to_string(&stray).unwrap(), "{\"side\":4,\"data\":\"\"}");
        assert_eq!(serde_json::to_string(&stray).unwrap(), serde_json::to_string(&empty).unwrap());
        let with_data = SegMask { grid: Some(3), ..SegMask::from_logits(4, &[1.0; 16]) };
        assert!(!serde_json::to_string(&with_data).unwrap().contains("grid"), "no grid key outside the interner");
    }

    /// Without the snapshot interner the JSON is exactly the derived form it always was; with
    /// it, a pooled grid serializes as an index and the data is left to the table (issue storytold/lightcraft#2154).
    #[test]
    fn serialization_refs_only_under_the_interner() {
        let m = SegMask::from_logits(4, &[1.0; 16]);
        let plain = serde_json::to_string(&m).unwrap();
        assert_eq!(plain, format!("{{\"side\":4,\"data\":\"{}\"}}", m.data), "no interner: unchanged bytes");
        let with_rect = SegMask { rect: Some([0.0, 0.0, 0.5, 0.5]), ..m.clone() };
        assert_eq!(serde_json::to_string(&with_rect).unwrap(), format!("{{\"side\":4,\"data\":\"{}\",\"rect\":[0.0,0.0,0.5,0.5]}}", m.data));

        let mut table = std::collections::HashMap::new();
        table.insert(m.data.clone(), 3u32);
        let referenced = with_grid_interner(table, || serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(referenced, "{\"side\":4,\"grid\":3}", "pooled grid: an index, no data");
        // a grid the table doesn't hold, and an empty-data mask, stay inline
        let other = SegMask::from_logits(2, &[0.0; 4]);
        let mixed = with_grid_interner(std::collections::HashMap::new(), || serde_json::to_string(&other).unwrap()).unwrap();
        assert_eq!(mixed, format!("{{\"side\":2,\"data\":\"{}\"}}", other.data));
        // the interner is gone afterwards: serialization is inline again
        assert_eq!(serde_json::to_string(&m).unwrap(), plain);
        // nested calls are ignored: the outer writer keeps its table
        let outer_table = std::collections::HashMap::from([(m.data.clone(), 0u32)]);
        let nested = with_grid_interner(outer_table, || {
            let inner_table = std::collections::HashMap::from([(other.data.clone(), 1u32)]);
            (with_grid_interner(inner_table, || serde_json::to_string(&other).unwrap()), serde_json::to_string(&m).unwrap())
        })
        .unwrap();
        assert_eq!(nested.0, None, "the nested writer is refused");
        assert_eq!(nested.1, "{\"side\":4,\"grid\":0}");
    }
}
