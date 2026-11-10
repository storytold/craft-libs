//! [`SceneMesh`]: the decoded geometry of the reference app's scene, as plain arrays an application converts into its own
//! mesh type. See [`super::part`] for the conventions (world space, glTF UVs, tangent handedness).

/// The whole scene: material slots (the reference app's texture sets) and one part per mesh placement.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneMesh {
    /// Material slot names; [`ScenePart::material`] indexes into this list.
    pub materials: Vec<String>,
    pub parts: Vec<ScenePart>,
}

/// One placed mesh.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScenePart {
    pub name: String,
    pub material: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// xyz tangent and w handedness (±1). Empty when the source had none.
    pub tangents: Vec<[f32; 4]>,
    /// One list per UV set; set 0 is the one the reference app paints on.
    pub uv_sets: Vec<Vec<[f32; 2]>>,
    pub triangles: Vec<[u32; 3]>,
    /// Per triangle: true where a source polygon starts (the reference app's polygon fill picks whole polygons). Triangles of
    /// one polygon follow each other; the first triangle always starts one.
    pub polygon_starts: Vec<bool>,
}

impl SceneMesh {
    /// Checks the structure: every part's material slot exists, every vertex array matches `positions` (normals and
    /// tangents may be empty), every triangle index is in range, `polygon_starts` is empty or one per triangle, and
    /// every value is finite. Says what is wrong with the first problem found.
    pub fn validate(&self) -> Result<(), String> {
        for (i, part) in self.parts.iter().enumerate() {
            if part.material >= self.materials.len() {
                return Err(format!("part {i}: material {} of {}", part.material, self.materials.len()));
            }
            part.validate().map_err(|e| format!("part {i}: {e}"))?;
        }
        Ok(())
    }
}

impl ScenePart {
    /// [`SceneMesh::validate`] for this part alone (its material slot is not checked).
    pub fn validate(&self) -> Result<(), String> {
        let n = self.positions.len();
        let len = |what: &str, found: usize, may_be_empty: bool| match found == n || (may_be_empty && found == 0) {
            true => Ok(()),
            false => Err(format!("{what}: {found} values for {n} vertices")),
        };
        len("normals", self.normals.len(), true)?;
        len("tangents", self.tangents.len(), true)?;
        for (s, uv) in self.uv_sets.iter().enumerate() {
            len(&format!("UV set {s}"), uv.len(), false)?;
        }
        if !self.polygon_starts.is_empty() && self.polygon_starts.len() != self.triangles.len() {
            return Err(format!("{} polygon starts for {} triangles", self.polygon_starts.len(), self.triangles.len()));
        }
        if let Some((t, i)) = self.triangles.iter().enumerate().find_map(|(t, tri)| tri.iter().find(|&&i| i as usize >= n).map(|i| (t, *i))) {
            return Err(format!("triangle {t}: vertex {i} of {n}"));
        }
        finite("positions", &self.positions)?;
        finite("normals", &self.normals)?;
        finite("tangents", &self.tangents)?;
        for uv in &self.uv_sets {
            finite("UVs", uv)?;
        }
        Ok(())
    }
}

/// Fails on the first vertex with a value that is NaN or infinite.
fn finite<const N: usize>(what: &str, values: &[[f32; N]]) -> Result<(), String> {
    match values.iter().position(|v| !v.iter().all(|x| x.is_finite())) {
        None => Ok(()),
        Some(v) => Err(format!("{what}: vertex {v} is not finite")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> ScenePart {
        ScenePart {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            uv_sets: vec![vec![[0.0; 2], [1.0, 0.0], [0.0, 1.0]]],
            triangles: vec![[0, 1, 2]],
            polygon_starts: vec![true],
            ..ScenePart::default()
        }
    }

    #[test]
    fn structural_problems_are_named() {
        let mesh = SceneMesh { materials: vec!["m".into()], parts: vec![triangle()] };
        assert_eq!(mesh.validate(), Ok(()));
        let mut bad = triangle();
        bad.triangles[0][2] = 7;
        assert!(bad.validate().is_err_and(|e| e.contains("vertex 7")));
        let mut bad = triangle();
        bad.normals = vec![[0.0; 3]];
        assert!(bad.validate().is_err());
        let mut bad = triangle();
        bad.positions[1][0] = f32::NAN;
        assert!(bad.validate().is_err_and(|e| e.contains("positions")));
        let mut wrong_slot = mesh.clone();
        wrong_slot.parts[0].material = 3;
        assert!(wrong_slot.validate().is_err());
    }
}
