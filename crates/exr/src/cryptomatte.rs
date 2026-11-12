//! Cryptomatte (specification 1.2): per-pixel object and material IDs with coverage in EXR
//! channels, as written by Arnold, V-Ray, Redshift, Mantra/Karma, Cycles and others.
//!
//! Layers are announced by `cryptomatte/<key>/name|hash|conversion|manifest` header attributes;
//! the samples live in `<name>NN` RGBA channel sets, each holding two (ID, coverage) ranks:
//! `.red`/`.green` (rank 2·NN) and `.blue`/`.alpha` (rank 2·NN+1).
//!
//! An ID is a float *value*: MurmurHash3 x86-32 (seed 0) of the object name, with its exponent
//! fixed away from 0 and 255 so that no ID is denormal, infinite or NaN, read as an `f32` bit
//! pattern. [`hash`] gives the bits (what a manifest stores as hex), [`id`] the float (what the
//! channels store).

use std::collections::HashSet;

/// Longest manifest [`parse_manifest`] reads, in bytes. Manifests are file-controlled and can be
/// large; a longer one is ignored, like a manifest that is not valid JSON.
pub const MAX_MANIFEST_BYTES: usize = 16 << 20;

/// MurmurHash3, x86 32-bit variant (public-domain algorithm by Austin Appleby).
pub fn murmur3_32(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut h = seed;
    let (chunks, rest) = data.as_chunks::<4>();
    for c in chunks {
        let k = u32::from_le_bytes(*c).wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        h ^= k;
        h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    if !rest.is_empty() {
        let k = rest.iter().enumerate().fold(0u32, |k, (i, &b)| k | (u32::from(b) << (8 * i)));
        h ^= k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
    }
    h ^= data.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

/// The Cryptomatte hash of a name: the bits of its ID, which is what a manifest stores.
pub fn hash(name: &str) -> u32 {
    let bits = murmur3_32(name.as_bytes(), 0);
    // Keep the float finite and normal: flip the low exponent bit when the exponent is all 0s or 1s.
    let exponent = (bits >> 23) & 0xff;
    if exponent == 0 || exponent == 255 { bits ^ (1 << 23) } else { bits }
}

/// The Cryptomatte ID of a name: the float the ID channels store.
pub fn id(name: &str) -> f32 {
    f32::from_bits(hash(name))
}

/// The 7-hex-digit metadata key of a layer name (the ID bits without their last digit), as in
/// `cryptomatte/<key>/name`.
pub fn key(layer_name: &str) -> String {
    let mut hex = format!("{:08x}", hash(layer_name));
    hex.truncate(7);
    hex
}

/// The name of the header attribute holding `field` (`name`, `hash`, `conversion`, `manifest`,
/// `manif_file`) of the layer with metadata key `key`.
pub fn attribute_name(key: &str, field: &str) -> String {
    format!("cryptomatte/{key}/{field}")
}

/// Every `cryptomatte/<key>/…` key among a header's attribute names, each once, in the order
/// given.
pub fn keys<'a>(attribute_names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut keys = Vec::new();
    // The names are file-controlled: a set answers "seen before" so many distinct keys stay linear.
    let mut seen = HashSet::new();
    for name in attribute_names {
        let Some(rest) = name.strip_prefix("cryptomatte/") else { continue };
        let Some((key, _field)) = rest.split_once('/') else { continue };
        if !key.is_empty() && seen.insert(key) {
            keys.push(key.to_string());
        }
    }
    keys
}

/// The preview colour of an ID (the reference formula): green and blue shifted out of the ID's
/// bit pattern, red unused.
pub fn preview_color(id: f32) -> [f32; 3] {
    let bits = id.to_bits();
    [0.0, f32::from_bits(bits << 8), f32::from_bits(bits << 16)]
}

/// Parses a manifest (`{"object name": "<up to 8 hex digits of ID bits>", …}`) into (name, hash)
/// pairs sorted by name. Entries that are not hex strings are skipped, as the reference readers
/// tolerate a partly broken manifest; a manifest that is not a JSON object, or is longer than
/// [`MAX_MANIFEST_BYTES`], gives none. `f32::from_bits` of a hash is the ID.
pub fn parse_manifest(json: &str) -> Vec<(String, u32)> {
    if json.len() > MAX_MANIFEST_BYTES {
        return Vec::new();
    }
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut out: Vec<(String, u32)> = map
        .iter()
        .filter_map(|(name, hex)| {
            let hex = hex.as_str()?.trim();
            Some((name.clone(), u32::from_str_radix(hex.strip_prefix("0x").unwrap_or(hex), 16).ok()?))
        })
        .collect();
    out.sort();
    out
}

/// A rank channel of a Cryptomatte stream (`crypto_asset00.red`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChannel {
    /// The stream (channel set) name with its index, as the file spells it: `crypto_asset00`.
    pub stream: String,
    /// The stream's index: the trailing digits.
    pub index: u32,
    /// Whether the channel holds coverage (green, alpha) instead of IDs (red, blue).
    pub coverage: bool,
    /// The rank within the set: 0 for red/green, 1 for blue/alpha. The rank within the layer is
    /// `index × 2 + rank`.
    pub rank: usize,
}

/// Splits a rank channel name: `<name>NN` where NN is one or more digits (the reference readers
/// match `\d+`), then `.red`/`.r` or `.blue`/`.b` (ID) or `.green`/`.g` or `.alpha`/`.a`
/// (coverage), in any case (Blender writes `.R`/`.G`/`.B`/`.A`). `None` for any other channel.
pub fn split_stream(channel: &str) -> Option<StreamChannel> {
    let (stream, suffix) = channel.rsplit_once('.')?;
    let (coverage, rank) = match suffix.to_ascii_lowercase().as_str() {
        "red" | "r" => (false, 0),
        "green" | "g" => (true, 0),
        "blue" | "b" => (false, 1),
        "alpha" | "a" => (true, 1),
        _ => return None,
    };
    let digits = stream.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits == stream.len() {
        return None;
    }
    let index = stream.get(stream.len() - digits..)?.parse().ok()?;
    Some(StreamChannel { stream: stream.to_string(), index, coverage, rank })
}

/// The stream names of the layer called `layer_name` among `channel_names` (`crypto_asset00`,
/// `crypto_asset01`, …, as the file spells them), in index order. The match is lenient: the
/// channel prefix and the layer's `name` attribute differ in case or `_`/`-` between renderers
/// (`CryptoAsset` and `crypto_asset`).
pub fn layer_streams<'a>(channel_names: impl IntoIterator<Item = &'a str>, layer_name: &str) -> Vec<String> {
    let normalize = |s: &str| s.to_ascii_lowercase().replace(['_', '-'], "");
    let want = normalize(layer_name);
    let mut found: Vec<(u32, String)> = channel_names
        .into_iter()
        .filter_map(|c| {
            let s = split_stream(c)?;
            (!s.coverage).then_some((s.index, s.stream))
        })
        .filter(|(_, stream)| normalize(stream.trim_end_matches(|c: char| c.is_ascii_digit())) == want)
        .collect();
    found.sort();
    found.dedup();
    found.into_iter().map(|(_, stream)| stream).collect()
}

/// The Cryptomatte layers present among `channel_names`, found from the channels alone (a file
/// without the header attributes): every prefix that has a `00.R` channel, sorted. The names are
/// exact, including case and any layer prefix (`ViewLayer.CryptoObject`).
pub fn channel_layers<'a>(channel_names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> =
        channel_names.into_iter().filter_map(|n| n.strip_suffix("00.R").or_else(|| n.strip_suffix("00.r")).map(str::to_string)).collect();
    out.sort();
    out.dedup();
    out
}

/// The hash with the most coverage among a pixel's (ID, coverage) ranks, in rank order, summed
/// over the ranks it appears in. Ties go to the lower rank; ranks without positive coverage do
/// not count. `None` when nothing covers the pixel.
pub fn top_id(ranks: impl IntoIterator<Item = (f32, f32)>) -> Option<u32> {
    let mut ids: Vec<(u32, f32)> = Vec::new();
    for (id, coverage) in ranks {
        if coverage > 0.0 {
            match ids.iter_mut().find(|(h, _)| *h == id.to_bits()) {
                Some((_, sum)) => *sum += coverage,
                None => ids.push((id.to_bits(), coverage)),
            }
        }
    }
    // `min_by` keeps the first of equal elements: a tie goes to the lower rank.
    ids.into_iter().min_by(|a, b| b.1.total_cmp(&a.1)).map(|(h, _)| h)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// The official hash test vectors (cryptomatte_utilities_tests.py, CryptoHashing): the f32 IDs
    /// the reference implementation produces. The extra digits are quoted as published.
    #[test]
    #[allow(clippy::excessive_precision)]
    fn ids_match_the_official_vectors() {
        let vectors: &[(&str, f64)] = &[
            ("hello", 6.0705627102400005616e-17),
            ("cube", -4.08461912519e+15),
            ("sphere", 2.79018604383e+15),
            ("plane", 3.66557617593e-11),
            ("равнина", -1.3192631212399999468e-25),
            ("mädchen", 6.2361298211599995797e+25),
        ];
        for (name, want) in vectors {
            assert_eq!(id(name).to_bits(), (*want as f32).to_bits(), "ID bits of {name:?}");
            assert_eq!(hash(name), id(name).to_bits());
        }
    }

    /// MurmurHash3 x86-32 reference values, including a seed and every tail length.
    #[test]
    fn murmur3_reference_values() {
        assert_eq!(murmur3_32(b"", 0), 0);
        assert_eq!(murmur3_32(b"", 1), 0x514e_28b7);
        assert_eq!(murmur3_32(b"a", 0), 0x3c25_69b2);
        assert_eq!(murmur3_32(b"ab", 0), 0x9bbf_d75f);
        assert_eq!(murmur3_32(b"abc", 0), 0xb3dd_93fa);
        assert_eq!(murmur3_32(b"abcd", 0), 0x43ed_676a);
        assert_eq!(murmur3_32(b"Hello, world!", 0x9747_b28c), 0x2488_4cba);
    }

    #[test]
    fn every_id_is_a_finite_normal_float() {
        for name in ["", "a", "hello", "CryptoAsset", "very long object name with ünïcödé"] {
            let id = id(name);
            assert!(id.is_finite() && id.abs() >= f32::MIN_POSITIVE, "{name:?} -> {id}");
        }
        // A name whose raw hash has exponent 0 would be denormal without the fix.
        let name = (0u32..).map(|i| i.to_string()).find(|n| (murmur3_32(n.as_bytes(), 0) >> 23) & 0xff == 0).unwrap();
        assert_eq!(hash(&name), murmur3_32(name.as_bytes(), 0) ^ (1 << 23));
        assert!(id(&name).is_normal());
    }

    #[test]
    fn keys_and_attribute_names() {
        let k = key("CryptoAsset");
        assert_eq!(k.len(), 7);
        assert_eq!(k, format!("{:08x}", hash("CryptoAsset"))[..7]);
        assert_eq!(attribute_name(&k, "manifest"), format!("cryptomatte/{k}/manifest"));
        let names = [
            "cryptomatte/abc1234/name",
            "comment",
            "cryptomatte/abc1234/manifest",
            "cryptomatte/def5678/name",
            "cryptomatte//name",
            "cryptomatte/loose",
        ];
        assert_eq!(keys(names), ["abc1234", "def5678"]);
    }

    #[test]
    fn preview_color_shifts_the_id_bits() {
        let id = id("cube");
        assert_eq!(preview_color(id), [0.0, f32::from_bits(id.to_bits() << 8), f32::from_bits(id.to_bits() << 16)]);
    }

    #[test]
    fn manifests_parse_leniently() {
        let m = parse_manifest(r#"{"sphere": "4d8b0e5a", "cube": " 0xDEADBEEF ", "short": "1f", "bad": "xyz", "number": 5, "long": "123456789"}"#);
        assert_eq!(m, [("cube".to_string(), 0xDEAD_BEEF), ("short".to_string(), 0x1f), ("sphere".to_string(), 0x4d8b_0e5a)]);
        assert!(parse_manifest("not json").is_empty());
        assert!(parse_manifest("[1, 2]").is_empty());
        assert!(parse_manifest("").is_empty());
        // A real entry round-trips to the ID.
        let json = format!(r#"{{"hero": "{:08x}"}}"#, hash("hero"));
        assert_eq!(parse_manifest(&json).first().map(|(_, h)| f32::from_bits(*h)), Some(id("hero")));
    }

    #[test]
    fn the_manifest_size_limit() {
        let entry = r#"{"a": "1"}"#;
        assert_eq!(parse_manifest(entry).len(), 1);
        // Valid JSON padded to one byte past the limit is ignored; at the limit it is read.
        let pad = |len: usize| format!("{entry}{}", " ".repeat(len - entry.len()));
        assert_eq!(parse_manifest(&pad(MAX_MANIFEST_BYTES)).len(), 1);
        assert!(parse_manifest(&pad(MAX_MANIFEST_BYTES + 1)).is_empty());
    }

    #[test]
    fn rank_channels_split() {
        let s = split_stream("crypto_asset00.red").unwrap();
        assert_eq!(s, StreamChannel { stream: "crypto_asset00".into(), index: 0, coverage: false, rank: 0 });
        assert_eq!(split_stream("crypto_asset01.G").map(|s| (s.index, s.coverage, s.rank)), Some((1, true, 0)));
        assert_eq!(
            split_stream("ViewLayer.CryptoObject12.b").map(|s| (s.stream, s.index, s.coverage, s.rank)),
            Some(("ViewLayer.CryptoObject12".into(), 12, false, 1))
        );
        assert_eq!(split_stream("c7.ALPHA").map(|s| (s.index, s.coverage, s.rank)), Some((7, true, 1)));
        for not in ["crypto_asset.red", "00.red", "crypto00", "crypto00.Z", "crypto99999999999.r", "R", "é1é.r"] {
            assert_eq!(split_stream(not), None, "{not}");
        }
    }

    #[test]
    fn a_layers_streams_are_found_leniently_and_in_index_order() {
        let names = [
            "R",
            "crypto_asset01.red",
            "crypto_asset01.green",
            "crypto_asset00.blue",
            "crypto_asset00.green",
            "CryptoMaterial00.R",
            "crypto_asset10.r",
        ];
        assert_eq!(layer_streams(names, "CryptoAsset"), ["crypto_asset00", "crypto_asset01", "crypto_asset10"]);
        assert_eq!(layer_streams(names, "crypto-material"), ["CryptoMaterial00"]);
        assert!(layer_streams(names, "CryptoObject").is_empty());
        // Coverage channels alone do not make a stream.
        assert!(layer_streams(["x00.green", "x00.alpha"], "x").is_empty());
    }

    #[test]
    fn layers_from_channels_alone_are_exact() {
        let names = ["ViewLayer.CryptoObject00.R", "ViewLayer.CryptoObject00.G", "ViewLayer.CryptoObject01.R", "Crypto_A00.r", "crypto_a00.R", "Z"];
        assert_eq!(channel_layers(names), ["Crypto_A", "ViewLayer.CryptoObject", "crypto_a"]);
    }

    #[test]
    fn the_top_id_has_the_most_summed_coverage() {
        let (x, y) = (id("x"), id("y"));
        assert_eq!(top_id([(x, 0.3), (y, 0.6)]), Some(hash("y")));
        // Coverage of one ID in two ranks adds up.
        assert_eq!(top_id([(y, 0.4), (x, 0.3), (x, 0.3)]), Some(hash("x")));
        // A tie goes to the lower rank; zero and negative coverage do not count.
        assert_eq!(top_id([(x, 0.5), (y, 0.5)]), Some(hash("x")));
        assert_eq!(top_id([(x, 0.0), (y, -1.0)]), None);
        assert_eq!(top_id([]), None);
    }
}
