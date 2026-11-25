//! The project's resources (`/resources/<sha1>[.ext]`) and what kind each one is.

use super::layout::Layout;
use super::paths;
use crate::decode::image::ImageHeader;
use crate::format_names::PACKAGE_EXT;

/// 7z signature: embedded graph packages are 7z archives.
const SEVEN_ZIP_MAGIC: &[u8] = b"7z\xBC\xAF\x27\x1C";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    Image,
    /// A published compiled-graph material, filter or generator (a graph package).
    GraphPackage,
    Shader,
    ExportPreset,
    Other,
}

impl ResourceKind {
    /// Decides from the file name, and from the first bytes when the name has no extension.
    pub fn classify(file_name: &str, prefix: &[u8]) -> ResourceKind {
        match file_name.rsplit_once('.').map(|(_, ext)| ext) {
            Some(PACKAGE_EXT) => ResourceKind::GraphPackage,
            Some("shader") => ResourceKind::Shader,
            Some("spexp") => ResourceKind::ExportPreset,
            Some("image") => ResourceKind::Image,
            _ if prefix.starts_with(SEVEN_ZIP_MAGIC) => ResourceKind::GraphPackage,
            _ if ImageHeader::parse(prefix).is_ok() => ResourceKind::Image,
            _ => ResourceKind::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::Image => "image",
            ResourceKind::GraphPackage => PACKAGE_EXT,
            ResourceKind::Shader => "shader",
            ResourceKind::ExportPreset => "export preset",
            ResourceKind::Other => "other",
        }
    }
}

/// A resource dataset, not yet read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef {
    pub path: String,
    pub file_name: String,
    pub len: u64,
}

impl ResourceRef {
    /// All resources in the layout, without the `.alg_meta` metadata records.
    pub fn all_in(layout: &Layout) -> Vec<ResourceRef> {
        layout
            .datasets_under(paths::RESOURCES)
            .filter(|d| !d.path.starts_with(paths::RESOURCE_META))
            .map(|d| ResourceRef { path: d.path.to_string(), file_name: d.name().to_string(), len: d.len })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_extension_then_by_signature() {
        assert_eq!(ResourceKind::classify(&format!("ab{}", crate::format_names::PACKAGE_SUFFIX), &[]), ResourceKind::GraphPackage);
        assert_eq!(ResourceKind::classify("ab", b"7z\xBC\xAF\x27\x1C\0\x04"), ResourceKind::GraphPackage);
        assert_eq!(ResourceKind::classify("ab", b"nothing known"), ResourceKind::Other);
    }
}
