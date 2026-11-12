//! Type layers: the `TySh` block as a read-only view.
//!
//! Layout (Adobe's specification, "Type tool object setting"): a version, six doubles of
//! transform, a text version, a descriptor version and the text descriptor, whose `Txt ` is the
//! text, `EngineData` the text engine document ([`crate::engine_data`]) and `bounds` /
//! `boundingBox` the text's extent. The warp descriptor and the four bounds that follow are not
//! read here. The block stays raw in the model and writes back unchanged. Came from EffectCraft's
//! `effectcraft-psd`.

use crate::descriptor::Descriptor;
use crate::engine_data::{self, TextStyle};
use crate::error::Result;
use crate::io::Reader;
use crate::layer::LayerRecord;

/// What a type layer says: its text, where it sits and how its first style run looks.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeLayer {
    /// The text, with `\n` line breaks and without the line break that ends it.
    pub text: String,
    /// Affine transform (xx, xy, yx, yy, tx, ty): the text origin sits at (tx, ty). The identity
    /// when the file's transform is not finite.
    pub transform: [f64; 6],
    /// The first style run, read best effort from the text engine document.
    pub style: TextStyle,
    /// Text bounds (left, top, right, bottom) relative to the origin, when stored.
    pub bounds: Option<[f64; 4]>,
    /// The text descriptor, for everything else.
    pub descriptor: Descriptor,
}

impl TypeLayer {
    /// Reads the data of a `TySh` block.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let _version = r.u16()?;
        let mut transform = [0.0; 6];
        for v in &mut transform {
            *v = r.f64()?;
        }
        // A transform with NaN or infinity is unusable: fall back to the identity.
        if transform.iter().any(|v| !v.is_finite()) {
            transform = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        }
        let _text_version = r.u16()?;
        let _descriptor_version = r.u32()?;
        let descriptor = Descriptor::read(&mut r)?;
        let text = descriptor.get_text("Txt ").unwrap_or_default().replace("\r\n", "\n").replace('\r', "\n");
        let text = text.strip_suffix('\n').map(str::to_string).unwrap_or(text);
        let style = descriptor.get_raw_data("EngineData").map(|e| engine_data::first_style(&engine_data::parse_lossy(e))).unwrap_or_default();
        let bounds = descriptor
            .get_object("bounds")
            .or_else(|| descriptor.get_object("boundingBox"))
            .and_then(|o| Some([o.get_number("Left")?, o.get_number("Top ")?, o.get_number("Rght")?, o.get_number("Btom")?]));
        Ok(TypeLayer { text, transform, style, bounds, descriptor })
    }
}

impl LayerRecord {
    /// The layer's type (`TySh`), when it is a type layer. A malformed block counts as absent.
    pub fn type_layer(&self) -> Option<TypeLayer> {
        TypeLayer::from_bytes(&self.block(b"TySh")?.data).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{UnicodeString, Value};
    use crate::io::WriteExt;
    use crate::tagged::TaggedBlock;

    /// The data of a `TySh` block.
    fn type_block(transform: [f64; 6], text: &str, engine: Option<Vec<u8>>, bounds_key: Option<&str>) -> Vec<u8> {
        let mut d = Descriptor::new("TxLr").with("Txt ", Value::Text(UnicodeString::new_nul(text)));
        if let Some(e) = engine {
            d = d.with("EngineData", Value::RawData(e));
        }
        if let Some(key) = bounds_key {
            let b = ["Left", "Top ", "Rght", "Btom"]
                .iter()
                .zip([-1.0, -20.0, 80.0, 5.0])
                .fold(Descriptor::new("bounds"), |b, (k, v)| b.with(k, Value::UnitFloat { unit: *b"#Pnt", value: v }));
            d = d.with(key, Value::Descriptor(b));
        }
        let mut out = Vec::new();
        out.put_u16(1);
        for v in transform {
            out.put_f64(v);
        }
        out.put_u16(50);
        out.put_u32(16);
        d.write(&mut out);
        // The warp version, descriptor version and descriptor, and the four bounds that follow.
        out.put_u16(1);
        out.put_u32(16);
        Descriptor::new("warp").write(&mut out);
        out.extend_from_slice(&[0; 32]);
        out
    }

    #[test]
    fn a_type_block_gives_text_transform_style_and_bounds() {
        let engine = engine_data::minimal("Hello\rWorld", "Arial-BoldMT", 36.0, [1.0, 0.5, 0.0], 2);
        let data = type_block([1.0, 0.0, 0.0, 1.0, 40.0, 60.0], "Hello\rWorld\r", Some(engine), Some("bounds"));
        let t = TypeLayer::from_bytes(&data).unwrap();
        assert_eq!(t.text, "Hello\nWorld");
        assert_eq!(t.transform, [1.0, 0.0, 0.0, 1.0, 40.0, 60.0]);
        assert_eq!(
            (t.style.font.as_deref(), t.style.size, t.style.color, t.style.justification),
            (Some("Arial-BoldMT"), Some(36.0), Some([1.0, 0.5, 0.0]), Some(2))
        );
        assert_eq!(t.bounds, Some([-1.0, -20.0, 80.0, 5.0]));
        assert!(t.descriptor.get("EngineData").is_some());
        let layer = LayerRecord { blocks: vec![TaggedBlock::new(*b"TySh", data.clone())], ..Default::default() };
        assert_eq!(layer.type_layer(), Some(t));
        // Cut anywhere inside the text descriptor: an error, and the layer reads as no type layer.
        let descriptor_end = data.len() - 32 - 6 - Descriptor::new("warp").to_bytes().len();
        for cut in 0..descriptor_end {
            assert!(TypeLayer::from_bytes(&data[..cut]).is_err(), "cut {cut}");
        }
        let cut = LayerRecord { blocks: vec![TaggedBlock::new(*b"TySh", data[..40].to_vec())], ..Default::default() };
        assert_eq!(cut.type_layer(), None);
        assert_eq!(LayerRecord::default().type_layer(), None);
    }

    #[test]
    fn windows_line_breaks_a_bounding_box_and_missing_engine_data_read() {
        let t = TypeLayer::from_bytes(&type_block([2.0, 0.0, 0.0, 2.0, 0.0, 0.0], "a\r\nb", None, Some("boundingBox"))).unwrap();
        assert_eq!(t.text, "a\nb");
        assert_eq!(t.style, TextStyle::default());
        assert_eq!(t.bounds, Some([-1.0, -20.0, 80.0, 5.0]));
        assert_eq!(TypeLayer::from_bytes(&type_block([1.0; 6], "x", None, None)).unwrap().bounds, None);
    }

    #[test]
    fn a_transform_that_is_not_finite_becomes_the_identity() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let t = TypeLayer::from_bytes(&type_block([1.0, 0.0, bad, 1.0, 5.0, 5.0], "x", None, None)).unwrap();
            assert_eq!(t.transform, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        }
    }

    #[test]
    fn damaged_engine_data_still_gives_the_style_it_holds() {
        let mut engine = engine_data::minimal("x", "Courier", 12.0, [0.0, 0.0, 0.0], 0);
        let keep = engine.windows(8).position(|w| w == b"/FontSet").unwrap() + 40;
        engine.truncate(keep.min(engine.len() - 4));
        let t = TypeLayer::from_bytes(&type_block([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "x", Some(engine), None)).unwrap();
        assert_eq!(t.style.size, Some(12.0));
    }
}
