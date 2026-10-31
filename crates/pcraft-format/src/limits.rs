use crate::Error;

/// Deepest nesting of layer groups PhotoCraft saves.
pub const MAX_GROUP_DEPTH: usize = 100;

/// Default [`Limits::max_items`]: the most JSON items (arrays and objects opened, plus elements
/// and members after a comma) a manifest may hold.
pub const MAX_MANIFEST_ITEMS: usize = 4 << 20;

/// The largest [`Limits::max_depth`] the checks apply. A larger value is treated as this one,
/// so parsing stays within the stack.
pub const MAX_DEPTH: usize = 400;

/// The deepest JSON nesting of a manifest whose layer groups nest `groups` deep: three levels per
/// group (the layer object, its content object and the `children` array) and 64 for the document
/// and the leaf layers' fields.
pub const fn manifest_depth(groups: usize) -> usize {
    3usize.saturating_mul(groups.saturating_add(1)).saturating_add(64)
}

/// Limits applied while reading a bundle from an untrusted source. The defaults are the ones
/// PhotoCraft loads with.
///
/// The struct is `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Largest `manifest.json`, in bytes (256 MiB).
    pub max_manifest_bytes: usize,
    /// Largest decompressed blob, in bytes (1 GiB).
    pub max_blob_bytes: usize,
    /// Most decompressed tile and blob bytes in one bundle (16 GiB).
    pub max_total_bytes: u64,
    /// Deepest JSON nesting of the manifest (`manifest_depth(MAX_GROUP_DEPTH)`, 367). Values
    /// above [`MAX_DEPTH`] are treated as [`MAX_DEPTH`].
    pub max_depth: usize,
    /// Most JSON items in the manifest ([`MAX_MANIFEST_ITEMS`]).
    pub max_items: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_manifest_bytes: 256 << 20,
            max_blob_bytes: 1 << 30,
            max_total_bytes: 16 << 30,
            max_depth: manifest_depth(MAX_GROUP_DEPTH),
            max_items: MAX_MANIFEST_ITEMS,
        }
    }
}

/// Checks manifest JSON against [`Limits::max_manifest_bytes`], [`Limits::max_depth`] (at most
/// [`MAX_DEPTH`]) and
/// [`Limits::max_items`] in one pass without recursion. Brackets and commas inside strings are
/// not counted. Returns [`Error::LimitExceeded`] for the first limit passed.
pub fn check_manifest(json: &[u8], limits: &Limits) -> Result<(), Error> {
    if json.len() > limits.max_manifest_bytes {
        return Err(Error::LimitExceeded(format!("`{}` exceeds {} bytes", crate::MANIFEST, limits.max_manifest_bytes)));
    }
    let max_depth = limits.max_depth.min(MAX_DEPTH);
    let (mut depth, mut items, mut in_str, mut escaped) = (0usize, 0usize, false, false);
    for &b in json {
        if in_str {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max_depth {
                    return Err(Error::LimitExceeded(format!("manifest JSON nested deeper than {max_depth} levels")));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if matches!(b, b'[' | b'{' | b',') {
            items += 1;
            if items > limits.max_items {
                return Err(Error::LimitExceeded(format!("manifest JSON holds more than {} values", limits.max_items)));
            }
        }
    }
    Ok(())
}

/// Checks manifest JSON with [`check_manifest`], then parses it into a [`serde_json::Value`]
/// with serde_json's recursion limit turned off, so a manifest nested deeper than 128 levels
/// and within [`Limits::max_depth`] parses.
pub fn parse_manifest(json: &[u8], limits: &Limits) -> Result<serde_json::Value, Error> {
    check_manifest(json, limits)?;
    let mut de = serde_json::Deserializer::from_slice(json);
    de.disable_recursion_limit();
    let value = <serde_json::Value as serde::Deserialize>::deserialize(&mut de)?;
    de.end()?;
    Ok(value)
}
