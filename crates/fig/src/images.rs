//! Images that travel inside the message instead of in `images/`.

use std::collections::BTreeMap;
use std::sync::Arc;

use craft_kiwi::Kv;

/// Longest image hash (bytes) an image is found by: `images/` names and embedded images both
/// stop at 64 bytes (128 hex digits).
pub const MAX_IMAGE_HASH: usize = 64;

/// Image bytes that travel inside the message, by image hash (lowercase hex): a paint's
/// `image.dataBlob` indexes the message's `blobs`. Copies written by plugins and other tools embed
/// their images this way, and a bare canvas (what the clipboard carries) has no `images/`. The
/// walk is iterative (no recursion on nested data) and skips blobs larger than `max_image` bytes.
/// Returns the index of each hash's blob.
pub fn embedded_images(msg: &Kv, max_image: u64) -> BTreeMap<String, usize> {
    let blobs = msg.get("blobs").map(Kv::list).unwrap_or_default();
    let mut found = BTreeMap::new();
    let mut pending = vec![msg];
    while let Some(value) = pending.pop() {
        match value {
            Kv::Object(fields) => {
                if let Some((hash, index)) = value.get("image").and_then(|image| embedded_image(image, blobs, max_image)) {
                    found.entry(hash).or_insert(index);
                }
                pending.extend(fields.iter().filter(|(name, _)| *name != "blobs").map(|(_, field)| field));
            }
            Kv::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    found
}

/// The images that travel inside the message ([`embedded_images`]), copied out of it once per
/// blob: every hash naming the same blob shares its copy. A file can point any number of hashes
/// at one large blob, and a copy for each would multiply it.
pub fn embedded_copies(msg: &Kv, max_image: u64) -> BTreeMap<String, Arc<[u8]>> {
    let blobs = msg.get("blobs").map(Kv::list).unwrap_or_default();
    let mut copies: BTreeMap<usize, Arc<[u8]>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (hash, index) in embedded_images(msg, max_image) {
        let Some(bytes) = blob(blobs, index) else { continue };
        out.insert(hash, Arc::clone(copies.entry(index).or_insert_with(|| Arc::from(bytes))));
    }
    out
}

/// Embedded images ([`embedded_copies`]) whose blob an earlier hash already names, each with that
/// hash. Hashes that also have an `images/` member (`in_zip`) keep their own name: the member is
/// read first. Pointing every alias at one name lets a reader copy and hash each blob once,
/// however many hashes a file points at it.
pub fn embedded_aliases(embedded: &BTreeMap<String, Arc<[u8]>>, in_zip: impl Fn(&str) -> bool) -> BTreeMap<String, String> {
    let mut first: BTreeMap<usize, &String> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (hash, bytes) in embedded {
        if in_zip(hash) {
            continue;
        }
        // Hashes naming one blob share its copy, so the copy's address tells the blob.
        let canonical = *first.entry(bytes.as_ptr() as usize).or_insert(hash);
        if canonical != hash {
            out.insert(hash.clone(), canonical.clone());
        }
    }
    out
}

/// The hash and blob index of an `Image` value whose `dataBlob` points at a usable blob.
fn embedded_image(image: &Kv, blobs: &[Kv], max_image: u64) -> Option<(String, usize)> {
    let hash = image.get("hash").and_then(Kv::bytes).filter(|h| !h.is_empty() && h.len() <= MAX_IMAGE_HASH)?;
    let Some(Kv::Uint(index)) = image.get("dataBlob") else { return None };
    let index = usize::try_from(*index).ok()?;
    let bytes = blob(blobs, index)?;
    if bytes.is_empty() || bytes.len() as u64 > max_image {
        return None;
    }
    Some((hash.iter().map(|b| format!("{b:02x}")).collect(), index))
}

/// The bytes of blob `index`.
fn blob<'k>(blobs: &'k [Kv], index: usize) -> Option<&'k [u8]> {
    blobs.get(index)?.get("bytes").and_then(Kv::bytes)
}
