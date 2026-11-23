use super::{hammer, tri_mesh};
use crate::{Error, Limits, Scene, splat};

#[test]
fn splat_round_trip_and_limits() {
    let mut m = tri_mesh();
    m.colors = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.5]];
    let scene = Scene::from_meshes(vec![m], vec![]);
    let bytes = splat::write(&scene, &Limits::default()).unwrap();
    assert_eq!(bytes.len(), 4 * splat::RECORD_SIZE);

    let parsed = splat::read(&bytes, &Limits::default()).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.meshes.len(), 1);
    assert_eq!(parsed.meshes[0].positions.len(), 4);
    assert_eq!(parsed.meshes[0].colors.len(), 4);
    assert_eq!(parsed.meshes[0].triangle_count(), 2);

    let tight = Limits { max_vertices: 2, ..Limits::default() };
    assert!(matches!(splat::read(&bytes, &tight), Err(Error::TooLarge(_))));
    assert!(matches!(splat::write(&scene, &tight), Err(Error::TooLarge(_))));
    assert!(matches!(splat::read(&bytes[..31], &Limits::default()), Err(Error::Malformed(_))));
    assert!(matches!(splat::read(&[], &Limits::default()), Err(Error::Malformed(_))));
}

#[test]
fn splat_hostile_input() {
    let scene = Scene::from_meshes(vec![tri_mesh()], vec![]);
    let bytes = splat::write(&scene, &Limits::default()).unwrap();
    hammer(&bytes, &|b| splat::read(b, &Limits::default()));
}
