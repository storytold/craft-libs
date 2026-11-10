//! [`ProjectReport`]: everything `inspect` finds out about one project. Each section is decoded on its own,
//! so one unreadable part does not hide the others.

use super::cache_summary::CacheSummary;
use super::resource_summary::ResourceSummary;
use crate::decode::assbin::AssbinScene;
use crate::decode::document::ClassCensus;
use crate::decode::resource_toc::ResourceToc;
use crate::decode::settings::ProjectSettings;
use crate::project::{ProjectError, ProjectFile};
use std::path::{Path, PathBuf};

/// A section's result; the error is kept as text for display.
pub type Outcome<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, Default)]
pub struct InspectOptions {
    /// Decompress every image and cache entry to prove the pixels are readable (slow on large files).
    pub verify_pixels: bool,
}

#[derive(Debug)]
pub struct ProjectReport {
    pub path: PathBuf,
    pub file_bytes: u64,
    pub datasets: usize,
    pub settings: Outcome<Option<ProjectSettings>>,
    pub toc: Outcome<Option<ResourceToc>>,
    pub mesh: Outcome<AssbinScene>,
    pub resources: Outcome<ResourceSummary>,
    pub cache: Outcome<CacheSummary>,
    pub document: Outcome<ClassCensus>,
}

impl ProjectReport {
    /// Fails only if the file cannot be opened as a project file at all.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn build(path: &Path, options: InspectOptions) -> Result<Self, ProjectError> {
        let project = ProjectFile::open(path)?;
        let file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Ok(Self::of_project(&project, path, file_bytes, options))
    }

    /// The report of a project already open; `path` and `file_bytes` are shown as given.
    pub fn of_project(project: &ProjectFile, path: &Path, file_bytes: u64, options: InspectOptions) -> Self {
        ProjectReport {
            path: path.to_path_buf(),
            file_bytes,
            datasets: project.layout().datasets().count(),
            settings: text(project.settings()),
            toc: text(project.resource_toc()),
            mesh: text(project.mesh()),
            resources: text(ResourceSummary::collect(project, options.verify_pixels)),
            cache: text(CacheSummary::collect(project, options.verify_pixels)),
            document: text(project.document_census()),
        }
    }

    /// Number of sections that failed.
    pub fn failed_sections(&self) -> usize {
        [self.settings.is_err(), self.toc.is_err(), self.mesh.is_err(), self.resources.is_err(), self.cache.is_err(), self.document.is_err()]
            .into_iter()
            .filter(|&failed| failed)
            .count()
    }
}

fn text<T>(r: Result<T, ProjectError>) -> Outcome<T> {
    r.map_err(|e| e.to_string())
}
