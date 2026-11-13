//! The installed fonts the catalog sees, grouped by family.

use craft_font_catalog::{FaceInfo, Source};

/// One family from one source: a family installed on the system and in the font store is two
/// entries, since only the store's can be removed.
#[derive(Clone, Debug)]
pub struct InstalledFamily {
    pub name: String,
    pub source: Source,
    /// Upright before italic, then by weight and width.
    pub faces: Vec<FaceInfo>,
}

impl InstalledFamily {
    pub fn removable(&self) -> bool {
        self.source == Source::Store
    }
}

/// System and font store faces by family, sorted by name ignoring case. Fonts other sources
/// add (documents, the app's own) are left out.
pub fn group(faces: &[FaceInfo]) -> Vec<InstalledFamily> {
    let mut families: Vec<InstalledFamily> = Vec::new();
    let mut sorted: Vec<&FaceInfo> = faces.iter().filter(|f| matches!(f.source, Source::System | Source::Store)).collect();
    sorted.sort_by_cached_key(|f| (f.family.to_lowercase(), f.source == Source::Store));
    for face in sorted {
        match families.last_mut() {
            Some(last) if last.source == face.source && last.name.eq_ignore_ascii_case(&face.family) => last.faces.push(face.clone()),
            _ => families.push(InstalledFamily { name: face.family.clone(), source: face.source, faces: vec![face.clone()] }),
        }
    }
    for family in &mut families {
        family.faces.sort_by(|a, b| (a.italic, a.weight, a.width).partial_cmp(&(b.italic, b.weight, b.width)).unwrap_or(std::cmp::Ordering::Equal));
        family.faces.dedup_by(|a, b| a.path == b.path && a.index == b.index && a.variations == b.variations);
    }
    families
}
