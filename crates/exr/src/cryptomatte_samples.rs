//! Cryptomatte layers and their samples in a file (feature `decode`), on top of
//! [`crate::cryptomatte`] for the naming and [`crate::decode`] for the pixels.

use crate::cryptomatte::{attribute_name, keys, layer_streams, parse_manifest, split_stream};
use crate::decode::{Error, Limits, PartInfo, Result, decode_part, parts};

/// One Cryptomatte layer of a file: header facts and the parsed manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The part that holds it.
    pub part: usize,
    /// The `cryptomatte/<key>/name` value (`CryptoObject`, …).
    pub name: String,
    /// The 7-hex-digit metadata key.
    pub key: String,
    /// The `conversion` attribute (`uint32_to_float32`), when given.
    pub conversion: Option<String>,
    /// The embedded manifest, (name, hash) sorted by name; empty without one.
    pub manifest: Vec<(String, u32)>,
    /// The `manif_file` attribute: a sidecar manifest's file name, when given.
    pub sidecar_manifest: Option<String>,
    /// The rank channel sets, in index order (`crypto_asset00`, `crypto_asset01`, …).
    pub streams: Vec<String>,
}

/// The Cryptomatte layers of the parts, sorted by part, then name. A layer whose channels are
/// not in its part is left out.
pub fn layers_of(parts: &[PartInfo]) -> Vec<Layer> {
    let mut out = Vec::new();
    for p in parts {
        let names: Vec<&str> = p.channels.iter().map(|c| c.name.as_str()).collect();
        // Many file-controlled keys can name one layer: find its streams once.
        let mut streams_for: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for key in keys(p.text_attributes.iter().map(|(k, _)| k.as_str())) {
            let Some(name) = p.text(&attribute_name(&key, "name")) else { continue };
            let streams = streams_for.entry(name.to_string()).or_insert_with(|| layer_streams(names.iter().copied(), name)).clone();
            if streams.is_empty() {
                continue;
            }
            out.push(Layer {
                part: p.index,
                name: name.to_string(),
                conversion: p.text(&attribute_name(&key, "conversion")).map(str::to_string),
                manifest: p.text(&attribute_name(&key, "manifest")).map(parse_manifest).unwrap_or_default(),
                sidecar_manifest: p.text(&attribute_name(&key, "manif_file")).map(str::to_string),
                key,
                streams,
            });
        }
    }
    out.sort_by(|a, b| (a.part, &a.name).cmp(&(b.part, &b.name)));
    out
}

/// Every Cryptomatte layer of a file, from its headers.
pub fn layers(bytes: &[u8], limits: &Limits) -> Result<Vec<Layer>> {
    Ok(layers_of(&parts(bytes, limits)?))
}

/// The samples of one layer: per pixel of the part's data window, its (ID, coverage) pairs with
/// coverage above zero and a finite ID, sorted by descending coverage.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranks {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Vec<(f32, f32)>>,
}

/// Decodes the samples of `layer` (from [`layers`]). Only its rank channels are decoded.
pub fn decode(bytes: &[u8], layer: &Layer, limits: &Limits) -> Result<Ranks> {
    let all = parts(bytes, limits)?;
    let info = all.get(layer.part).ok_or_else(|| Error::Malformed("the Cryptomatte part is missing".into()))?;
    let (w, h) = (info.data_window.width, info.data_window.height);
    // Each pixel also gets a pair list: its header and up to one (ID, coverage) pair per rank,
    // on top of four f32 channels per set.
    let ranks = layer.streams.len().saturating_mul(2);
    let per_pixel = (layer.streams.len().max(1) as u64)
        .saturating_mul(16)
        .saturating_add(std::mem::size_of::<Vec<(f32, f32)>>() as u64)
        .saturating_add((ranks as u64).saturating_mul(std::mem::size_of::<(f32, f32)>() as u64));
    limits.check_bytes(w, h, per_pixel)?;
    let wanted = |name: &str| split_stream(name).is_some_and(|s| layer.streams.contains(&s.stream));
    let part = decode_part(bytes, layer.part, &wanted, limits)?;
    let npx = w as usize * h as usize;
    // Two (ID, coverage) ranks per set: rank `slot × 2 + stream.rank`.
    let mut ids: Vec<Vec<f32>> = vec![Vec::new(); ranks];
    let mut coverages: Vec<Vec<f32>> = vec![Vec::new(); ranks];
    for c in &part.channels {
        let Some(stream) = split_stream(&c.name) else { continue };
        let Some(slot) = layer.streams.iter().position(|s| *s == stream.stream) else { continue };
        let values = c.samples.to_f32();
        if values.len() != npx {
            return Err(Error::Malformed("a Cryptomatte channel is smaller than its layer".into()));
        }
        let bucket = if stream.coverage { &mut coverages } else { &mut ids };
        if let Some(dst) = slot.checked_mul(2).and_then(|r| r.checked_add(stream.rank)).and_then(|r| bucket.get_mut(r)) {
            *dst = values;
        }
    }
    let mut pixels = vec![Vec::new(); npx];
    for (p, out) in pixels.iter_mut().enumerate() {
        for (id, cov) in ids.iter().zip(&coverages) {
            let (id, cov) = (id.get(p).copied().unwrap_or(0.0), cov.get(p).copied().unwrap_or(0.0));
            if cov > 0.0 && id.is_finite() {
                out.push((id, cov));
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
    }
    Ok(Ranks { width: w, height: h, pixels })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use exr::meta::attribute::{AttributeValue, Text};
    use exr::prelude::{
        AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, ImageAttributes, IntegerBounds, Layer as ExrLayer, LayerAttributes,
        LineOrder, WritableImage,
    };

    use super::*;
    use crate::cryptomatte::{id, key};

    /// A 2 × 1 file with a `CryptoAsset` layer of two rank sets (`crypto_asset00`, `01`) and a
    /// beauty RGB, its manifest naming `cube` and `sphere`.
    fn file() -> Vec<u8> {
        let (cube, sphere) = (id("cube"), id("sphere"));
        let k = key("CryptoAsset");
        let mut attrs = LayerAttributes::default();
        let mut text = |field: &str, v: String| {
            attrs.other.insert(Text::from(attribute_name(&k, field).as_str()), AttributeValue::Text(Text::from(v.as_str())));
        };
        text("name", "CryptoAsset".into());
        text("conversion", "uint32_to_float32".into());
        text("manifest", format!(r#"{{"cube":"{:08x}","sphere":"{:08x}"}}"#, cube.to_bits(), sphere.to_bits()));
        let ch = |n: &str, v: [f32; 2]| AnyChannel::new(n, FlatSamples::F32(v.to_vec()));
        let list = vec![
            ch("R", [1.0, 1.0]),
            ch("G", [1.0, 1.0]),
            ch("B", [1.0, 1.0]),
            // Pixel 0: cube 0.75, sphere 0.25 (in the second rank); pixel 1: sphere 1.
            ch("crypto_asset00.red", [cube, sphere]),
            ch("crypto_asset00.green", [0.75, 1.0]),
            ch("crypto_asset00.blue", [sphere, 0.0]),
            ch("crypto_asset00.alpha", [0.25, 0.0]),
            ch("crypto_asset01.R", [0.0, cube]),
            ch("crypto_asset01.G", [0.0, 0.0]),
            ch("crypto_asset01.B", [f32::NAN, 0.0]),
            ch("crypto_asset01.A", [0.5, 0.0]),
        ];
        let layer = ExrLayer::new(
            (2, 1),
            attrs,
            Encoding { compression: Compression::ZIP1, blocks: Blocks::ScanLines, line_order: LineOrder::Increasing },
            AnyChannels::sort(list.into()),
        );
        let image = exr::image::Image::from_layers(ImageAttributes::new(IntegerBounds::new((0, 0), (2, 1))), vec![layer]);
        let mut out = std::io::Cursor::new(Vec::new());
        image.write().to_buffered(&mut out).unwrap();
        out.into_inner()
    }

    #[test]
    fn layers_come_from_the_headers() {
        let bytes = file();
        let found = layers(&bytes, &Limits::default()).unwrap();
        let [l] = found.as_slice() else { panic!("{found:?}") };
        assert_eq!(
            (l.part, l.name.as_str(), l.key.as_str(), l.conversion.as_deref()),
            (0, "CryptoAsset", key("CryptoAsset").as_str(), Some("uint32_to_float32"))
        );
        assert_eq!(l.streams, ["crypto_asset00", "crypto_asset01"]);
        assert_eq!(l.manifest.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["cube", "sphere"]);
        assert_eq!(l.sidecar_manifest, None);
    }

    #[test]
    fn ranks_are_sorted_by_coverage_and_skip_empty_and_nan_ids() {
        let bytes = file();
        let layer = layers(&bytes, &Limits::default()).unwrap().remove(0);
        let r = decode(&bytes, &layer, &Limits::default()).unwrap();
        assert_eq!((r.width, r.height), (2, 1));
        assert_eq!(r.pixels[0], [(id("cube"), 0.75), (id("sphere"), 0.25)]);
        assert_eq!(r.pixels[1], [(id("sphere"), 1.0)]);
        // The per-pixel lists count against the budget.
        let tight = Limits { max_bytes: 2 * 64, ..Limits::default() };
        assert!(matches!(decode(&bytes, &layer, &tight), Err(Error::LimitExceeded(_))));
        let missing = Layer { part: 3, ..layer };
        assert!(matches!(decode(&bytes, &missing, &Limits::default()), Err(Error::Malformed(_))));
    }
}
