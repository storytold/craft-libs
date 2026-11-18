//! Writing the placed-layer blocks (`SoLd`, `PlLd`) of a [`SmartObject`], in the layout
//! Photoshop writes and [`SmartObject::from_placed_layer_data`] / [`SmartObject::from_placed_layer`]
//! read back. Came from PhotoCraft's `crates/io/src/smart_map/placed.rs`, on this crate's
//! placement types: the caller works out the quad and the warp from its own geometry.

use super::{SmartObject, Warp};
use crate::descriptor::{Descriptor, Id, ObjectArray, UnicodeString, Value, VersionedDescriptor};
use crate::io::WriteExt;

/// What a `SoLd` block holds besides the placement.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlacedLayerOptions<'a> {
    /// The placed layer's own id (`placed`), distinct from the linked file's id
    /// ([`SmartObject::uuid`]): a duplicated smart object shares the file but not this id.
    pub placed: &'a str,
    /// The content's resolution in pixels per inch (`Rslt`).
    pub resolution: f64,
    /// The smart filter stack (`filterFX`), when the layer has filters.
    pub filter_fx: Option<&'a Descriptor>,
    /// The descriptor the block was read with. Every key it has is kept, and only the placed id,
    /// the transform (when the quad moved), the warp and the filter stack are rewritten, so an
    /// unchanged layer writes back what it read.
    pub template: Option<&'a Descriptor>,
}

fn enumerated(type_id: &str, value: &str) -> Value {
    Value::Enumerated { type_id: Id::new(type_id), value: Id::new(value) }
}

fn ratio(n: i32, d: i32) -> Value {
    Value::Descriptor(Descriptor::new("null").with("numerator", Value::Integer(n)).with("denominator", Value::Integer(d)))
}

fn pad4(mut v: Vec<u8>) -> Vec<u8> {
    v.resize(v.len().next_multiple_of(4), 0);
    v
}

/// Replaces the value of `key`, or appends it.
fn set(d: &mut Descriptor, key: &str, v: Value) {
    match d.items.iter_mut().find(|(k, _)| k.is(key)) {
        Some(item) => item.1 = v,
        None => d.items.push((Id::new(key), v)),
    }
}

/// Replaces the value of `key`, or inserts it before `before` (appends when that is absent).
fn set_before(d: &mut Descriptor, key: &str, v: Value, before: &str) {
    if let Some(item) = d.items.iter_mut().find(|(k, _)| k.is(key)) {
        item.1 = v;
        return;
    }
    let at = d.items.iter().position(|(k, _)| k.is(before)).unwrap_or(d.items.len());
    d.items.insert(at, (Id::new(key), v));
}

impl Warp {
    /// Whether a custom warp's mesh has the (3·cols+1)×(3·rows+1) control points its patches
    /// need. Other styles have no mesh and are always valid.
    pub fn mesh_is_valid(&self) -> bool {
        if self.style != "warpCustom" {
            return true;
        }
        let n = self.cols.checked_mul(3).zip(self.rows.checked_mul(3)).and_then(|(c, r)| c.checked_add(1)?.checked_mul(r.checked_add(1)?));
        self.cols > 0 && self.rows > 0 && n == Some(self.mesh.len())
    }

    /// A custom warp of more than one patch: `SoLd` stores it as `quiltWarp`, `PlLd` cannot.
    fn is_split(&self) -> bool {
        self.style == "warpCustom" && self.mesh_is_valid() && (self.cols, self.rows) != (1, 1)
    }

    /// No warp over `bounds`.
    fn none(bounds: [f64; 4]) -> Warp {
        Warp { style: "warpNone".into(), bounds, cols: 1, rows: 1, ..Warp::default() }
    }

    /// Style, bend, distortions, axis, bounds and order: the keys a `warp` and a `quiltWarp`
    /// share, in Photoshop's order.
    fn head(&self, class: &str) -> Descriptor {
        let [l, t, r, b] = self.bounds;
        let bounds = Descriptor::new("classFloatRect")
            .with("Top ", Value::Double(t))
            .with("Left", Value::Double(l))
            .with("Btom", Value::Double(b))
            .with("Rght", Value::Double(r));
        Descriptor::new(class)
            .with("warpStyle", enumerated("warpStyle", &self.style))
            .with("warpValue", Value::Double(self.bend))
            .with("warpPerspective", Value::Double(self.horizontal))
            .with("warpPerspectiveOther", Value::Double(self.vertical))
            .with("warpRotate", enumerated("Ornt", if self.rotate_vertical { "Vrtc" } else { "Hrzn" }))
            .with("bounds", Value::Descriptor(bounds))
            .with("uOrder", Value::Integer(4))
            .with("vOrder", Value::Integer(4))
    }

    fn mesh_points(&self) -> Value {
        let unfl = |values: Vec<f64>| Value::UnitFloats { unit: *b"#Pxl", values };
        let body = Descriptor::new("rationalPoint")
            .with("Hrzn", unfl(self.mesh.iter().map(|p| p[0]).collect()))
            .with("Vrtc", unfl(self.mesh.iter().map(|p| p[1]).collect()));
        Value::ObjectArray(ObjectArray { prefix: self.mesh.len() as u32, body })
    }

    /// The `warp` descriptor. A one-patch custom warp holds its mesh here; a split one is written
    /// as no warp (`SoLd` keeps it in [`Self::quilt_descriptor`]), and so is a custom warp whose
    /// mesh is not valid ([`Self::mesh_is_valid`]).
    pub fn to_descriptor(&self) -> Descriptor {
        if self.style == "warpCustom" && (self.is_split() || !self.mesh_is_valid()) {
            return Warp::none(self.bounds).head("warp");
        }
        let d = self.head("warp");
        if self.style != "warpCustom" {
            return d;
        }
        d.with("customEnvelopeWarp", Value::Descriptor(Descriptor::new("customEnvelopeWarp").with("meshPoints", self.mesh_points())))
    }

    /// The `quiltWarp` descriptor of a split warp, in Photoshop's layout: the control points,
    /// `deformNumCols` × `deformNumRows` of them, and the patch boundaries (`quiltSliceX` /
    /// `quiltSliceY`) evenly across the warp's bounds. `None` for any other warp.
    pub fn quilt_descriptor(&self) -> Option<Descriptor> {
        if !self.is_split() {
            return None;
        }
        let [l, t, r, b] = self.bounds;
        let slices = |key: &str, n: usize, lo: f64, hi: f64| {
            let (lo, hi) = if (hi - lo).is_finite() && hi > lo { (lo, hi) } else { (0.0, 1.0) };
            let values: Vec<f64> = (0..=n).map(|i| lo + (hi - lo) * i as f64 / n as f64).collect();
            let prefix = values.len() as u32;
            Value::ObjectArray(ObjectArray { prefix, body: Descriptor::new("UntF").with(key, Value::UnitFloats { unit: *b"#Pxl", values }) })
        };
        let envelope = Descriptor::new("customEnvelopeWarp")
            .with("quiltSliceX", slices("quiltSliceX", self.cols, l, r))
            .with("quiltSliceY", slices("quiltSliceY", self.rows, t, b))
            .with("meshPoints", self.mesh_points());
        let count = |n: usize| Value::Integer(i32::try_from(n.saturating_mul(3).saturating_add(1)).unwrap_or(i32::MAX));
        Some(
            self.head("quiltWarp")
                .with("deformNumRows", count(self.rows))
                .with("deformNumCols", count(self.cols))
                .with("customEnvelopeWarp", Value::Descriptor(envelope)),
        )
    }
}

impl SmartObject {
    fn quad_value(&self) -> Value {
        Value::List(self.quad.iter().flatten().map(|&v| Value::Double(v)).collect())
    }

    /// Whether the quad differs from the `Trnf` a template stores (beyond rounding).
    fn quad_moved(&self, template: &Descriptor) -> bool {
        let old = template.get_numbers("Trnf");
        old.len() != 8 || old.iter().zip(self.quad.iter().flatten()).any(|(a, b)| (a - b).abs() > 1e-6 * (1.0 + a.abs()))
    }

    /// The warp written, over the content (or `[0, 0, 0, 0]` without a size) when there is none.
    fn warp_or_none(&self) -> Warp {
        let [w, h] = self.size.unwrap_or([0.0; 2]);
        self.warp.clone().unwrap_or_else(|| Warp::none([0.0, 0.0, w, h]))
    }

    /// The data of a `SoLd` block (`soLD`, version 4, a version-16 descriptor, padded to 4).
    /// `Trnf` and `nonAffineTransform` are both the quad; a split warp goes in `quiltWarp` with
    /// `warp` holding no warp, as Photoshop writes it.
    pub fn to_placed_layer_data(&self, options: &PlacedLayerOptions<'_>) -> Vec<u8> {
        let warp = self.warp_or_none();
        let quilt = warp.quilt_descriptor();
        let d = match options.template {
            Some(t) => {
                let mut d = t.clone();
                if t.get_text("placed").as_deref() != Some(options.placed) {
                    set(&mut d, "placed", Value::Text(UnicodeString::new_nul(options.placed)));
                }
                if self.quad_moved(t) {
                    set(&mut d, "Trnf", self.quad_value());
                    set(&mut d, "nonAffineTransform", self.quad_value());
                }
                set(&mut d, "warp", Value::Descriptor(warp.to_descriptor()));
                // The imported `quiltWarp` stays while it still says what the warp is.
                let kept = t.get_object("quiltWarp").is_some_and(|q| Warp::from_descriptor(q).as_ref() == self.warp.as_ref());
                if !kept {
                    match quilt {
                        Some(q) => set_before(&mut d, "quiltWarp", Value::Descriptor(q), "warp"),
                        None => d.items.retain(|(k, _)| !k.is("quiltWarp")),
                    }
                }
                match options.filter_fx {
                    Some(fx) => set(&mut d, "filterFX", Value::Descriptor(fx.clone())),
                    None => d.items.retain(|(k, _)| !k.is("filterFX")),
                }
                d
            }
            None => {
                let mut d = Descriptor::new("null")
                    .with("Idnt", Value::Text(UnicodeString::new_nul(&self.uuid)))
                    .with("placed", Value::Text(UnicodeString::new_nul(options.placed)))
                    .with("PgNm", Value::Integer(1))
                    .with("totalPages", Value::Integer(1))
                    .with("Crop", Value::Integer(1))
                    .with("frameStep", ratio(0, 600))
                    .with("duration", ratio(0, 600))
                    .with("frameCount", Value::Integer(1))
                    .with("Annt", Value::Integer(16))
                    .with("Type", Value::Integer(2))
                    .with("Trnf", self.quad_value())
                    .with("nonAffineTransform", self.quad_value());
                if let Some(q) = quilt {
                    d = d.with("quiltWarp", Value::Descriptor(q));
                }
                d = d.with("warp", Value::Descriptor(warp.to_descriptor()));
                if let Some([w, h]) = self.size {
                    d = d.with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(w)).with("Hght", Value::Double(h))));
                }
                d = d.with("Rslt", Value::UnitFloat { unit: *b"#Rsl", value: options.resolution });
                if let Some(fx) = options.filter_fx {
                    d = d.with("filterFX", Value::Descriptor(fx.clone()));
                }
                d.with("comp", Value::Integer(-1)).with(
                    "compInfo",
                    Value::Descriptor(Descriptor::new("null").with("compID", Value::Integer(-1)).with("originalCompID", Value::Integer(-1))),
                )
            }
        };
        let mut out = b"soLD".to_vec();
        out.put_u32(4);
        VersionedDescriptor::new(d).write(&mut out);
        pad4(out)
    }

    /// The data of a `PlLd` block (`plcL`, version 3), the legacy placed-layer record Photoshop
    /// writes next to `SoLd`: the id (cut to 255 bytes), page 1 of 1, anti-aliasing, a raster
    /// layer, the quad and the warp. A split warp cannot be stored here and is written as none.
    pub fn to_placed_layer(&self) -> Vec<u8> {
        let mut warp = self.warp_or_none();
        if warp.is_split() {
            warp = Warp::none(warp.bounds);
        }
        let mut out = b"plcL".to_vec();
        out.put_u32(3);
        let id = self.uuid.as_bytes().get(..self.uuid.len().min(255)).unwrap_or_default();
        out.put_u8(id.len() as u8);
        out.put(id);
        for v in [1i32, 1, 16, 2] {
            out.put_i32(v);
        }
        for &v in self.quad.iter().flatten() {
            out.put_f64(v);
        }
        out.put_u32(0);
        VersionedDescriptor::new(warp.to_descriptor()).write(&mut out);
        pad4(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tagged::TaggedBlock;

    fn grid(cols: usize, rows: usize, w: f64, h: f64) -> Vec<[f64; 2]> {
        let (nx, ny) = (3 * cols + 1, 3 * rows + 1);
        (0..ny).flat_map(|j| (0..nx).map(move |i| [i as f64 * w / (nx - 1) as f64 + (j % 2) as f64, j as f64 * h / (ny - 1) as f64])).collect()
    }

    fn placed(warp: Option<Warp>) -> SmartObject {
        SmartObject { uuid: "file-id".into(), quad: [[10.0, 20.0], [110.5, 20.0], [110.5, 70.25], [10.0, 70.25]], size: Some([200.0, 100.0]), warp }
    }

    fn warps() -> Vec<Option<Warp>> {
        let bounds = [0.0, 0.0, 200.0, 100.0];
        vec![
            None,
            Some(Warp {
                style: "warpArc".into(),
                bend: 25.0,
                horizontal: -10.0,
                vertical: 5.0,
                rotate_vertical: true,
                bounds,
                cols: 1,
                rows: 1,
                mesh: vec![],
            }),
            Some(Warp { style: "warpCustom".into(), bounds, cols: 1, rows: 1, mesh: grid(1, 1, 200.0, 100.0), ..Warp::default() }),
            Some(Warp { style: "warpCustom".into(), bounds, cols: 3, rows: 2, mesh: grid(3, 2, 200.0, 100.0), ..Warp::default() }),
        ]
    }

    #[test]
    fn placed_layer_data_reads_back_and_passes_the_strict_check() {
        let filter = Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Gaussian Blur")))
            .with("blendOptions", Value::Descriptor(Descriptor::new("blendOptions")))
            .with("enab", Value::Boolean(true))
            .with("filterID", Value::Integer(1197632612));
        let fx =
            Descriptor::new("filterFXStyle").with("enab", Value::Boolean(true)).with("filterFXList", Value::List(vec![Value::Descriptor(filter)]));
        for warp in warps() {
            let so = placed(warp.clone());
            for filter_fx in [None, Some(&fx)] {
                let data = so.to_placed_layer_data(&PlacedLayerOptions { placed: "layer-id", resolution: 300.0, filter_fx, template: None });
                assert_eq!(data.len() % 4, 0);
                assert_eq!(SmartObject::from_placed_layer_data(&data).unwrap(), so, "{warp:?}");
                TaggedBlock::new(*b"SoLd", data.clone()).check_structure().unwrap();
                let d = VersionedDescriptor::parse_prefix(&data[8..]).unwrap().0.descriptor;
                assert_eq!(d.get_text("placed").as_deref(), Some("layer-id"));
                assert_eq!(d.get("Rslt"), Some(&Value::UnitFloat { unit: *b"#Rsl", value: 300.0 }));
                assert_eq!(d.get("filterFX").is_some(), filter_fx.is_some());
                // A split warp is in `quiltWarp`, before a `warp` that holds none.
                let split = warp.as_ref().is_some_and(|w| (w.cols, w.rows) != (1, 1));
                assert_eq!(d.get("quiltWarp").is_some(), split);
                if split {
                    assert_eq!(d.get_object("warp").and_then(|w| w.get_enum("warpStyle")).as_deref(), Some("warpNone"));
                    let keys: Vec<&[u8]> = d.items.iter().map(|(k, _)| k.as_bytes()).collect();
                    assert!(keys.iter().position(|k| *k == b"quiltWarp") < keys.iter().position(|k| *k == b"warp"));
                }
            }
        }
    }

    #[test]
    fn placed_layer_reads_back_without_a_split_warp() {
        for warp in warps() {
            let so = placed(warp.clone());
            let data = so.to_placed_layer();
            assert_eq!(data.len() % 4, 0);
            let back = SmartObject::from_placed_layer(&data).unwrap();
            assert_eq!((back.uuid.as_str(), back.quad), ("file-id", so.quad));
            TaggedBlock::new(*b"PlLd", data.clone()).check_structure().unwrap();
            // The warp after the record: the same warp, or none for a split one.
            // Kind, version, the id, page fields, the quad and the warp version come first.
            let at = 4 + 4 + 1 + so.uuid.len() + 16 + 64 + 4;
            let stored = Warp::from_descriptor(&VersionedDescriptor::parse_prefix(&data[at..]).unwrap().0.descriptor);
            let split = warp.as_ref().is_some_and(|w| (w.cols, w.rows) != (1, 1));
            assert_eq!(stored, if split { None } else { warp });
        }
        let long = SmartObject { uuid: "x".repeat(300), ..placed(None) };
        assert_eq!(SmartObject::from_placed_layer(&long.to_placed_layer()).unwrap().uuid, "x".repeat(255));
    }

    #[test]
    fn a_custom_warp_with_a_wrong_sized_mesh_is_written_as_none() {
        let bad = Warp {
            style: "warpCustom".into(),
            bounds: [0.0, 0.0, 200.0, 100.0],
            cols: 2,
            rows: 1,
            mesh: grid(1, 1, 200.0, 100.0),
            ..Warp::default()
        };
        assert!(!bad.mesh_is_valid() && bad.quilt_descriptor().is_none());
        assert_eq!(bad.to_descriptor().get_enum("warpStyle").as_deref(), Some("warpNone"));
        let so = placed(Some(bad));
        let data = so.to_placed_layer_data(&PlacedLayerOptions { placed: "p", resolution: 72.0, ..Default::default() });
        assert_eq!(SmartObject::from_placed_layer_data(&data).unwrap().warp, None);
        assert!(Warp { cols: 0, ..Warp::default() }.mesh_is_valid(), "not custom");
        assert!(!Warp { style: "warpCustom".into(), cols: usize::MAX, rows: usize::MAX, ..Warp::default() }.mesh_is_valid());
    }

    #[test]
    fn a_template_keeps_its_keys_and_rewrites_only_what_changed() {
        let so = placed(warps()[1].clone());
        let fresh = so.to_placed_layer_data(&PlacedLayerOptions { placed: "layer-id", resolution: 300.0, ..Default::default() });
        let mut template = VersionedDescriptor::parse_prefix(&fresh[8..]).unwrap().0.descriptor;
        template.items.push((Id::new("vendorKey"), Value::Integer(7)));
        let same = PlacedLayerOptions { placed: "layer-id", resolution: 300.0, filter_fx: None, template: Some(&template) };
        // Unchanged: the template's own bytes come back.
        let mut expected = b"soLD".to_vec();
        expected.put_u32(4);
        VersionedDescriptor::new(template.clone()).write(&mut expected);
        assert_eq!(so.to_placed_layer_data(&same), pad4(expected));
        // A tiny rounding difference does not rewrite the transform.
        let mut nudged = so.clone();
        nudged.quad[0][0] += 1e-9;
        assert_eq!(nudged.to_placed_layer_data(&same), so.to_placed_layer_data(&same));
        // Moved, warped differently, duplicated: those keys change, the vendor key stays.
        let mut moved = placed(warps()[3].clone());
        moved.quad[1] = [150.0, 25.0];
        let data = moved.to_placed_layer_data(&PlacedLayerOptions { placed: "copy-id", ..same });
        let d = VersionedDescriptor::parse_prefix(&data[8..]).unwrap().0.descriptor;
        assert_eq!(d.get("vendorKey"), Some(&Value::Integer(7)));
        assert_eq!(d.get_text("placed").as_deref(), Some("copy-id"));
        assert_eq!(SmartObject::from_placed_layer_data(&data).unwrap(), moved);
        // Back to no split warp: `quiltWarp` goes.
        let unsplit = so.to_placed_layer_data(&PlacedLayerOptions { template: Some(&d), ..same });
        assert!(VersionedDescriptor::parse_prefix(&unsplit[8..]).unwrap().0.descriptor.get("quiltWarp").is_none());
    }
}
