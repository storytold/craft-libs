//! What a project's texture cache holds: entries per GL format and size, and optionally a pixel check.

use super::tally::{Tally, Verification};
use crate::decode::texture_cache::{CacheHeader, CacheTexture};
use crate::project::{ProjectError, ProjectFile};

const HEADER_PREFIX: usize = 4096;

#[derive(Debug, Clone, Default)]
pub struct CacheSummary {
    /// Entries by "GL format width×height".
    pub formats: Tally,
    /// Entries by channel bit.
    pub channels: Tally,
    pub unreadable_headers: Vec<(String, String)>,
    pub pixels: Option<Verification>,
}

impl CacheSummary {
    pub fn collect(project: &ProjectFile, verify_pixels: bool) -> Result<Self, ProjectError> {
        let mut s = CacheSummary { pixels: verify_pixels.then(Verification::default), ..Default::default() };
        for e in project.cache_entries() {
            s.channels.add(e.channel_bit.to_string(), e.len);
            let prefix = project.read_prefix(&e.path, HEADER_PREFIX)?;
            match CacheHeader::parse(&prefix) {
                Ok(h) => s.formats.add(format!("{} {}×{}", h.gl_format_name(), h.width, h.height), e.len),
                Err(err) => s.unreadable_headers.push((e.path.clone(), err.to_string())),
            }
            if let Some(v) = s.pixels.as_mut() {
                let data = project.read(&e.path)?;
                let outcome = CacheTexture::parse(&data).and_then(|t| t.decompress()).map(|_| ());
                v.record(&e.path, outcome.map_err(|err| err.to_string()));
            }
        }
        Ok(s)
    }
}
