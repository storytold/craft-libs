use super::*;
use crate::dae::{UpAxis, read, read_document, read_document_with_cameras};

const DAE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
  <asset><unit name="centimeter" meter="0.5"/><up_axis>Z_UP</up_axis></asset>
  <library_materials>
    <material id="red-mat" name="Red"/>
    <material id="blue-mat" name="Blue"/>
  </library_materials>
  <library_geometries>
    <geometry id="quad" name="Quad">
      <mesh>
        <source id="quad-pos">
          <float_array id="quad-pos-array" count="12">0 0 0 2 0 0 2 2 0 0 2 0</float_array>
          <technique_common><accessor source="#quad-pos-array" count="4" stride="3"/></technique_common>
        </source>
        <source id="quad-nrm">
          <float_array id="quad-nrm-array" count="3">0 0 1</float_array>
          <technique_common><accessor source="#quad-nrm-array" count="1" stride="3"/></technique_common>
        </source>
        <source id="quad-uv">
          <float_array id="quad-uv-array" count="8">0 0 1 0 1 1 0 1</float_array>
          <technique_common><accessor source="#quad-uv-array" count="4" stride="2"/></technique_common>
        </source>
        <vertices id="quad-verts"><input semantic="POSITION" source="#quad-pos"/></vertices>
        <polylist material="mat0" count="1">
          <input semantic="VERTEX" source="#quad-verts" offset="0"/>
          <input semantic="NORMAL" source="#quad-nrm" offset="1"/>
          <input semantic="TEXCOORD" source="#quad-uv" offset="2" set="0"/>
          <vcount>4</vcount>
          <p>0 0 0 1 0 1 2 0 2 3 0 3</p>
        </polylist>
      </mesh>
    </geometry>
    <geometry id="tris" name="Tris">
      <mesh>
        <source id="tris-pos">
          <float_array id="tris-pos-array" count="21">4 0 0 6 0 0 4 2 0 8 0 0 10 0 0 10 2 0 8 2 0</float_array>
          <technique_common><accessor source="#tris-pos-array" count="7" stride="3"/></technique_common>
        </source>
        <source id="tris-uv">
          <float_array id="tris-uv-array" count="14">0 0 1 0 0 1 0 0 1 0 1 1 0 1</float_array>
          <technique_common><accessor source="#tris-uv-array" count="7" stride="2"/></technique_common>
        </source>
        <vertices id="tris-verts">
          <input semantic="POSITION" source="#tris-pos"/>
          <input semantic="TEXCOORD" source="#tris-uv"/>
        </vertices>
        <triangles material="mat1" count="1"><input semantic="VERTEX" source="#tris-verts" offset="0"/><p>0 1 2</p></triangles>
        <polygons material="mat1" count="1"><input semantic="VERTEX" source="#tris-verts" offset="0"/><p>3 4 5 6</p></polygons>
        <lines count="1"><input semantic="VERTEX" source="#tris-verts" offset="0"/><p>0 1</p></lines>
      </mesh>
    </geometry>
  </library_geometries>
  <library_cameras>
    <camera id="lens"><optics><technique_common><perspective><yfov>30</yfov></perspective></technique_common></optics></camera>
  </library_cameras>
  <library_visual_scenes>
    <visual_scene id="scene">
      <node id="parent">
        <translate>0 0 2</translate>
        <rotate>0 0 1 90</rotate>
        <node id="child">
          <scale>2 2 2</scale>
          <instance_geometry url="#quad">
            <bind_material><technique_common><instance_material symbol="mat0" target="#red-mat"/></technique_common></bind_material>
          </instance_geometry>
        </node>
      </node>
      <node id="cam" name="Overview">
        <lookat>0 -10 0 0 0 0 0 0 1</lookat>
        <instance_camera url="#lens"/>
      </node>
      <node id="other">
        <instance_geometry url="#tris">
          <bind_material><technique_common><instance_material symbol="mat1" target="#blue-mat"/></technique_common></bind_material>
        </instance_geometry>
      </node>
    </visual_scene>
  </library_visual_scenes>
  <scene><instance_visual_scene url="#scene"/></scene>
</COLLADA>
"##;

#[test]
fn reads_polylist_transforms_units_and_cameras() {
    let doc = read_document(DAE.as_bytes(), &Limits::default()).unwrap();
    doc.scene.validate().unwrap();
    assert_eq!(doc.meter, 0.5);
    assert_eq!(doc.up_axis, UpAxis::ZUp);
    assert_eq!(doc.scene.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Red", "Blue"]);
    assert!(doc.scene.warnings.iter().any(|w| w.contains("lines")));
    assert_eq!(doc.scene.cameras.len(), 1);
    assert_eq!(doc.scene.cameras[0].name, "Overview");

    let global = doc.global_transform();
    let inst = doc.scene.instances(&Limits::default()).unwrap();
    assert_eq!(inst.len(), 2);
    let mut tris = 0usize;
    for (m_idx, xform) in inst {
        let m = &doc.scene.meshes[m_idx];
        tris += m.triangle_count();
        let w = mat_mul(&global, &xform);
        for p in &m.positions {
            let wp = transform_point(&w, *p);
            assert!(wp.iter().all(|v| v.is_finite()));
        }
    }
    assert_eq!(tris, 5);
}

#[test]
fn entities_and_limits_are_enforced() {
    let dae = DAE
        .replace(r#"<node id="other">"#, r#"<node id="other" name="Tom &amp; Jerry &lt;&quot;&apos;&#65;&#x42;">"#)
        .replace(r#"name="Red""#, r#"name="R&amp;D""#);
    let scene = read(dae.as_bytes(), &Limits::default()).unwrap();
    scene.validate().unwrap();
    assert!(scene.nodes.iter().any(|n| n.name == "Tom & Jerry <\"'AB"));
    assert_eq!(scene.materials[0].name, "R&D");

    let tight = Limits { max_triangles: 2, ..Limits::default() };
    assert!(matches!(read(DAE.as_bytes(), &tight), Err(Error::TooLarge(_))));
}

#[test]
fn hammer_never_panics() {
    hammer(DAE.as_bytes(), &|b| read(b, &Limits::default()));
}

// Synthetic COLLADA scene with two lenses on a node at the maximum allowed depth.
const CAMERA_RIG: &str = r##"<COLLADA>
  <library_cameras>
    <camera id="wide"><optics><technique_common><perspective><yfov>60</yfov></perspective></technique_common></optics></camera>
    <camera id="ortho"><optics><technique_common><orthographic><ymag>2</ymag></orthographic></technique_common></optics></camera>
  </library_cameras>
  <library_visual_scenes><visual_scene id="scene">
    <node name="Parent"><translate>1 2 3</translate><node name="Rig"><translate>4 5 6</translate>
      <instance_camera url="#wide"/><instance_camera url="#missing"/><instance_camera url="#ortho"/>
    </node></node>
    <node><instance_camera url="#wide"/></node>
  </visual_scene></library_visual_scenes>
</COLLADA>"##;

#[test]
fn additional_dae_cameras_share_their_node_without_spending_graph_budget() {
    let limits = Limits { max_nodes: 3, max_depth: 1, ..Limits::default() };
    let old = read_document(CAMERA_RIG.as_bytes(), &limits).unwrap();
    let all = read_document_with_cameras(CAMERA_RIG.as_bytes(), &limits).unwrap();
    // Additional cameras neither add nodes nor change any existing transform or primary index.
    assert_eq!(all.document(), &old);
    let parent = all.document().scene.roots[0];
    let rig = all.document().scene.nodes[parent].children[0];
    let cameras: Vec<_> = all.cameras_for_node(rig).collect();
    let [first, second] = cameras.as_slice() else { panic!("expected both camera instances: {cameras:?}") };
    assert_eq!((first.name.as_str(), second.name.as_str()), ("Rig", "Rig"));
    assert!(matches!(first.projection, Projection::Perspective { .. }));
    assert!(matches!(second.projection, Projection::Orthographic { .. }));
    assert_eq!(all.cameras_for_node(all.document().scene.roots[1]).next().unwrap().name, "wide");
}

#[test]
fn additional_dae_cameras_share_the_extras_limit_in_traversal_order() {
    let limits = Limits { max_extras: 2, ..Limits::default() };
    let all = read_document_with_cameras(CAMERA_RIG.as_bytes(), &limits).unwrap();
    let scene = &all.document().scene;
    let rig = scene.nodes[scene.roots[0]].children[0];
    assert_eq!(all.cameras_for_node(rig).count(), 2);
    assert!(all.cameras_for_node(scene.roots[1]).next().is_none());
    // Compatibility: the original API's cap still counts only primary cameras.
    let original = read(CAMERA_RIG.as_bytes(), &limits).unwrap();
    assert_eq!(original.cameras.len(), 2);
    assert!(original.nodes[original.roots[1]].camera.is_some());
}
