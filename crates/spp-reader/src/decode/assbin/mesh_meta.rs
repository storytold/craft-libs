//! [`MeshMetadata`]: the record the reference app keeps next to the mesh, `/paint/.alg_meta/scene.assbin`, a `QVariantMap`
//! (Qt encoding). Every project of the corpus has one, also the 50 whose scene root carries no normalisation.
//!
//! Keys used (Observed on all 156 corpus projects):
//! - `PRE_NORMALIZATION_SCENE_CENTER` (QVector3D), `PRE_NORMALIZATION_SCENE_RADIUS` (float): the normalisation,
//!   stored points are `(p - center) / radius`. Equal to the scene root's normalisation keys wherever both
//!   exist. The reference app keeps it when the mesh is replaced, so the new mesh and the high-poly files modelled next to it
//!   land where the old ones were (Observed: on replaced meshes, high-poly files line up with this transform and
//!   miss with a fit to the new mesh's box).
//! - `OLD_SCENE_BB_CENTER`, `OLD_SCENE_BB_RADIUS` (same types; only some projects): the stored mesh's box in
//!   the reference app's space (centre, half the longest side); the unit box where the mesh was never replaced. Matches the
//!   stored mesh in 141 of 156 projects; informational only.
//! - `originUrl`: the mesh file imported (path on the machine the reference app ran on).

use super::Normalization;
use crate::container::qds::{QDataStream, QVariant, QVariantMap};
use crate::decode::DecodeError;

const RECORD: &str = crate::project::paths::SCENE_META;
const PRE_CENTER: &str = "PRE_NORMALIZATION_SCENE_CENTER";
const PRE_RADIUS: &str = "PRE_NORMALIZATION_SCENE_RADIUS";
const OLD_CENTER: &str = "OLD_SCENE_BB_CENTER";
const OLD_RADIUS: &str = "OLD_SCENE_BB_RADIUS";
const ORIGIN: &str = "originUrl";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshMetadata {
    pub normalization: Option<Normalization>,
    /// `OLD_SCENE_BB_*` as a centre and radius in the reference app's space.
    pub stored_box: Option<Normalization>,
    pub origin_url: Option<String>,
}

impl MeshMetadata {
    pub fn parse(data: &[u8]) -> Result<MeshMetadata, DecodeError> {
        let map = QDataStream::new(data).read_variant_map().map_err(DecodeError::qds(RECORD))?;
        Ok(MeshMetadata {
            normalization: center_radius(&map, PRE_CENTER, PRE_RADIUS),
            stored_box: center_radius(&map, OLD_CENTER, OLD_RADIUS),
            origin_url: map.get(ORIGIN).and_then(QVariant::as_str).map(str::to_owned),
        })
    }
}

fn center_radius(map: &QVariantMap, center: &str, radius: &str) -> Option<Normalization> {
    let center = match map.get(center)? {
        QVariant::Vector3D(c) if c.iter().all(|v| v.is_finite()) => *c,
        _ => return None,
    };
    let radius = match map.get(radius)? {
        QVariant::Float(r) => *r,
        QVariant::Double(r) => *r as f32,
        _ => return None,
    };
    (radius > 0.0 && radius.is_finite()).then_some(Normalization { center, radius })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::qds::test_encoder::QdsWriter as W;

    fn record(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = W::new();
        w.raw_map(entries);
        w.finish()
    }

    #[test]
    fn reads_normalisation_box_and_origin() {
        let data = record(&[
            (ORIGIN, W::encode(|w| w.url("file:///C:/a/Sword LP.OBJ"))),
            ("SPLIT_MATERIALS_PER_UDIM", W::encode(|w| w.bool(false))),
            (PRE_RADIUS, W::encode(|w| w.float(34.0))),
            (PRE_CENTER, W::encode(|w| w.vector3d([0.0, 22.0, 10.5]))),
            (OLD_RADIUS, W::encode(|w| w.float(2.5))),
            (OLD_CENTER, W::encode(|w| w.vector3d([2.0, 1.0, 0.0]))),
        ]);
        let meta = MeshMetadata::parse(&data).unwrap();
        assert_eq!(meta.normalization, Some(Normalization { center: [0.0, 22.0, 10.5], radius: 34.0 }));
        assert_eq!(meta.stored_box, Some(Normalization { center: [2.0, 1.0, 0.0], radius: 2.5 }));
        assert_eq!(meta.origin_url.as_deref(), Some("file:///C:/a/Sword LP.OBJ"));
    }

    #[test]
    fn missing_or_bad_values_are_none_and_truncation_errors() {
        let data = record(&[(PRE_RADIUS, W::encode(|w| w.float(0.0))), (PRE_CENTER, W::encode(|w| w.vector3d([0.0; 3])))]);
        assert_eq!(MeshMetadata::parse(&data).unwrap(), MeshMetadata::default());
        let full = record(&[(PRE_RADIUS, W::encode(|w| w.float(1.0)))]);
        for len in 0..full.len() {
            let _ = MeshMetadata::parse(&full[..len]);
        }
        assert!(MeshMetadata::parse(&[0, 0]).is_err());
    }
}
