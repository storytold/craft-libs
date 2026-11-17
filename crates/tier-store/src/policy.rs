use crate::StoreKey;

/// Which tier owns a key by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    Config,
    Media,
    Cache,
    RenderTemp,
}

/// Classify keys for tier placement and eviction.
pub trait TierPolicy {
    fn classify(&self, key: &StoreKey) -> KeyClass;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultTierPolicy;

impl TierPolicy for DefaultTierPolicy {
    fn classify(&self, key: &StoreKey) -> KeyClass {
        let s = key.0.as_str();
        if s.starts_with("config/") {
            KeyClass::Config
        } else if s.starts_with("cache/") {
            KeyClass::Cache
        } else if s.starts_with("render/") {
            KeyClass::RenderTemp
        } else {
            KeyClass::Media
        }
    }
}
