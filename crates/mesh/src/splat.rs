//! 3D Gaussian Splatting (`.splat`) particle and point-cloud reader and writer.
//!
//! Each splat record is 32 bytes in little-endian layout:
//! - Position `[f32; 3]` (12 bytes)
//! - Scale `[f32; 3]` (12 bytes)
//! - Colour RGBA `[u8; 4]` (4 bytes, sRGB)
//! - Rotation quaternion `[u8; 4]` (4 bytes, normalized `0..=255`)
//!
//! [`read`] decodes the particle positions and vertex colours (converted to linear RGBA, matching
//! [`Mesh::colors`]) into a [`Scene`] of one mesh, within [`Limits`]. [`write`] serializes every
//! placed vertex of a [`Scene`] as a 32-byte `.splat` record.
//!
//! Moved from `encoderCraft`'s `encodercraft-spatial-io::splat` module.

use crate::{Corner, Error, Limits, Mesh, Part, Result, Scene, face_normal, linear_to_srgb, malformed, srgb_to_linear, too_large, transform_point};

const KIND: &str = "SPLAT";
/// Size in bytes of one 3D Gaussian Splat record.
pub const RECORD_SIZE: usize = 32;

/// Read a 3D Gaussian Splat (`.splat`) binary buffer into a [`Scene`], within [`Limits`].
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(RECORD_SIZE) {
        return Err(malformed(KIND, "file length must be a non-zero multiple of 32 bytes"));
    }
    if bytes.len() > limits.max_data_bytes {
        return Err(too_large(KIND, format!("{} bytes exceeds the {}-byte data limit", bytes.len(), limits.max_data_bytes)));
    }
    let splat_count = bytes.len() / RECORD_SIZE;
    if splat_count > limits.max_vertices {
        return Err(too_large(KIND, format!("{splat_count} splats exceeds the {}-vertex limit", limits.max_vertices)));
    }
    let tri_count = splat_count.saturating_sub(2);
    if tri_count > limits.max_triangles {
        return Err(too_large(KIND, format!("{tri_count} triangles exceeds the {}-triangle limit", limits.max_triangles)));
    }

    let mut mesh =
        Mesh { name: "SplatCloud".into(), positions: Vec::with_capacity(splat_count), colors: Vec::with_capacity(splat_count), ..Mesh::default() };

    for chunk in bytes.as_chunks::<RECORD_SIZE>().0 {
        let x = f64::from(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        let y = f64::from(f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
        let z = f64::from(f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]));
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err(malformed(KIND, "non-finite splat position"));
        }
        mesh.positions.push([x, y, z]);
        let r = srgb_to_linear(f32::from(chunk[24]) / 255.0);
        let g = srgb_to_linear(f32::from(chunk[25]) / 255.0);
        let b = srgb_to_linear(f32::from(chunk[26]) / 255.0);
        let a = f32::from(chunk[27]) / 255.0;
        mesh.colors.push([r, g, b, a]);
    }

    if tri_count > 0 {
        let mut part = Part::default();
        for i in 0..tri_count {
            let (Ok(i0), Ok(i1), Ok(i2), Ok(normal_idx)) =
                (u32::try_from(i), u32::try_from(i + 1), u32::try_from(i + 2), u32::try_from(mesh.normals.len()))
            else {
                return Err(too_large(KIND, "more vertices than a 32-bit index holds"));
            };
            let n = face_normal([mesh.positions[i], mesh.positions[i + 1], mesh.positions[i + 2]]).unwrap_or([0.0, 0.0, 1.0]);
            mesh.normals.push(n);
            for pos in [i0, i1, i2] {
                part.corners.push(Corner { position: pos, normal: normal_idx, uv: crate::NONE });
            }
        }
        mesh.parts.push(part);
    }

    Ok(Scene::from_meshes(vec![mesh], Vec::new()))
}

/// Serialize every mesh of `scene`, placed by its nodes, into a 3D Gaussian Splat (`.splat`) binary buffer.
pub fn write(scene: &Scene, limits: &Limits) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut count = 0usize;
    for (index, world) in scene.instances(limits)? {
        let Some(mesh) = scene.meshes.get(index) else { continue };
        for (i, &pos) in mesh.positions.iter().enumerate() {
            count = count.saturating_add(1);
            if count > limits.max_vertices {
                return Err(Error::TooLarge(format!("SPLAT: more than {} vertices", limits.max_vertices)));
            }
            let p = transform_point(&world, pos);
            out.extend_from_slice(&(p[0] as f32).to_le_bytes());
            out.extend_from_slice(&(p[1] as f32).to_le_bytes());
            out.extend_from_slice(&(p[2] as f32).to_le_bytes());
            for _ in 0..3 {
                out.extend_from_slice(&0.1f32.to_le_bytes());
            }
            let rgba = mesh.colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            out.extend_from_slice(&[to_u8(linear_to_srgb(rgba[0])), to_u8(linear_to_srgb(rgba[1])), to_u8(linear_to_srgb(rgba[2])), to_u8(rgba[3])]);
            out.extend_from_slice(&[128, 128, 128, 255]);
        }
    }
    Ok(out)
}
