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

fn polygon_step(points: &[[f64; 2]]) -> String {
    let mut text = String::from("ISO-10303-21;DATA;");
    for (i, p) in points.iter().enumerate() {
        text += &format!("#{}=CARTESIAN_POINT('',({:?},{:?},0.));", i + 1, p[0], p[1]);
    }
    let refs: Vec<String> = (1..=points.len()).map(|i| format!("#{i}")).collect();
    text += &format!("#100=POLY_LOOP('',({}));ENDSEC;END-ISO-10303-21;", refs.join(","));
    text
}

fn area(scene: &Scene) -> f64 {
    let mesh = &scene.meshes[0];
    mesh.parts
        .iter()
        .flat_map(|p| p.triangles())
        .map(|t| {
            let [a, b, c] = t.map(|c| mesh.positions[c.position as usize]);
            0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs()
        })
        .sum()
}

#[test]
fn step_strings_and_comments_do_not_split_entities() {
    let text = b"ISO-10303-21;DATA;\
        /* a comment; with a semicolon */ #1=CARTESIAN_POINT('left;right',(0.,0.,0.));\
        #2=CARTESIAN_POINT('it''s',(1.,0.,0.));#3=CARTESIAN_POINT('',(0.,1.,0.));\
        #4=POLY_LOOP('',(#1,#2,#3));ENDSEC;END-ISO-10303-21;";
    let scene = step::read(text, &Limits::default()).unwrap();
    assert_eq!(scene.meshes[0].triangle_count(), 1);
    assert!(matches!(step::read(b"ISO-10303-21;DATA;#1=CARTESIAN_POINT('open", &Limits::default()), Err(Error::Malformed(_))));
}

#[test]
fn step_geometry_limits_do_not_cap_the_entity_count() {
    let text = polygon_step(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    let limits = Limits { max_vertices: 3, max_triangles: 1, ..Limits::default() };
    assert_eq!(step::read(text.as_bytes(), &limits).unwrap().meshes[0].triangle_count(), 1);
}

#[test]
fn step_concave_polygons_keep_their_notch() {
    let notch = [[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [2.0, 3.0], [2.0, 1.0], [1.0, 1.0], [1.0, 3.0], [0.0, 3.0]];
    let scene = step::read(polygon_step(&notch).as_bytes(), &Limits::default()).unwrap();
    // 3x3 square minus the 1x2 notch.
    assert!((area(&scene) - 7.0).abs() < 1e-9, "area {}", area(&scene));
}

#[test]
fn step_faces_with_inner_bounds_are_refused() {
    let text = b"ISO-10303-21;DATA;\
        #1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(1.,0.,0.));#3=CARTESIAN_POINT('',(0.,1.,0.));\
        #4=POLY_LOOP('',(#1,#2,#3));#5=POLY_LOOP('',(#1,#2,#3));\
        #6=FACE_OUTER_BOUND('',#4,.T.);#7=FACE_BOUND('',#5,.T.);#8=ADVANCED_FACE('',(#6,#7),#9,.T.);\
        ENDSEC;END-ISO-10303-21;";
    assert!(matches!(step::read(text, &Limits::default()), Err(Error::Unsupported(_))));
}

#[test]
fn step_write_enforces_limits_and_keeps_small_coordinates() {
    let scene = Scene::from_meshes(vec![tri_mesh()], vec![]);
    let tight = Limits { max_triangles: 1, ..Limits::default() };
    assert!(matches!(step::write(&scene, "m", &tight), Err(Error::TooLarge(_))));
    let few_points = Limits { max_vertices: 4, ..Limits::default() };
    assert!(matches!(step::write(&scene, "m", &few_points), Err(Error::TooLarge(_))));

    let mut tiny = tri_mesh();
    tiny.positions = vec![[0.0, 0.0, 0.0], [1e-7, 0.0, 0.0], [0.0, 1e-7, 0.0], [1e-7, 1e-7, 0.0]];
    let text = step::write(&Scene::from_meshes(vec![tiny], vec![]), "tiny", &Limits::default()).unwrap();
    let back = step::read(text.as_bytes(), &Limits::default()).unwrap();
    assert!(area(&back) > 0.0, "small triangles must not collapse");
}

#[test]
fn step_write_emits_faces_in_a_shape() {
    let scene = Scene::from_meshes(vec![tri_mesh()], vec![]);
    let text = step::write(&scene, "m", &Limits::default()).unwrap();
    for entity in ["FACE_BOUND", "=FACE(", "OPEN_SHELL", "SHELL_BASED_SURFACE_MODEL", "SHAPE_REPRESENTATION"] {
        assert!(text.contains(entity), "{entity}");
    }
}

#[test]
fn step_poly_loop_applies_face_bound_orientation() {
    let reversed = b"ISO-10303-21;DATA;\
        #1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(1.,0.,0.));#3=CARTESIAN_POINT('',(0.,1.,0.));\
        #4=POLY_LOOP('',(#1,#2,#3));#5=FACE_BOUND('',#4,.F.);#6=FACE('',(#5));\
        ENDSEC;END-ISO-10303-21;";
    let scene = step::read(reversed, &Limits::default()).unwrap();
    assert_eq!(scene.meshes[0].normals[0], [0.0, 0.0, -1.0]);

    let referenced_bound_wins = b"ISO-10303-21;DATA;\
        #1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(1.,0.,0.));#3=CARTESIAN_POINT('',(0.,1.,0.));\
        #4=POLY_LOOP('',(#1,#2,#3));#5=FACE_BOUND('',#4,.T.);#6=FACE_OUTER_BOUND('',#4,.F.);#7=FACE('',(#5));\
        ENDSEC;END-ISO-10303-21;";
    let scene = step::read(referenced_bound_wins, &Limits::default()).unwrap();
    assert_eq!(scene.meshes[0].normals[0], [0.0, 0.0, 1.0]);
}
