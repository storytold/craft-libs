//! `/projectsettings.ini`: which the reference app version created and last saved the project.
//!
//! A bare QVariantMap (Verified, version 6.2–9.0) with int keys `versionAtCreation/{major,minor,patch}` and,
//! since 8.x, `versionAtLastSave/{major,minor,patch}`; plus unrelated keys (`anorigami` JSON, `baking/...`).
//! Projects from 2019 have no such dataset at all.

use super::DecodeError;
use crate::container::qds::{QDataStream, QVariantMap};
use std::fmt;

const RECORD: &str = "project settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SavedWithVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for SavedWithVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl SavedWithVersion {
    /// Reads `<prefix>/major`, `/minor`, `/patch`; `None` unless all three are present.
    fn from_map(map: &QVariantMap, prefix: &str) -> Result<Option<Self>, DecodeError> {
        let part = |name: &str| -> Result<Option<u32>, DecodeError> {
            let key = format!("{prefix}/{name}");
            let Some(value) = map.get(&key) else {
                return Ok(None);
            };
            value
                .as_i64()
                .and_then(|v| u32::try_from(v).ok())
                .map(Some)
                .ok_or_else(|| DecodeError::invalid(RECORD, format!("'{key}' is not a version number: {value:?}")))
        };
        Ok(match (part("major")?, part("minor")?, part("patch")?) {
            (Some(major), Some(minor), Some(patch)) => Some(SavedWithVersion { major, minor, patch }),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSettings {
    pub created_with: Option<SavedWithVersion>,
    pub last_saved_with: Option<SavedWithVersion>,
}

impl ProjectSettings {
    pub fn parse(data: &[u8]) -> Result<Self, DecodeError> {
        let map = QDataStream::new(data).read_variant_map().map_err(DecodeError::qds(RECORD))?;
        Ok(ProjectSettings {
            created_with: SavedWithVersion::from_map(&map, "versionAtCreation")?,
            last_saved_with: SavedWithVersion::from_map(&map, "versionAtLastSave")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::qds::test_encoder::QdsWriter as W;

    fn settings(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = W::new();
        w.raw_map(entries);
        w.finish()
    }

    fn int(v: i32) -> Vec<u8> {
        W::encode(|w| w.int(v))
    }

    #[test]
    fn reads_both_versions_in_any_order() {
        let data = settings(&[
            ("versionAtLastSave/patch", int(0)),
            ("versionAtLastSave/minor", int(0)),
            ("versionAtLastSave/major", int(9)),
            ("versionAtCreation/patch", int(1)),
            ("versionAtCreation/minor", int(3)),
            ("versionAtCreation/major", int(8)),
            ("anorigami", W::encode(|w| w.byte_array(b"{}"))),
        ]);
        let s = ProjectSettings::parse(&data).unwrap();
        assert_eq!(s.created_with.unwrap().to_string(), "8.3.1");
        assert_eq!(s.last_saved_with.unwrap().to_string(), "9.0.0");
    }

    #[test]
    fn missing_keys_give_none() {
        let data = settings(&[
            ("versionAtCreation/patch", int(2)),
            ("versionAtCreation/minor", int(2)),
            ("versionAtCreation/major", int(6)),
            ("baking/collapsed_materials", W::encode_list(&[])),
            ("versionAtLastSave/major", int(9)),
        ]);
        let s = ProjectSettings::parse(&data).unwrap();
        assert_eq!(s.created_with, Some(SavedWithVersion { major: 6, minor: 2, patch: 2 }));
        assert_eq!(s.last_saved_with, None);
        assert_eq!(ProjectSettings::parse(&settings(&[])).unwrap().created_with, None);
    }

    #[test]
    fn bad_values_and_bytes_are_errors() {
        let data = settings(&[("versionAtCreation/major", W::encode(|w| w.string("9")))]);
        assert!(ProjectSettings::parse(&data).is_err());
        assert!(ProjectSettings::parse(&settings(&[("versionAtCreation/major", int(-1))])).is_err());
        assert!(ProjectSettings::parse(&[0, 0, 0, 5]).is_err());
    }
}
