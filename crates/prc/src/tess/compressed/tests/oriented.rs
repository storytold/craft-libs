//! Walks oriented by stored normals, the retries and the search.

use super::*;

/// Four triangles, the third's apex `(1, 3, 1)` off its parent's plane so the default rule sees
/// no fold; `bits`/`angles` are the stored normals, `planar` the one face's flag.
fn oriented(bits: &[u8], angles: &[i32], planar: bool) -> Rebuilt {
    let binary: Vec<bool> = bits.iter().map(|&b| b == 1).collect();
    let pts = [0, 0, 0, 4, 0, 0, -2, 4, 0, 0, -3, i64::from(!planar), 1, 3, 1];
    let mut is_ref = [false; 6];
    is_ref[5] = true;
    rebuild(&Arrays {
        tolerance: 0.5,
        origin: [10.0, 0.0, 0.0],
        points: &pts,
        edge_status: &[3, 3, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        triangles: 4,
        is_reference: &is_ref,
        references: &[0],
        normals: Some(NormalArrays { bits: 10, binary: &binary, angles, planar: &[planar], face_of: &[0; 4] }),
        ortho_turned: false,
        limit: usize::MAX,
        steps: unlimited(),
    })
    .unwrap()
}

const FOLDED: [[u32; 3]; 4] = [[0, 1, 2], [2, 1, 3], [3, 1, 4], [3, 4, 0]];

fn one_face(pts: &[i64], planar: bool) -> Result<Rebuilt, &'static str> {
    let mut status = [0; 6];
    status[0] = 2;
    rebuild(&Arrays {
        tolerance: 0.5,
        origin: [10.0, 0.0, 0.0],
        points: pts,
        edge_status: &status,
        triangles: 2,
        is_reference: &[false; 4],
        references: &[],
        normals: Some(NormalArrays { bits: 10, binary: &[false; 4], angles: &[0, 1023], planar: &[planar], face_of: &[0; 2] }),
        ortho_turned: false,
        limit: usize::MAX,
        steps: unlimited(),
    })
}

/// One planar face of `t` triangles whose sixth slot is its one reference.
fn face<'a>(pts: &'a [i64], status: &'a [i32], is_ref: &'a [bool], refs: &'a [u32], angles: &'a [i32], face_of: &'a [u32]) -> Arrays<'a> {
    Arrays {
        tolerance: 0.5,
        origin: [10.0, 0.0, 0.0],
        points: pts,
        edge_status: status,
        triangles: face_of.len(),
        is_reference: is_ref,
        references: refs,
        normals: Some(NormalArrays { bits: 10, binary: &[false; 4], angles, planar: &[true], face_of }),
        ortho_turned: false,
        limit: usize::MAX,
        steps: unlimited(),
    }
}

/// The sixth of `slots` slots flagged as a reference.
fn sixth_is_reference(slots: usize) -> Vec<bool> {
    let mut is_ref = vec![false; slots];
    is_ref[5] = true;
    is_ref
}

/// A record read at a triangle orients it exactly: the third triangle's fresh record at vertex
/// 4 has `reversed` clear while its winding runs `max` to `min`, so it folds [WD 7.8.9.1];
/// vertex 3's set bit, read at `[2 1 3]`, keeps the second unfolded.
#[test]
fn a_reversed_bit_orients_the_walk() {
    let mut bits = Vec::new();
    for rev in [0, 0, 0, 1, 0] {
        bits.extend([0, rev, 0, 0]);
    }
    let angles: Vec<i32> = [0, 1023].repeat(5);
    let m = oriented(&bits, &angles, false);
    assert_eq!(m.triangles, FOLDED);
    assert_eq!(m.normals.len(), 5);
    // Without the normals the default rule leaves it unfolded.
    let pts = [0, 0, 0, 4, 0, 0, -2, 4, 0, 0, -3, 1, 1, 3, 1];
    let plain = run(&pts, &[3, 3, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0], 4, &sixth_is_reference(6), &[0]).unwrap();
    assert_eq!(plain.triangles[3], [4, 1, 0]);
}

/// On a planar face only the seed reads a record; the third triangle winds against that face
/// normal, so it folds [WD 7.8.9].
#[test]
fn a_planar_face_normal_orients_the_walk() {
    let m = oriented(&[0, 0, 0, 0], &[0, 1023], true);
    assert_eq!(m.triangles, FOLDED);
    assert_eq!(m.triangle_normals, [[0; 3]; 4]);
}

/// Normals the oriented walk cannot use up send the mesh back to the default rule, which keeps
/// its triangles and drops the normals.
#[test]
fn normals_that_do_not_fit_fall_back_to_the_default_fold() {
    let m = oriented(&[0, 0, 0, 0, 1], &[0, 1023], true);
    assert_eq!(m.triangles[3], [4, 1, 0]);
    assert!(m.normals.is_empty() && m.triangle_normals.is_empty());
}

/// One planar face. The third triangle's apex `(4, 0, 0)` lies on its entry edge's line, a
/// sliver its face normal cannot orient, so it takes the default (unfolded). The fourth
/// triangle's reference then reuses an edge a third time, and inverting the second triangle
/// does not help: only the sliver, folded, fits.
#[test]
fn a_sliver_fold_is_found_by_retrying_at_the_sliver() {
    let pts = [0, 0, 0, 4, 0, 0, -2, 4, 0, 0, 3, 0, 4, 0, 0, 0, 3, 0];
    let mut status = [0; 15];
    status[..3].copy_from_slice(&[1, 1, 3]);
    let is_ref = sixth_is_reference(7);
    let a = face(&pts, &status, &is_ref, &[0], &[0, 1023], &[0; 5]);
    let m = rebuild(&a).unwrap();
    assert_eq!(m.triangles, [[0, 1, 2], [0, 2, 3], [3, 2, 4], [3, 4, 0], [4, 2, 5]]);
    // Found by the retry, not left to the search.
    assert_eq!(oriented_fit(&a).unwrap().triangles, m.triangles);
    assert_eq!(m.normals.len(), 1);
}

/// Corners whose normals were all read at an earlier triangle still orient it [WD 7.8.9]; one
/// at most a tolerance thick keeps the default fold, as does a thin one's fresh record.
#[test]
fn reused_normals_orient_unless_thin() {
    let na = NormalArrays { bits: 10, binary: &[false; 16], angles: &[0, 1023, 0, 1023, 0, 1023, 0, 1023], planar: &[false], face_of: &[0; 8] };
    let mut w = Walk::new(3, usize::MAX, Some(NormalReader::new(&na)));
    w.pos = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
    assert!(!w.fold([0, 1, 2], false).unwrap());
    let both = |w: &mut Walk<'_>| [[0, 1, 2], [0, 2, 1]].map(|t| w.fold(t, false).unwrap());
    assert_eq!(both(&mut w), [false, true]);
    w.thin = true;
    assert_eq!(both(&mut w), [false, false]);
    // A fresh record's reversed bit, which would fold it, too.
    assert!(!w.fold([1, 0, 3], false).unwrap());
}

/// A fresh record whose normal lies in its triangle's plane carries a
/// `triangle_normal_reversed` bit set from a rounding residue ([`BLIND_NORMAL`]): the default
/// fold stands, unsignalled, and the triangle is marked for the sliver retry. One whose normal
/// leaves the plane decides as before.
#[test]
fn a_record_normal_in_the_plane_is_blind() {
    let na = NormalArrays { bits: 10, binary: &[false; 16], angles: &[0, 0, 0, 0, 0, 0, 0, 1023], planar: &[false], face_of: &[0; 8] };
    let mut w = Walk::new(3, usize::MAX, Some(NormalReader::new(&na)));
    w.pos = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
    assert!(!w.fold([0, 1, 2], false).unwrap());
    assert!(!w.signalled);
    assert_eq!(w.sliver, Some(0));
    w.sliver = None;
    assert!(w.fold([1, 0, 3], false).unwrap());
    assert!(w.signalled);
    assert_eq!(w.sliver, None);
}

/// Two triangles of one planar face folded about 70 degrees along their shared edge, far
/// enough that no plane holds all four corners within [`MAX_BEND`] tolerances: the arrays fit,
/// the face does not.
#[test]
fn a_planar_face_that_bends_is_refused() {
    let bent = [0, 0, 0, 80, 0, 0, -40, 80, 0, 0, -80, 240];
    assert!(one_face(&bent, true).is_err());
    assert!(one_face(&bent, false).is_ok());
    assert!(one_face(&[0, 0, 0, 80, 0, 0, -40, 80, 0, 0, -80, 0], true).is_ok());
}

/// No retry fits: the unsignalled reference triangles need folds the retries do not try. The
/// search finds exactly one fit.
#[test]
fn a_mesh_no_retry_fits_is_found_by_the_search() {
    let pts = [0, 0, 0, 4, 0, 0, -2, 4, 0, 1, 2, 3, 2, 3, 5, 3, 4, 7];
    let mut status = [0; 18];
    status[..6].copy_from_slice(&[3, 0, 1, 3, 0, 0]);
    let mut is_ref = [false; 8];
    is_ref[5] = true;
    is_ref[7] = true;
    let a = Arrays {
        tolerance: 0.5,
        origin: [10.0, 0.0, 0.0],
        points: &pts,
        edge_status: &status,
        triangles: 6,
        is_reference: &is_ref,
        references: &[3, 2],
        normals: Some(NormalArrays { bits: 10, binary: &[false; 24], angles: &[0; 12], planar: &[false], face_of: &[0; 6] }),
        ortho_turned: false,
        limit: usize::MAX,
        steps: unlimited(),
    };
    assert!(walk(&a, None).is_err());
    let m = rebuild(&a).unwrap();
    assert_eq!(m.triangles, [[0, 1, 2], [2, 1, 3], [0, 2, 4], [0, 4, 3], [0, 3, 5], [3, 4, 2]]);
}

/// Rebuilds a one-face mesh with the WD frame and with the frame turned; the WD frame must not
/// orient it to the end, and the rebuild must equal `expected` on the turned frame.
fn with_turned_frame(
    pts: &[i64],
    status: &[i32],
    slots: usize,
    refs: &[u32],
    angles: &[i32],
    walks: bool,
    expected: fn(&Arrays<'_>) -> Option<Rebuilt>,
) {
    let t = status.len() / 3;
    let is_ref = sixth_is_reference(slots);
    let face_of = vec![0; t];
    // An origin that keeps the arithmetic exact, so the degenerate apex's residue is zero and
    // the variant decides its turn.
    let a = Arrays { origin: [0.0; 3], ..face(pts, status, &is_ref, refs, angles, &face_of) };
    assert!(oriented_fit(&a).is_none());
    assert_eq!(walk(&a, None).is_ok(), walks);
    // The search met the apex the turn moves, so the turned reading runs.
    assert!(walks || search::unique_fit_reading(&a).1);
    let want = expected(&Arrays { ortho_turned: true, ..a }).unwrap();
    let m = rebuild(&a).unwrap();
    assert_eq!(m.triangles, want.triangles);
    assert_eq!(m.positions, want.positions);
}

fn unique_fit(a: &Arrays<'_>) -> Option<Rebuilt> {
    search::unique_fit_reading(a).0
}

/// An apex whose `W` lies on the edge's line takes its frame from `MakeOrthoRep`. In each mesh
/// the WD's frame folds a later triangle the wrong way, and only the frame turned a half-turn
/// about `X` rebuilds it: oriented by the stored normals after the plain walk fits, or found by
/// the search after it fails.
#[test]
fn a_frame_only_turned_fits_is_rebuilt_turned() {
    let tri0 = [0, 0, 0, 4, 0, 0, 2, 4, 0];
    let st = [1, 1, 3, 3, 3, 2, 3, 1, 3, 2, 3, 0];
    let pts = [&tri0[..], &[0, 0, 0, 3, 2, -1]].concat();
    with_turned_frame(&pts, &st, 6, &[2], &[414, 420], true, oriented_fit);
    let st = [1, 3, 1, 1, 1, 3, 0, 0, 1, 3, 2, 0];
    let pts = [&tri0[..], &[0, 0, 0, -4, -4, 2]].concat();
    with_turned_frame(&pts, &st, 6, &[2], &[470, 724], false, unique_fit);
    let st = [1, 3, 2, 1, 2, 2, 2, 0, 1, 1, 2, 0, 2, 3, 3];
    let pts = [&tri0[..], &[-4, 0, 0, 3, 3, 3, -1, 0, 0]].concat();
    with_turned_frame(&pts, &st, 7, &[0], &[837, 402], false, unique_fit);
    let d = [1.0, 2.0, 3.0];
    assert_eq!(apex([0.0; 3], [-2.0, 0.0, 0.0], [5.0, 0.0, 0.0], d, true, false), ([0.0, 2.0, 3.0], true));
}

/// The turned variant moves only a residue-free degenerate frame; `invert` reverses whichever
/// turn applies.
#[test]
fn a_degenerate_frame_turns_by_variant_and_inverts() {
    let (lo, hi, d) = ([0.0; 3], [-2.0, 0.0, 0.0], [1.0, 2.0, 3.0]);
    let along = [5.0, -1e-9, 0.0];
    assert_eq!(apex(lo, hi, along, d, true, false), ([0.0, -2.0, -3.0], true));
    assert_eq!(apex(lo, hi, along, d, true, true), ([0.0, 2.0, 3.0], true));
    assert_eq!(apex(lo, hi, [5.0, 0.0, 0.0], d, false, true), ([0.0, 2.0, 3.0], true));
}

/// A mesh whose degenerate apex's residue turns its frame the wrong way for any fold-only
/// search: the search's turn choice finds the one fit.
#[test]
fn a_turn_only_the_search_tries_is_rebuilt() {
    let tri0 = [0, 0, 0, 4, 0, 0, 2, 4, 0];
    let st = [1, 3, 1, 1, 1, 3, 0, 0, 1, 3, 2, 0];
    let pts = [&tri0[..], &[0, 0, 0, -4, -4, 2]].concat();
    let is_ref = sixth_is_reference(6);
    let a = face(&pts, &st, &is_ref, &[2], &[470, 724], &[0; 4]);
    assert!(!search::fits_within(&a, false, 6));
    assert!(unique_fit(&a).is_some());
    // With the fourth point moved, every degenerate apex the search meets keeps a residue: it
    // still fits, no walk reads the turn, and the turned reading is skipped.
    let mut moved = pts.clone();
    moved[9] = 1;
    let moved = Arrays { points: &moved, ..a };
    let (fit, reads) = search::unique_fit_reading(&moved);
    assert!(fit.is_some() && !reads);
}

/// The search is bounded by its depth: the one-turn mesh above has no fit within one inverted
/// choice and one within two, and the last stage searches deeper than the first two.
#[test]
fn the_search_depth_bounds_its_choices() {
    let tri0 = [0, 0, 0, 4, 0, 0, 2, 4, 0];
    let st = [1, 3, 1, 1, 1, 3, 0, 0, 1, 3, 2, 0];
    let pts = [&tri0[..], &[0, 0, 0, -4, -4, 2]].concat();
    let is_ref = sixth_is_reference(6);
    let a = face(&pts, &st, &is_ref, &[2], &[470, 724], &[0; 4]);
    assert!(!search::fits_within(&a, true, 1));
    assert!(search::fits_within(&a, true, 2));
    let [.., (_, shallow), (turns, deep)] = search::STAGES;
    assert!(turns && deep > shallow);
}

/// A sliver, at most a tolerance from collinear, whose stored decision orients its fold the
/// wrong way: the search still tries inverting it, and that is the only fit.
#[test]
fn a_signalled_sliver_is_a_search_candidate() {
    let pts = [0, 0, 0, 4, 0, 0, 2, 4, 0, -4, -1, 1, -1, -1, 0, 4, 0, -3];
    let status = [1, 3, 3, 1, 2, 1, 1, 0, 1, 2, 2, 1, 3, 1, 3];
    let is_ref = sixth_is_reference(7);
    let a = face(&pts, &status, &is_ref, &[0], &[60, 476], &[0; 5]);
    assert!(walk(&a, None).is_err());
    assert_eq!(rebuild(&a).unwrap().triangles, [[0, 1, 2], [0, 2, 3], [3, 2, 4], [3, 4, 0], [0, 4, 5]]);
}

/// A mesh whose `MakeOrthoRep` frames fit, after one inverted fold, the WD's way round: the
/// search finds that fit, and the turned frame fits nothing else.
#[test]
fn a_wd_frame_fit_is_found_by_the_search() {
    let pts = [0, 0, 0, 4, 0, 0, 2, 4, 0, 1, 0, 0, -3, 1, -1, -2, 3, 1];
    let status = [3, 3, 3, 2, 2, 1, 1, 2, 3, 3, 1, 3, 1, 1, 0];
    let is_ref = sixth_is_reference(7);
    let a = Arrays {
        normals: Some(NormalArrays { bits: 10, binary: &[true, false, false, true], angles: &[302, 340], planar: &[true], face_of: &[0; 5] }),
        ..face(&pts, &status, &is_ref, &[2], &[], &[0; 5])
    };
    assert!(walk(&a, None).is_err());
    let wd = [[0, 1, 2], [2, 1, 3], [3, 1, 4], [3, 4, 2], [2, 4, 5]];
    let other = unique_fit(&Arrays { ortho_turned: true, ..a }).map(|m| m.triangles);
    assert!(other.is_none() || other == Some(wd.to_vec()));
    assert_eq!(rebuild(&a).unwrap().triangles, wd);
}

/// A planar face's normal tilted within its own quantum cannot orient a triangle that is tall
/// in space but nearly flat along the face: the fourth triangle's in-face height is a twelfth
/// of a tolerance and the sign of its winding against the normal is noise. It takes the
/// default fold and the mesh matches the untilted control ([`MIN_PLANE_HEIGHT`]); oriented by
/// the tilted normal it would close a different fifth triangle, `[3, 0, 5]`.
#[test]
fn a_triangle_flat_along_its_face_is_a_sliver() {
    let pts = [0, 0, 0, 4, 0, 0, -2, 4, 0, 0, -3, 0, 1, 3, 1, 0, 3, 1];
    let status = [3, 3, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let is_ref = sixth_is_reference(7);
    let mesh = |angles: &[i32]| rebuild(&face(&pts, &status, &is_ref, &[0], angles, &[0; 5])).unwrap().triangles;
    let expected = [[0, 1, 2], [2, 1, 3], [3, 1, 4], [3, 4, 0], [0, 4, 5]];
    assert_eq!(mesh(&[341, 909]), expected);
    assert_eq!(mesh(&[0, 1023]), expected);
}
