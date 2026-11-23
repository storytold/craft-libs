use super::{hammer, tri_mesh};
use crate::{Error, Limits, Scene, step};

#[test]
fn step_round_trip_and_brep_tessellation() {
    let scene = Scene::from_meshes(vec![tri_mesh()], vec![]);
    let text = step::write(&scene, "Tetrahedron", &Limits::default()).unwrap();
    assert!(step::is_step(text.as_bytes()));
    assert!(text.contains("CARTESIAN_POINT"));
    assert!(text.contains("POLY_LOOP"));

    let parsed = step::read(text.as_bytes(), &Limits::default()).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.meshes.len(), 1);
    assert_eq!(parsed.meshes[0].triangle_count(), 2);

    let brep = b"ISO-10303-21;DATA;\
        #1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(1.,0.,0.));\
        #3=CARTESIAN_POINT('',(1.,1.,0.));#4=CARTESIAN_POINT('',(0.,1.,0.));\
        #11=VERTEX_POINT('',#1);#12=VERTEX_POINT('',#2);#13=VERTEX_POINT('',#3);#14=VERTEX_POINT('',#4);\
        #21=LINE('',#1,#31);#22=LINE('',#2,#32);#23=LINE('',#3,#33);#24=LINE('',#4,#34);\
        #41=EDGE_CURVE('',#11,#12,#21,.T.);#42=EDGE_CURVE('',#12,#13,#22,.T.);\
        #43=EDGE_CURVE('',#13,#14,#23,.T.);#44=EDGE_CURVE('',#14,#11,#24,.T.);\
        #51=ORIENTED_EDGE('',*,*,#41,.T.);#52=ORIENTED_EDGE('',*,*,#42,.T.);\
        #53=ORIENTED_EDGE('',*,*,#43,.T.);#54=ORIENTED_EDGE('',*,*,#44,.T.);\
        #60=EDGE_LOOP('',(#51,#52,#53,#54));#61=FACE_OUTER_BOUND('',#60,.T.);ENDSEC;END-ISO-10303-21;";
    let brep_scene = step::read(brep, &Limits::default()).unwrap();
    brep_scene.validate().unwrap();
    assert_eq!(brep_scene.meshes[0].triangle_count(), 2);
    assert_eq!(brep_scene.meshes[0].positions.len(), 6);
}

#[test]
fn step_rejects_malformed_and_exceeding_limits() {
    let no_loops = b"ISO-10303-21;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(1.,0.,0.));#3=CARTESIAN_POINT('',(0.,1.,0.));ENDSEC;";
    assert!(matches!(step::read(no_loops, &Limits::default()), Err(Error::Unsupported(_))));

    let bad_coord = b"ISO-10303-21;DATA;#1=CARTESIAN_POINT('',(bad,0.,0.));#2=CARTESIAN_POINT('',(1.,0.,0.));#3=CARTESIAN_POINT('',(0.,1.,0.));#4=POLY_LOOP('',(#1,#2,#3));ENDSEC;";
    assert!(matches!(step::read(bad_coord, &Limits::default()), Err(Error::Malformed(_))));

    let missing_ref = b"ISO-10303-21;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(1.,0.,0.));#4=POLY_LOOP('',(#1,#2,#3));ENDSEC;";
    assert!(matches!(step::read(missing_ref, &Limits::default()), Err(Error::Malformed(_))));

    let scene = Scene::from_meshes(vec![tri_mesh()], vec![]);
    let text = step::write(&scene, "Tetra", &Limits::default()).unwrap();
    let tight = Limits { max_triangles: 1, ..Limits::default() };
    assert!(matches!(step::read(text.as_bytes(), &tight), Err(Error::TooLarge(_))));
}

#[test]
fn step_hostile_input() {
    let scene = Scene::from_meshes(vec![tri_mesh()], vec![]);
    let text = step::write(&scene, "Tetra", &Limits::default()).unwrap();
    hammer(text.as_bytes(), &|b| step::read(b, &Limits::default()));
}
