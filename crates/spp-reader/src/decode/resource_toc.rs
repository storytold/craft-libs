//! `/resources.toc`: the named index of the project's resources.
//!
//! A `QMap<QString, QStringList>` (Verified, 2019–2023): u32 class count, then per class a QString
//! (only `Alg::ResourceImage` seen) and a QStringList of `/<name>?version=<id>` entries, where `<id>` is
//! the dataset name under `/resources` (a sha1, sometimes with an `.image` suffix). Nothing follows.

use super::DecodeError;
use crate::container::qds::QDataStream;

const RECORD: &str = "resource toc";
const VERSION_MARKER: &str = "?version=";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    /// Resource class, e.g. `Alg::ResourceImage`.
    pub kind: String,
    /// Display name, e.g. `Ambient Occlusion Map from Mesh <name>`.
    pub name: String,
    /// The `?version=<sha1>` suffix; matches a dataset name under `/resources`.
    pub version: Option<String>,
}

impl TocEntry {
    /// Splits a stored path like `/Moth?version=7ee9…d.image` into name and version.
    fn from_path(kind: &str, path: &str) -> Self {
        let path = path.strip_prefix('/').unwrap_or(path);
        let (name, version) = match path.rfind(VERSION_MARKER) {
            Some(i) => (&path[..i], Some(path[i + VERSION_MARKER.len()..].to_string())),
            None => (path, None),
        };
        TocEntry { kind: kind.to_string(), name: name.to_string(), version }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceToc {
    pub entries: Vec<TocEntry>,
}

impl ResourceToc {
    pub fn parse(data: &[u8]) -> Result<Self, DecodeError> {
        let mut stream = QDataStream::new(data);
        let qds = DecodeError::qds;
        let classes = stream.read_u32().map_err(qds(RECORD))?;
        let mut entries = Vec::new();
        for _ in 0..classes {
            let kind = stream.read_string().map_err(qds(RECORD))?.ok_or_else(|| DecodeError::invalid(RECORD, "null resource class name"))?;
            for path in stream.read_string_list().map_err(qds(RECORD))? {
                entries.push(TocEntry::from_path(&kind, &path));
            }
        }
        if !stream.is_at_end() {
            return Err(DecodeError::invalid(RECORD, format!("unexpected data after the index at offset {}", stream.position())));
        }
        Ok(ResourceToc { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::qds::test_encoder::QdsWriter as W;

    fn toc(classes: &[(&str, &[&str])]) -> Vec<u8> {
        let mut w = W::new();
        w.raw_u32(classes.len() as u32);
        for (kind, paths) in classes {
            w.raw_string(kind).raw_u32(paths.len() as u32);
            for p in *paths {
                w.raw_string(p);
            }
        }
        w.finish()
    }

    #[test]
    fn splits_names_and_versions_across_classes() {
        let data =
            toc(&[("Alg::ResourceImage", &["/AO Map from Mesh Vase?version=abc", "/Moth?version=def.image"]), ("Alg::Other", &["/Unversioned"])]);
        let entries = ResourceToc::parse(&data).unwrap().entries;
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "AO Map from Mesh Vase");
        assert_eq!(entries[0].version.as_deref(), Some("abc"));
        assert_eq!(entries[1].version.as_deref(), Some("def.image"));
        assert_eq!((entries[2].kind.as_str(), entries[2].version.as_ref()), ("Alg::Other", None));
    }

    #[test]
    fn empty_truncated_and_trailing_data() {
        assert_eq!(ResourceToc::parse(&toc(&[])).unwrap(), ResourceToc::default());
        let data = toc(&[("Alg::ResourceImage", &["/A?version=1"])]);
        assert!(ResourceToc::parse(&data[..data.len() - 1]).is_err());
        let mut extra = data.clone();
        extra.push(0);
        assert!(ResourceToc::parse(&extra).is_err());
    }
}
