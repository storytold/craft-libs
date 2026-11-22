use super::test_writer::{TestMesh, compressed_file, file, header};
use super::*;

fn two_meshes() -> [TestMesh<'static>; 2] {
    [
        TestMesh { name: "Body", material_index: 0, vertices: 4, triangles: 2, uv_sets: 1, color_sets: 0 },
        TestMesh { name: "Lid", material_index: 1, vertices: 3, triangles: 1, uv_sets: 2, color_sets: 1 },
    ]
}

fn expected() -> AssbinScene {
    AssbinScene {
        meshes: vec![
            MeshSummary { name: "Body".into(), vertices: 4, faces: 2, uv_channels: 1, material_index: 0 },
            MeshSummary { name: "Lid".into(), vertices: 3, faces: 1, uv_channels: 2, material_index: 1 },
        ],
        materials: vec![MaterialSummary { name: "Vase".into(), uid: Some(4) }, MaterialSummary { name: "Cup".into(), uid: Some(3) }],
    }
}

#[test]
fn reads_meshes_and_material_names() {
    let data = file(&two_meshes(), &["Vase", "Cup"]);
    assert_eq!(AssbinScene::parse(&data), Ok(expected()));
}

#[test]
fn reads_compressed_scenes() {
    let data = compressed_file(&two_meshes(), &["Vase", "Cup"]);
    assert_eq!(AssbinScene::parse(&data), Ok(expected()));
}

#[test]
fn reads_header_fields() {
    let h = AssbinHeader::parse(&file(&[], &[])).unwrap();
    assert_eq!((h.version_major, h.version_minor), (1, 7));
    assert!(!h.shortened && !h.compressed);
    assert_eq!(h.source_name, "file:///test.fbx");
}

#[test]
fn rejects_other_signatures() {
    let mut data = file(&[], &[]);
    data[0] = b'X';
    assert_eq!(AssbinScene::parse(&data), Err(DecodeError::BadMagic { record: RECORD }));
}

#[test]
fn rejects_shortened_dumps() {
    let mut data = header(7, false).0;
    data[60] = 1;
    assert!(matches!(AssbinScene::parse(&data), Err(DecodeError::Invalid { .. })));
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let data = file(&two_meshes(), &["Vase", "Cup"]);
    for len in 0..data.len() - 16 {
        assert!(AssbinScene::parse(&data[..len]).is_err(), "length {len}");
    }
}

#[test]
fn huge_counts_are_rejected() {
    let mut data = file(&two_meshes(), &["Vase"]);
    // The mesh count follows the 512-byte header, the scene size, the flags and the root node.
    let root_size_at = 512 + 8 + 4;
    let root_size = u64::from_le_bytes(data[root_size_at..root_size_at + 8].try_into().unwrap());
    let count_at = root_size_at + root_size as usize;
    data[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(AssbinScene::parse(&data), Err(DecodeError::Invalid { .. })));
}

/// Real files are user content, so they stay outside the repository.
/// Run with `TC_IMPORT_CORPUS=<folders or project files> cargo test -p craft-spp-reader -- --ignored`: prints each project's scene.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore]
fn parses_real_samples() {
    let Some(files) = crate::test_corpus::files() else {
        return;
    };
    for path in &files {
        let project = crate::project::ProjectFile::open(path).unwrap();
        let data = project.read(crate::project::paths::SCENE).unwrap();
        let version = AssbinHeader::parse(&data).map(|h| (h.version_minor, h.compressed));
        println!("{} {:?} {:?}", path.display(), version, AssbinScene::parse(&data));
    }
}
