//! Checks over real projects, which stay outside the repository (user content).
//! Run with `TC_IMPORT_CORPUS=<folders or project files> cargo test --release -p texturecraft-import-reader corpus -- --ignored --nocapture`.

use super::node::mesh_instances;
use super::scene::{SceneBlocks, scene_bytes};
use super::scene_mesh::SceneMesh as Mesh;
use super::transform::Transform;
use super::*;
use crate::project::ProjectFile;

use crate::project::paths::SCENE;

/// Node-tree facts about one scene.
#[derive(Default, Debug)]
struct Placement {
    non_identity: usize,
    mirrored: usize,
    repeated_meshes: usize,
    unplaced_meshes: usize,
}

fn placement(data: &[u8]) -> Placement {
    let scene = scene_bytes(data).unwrap();
    let blocks = SceneBlocks::split(&scene).unwrap();
    let instances = mesh_instances(blocks.root_node).unwrap();
    let mut uses = vec![0usize; blocks.meshes.len()];
    instances.iter().for_each(|i| uses[i.mesh as usize] += 1);
    Placement {
        non_identity: instances.iter().filter(|i| i.world != Transform::IDENTITY).count(),
        mirrored: instances.iter().filter(|i| i.world.determinant() < 0.0).count(),
        repeated_meshes: uses.iter().filter(|&&u| u > 1).count(),
        unplaced_meshes: uses.iter().filter(|&&u| u == 0).count(),
    }
}

/// Compares the geometry with the summary: one part per mesh, same vertex and face counts.
fn compare(summary: &AssbinScene, mesh: &Mesh) -> Result<(), String> {
    mesh.validate().map_err(|e| format!("validate: {e}"))?;
    if mesh.parts.len() != summary.meshes.len() {
        return Err(format!("{} parts for {} meshes", mesh.parts.len(), summary.meshes.len()));
    }
    for part in &mesh.parts {
        let found = summary.meshes.iter().find(|m| {
            m.name == part.name
                && m.vertices as usize == part.positions.len()
                && m.faces as usize == part.triangles.len()
                && m.uv_channels as usize == part.uv_sets.len()
        });
        if found.is_none() {
            return Err(format!("part '{}' matches no summary mesh", part.name));
        }
        if part.tangents.is_empty() || part.normals.is_empty() {
            return Err(format!("part '{}' lacks normals or tangents", part.name));
        }
    }
    Ok(())
}

#[test]
#[ignore]
fn corpus_geometry_matches_summary() {
    let Some(files) = crate::test_corpus::files() else { return };
    let (mut ok, mut failed) = (0, 0);
    let mut totals = Placement::default();
    let (mut parts, mut triangles, mut flipped_w) = (0usize, 0usize, 0usize);
    for path in &files {
        let result = ProjectFile::open(path).map_err(|e| e.to_string()).and_then(|p| p.read(SCENE).map_err(|e| e.to_string())).and_then(|data| {
            let summary = AssbinScene::parse(&data).map_err(|e| e.to_string())?;
            let mesh = AssbinGeometry::parse(&data).map_err(|e| e.to_string())?;
            compare(&summary, &mesh)?;
            Ok((placement(&data), mesh))
        });
        match result {
            Ok((p, mesh)) => {
                ok += 1;
                totals.non_identity += p.non_identity;
                totals.mirrored += p.mirrored;
                totals.repeated_meshes += p.repeated_meshes;
                totals.unplaced_meshes += p.unplaced_meshes;
                parts += mesh.parts.len();
                triangles += mesh.parts.iter().map(|p| p.triangles.len()).sum::<usize>();
                flipped_w += mesh.parts.iter().flat_map(|p| &p.tangents).filter(|t| t[3] < 0.0).count();
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {}: {e}", path.display());
            }
        }
    }
    println!("{} files: {ok} ok, {failed} failed; {parts} parts, {triangles} triangles", files.len());
    println!("placements: {totals:?}; tangents with w = -1: {flipped_w}");
    assert_eq!(failed, 0);
}
