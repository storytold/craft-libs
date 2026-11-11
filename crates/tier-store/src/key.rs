use serde::{Deserialize, Serialize};

/// Namespaced store key (EffectCraft: `config/*`, `files/*`, cache keys, …).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StoreKey(pub String);

impl StoreKey {
    /// Strips every leading `/`: CodeRabbit finding on opfs-flush.js:60, `docs/a` and `/docs/a`
    /// reached the same OPFS file (`keyToSegments` already strips a leading slash there) while
    /// `StoreKey` kept them as two distinct hot-tier keys with independent revisions and pending
    /// writes. Canonicalizing here, at construction, means every component -- the mirror, the
    /// warm OPFS mapping, `DefaultTierCoordinator`'s raw-key join -- agrees on one key for both
    /// spellings, instead of needing the same normalization re-implemented (and kept in sync) in
    /// each of them.
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        if path.starts_with('/') { Self(path.trim_start_matches('/').to_string()) } else { Self(path) }
    }

    /// Cache key for a ranged read of blob bytes identified by `content_hash`.
    ///
    /// Layout mirrors Legion `shardCache` expert keys: stable content id + byte range.
    /// Always classified as [`KeyClass::Cache`](crate::KeyClass::Cache) via the `cache/` prefix.
    /// Warm OPFS path: see [`DefaultTierCoordinator`](crate::DefaultTierCoordinator).
    pub fn slice_cache(content_hash: &[u8], offset: u64, length: u64) -> Self {
        Self(format!("cache/slice/{}/{}+{}", hex_lower(content_hash), offset, length))
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
