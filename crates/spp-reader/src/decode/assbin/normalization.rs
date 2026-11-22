//! [`Normalization`]: how the reference app moved the mesh into its normalised space (about unit radius).
//!
//! The root node's metadata, after its children, holds the normalisation centre (vec3) and
//! radius (float) ([`crate::format_names::SCENE_NORMALIZATION_CENTER`], `_RADIUS`); stored vertices are `(p − center) / radius` (observed on a corpus project:
//! the root's total transform is exactly that). Metadata layout in the reference app's variant: `u16` flag
//! (1 = present), `u32` key count, keys (strings), `u32` value count, values as `u32` type + payload
//! (3 = f32, 5 = vec3, 6 = 4×4 matrix). Unknown types end the reading.
//!
//! The root record is missing in about a third of the projects; the mesh's own metadata record
//! ([`MeshMetadata`]) has the same numbers in every project, so [`Normalization::of_scene`] reads that first.

use super::RECORD;
use super::block::{read_block, read_count, read_string, read_u32};
use super::mesh_meta::MeshMetadata;
use super::scene::{SceneBlocks, scene_bytes};
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

use crate::format_names::{SCENE_NORMALIZATION_CENTER as CENTER, SCENE_NORMALIZATION_RADIUS as RADIUS};
const MATRIX_LEN: usize = 64;

/// Centre and radius the reference app normalised the scene with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normalization {
    pub center: [f32; 3],
    pub radius: f32,
}

impl Normalization {
    /// The normalisation of the stored mesh: from the mesh metadata (`meta`, the `.alg_meta` record) when it has
    /// one, else from the scene root's record.
    pub fn of_scene(scene: &[u8], meta: Option<&MeshMetadata>) -> Result<Option<Normalization>, DecodeError> {
        match meta.and_then(|m| m.normalization) {
            Some(n) => Ok(Some(n)),
            None => Normalization::read(scene),
        }
    }

    /// Reads it from a whole `scene.assbin`; `None` when the root node carries no such metadata.
    pub fn read(data: &[u8]) -> Result<Option<Normalization>, DecodeError> {
        let scene = scene_bytes(data)?;
        let blocks = SceneBlocks::split(&scene)?;
        let meta = root_metadata(blocks.root_node)?;
        Ok(Normalization::from_metadata(&meta))
    }

    /// The centre and radius entries of the root node's metadata; a centre or radius that is not finite (or a radius
    /// that is not above zero) gives `None`, as in the mesh metadata, instead of NaN coordinates from [`Self::apply`].
    fn from_metadata(meta: &[(String, MetaValue)]) -> Option<Normalization> {
        let center = meta.iter().find_map(|(k, v)| match v {
            MetaValue::Vec3(c) if k == CENTER && c.iter().all(|x| x.is_finite()) => Some(*c),
            _ => None,
        });
        let radius = meta.iter().find_map(|(k, v)| match v {
            MetaValue::Float(r) if k == RADIUS && *r > 0.0 && r.is_finite() => Some(*r),
            _ => None,
        });
        center.zip(radius).map(|(center, radius)| Normalization { center, radius })
    }

    /// A point of the original scene in the file's normalised space.
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        [(p[0] - self.center[0]) / self.radius, (p[1] - self.center[1]) / self.radius, (p[2] - self.center[2]) / self.radius]
    }
}

#[derive(Debug, Clone, PartialEq)]
enum MetaValue {
    Float(f32),
    Vec3([f32; 3]),
    Matrix,
}

/// Skips the root node's own fields and children, then reads its metadata.
fn root_metadata(node: &[u8]) -> Result<Vec<(String, MetaValue)>, DecodeError> {
    let mut r = ByteReader::new(node);
    read_string(&mut r)?;
    r.skip(MATRIX_LEN).map_err(DecodeError::truncated(RECORD))?;
    let _children = read_u32(&mut r)?;
    let _meshes = read_u32(&mut r)?;
    let mesh_indices = read_count(&mut r, 4)?;
    r.skip(mesh_indices * 4).map_err(DecodeError::truncated(RECORD))?;
    let children = read_count(&mut r, 8)?;
    for _ in 0..children {
        read_block(&mut r)?;
    }
    read_metadata(&mut r)
}

fn read_metadata(r: &mut ByteReader) -> Result<Vec<(String, MetaValue)>, DecodeError> {
    if r.remaining() < 2 || r.u16_le().map_err(DecodeError::truncated(RECORD))? == 0 {
        return Ok(Vec::new());
    }
    let count = read_count(r, 4)?;
    let keys = (0..count).map(|_| read_string(r)).collect::<Result<Vec<_>, _>>()?;
    let values = read_count(r, 4)?;
    let mut out = Vec::with_capacity(values.min(count));
    for key in keys.into_iter().take(values) {
        let f = |r: &mut ByteReader| r.f32_le().map_err(DecodeError::truncated(RECORD));
        let value = match read_u32(r)? {
            3 => MetaValue::Float(f(r)?),
            5 => MetaValue::Vec3([f(r)?, f(r)?, f(r)?]),
            6 => {
                r.skip(MATRIX_LEN).map_err(DecodeError::truncated(RECORD))?;
                MetaValue::Matrix
            }
            _ => break,
        };
        out.push((key, value));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    fn node_with_metadata() -> Vec<u8> {
        let mut n = Vec::new();
        string(&mut n, "root");
        n.extend_from_slice(&[0; MATRIX_LEN]);
        n.extend_from_slice(&[0; 12]); // child count, mesh count, empty mesh array
        n.extend_from_slice(&0u32.to_le_bytes()); // no child blocks
        n.extend_from_slice(&1u16.to_le_bytes());
        n.extend_from_slice(&3u32.to_le_bytes());
        for k in [crate::format_names::SCENE_TOTAL_TRANSFORM, CENTER, RADIUS] {
            string(&mut n, k);
        }
        n.extend_from_slice(&3u32.to_le_bytes());
        n.extend_from_slice(&6u32.to_le_bytes());
        n.extend_from_slice(&[0; MATRIX_LEN]);
        n.extend_from_slice(&5u32.to_le_bytes());
        for v in [1.0f32, 2.0, 3.0] {
            n.extend_from_slice(&v.to_le_bytes());
        }
        n.extend_from_slice(&3u32.to_le_bytes());
        n.extend_from_slice(&4.0f32.to_le_bytes());
        n
    }

    #[test]
    fn reads_root_metadata() {
        let meta = root_metadata(&node_with_metadata()).unwrap();
        assert_eq!(meta.len(), 3);
        assert_eq!(meta[1], (CENTER.to_string(), MetaValue::Vec3([1.0, 2.0, 3.0])));
        assert_eq!(meta[2], (RADIUS.to_string(), MetaValue::Float(4.0)));
        let n = Normalization { center: [1.0, 2.0, 3.0], radius: 4.0 };
        assert_eq!(n.apply([5.0, 2.0, 3.0]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_centre_that_is_not_finite_is_not_a_normalisation() {
        let meta = |centre: [f32; 3]| vec![(CENTER.to_string(), MetaValue::Vec3(centre)), (RADIUS.to_string(), MetaValue::Float(1.0))];
        assert_eq!(Normalization::from_metadata(&meta([1.0, 2.0, 3.0])), Some(Normalization { center: [1.0, 2.0, 3.0], radius: 1.0 }));
        assert_eq!(Normalization::from_metadata(&meta([f32::NAN, 0.0, 0.0])), None);
        assert_eq!(Normalization::from_metadata(&meta([0.0, f32::INFINITY, 0.0])), None);
    }

    #[test]
    fn bad_input_errors_or_is_empty() {
        let data = node_with_metadata();
        for len in 0..data.len() {
            let _ = root_metadata(&data[..len]);
        }
        assert!(root_metadata(&[]).is_err());
    }
}
