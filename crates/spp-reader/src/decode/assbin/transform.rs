//! Assimp's 4x4 matrices: row-major, applied to column vectors (`p' = M p`), translation in
//! the last column.

pub type Vec3 = [f32; 3];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    m: [[f32; 4]; 4],
}

impl Transform {
    pub const IDENTITY: Transform = Transform { m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]] };

    /// From 16 values in Assimp's storage order (row by row).
    pub fn from_rows(v: [f32; 16]) -> Self {
        let [a0, a1, a2, a3, b0, b1, b2, b3, c0, c1, c2, c3, d0, d1, d2, d3] = v;
        Transform { m: [[a0, a1, a2, a3], [b0, b1, b2, b3], [c0, c1, c2, c3], [d0, d1, d2, d3]] }
    }

    /// `self * other`: applies `other` first.
    pub fn then_local(&self, other: &Transform) -> Transform {
        let columns = transpose(other.m);
        let m = self.m.map(|row| columns.map(|col| row.iter().zip(&col).map(|(a, b)| a * b).sum()));
        Transform { m }
    }

    pub fn point(&self, p: Vec3) -> Vec3 {
        let [x, y, z] = self.direction(p);
        let [[.., tx], [.., ty], [.., tz], _] = self.m;
        [x + tx, y + ty, z + tz]
    }

    /// Applies the upper 3x3 part (for tangents and bitangents).
    pub fn direction(&self, d: Vec3) -> Vec3 {
        let [r0, r1, r2, _] = self.m;
        let [x, y, z] = d;
        [r0, r1, r2].map(|[a, b, c, _]| a * x + b * y + c * z)
    }

    /// Determinant of the upper 3x3 part; negative when the transform mirrors.
    pub fn determinant(&self) -> f32 {
        let [[m00, m01, m02, _], [m10, m11, m12, _], [m20, m21, m22, _], _] = self.m;
        m00 * (m11 * m22 - m12 * m21) - m01 * (m10 * m22 - m12 * m20) + m02 * (m10 * m21 - m11 * m20)
    }

    /// Transforms a normal by the inverse transpose of the upper 3x3 part, using the cofactor
    /// matrix (inverse transpose times determinant); the result is normalised by the caller.
    /// A singular part gives a zero vector, which [`normalize`] leaves as it is.
    pub fn normal(&self, n: Vec3) -> Vec3 {
        let [[m00, m01, m02, _], [m10, m11, m12, _], [m20, m21, m22, _], _] = self.m;
        // The 2x2 minor `| a b ; c d |`.
        let minor = |a: f32, b: f32, c: f32, d: f32| a * d - b * c;
        let cofactors = [
            [minor(m11, m12, m21, m22), -minor(m10, m12, m20, m22), minor(m10, m11, m20, m21)],
            [-minor(m01, m02, m21, m22), minor(m00, m02, m20, m22), -minor(m00, m01, m20, m21)],
            [minor(m01, m02, m11, m12), -minor(m00, m02, m10, m12), minor(m00, m01, m10, m11)],
        ];
        let sign = if self.determinant() < 0.0 { -1.0 } else { 1.0 };
        let [x, y, z] = n;
        cofactors.map(|[a, b, c]| sign * (a * x + b * y + c * z))
    }
}

fn transpose(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let [[a0, a1, a2, a3], [b0, b1, b2, b3], [c0, c1, c2, c3], [d0, d1, d2, d3]] = m;
    [[a0, b0, c0, d0], [a1, b1, c1, d1], [a2, b2, c2, d2], [a3, b3, c3, d3]]
}

pub fn normalize(v: Vec3) -> Vec3 {
    let len = dot(v, v).sqrt();
    if len > 0.0 && len.is_finite() { v.map(|x| x / len) } else { v }
}

pub fn dot(a: Vec3, b: Vec3) -> f32 {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    ax * bx + ay * by + az * bz
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale_translate(s: [f32; 3], t: [f32; 3]) -> Transform {
        Transform::from_rows([s[0], 0., 0., t[0], 0., s[1], 0., t[1], 0., 0., s[2], t[2], 0., 0., 0., 1.])
    }

    #[test]
    fn translation_is_in_the_last_column_and_parents_apply_last() {
        let parent = scale_translate([2.0; 3], [0.0, 0.0, 0.0]);
        let child = scale_translate([1.0; 3], [1.0, 0.0, 0.0]);
        assert_eq!(parent.then_local(&child).point([0.0, 1.0, 0.0]), [2.0, 2.0, 0.0]);
    }

    #[test]
    fn normals_use_the_inverse_transpose_and_mirrors_have_negative_determinant() {
        let squash = scale_translate([1.0, 2.0, 1.0], [0.0; 3]);
        let n = normalize(squash.normal(normalize([1.0, 1.0, 0.0])));
        let expected = normalize([2.0, 1.0, 0.0]);
        assert!((0..3).all(|i| (n[i] - expected[i]).abs() < 1e-6), "{n:?}");
        let mirror = scale_translate([-1.0, 1.0, 1.0], [0.0; 3]);
        assert!(mirror.determinant() < 0.0);
        assert_eq!(mirror.normal([1.0, 0.0, 0.0]), [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn singular_transforms_give_zero_normals_that_stay_finite() {
        let flatten = scale_translate([1.0, 1.0, 0.0], [0.0; 3]);
        assert_eq!(flatten.determinant(), 0.0);
        let n = normalize(flatten.normal([1.0, 0.0, 0.0]));
        assert!(n.iter().all(|c| c.is_finite()), "{n:?}");
        assert_eq!(normalize([0.0; 3]), [0.0; 3]);
    }

    #[test]
    fn rows_and_products_keep_assimp_order() {
        let a = Transform::from_rows(std::array::from_fn(|i| i as f32));
        assert_eq!(a.point([0.0; 3]), [3.0, 7.0, 11.0]);
        assert_eq!(Transform::IDENTITY.then_local(&a), a);
        assert_eq!(a.then_local(&Transform::IDENTITY), a);
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    }
}
