use serde::{Deserialize, Serialize};

/// Namespaced store key (EffectCraft: `config/*`, `files/*`, cache keys, …).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StoreKey(pub String);

impl StoreKey {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
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
