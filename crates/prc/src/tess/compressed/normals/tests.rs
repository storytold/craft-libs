#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;

fn close(a: V, b: V) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
}

fn normals(pos: &[V], tris: &[[u32; 3]], binary: &[u8], angles: &[i32], planar: bool) -> Option<(Vec<V>, Vec<[u32; 3]>)> {
    let binary: Vec<bool> = binary.iter().map(|&b| b == 1).collect();
    stored_normals(pos, tris, &NormalArrays { bits: 10, binary: &binary, angles, planar: &[planar], face_of: &vec![0; tris.len()] })
}

const QUAD: [V; 4] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];

/// Corners are read `min`, `max`, apex but returned in the triangle's
/// own order. Each record lands in the frame `X = -V2`, `Z` the face
/// normal: phi 0 points along `X`, phi at full scale along `Z`, and
/// the reversed flag flips `Z`.
#[test]
fn records_decode_in_the_triangle_frame() {
    let (n, c) = normals(&QUAD, &[[1, 0, 2]], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0], &[0, 0, 0, 1023, 0, 1023], false).unwrap();
    assert_eq!(c, [[1, 0, 2]]);
    assert!(close(n[0], [0.0, -1.0, 0.0]), "{:?}", n[0]);
    assert!(close(n[1], [0.0, 0.0, 1.0]), "{:?}", n[1]);
    assert!(close(n[2], [0.0, 0.0, -1.0]), "{:?}", n[2]);
}

/// A vertex with multiple normals references one by index from its
/// most recent, or stores another; a vertex with one normal reuses it
/// without reading; leftover bits refuse the whole mesh.
#[test]
fn references_reuse_and_new_records() {
    let tris = [[0, 1, 2], [2, 1, 3]];
    let first = [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    let a = [0, 1023];
    let angles: Vec<i32> = a.iter().cycle().take(8).copied().collect();
    let mut bits = first.to_vec();
    bits.extend([1, 0, 0, 0, 0, 0]);
    let (n, c) = normals(&QUAD, &tris, &bits, &angles, false).unwrap();
    assert_eq!(c, [[0, 1, 2], [2, 1, 3]]);
    assert_eq!(n.len(), 4);
    let mut bits = first.to_vec();
    bits.extend([0, 0, 0, 0, 0, 0, 0, 0]);
    let angles: Vec<i32> = a.iter().cycle().take(10).copied().collect();
    let (n, c) = normals(&QUAD, &tris, &bits, &angles, false).unwrap();
    assert_eq!(c, [[0, 1, 2], [2, 3, 4]]);
    assert_eq!(n.len(), 5);
    // Index 0 is vertex 1's most recent normal, 3, not its first.
    let mut more = tris.to_vec();
    more.push([3, 1, 0]);
    let mut three = bits.clone();
    three.extend([1, 0]);
    let (_, c) = normals(&QUAD, &more, &three, &angles, false).unwrap();
    assert_eq!(c[2], [4, 3, 0]);
    bits.push(0);
    assert!(normals(&QUAD, &tris, &bits, &angles, false).is_none());
}

/// The reference width at every measured stored count.
#[test]
fn reference_width_matches_the_measured_counts() {
    let measured = [(1, 1), (2, 1), (3, 2), (4, 3), (5, 3), (6, 4), (8, 4), (12, 5), (14, 5)];
    for (n, w) in measured {
        assert_eq!(reference_bits(n), w, "n = {n}");
    }
}

/// A planar face reads one record, at the first corner of its first
/// triangle, and every corner takes it.
#[test]
fn a_planar_face_stores_one_normal() {
    let tris = [[0, 1, 2], [2, 1, 3]];
    let (n, c) = normals(&QUAD, &tris, &[0, 0, 0, 0], &[0, 1023], true).unwrap();
    assert_eq!(c, [[0; 3]; 2]);
    assert!(close(n[0], [0.0, 0.0, 1.0]), "{:?}", n[0]);
}

/// A planar face whose first corner already holds another face's one
/// normal borrows it, and says so; marked blind, it says that instead,
/// until it is rewound and read again.
#[test]
fn a_planar_face_borrowing_or_blind_says_so() {
    let binary = [false; 4];
    let a = NormalArrays { bits: 10, binary: &binary, angles: &[0, 1023], planar: &[true, true], face_of: &[0, 1, 1] };
    let mut r = NormalReader::new(&a);
    let own = r.read(0, [0, 1, 2], &QUAD).unwrap();
    assert!(matches!(own, Orientation::Reversed(..)));
    let m = r.mark();
    r.read(1, [0, 3, 1], &QUAD).unwrap();
    let next = r.read(2, [3, 0, 2], &QUAD).unwrap();
    assert!(matches!(next, Orientation::Borrowed(1, _)));
    r.blind(2);
    let next = r.read(2, [3, 0, 2], &QUAD).unwrap();
    assert!(matches!(next, Orientation::Blind(1)));
    r.rewind(m);
    r.read(1, [0, 3, 1], &QUAD).unwrap();
    let next = r.read(2, [3, 0, 2], &QUAD).unwrap();
    assert!(matches!(next, Orientation::Borrowed(1, _)));
}
