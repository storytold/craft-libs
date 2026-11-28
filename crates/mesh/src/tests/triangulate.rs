//! Triangulating concave faces ([`Part::triangles_in`]).

use super::*;

/// A one-face part over corners `0..n`.
fn polygon(n: usize) -> Part {
    Part { face_sizes: vec![n as u32], corners: (0..n as u32).map(Corner::at).collect(), ..Part::default() }
}

/// Twice the signed area of a 2D triangle or of a closed 2D polygon.
fn area2(p: &[[f64; 2]]) -> f64 {
    (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum()
}

/// Whether `q` is strictly inside the 2D polygon `p`, by the even-odd rule.
fn inside(p: &[[f64; 2]], q: [f64; 2]) -> bool {
    let mut odd = false;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        if (a[1] > q[1]) != (b[1] > q[1]) && q[0] < a[0] + (q[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            odd = !odd;
        }
    }
    odd
}

/// Places the 2D outline on a tilted plane in 3D, so the triangulation has to find the plane.
fn lift(outline: &[[f64; 2]]) -> Vec<[f64; 3]> {
    // An orthonormal basis of the plane through (1, 2, 3), its normal leaning on every axis.
    let (u, v) = ([2.0 / 3.0, 2.0 / 3.0, -1.0 / 3.0], [-1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]);
    outline.iter().map(|p| [0, 1, 2].map(|k| [1.0, 2.0, 3.0][k] + p[0] * u[k] + p[1] * v[k])).collect()
}

/// Triangulates `outline` as one face lifted into 3D, and checks the triangles tile it: one
/// fewer than two per corner, each wound as the outline is and lying inside it (three points
/// inside every triangle are inside the outline), and their areas adding up to the outline's.
fn assert_tiles(outline: &[[f64; 2]]) -> Vec<[u32; 3]> {
    let positions = lift(outline);
    let tris: Vec<[u32; 3]> = polygon(outline.len()).triangles_in(&positions).map(|t| t.map(|c| c.position)).collect();
    assert_eq!(tris.len(), outline.len() - 2, "{tris:?}");
    let whole = area2(outline);
    let mut sum = 0.0;
    for t in &tris {
        let p = t.map(|i| outline[i as usize]);
        let a = area2(&p);
        assert!(a * whole.signum() > 0.0, "{t:?} is wound against the outline or has no area");
        sum += a.abs();
        for w in [[1.0, 1.0, 1.0], [4.0, 1.0, 1.0], [1.0, 4.0, 1.0], [1.0, 1.0, 4.0]] {
            let n: f64 = w.iter().sum();
            let q = [0, 1].map(|k| (0..3).map(|j| p[j][k] * w[j]).sum::<f64>() / n);
            assert!(inside(outline, q), "{t:?} leaves the outline at {q:?}");
        }
    }
    assert!((sum - whole.abs()).abs() < 1e-9 * whole.abs(), "areas {sum} and {whole}");
    tris
}

#[test]
fn concave_faces_are_not_fanned_across_their_notch() {
    // The issue's face: a fan from (0, 0) covers the notch between (2, 2), (1, 0.5) and (0, 2).
    let notch = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [1.0, 0.5], [0.0, 2.0]];
    let fan: Vec<[u32; 3]> = polygon(5).triangles().map(|t| t.map(|c| c.position)).collect();
    assert_eq!(fan, [[0, 1, 2], [0, 2, 3], [0, 3, 4]]);
    let tris = assert_tiles(&notch);
    assert_ne!(tris, fan);
    // The same face wound the other way.
    let mut cw = notch;
    cw.reverse();
    assert_tiles(&cw);
    // An L, a comb with nine teeth, a ten-pointed star and an arrow with straight corners.
    assert_tiles(&[[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [1.0, 1.0], [1.0, 3.0], [0.0, 3.0]]);
    let mut comb = vec![[0.0, 0.0], [18.0, 0.0]];
    for k in (0..9).rev() {
        let x = 2.0 * k as f64;
        comb.extend([[x + 2.0, 5.0], [x + 1.0, 5.0], [x + 1.0, 1.0]]);
    }
    comb.pop();
    assert_tiles(&comb);
    let star: Vec<[f64; 2]> = (0..20)
        .map(|k| {
            let (r, a) = (if k % 2 == 0 { 3.0 } else { 1.0 }, k as f64 * std::f64::consts::PI / 10.0);
            [r * a.cos(), r * a.sin()]
        })
        .collect();
    assert_tiles(&star);
    assert_tiles(&[[0.0, 1.0], [2.0, 1.0], [4.0, 1.0], [4.0, 0.0], [6.0, 2.0], [4.0, 4.0], [4.0, 3.0], [2.0, 3.0], [0.0, 3.0]]);
}

#[test]
fn convex_faces_keep_their_fan() {
    let hexagon: Vec<[f64; 2]> = (0..6).map(|k| [(k as f64).cos(), (k as f64).sin()]).collect();
    let positions = lift(&hexagon);
    let part = polygon(6);
    assert_eq!(part.triangles_in(&positions).collect::<Vec<_>>(), part.triangles().collect::<Vec<_>>());
    // A square with a straight corner on one side is still convex.
    let square = lift(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]);
    assert_eq!(polygon(5).triangles_in(&square).collect::<Vec<_>>(), polygon(5).triangles().collect::<Vec<_>>());
}

#[test]
fn faces_that_cannot_be_clipped_keep_their_triangle_count() {
    // No positions, a face with no area, and a bow tie that crosses itself.
    let bow = lift(&[[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]]);
    let flat = lift(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]]);
    for positions in [&[][..], &flat, &bow] {
        assert_eq!(polygon(4).triangles_in(positions).count(), 2);
    }
    // A comb of 4000 teeth: the work stays bounded and every triangle is still given.
    let mut comb = vec![[0.0, 0.0], [8000.0, 0.0]];
    for k in (0..4000).rev() {
        let x = 2.0 * k as f64;
        comb.extend([[x + 2.0, 5.0], [x + 1.0, 5.0], [x + 1.0, 1.0]]);
    }
    let positions = lift(&comb);
    let part = polygon(comb.len());
    assert_eq!(part.triangles_in(&positions).count(), part.triangle_count());
}

#[test]
fn unified_and_stl_use_the_clipped_triangles() {
    let notch = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [1.0, 0.5], [0.0, 2.0]];
    let mesh = Mesh { name: "Notch".into(), positions: lift(&notch), parts: vec![polygon(5)], ..Mesh::default() };
    let expected: Vec<[u32; 3]> = mesh.parts[0].triangles_in(&mesh.positions).map(|t| t.map(|c| c.position)).collect();
    let u = mesh.unified();
    let got: Vec<[u32; 3]> = u.parts[0].iter().map(|t| t.map(|i| u.corners[i as usize].position)).collect();
    assert_eq!(got, expected);
    #[cfg(feature = "stl")]
    {
        let scene = Scene::from_meshes(vec![mesh.clone()], vec![]);
        let text = crate::stl::write_ascii(&scene, "n", &Limits::default()).unwrap();
        let back = crate::stl::read(text.as_bytes(), &Limits::default()).unwrap();
        // The facets' area is the face's, 2.5, not the fan's 3.5.
        let m = &back.meshes[0];
        let area: f64 = m
            .parts
            .iter()
            .flat_map(Part::triangles)
            .map(|t| {
                let p = t.map(|c| m.positions[c.position as usize]);
                let (a, b) = ([0, 1, 2].map(|k| p[1][k] - p[0][k]), [0, 1, 2].map(|k| p[2][k] - p[0][k]));
                let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
            })
            .sum();
        assert!((area - 2.5).abs() < 1e-6, "{area}");
    }
}
