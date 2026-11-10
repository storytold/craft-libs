//! Faces of one mesh, turned into triangles.
//!
//! Each face is `u32` flag, `u32` index count, then that many `u32` vertex indices.
//! The flag is 1 on the first triangle of a source polygon and 0 on the following ones
//! (the reference app's meshes are already triangulated); [`polygon_starts`] reads it for polygon-fill picks.

use super::RECORD;
use super::block::{read_count, read_u32};
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

/// Smallest face: flag and an index count of zero.
const MIN_FACE_LEN: usize = 8;
const INDEX_LEN: usize = 4;

/// Reads `face_count` faces from `faces`; polygons are fan-triangulated, points and lines skipped.
/// Every index must be below `vertex_count`.
pub fn triangulate(faces: &[u8], face_count: u32, vertex_count: usize) -> Result<Vec<[u32; 3]>, DecodeError> {
    let face_count = face_count as usize;
    if face_count.saturating_mul(MIN_FACE_LEN) > faces.len() {
        return Err(DecodeError::invalid(RECORD, format!("{face_count} faces do not fit")));
    }
    let mut r = ByteReader::new(faces);
    let mut triangles = Vec::with_capacity(face_count);
    let mut polygon = Vec::new();
    for _ in 0..face_count {
        let _starts_polygon = read_u32(&mut r)?;
        let len = read_count(&mut r, INDEX_LEN)?;
        polygon.clear();
        for _ in 0..len {
            polygon.push(checked_index(read_u32(&mut r)?, vertex_count)?);
        }
        if let [first, rest @ ..] = polygon.as_slice() {
            triangles.extend(rest.iter().zip(rest.iter().skip(1)).map(|(&b, &c)| [*first, b, c]));
        }
    }
    Ok(triangles)
}

/// Per triangle of [`triangulate`]'s result: true on the first triangle of a source polygon (the face's flag;
/// the further triangles of a fanned face never start one).
pub fn polygon_starts(faces: &[u8], face_count: u32) -> Result<Vec<bool>, DecodeError> {
    let face_count = face_count as usize;
    if face_count.saturating_mul(MIN_FACE_LEN) > faces.len() {
        return Err(DecodeError::invalid(RECORD, format!("{face_count} faces do not fit")));
    }
    let mut r = ByteReader::new(faces);
    let mut starts = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        let flag = read_u32(&mut r)? != 0;
        let len = read_count(&mut r, INDEX_LEN)?;
        for _ in 0..len {
            read_u32(&mut r)?;
        }
        for k in 0..len.saturating_sub(2) {
            starts.push(flag && k == 0);
        }
    }
    Ok(starts)
}

fn checked_index(index: u32, vertex_count: usize) -> Result<u32, DecodeError> {
    if (index as usize) < vertex_count {
        Ok(index)
    } else {
        Err(DecodeError::invalid(RECORD, format!("face index {index} is out of range for {vertex_count} vertices")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(indices: &[u32]) -> Vec<u8> {
        let mut out = vec![1, 0, 0, 0];
        out.extend_from_slice(&(indices.len() as u32).to_le_bytes());
        indices.iter().for_each(|i| out.extend_from_slice(&i.to_le_bytes()));
        out
    }

    #[test]
    fn fans_polygons_and_skips_points_and_lines() {
        let data = [face(&[0, 1, 2, 3]), face(&[4]), face(&[4, 5]), face(&[3, 4, 5])].concat();
        let tris = triangulate(&data, 4, 6).unwrap();
        assert_eq!(tris, vec![[0, 1, 2], [0, 2, 3], [3, 4, 5]]);
    }

    #[test]
    fn polygon_flags_follow_the_triangles() {
        let mut second = face(&[1, 2, 3]);
        second[0] = 0;
        let data = [face(&[0, 1, 2, 3]), face(&[4]), second, face(&[3, 4, 5])].concat();
        assert_eq!(polygon_starts(&data, 4).unwrap(), vec![true, false, false, true]);
    }

    #[test]
    fn rejects_bad_indices_and_counts() {
        assert!(triangulate(&face(&[0, 1, 9]), 1, 3).is_err());
        assert!(triangulate(&face(&[0, 1, 2]), 2, 3).is_err());
        assert!(triangulate(&face(&[0, 1, 2]), u32::MAX, 3).is_err());
    }
}
