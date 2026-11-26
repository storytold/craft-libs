use super::*;
use crate::abc::read;

const DATA_BIT: u64 = 1 << 63;
const OGAWA_MAGIC: &[u8] = b"Ogawa";
const KEY_BYTES: usize = 16;
const OBJECT_HASH_BYTES: usize = 32;
const F32: u32 = 10;
const I32: u32 = 6;
const U32: u32 = 5;
const U8: u32 = 1;
const F64: u32 = 11;

struct Writer {
    b: Vec<u8>,
}

impl Writer {
    fn new() -> Writer {
        Writer { b: vec![0; 16] }
    }

    fn data(&mut self, bytes: &[u8]) -> u64 {
        let at = self.b.len() as u64;
        self.b.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.b.extend_from_slice(bytes);
        at | DATA_BIT
    }

    fn group(&mut self, children: &[u64]) -> u64 {
        let at = self.b.len() as u64;
        self.b.extend_from_slice(&(children.len() as u64).to_le_bytes());
        for c in children {
            self.b.extend_from_slice(&c.to_le_bytes());
        }
        at
    }

    fn finish(mut self, root: u64) -> Vec<u8> {
        self.b[..5].copy_from_slice(OGAWA_MAGIC);
        self.b[5] = 0xff;
        self.b[6..8].copy_from_slice(&[0, 1]);
        self.b[8..16].copy_from_slice(&root.to_le_bytes());
        self.b
    }
}

struct P {
    name: &'static str,
    meta: String,
    info: u32,
    group: u64,
}

struct Abc {
    w: Writer,
}

impl Abc {
    fn sample(&mut self, raw: &[u8]) -> u64 {
        let mut v = vec![7u8; KEY_BYTES];
        v.extend_from_slice(raw);
        self.w.data(&v)
    }

    fn array(&mut self, name: &'static str, meta: &str, pod: u32, extent: u32, raw: &[u8]) -> P {
        let (d, dims) = (self.sample(raw), self.w.data(&[]));
        let group = self.w.group(&[d, dims]);
        P { name, meta: meta.into(), info: 2 | (2 << 2) | (pod << 4) | (extent << 12), group }
    }

    fn scalar(&mut self, name: &'static str, pod: u32, extent: u32, raw: &[u8]) -> P {
        let d = self.sample(raw);
        let group = self.w.group(&[d]);
        P { name, meta: String::new(), info: 1 | (2 << 2) | (pod << 4) | (extent << 12), group }
    }

    fn compound(&mut self, name: &'static str, meta: &str, props: Vec<P>) -> P {
        let mut header = Vec::new();
        for p in &props {
            let info = p.info | (0xff << 20);
            header.extend_from_slice(&info.to_le_bytes());
            if info & 3 != 0 {
                header.extend_from_slice(&1u32.to_le_bytes());
            }
            header.extend_from_slice(&(p.name.len() as u32).to_le_bytes());
            header.extend_from_slice(p.name.as_bytes());
            header.extend_from_slice(&(p.meta.len() as u32).to_le_bytes());
            header.extend_from_slice(p.meta.as_bytes());
        }
        let mut children: Vec<u64> = props.iter().map(|p| p.group).collect();
        children.push(self.w.data(&header));
        let group = self.w.group(&children);
        P { name, meta: meta.into(), info: 2 << 2, group }
    }

    fn object(&mut self, props: Vec<P>, children: &[(&str, &str, u64)]) -> u64 {
        let props = self.compound("", "", props).group;
        let mut header = Vec::new();
        for (name, meta, _) in children {
            header.extend_from_slice(&(name.len() as u32).to_le_bytes());
            header.extend_from_slice(name.as_bytes());
            if meta.is_empty() {
                header.push(1);
            } else {
                header.push(0xff);
                header.extend_from_slice(&(meta.len() as u32).to_le_bytes());
                header.extend_from_slice(meta.as_bytes());
            }
        }
        header.extend_from_slice(&[0; OBJECT_HASH_BYTES]);
        let mut kids = vec![props];
        kids.extend(children.iter().map(|c| c.2));
        kids.push(self.w.data(&header));
        self.w.group(&kids)
    }

    fn archive(mut self, top: u64, indexed_meta: &[&str]) -> Vec<u8> {
        let mut im = Vec::new();
        for m in indexed_meta {
            im.push(m.len() as u8);
            im.extend_from_slice(m.as_bytes());
        }
        let kids = [
            self.w.data(&0i32.to_le_bytes()),
            self.w.data(&10709i32.to_le_bytes()),
            top,
            self.w.data(b"_ai_Application=test"),
            self.w.data(&[]),
            self.w.data(&im),
        ];
        let root = self.w.group(&kids);
        self.w.finish(root)
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn i32s(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn sample_file() -> Vec<u8> {
    let mut a = Abc { w: Writer::new() };
    let p = f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 2.0, 1.0, 0.0]);
    let indices = i32s(&[0, 3, 4, 1, 1, 4, 5, 2]);
    let counts = i32s(&[4, 4]);
    let pp = a.array("P", "interpretation=point", F32, 3, &p);
    let fi = a.array(".faceIndices", "", I32, 1, &indices);
    let fc = a.array(".faceCounts", "", I32, 1, &counts);
    let vals = a.array(".vals", "", F32, 2, &f32s(&[0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 1.0, 1.0, 1.0]));
    let uvi = a.array(".indices", "", U32, 1, &i32s(&[0, 3, 4, 1, 1, 4, 5, 2]));
    let uv = a.compound("uv", "geoScope=fvr;podName=float32_t;podExtent=2", vec![vals, uvi]);
    let n = a.array("N", "geoScope=vtx", F32, 3, &f32s(&[0.0, 0.0, 1.0].repeat(6)));
    let geom = a.compound(".geom", "schema=AbcGeom_PolyMesh_v1", vec![pp, fi, fc, uv, n]);
    let left_faces = a.array(".faces", "", I32, 1, &i32s(&[0]));
    let left_fs = a.compound(".faceset", "schema=AbcGeom_FaceSet_v1", vec![left_faces]);
    let left = a.object(vec![left_fs], &[]);
    let right_faces = a.array(".faces", "", I32, 1, &i32s(&[1]));
    let right_fs = a.compound(".faceset", "schema=AbcGeom_FaceSet_v1", vec![right_faces]);
    let right = a.object(vec![right_fs], &[]);
    let fs_meta = "schema=AbcGeom_FaceSet_v1;schemaObjTitle=AbcGeom_FaceSet_v1:.faceset";
    let mesh = a.object(vec![geom], &[("left", fs_meta, left), ("right", fs_meta, right)]);
    let ops = a.scalar(".ops", U8, 2, &[0x10, 0x00]);
    let xv: Vec<u8> = [1.0f64, 2.0, 3.0, 2.0, 2.0, 2.0].iter().flat_map(|x| x.to_le_bytes()).collect();
    let vals = a.scalar(".vals", F64, 6, &xv);
    let xform = a.compound(".xform", "schema=AbcGeom_Xform_v3", vec![ops, vals]);
    let xf = a.object(vec![xform], &[("stripShape", "schema=AbcGeom_PolyMesh_v1;schemaObjTitle=AbcGeom_PolyMesh_v1:.geom", mesh)]);
    let top = a.object(vec![], &[("strip", "schema=AbcGeom_Xform_v3", xf)]);
    a.archive(top, &["schema=AbcGeom_PolyMesh_v1"])
}

#[test]
fn reads_polymesh_xform_face_sets_and_uvs() {
    let scene = read(&sample_file(), &Limits::default()).unwrap();
    scene.validate().unwrap();
    assert_eq!(scene.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["left", "right"]);
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.meshes[0].name, "strip");
    assert_eq!(scene.meshes[0].triangle_count(), 4);
    let inst = scene.instances(&Limits::default()).unwrap();
    assert_eq!(inst.len(), 1);
    let p0 = transform_point(&inst[0].1, scene.meshes[0].positions[0]);
    assert_eq!(p0, [1.0, 2.0, 3.0]);
}

#[test]
fn hdf5_returns_unsupported() {
    let err = read(b"\x89HDF\r\n\x1a\n rest", &Limits::default()).unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
}

#[test]
fn hammer_never_panics() {
    hammer(&sample_file(), &|b| read(b, &Limits::default()));
}
