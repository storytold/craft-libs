//! `shmd` (metadata setting) layer block: a list of keyed sub-blocks, such as `cmls` (layer comp
//! state, a versioned descriptor), `cust` (the layer's modify time and XMP packet) and `mlst`.
//!
//! Layout (Adobe PSD spec, "Metadata setting"): u32 count, then per item the signature `8BIM`,
//! a 4-byte key, a copy-on-sheet-duplication byte, 3 padding bytes, a u32 length and the data.
//! Photoshop pads item data to an even length (the padding is part of the stored length), so
//! parsing and writing are exact: `write(parse(b)) == b`.

use crate::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt};

/// One `shmd` item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MetadataItem {
    /// Usually `8BIM`.
    pub signature: [u8; 4],
    /// Item key, e.g. `cmls`.
    pub key: [u8; 4],
    /// Copy on sheet duplication.
    pub copy: bool,
    /// The 3 bytes after the copy flag (zero in Photoshop files), kept for exactness.
    pub reserved: [u8; 3],
    /// Data including any padding counted in the stored length.
    pub data: Vec<u8>,
}

impl MetadataItem {
    /// Typed animation descriptor for `mlst` (sparse layer frame states) or `tmln`
    /// (layer timeline tracks). Other metadata, including `cust` / XMP, is not interpreted.
    /// The original item stays unchanged; inheritance and track evaluation belong to the importer.
    pub fn animation_descriptor(&self) -> Option<Result<crate::VersionedDescriptor>> {
        matches!(&self.key, b"mlst" | b"tmln").then(|| {
            if self.signature != *b"8BIM" {
                return Err(PsdError::InvalidSignature { expected: "8BIM animation metadata", found: self.signature });
            }
            crate::animation::parse_descriptor(&self.data)
        })
    }

    /// A new item; `data` is zero-padded to an even length as Photoshop writes it.
    pub fn new(key: [u8; 4], mut data: Vec<u8>) -> Self {
        if data.len() % 2 == 1 {
            data.push(0);
        }
        MetadataItem { signature: *b"8BIM", key, copy: false, reserved: [0; 3], data }
    }
}

/// Parses `shmd` block data.
pub fn parse_shmd(data: &[u8]) -> Result<Vec<MetadataItem>> {
    let mut r = Reader::new(data);
    let count = r.u32()?;
    r.check_count(u64::from(count), 16)?;
    let mut out = crate::descriptor::vec_for_count(count);
    for _ in 0..count {
        let signature = r.array()?;
        let key = r.array()?;
        let copy = r.u8()? != 0;
        let reserved = r.array()?;
        let len = r.u32()?;
        let data = r.bytes_u64(u64::from(len))?.to_vec();
        out.push(MetadataItem { signature, key, copy, reserved, data });
    }
    if !r.is_empty() {
        return Err(PsdError::invalid("trailing bytes after metadata items"));
    }
    Ok(out)
}

/// Serializes `shmd` block data.
pub fn write_shmd(items: &[MetadataItem]) -> Vec<u8> {
    let mut out = Vec::new();
    out.put_u32(items.len() as u32);
    for it in items {
        out.put(&it.signature);
        out.put(&it.key);
        out.push(u8::from(it.copy));
        out.put(&it.reserved);
        out.put_u32(it.data.len() as u32);
        out.put(&it.data);
    }
    out
}

/// The `shmd` item that holds a layer's own metadata (its modify time and XMP).
const CUST: [u8; 4] = *b"cust";
/// Key of a layer's XMP packet in the `cust` descriptor.
const LAYER_XMP: &str = "layerXMP";

// A layer's XMP packet (Photoshop's `layer.xmpMetadata.rawData`), as observed in files Photoshop
// 2026 saves: a `TEXT` item `layerXMP` (UTF-16 with a terminating NUL) in the `metadata`
// descriptor (version 16, class name a lone NUL) of the `shmd` item `cust`, after the
// `layerTime` double that every Photoshop layer has. A layer without XMP has no `layerXMP` item.

fn pad_even(mut v: Vec<u8>) -> Vec<u8> {
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// The `cust` descriptor of `item`, or `None` when it isn't one.
fn cust_descriptor(item: &MetadataItem) -> Option<VersionedDescriptor> {
    (item.key == CUST).then(|| VersionedDescriptor::parse_prefix(&item.data).ok().map(|(d, _)| d)).flatten()
}

/// The XMP packet stored on a layer whose `shmd` block data is `shmd`, or `None` when it has
/// none (or the block can't be read).
pub fn layer_xmp(shmd: &[u8]) -> Option<String> {
    let items = parse_shmd(shmd).ok()?;
    items.iter().filter_map(cust_descriptor).find_map(|d| match d.descriptor.get(LAYER_XMP) {
        Some(Value::Text(t)) => Some(t.to_string_lossy()),
        _ => None,
    })
}

/// Splits a layer's XMP packet out of its `shmd` block data: the packet, and the block without
/// it (`None` when nothing is left). `None` when the block holds no packet, so the caller keeps
/// the original bytes. A `cust` item left empty is dropped, the other items are kept as stored.
pub fn take_layer_xmp(shmd: &[u8]) -> Option<(String, Option<Vec<u8>>)> {
    let mut items = parse_shmd(shmd).ok()?;
    let (i, mut vd) =
        items.iter().enumerate().find_map(|(i, it)| cust_descriptor(it).filter(|d| d.descriptor.get(LAYER_XMP).is_some()).map(|d| (i, d)))?;
    let at = vd.descriptor.items.iter().position(|(k, _)| k.is(LAYER_XMP))?;
    let xmp = match vd.descriptor.items.remove(at).1 {
        Value::Text(t) => t.to_string_lossy(),
        _ => return None,
    };
    if vd.descriptor.items.is_empty() {
        items.remove(i);
    } else {
        items[i].data = pad_even(vd.to_bytes());
    }
    Some((xmp, (!items.is_empty()).then(|| write_shmd(&items))))
}

/// `shmd` block data (`shmd`, or a new block when `None`) with `xmp` as the layer's XMP packet,
/// written as Photoshop writes it: it replaces an existing packet in place, or is appended to
/// the `cust` descriptor (a new `cust` item when there is none). An unreadable block is an
/// error, so the caller never replaces data it couldn't read.
pub fn put_layer_xmp(shmd: Option<&[u8]>, xmp: &str) -> Result<Vec<u8>> {
    let mut items = match shmd {
        Some(b) => parse_shmd(b)?,
        None => Vec::new(),
    };
    let value = Value::Text(UnicodeString::new_nul(xmp));
    match items.iter().position(|it| it.key == CUST) {
        Some(i) => {
            let (mut vd, _) = VersionedDescriptor::parse_prefix(&items[i].data)?;
            match vd.descriptor.items.iter_mut().find(|(k, _)| k.is(LAYER_XMP)) {
                Some(entry) => entry.1 = value,
                None => vd.descriptor.items.push((Id::new(LAYER_XMP), value)),
            }
            items[i].data = pad_even(vd.to_bytes());
        }
        None => {
            let d = Descriptor { name: UnicodeString::new_nul(""), class_id: Id::new("metadata"), items: vec![(Id::new(LAYER_XMP), value)] };
            items.push(MetadataItem::new(CUST, VersionedDescriptor::new(d).to_bytes()));
        }
    }
    Ok(write_shmd(&items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(lines: &[&str]) -> Vec<u8> {
        let h: String = lines.concat();
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }

    /// `shmd` of a layer saved by Photoshop 2026 after a script set its XMP to [`PS_PACKET`].
    const PS_WITH_XMP: &[&str] = &[
        "000000013842494d63757374000000000000024800000010000000010000000000086d65746164617461000000020000",
        "00096c6179657254696d65646f756241dab643ffd54363000000086c61796572584d505445585400000100003c007800",
        "3a0078006d0070006d00650074006100200078006d006c006e0073003a0078003d002200610064006f00620065003a00",
        "6e0073003a006d006500740061002f0022003e003c007200640066003a00520044004600200078006d006c006e007300",
        "3a007200640066003d00220068007400740070003a002f002f007700770077002e00770033002e006f00720067002f00",
        "31003900390039002f00300032002f00320032002d007200640066002d00730079006e007400610078002d006e007300",
        "230022003e003c007200640066003a004400650073006300720069007000740069006f006e0020007200640066003a00",
        "610062006f00750074003d0022002200200078006d006c006e0073003a00730061006d0070006c0065003d0022006800",
        "7400740070003a002f002f006500780061006d0070006c0065002e0063006f006d002f00730061006d0070006c006500",
        "2f0031002e0030002f0022003e003c00730061006d0070006c0065003a00760061006c00750065003e0061006c007000",
        "680061002d003100320033003c002f00730061006d0070006c0065003a00760061006c00750065003e003c002f007200",
        "640066003a004400650073006300720069007000740069006f006e003e003c002f007200640066003a00520044004600",
        "3e003c002f0078003a0078006d0070006d006500740061003e000000",
    ];
    /// `shmd` of a layer without XMP in the same file (its own `layerTime` only).
    const PS_WITHOUT_XMP: &[&str] = &[
        "000000013842494d63757374000000000000003400000010000000010000000000086d6574616461746100000001000000096c6179657254696d65646f756241dab643ffd86bec00",
    ];
    const PS_PACKET: &str = concat!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">"#,
        r#"<rdf:Description rdf:about="" xmlns:sample="http://example.com/sample/1.0/"><sample:value>alpha-123</sample:value>"#,
        r#"</rdf:Description></rdf:RDF></x:xmpmeta>"#
    );

    #[test]
    fn reads_photoshop_layer_xmp() {
        assert_eq!(layer_xmp(&unhex(PS_WITH_XMP)).as_deref(), Some(PS_PACKET));
        assert_eq!(layer_xmp(&unhex(PS_WITHOUT_XMP)), None);
        assert!(take_layer_xmp(&unhex(PS_WITHOUT_XMP)).is_none());
    }

    #[test]
    fn take_then_put_is_exact() {
        let ps = unhex(PS_WITH_XMP);
        let (xmp, rest) = take_layer_xmp(&ps).unwrap();
        assert_eq!(xmp, PS_PACKET);
        let rest = rest.unwrap();
        // What's left is a layer without XMP, shaped as Photoshop writes one.
        assert_eq!(rest.len(), unhex(PS_WITHOUT_XMP).len());
        assert_eq!(layer_xmp(&rest), None);
        assert_eq!(put_layer_xmp(Some(&rest), &xmp).unwrap(), ps);
    }

    #[test]
    fn put_replaces_in_place_and_creates_a_block() {
        let ps = unhex(PS_WITH_XMP);
        let edited = put_layer_xmp(Some(&ps), "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>").unwrap();
        assert_eq!(layer_xmp(&edited).as_deref(), Some("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>"));
        // `layerTime` is kept, still first.
        let items = parse_shmd(&edited).unwrap();
        let d = cust_descriptor(&items[0]).unwrap().descriptor;
        assert!(d.items[0].0.is("layerTime") && d.items[1].0.is(LAYER_XMP) && d.items.len() == 2);

        let fresh = put_layer_xmp(None, PS_PACKET).unwrap();
        assert_eq!(layer_xmp(&fresh).as_deref(), Some(PS_PACKET));
        assert_eq!(take_layer_xmp(&fresh), Some((PS_PACKET.to_string(), None)));
        // Other items (here a layer comp state) stay as they were.
        let comps = write_shmd(&[MetadataItem::new(*b"cmls", vec![1, 2])]);
        let both = put_layer_xmp(Some(&comps), PS_PACKET).unwrap();
        assert_eq!(parse_shmd(&both).unwrap()[0], parse_shmd(&comps).unwrap()[0]);
        assert_eq!(take_layer_xmp(&both).unwrap().1.unwrap(), comps);
    }

    #[test]
    fn put_refuses_an_unreadable_block() {
        assert!(put_layer_xmp(Some(&[0, 0, 0, 9]), PS_PACKET).is_err());
        let bad_cust = write_shmd(&[MetadataItem::new(CUST, vec![0, 0, 0, 16, 1])]);
        assert!(put_layer_xmp(Some(&bad_cust), PS_PACKET).is_err());
        assert_eq!(layer_xmp(&bad_cust), None);
        assert!(take_layer_xmp(&bad_cust).is_none());
    }

    #[test]
    fn roundtrip_exact() {
        let items = vec![MetadataItem::new(*b"cmls", vec![1, 2, 3]), MetadataItem { copy: true, ..MetadataItem::new(*b"cust", vec![9; 4]) }];
        assert_eq!(items[0].data.len(), 4);
        let bytes = write_shmd(&items);
        assert_eq!(parse_shmd(&bytes).unwrap(), items);
        assert_eq!(write_shmd(&parse_shmd(&bytes).unwrap()), bytes);
    }

    #[test]
    fn malformed_is_an_error() {
        assert!(parse_shmd(&[]).is_err());
        assert!(parse_shmd(&[0, 0, 0, 9]).is_err());
        let mut b = write_shmd(&[MetadataItem::new(*b"cmls", vec![0; 2])]);
        b.push(0);
        assert!(parse_shmd(&b).is_err());
        b.truncate(b.len() - 3);
        assert!(parse_shmd(&b).is_err());
    }
}
