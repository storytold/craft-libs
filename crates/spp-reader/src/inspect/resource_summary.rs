//! What a project's resources contain: counts per kind, image formats, and optionally a pixel check.

use super::tally::{Tally, Verification};
use crate::decode::image::{ImageHeader, ImageResource};
use crate::project::{ProjectError, ProjectFile, ResourceKind};

/// Enough bytes to cover any resource header seen so far (they end after a few hundred bytes).
const HEADER_PREFIX: usize = 4096;

#[derive(Debug, Clone, Default)]
pub struct ResourceSummary {
    pub kinds: Tally,
    /// Images by "format width×height".
    pub image_formats: Tally,
    /// Images whose header could not be decoded: (file name, reason).
    pub unreadable_headers: Vec<(String, String)>,
    /// Present when pixel verification was requested.
    pub pixels: Option<Verification>,
}

impl ResourceSummary {
    pub fn collect(project: &ProjectFile, verify_pixels: bool) -> Result<Self, ProjectError> {
        let mut s = ResourceSummary { pixels: verify_pixels.then(Verification::default), ..Default::default() };
        for r in project.resources() {
            let prefix = project.read_prefix(&r.path, HEADER_PREFIX)?;
            let kind = ResourceKind::classify(&r.file_name, &prefix);
            s.kinds.add(kind.label(), r.len);
            if kind != ResourceKind::Image {
                continue;
            }
            match ImageHeader::parse(&prefix) {
                Ok(h) => s.image_formats.add(format!("{:?} {}×{}", h.format, h.width, h.height), r.len),
                Err(e) => s.unreadable_headers.push((r.file_name.clone(), e.to_string())),
            }
            if let Some(v) = s.pixels.as_mut() {
                let data = project.read(&r.path)?;
                let outcome = ImageResource::parse(&data).and_then(|i| i.decompress()).map(|_| ());
                v.record(&r.file_name, outcome.map_err(|e| e.to_string()));
            }
        }
        Ok(s)
    }
}
