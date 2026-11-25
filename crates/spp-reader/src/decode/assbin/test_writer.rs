//! Test-only writer for small synthetic assbin files in the reference app's layout.

pub struct TestMesh<'a> {
    pub name: &'a str,
    pub material_index: u32,
    pub vertices: u32,
    pub triangles: u32,
    pub uv_sets: u32,
    pub color_sets: u32,
}

#[derive(Default)]
pub struct Buf(pub Vec<u8>);

impl Buf {
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32s(&mut self, values: &[f32]) -> &mut Self {
        values.iter().for_each(|v| self.0.extend_from_slice(&v.to_le_bytes()));
        self
    }
    pub fn zeros(&mut self, n: usize) -> &mut Self {
        self.0.resize(self.0.len() + n, 0);
        self
    }
    pub fn string(&mut self, s: &str) -> &mut Self {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
        self
    }
    /// Appends `content` as a block whose `u64` size counts itself.
    pub fn block(&mut self, content: Buf) -> &mut Self {
        self.0.extend_from_slice(&(content.0.len() as u64 + 8).to_le_bytes());
        self.0.extend_from_slice(&content.0);
        self
    }
}

pub fn header(minor: u32, compressed: bool) -> Buf {
    let mut b = Buf::default();
    let mut signature = b"ASSIMP.binary-dump.Thu Jan  1 00:00:00 1970\n".to_vec();
    signature.resize(44, 0);
    b.0.extend_from_slice(&signature);
    b.u32(1).u32(minor).u32(0).u32(0x11).u16(0).u16(compressed as u16);
    b.0.extend_from_slice(b"file:///test.fbx");
    b.zeros(256 - 16 + 128);
    b.0.resize(512, 0xcd);
    b
}

fn mesh(m: &TestMesh) -> Buf {
    let mut b = Buf::default();
    let mut components = 0x1 | 0x2 | 0x4;
    for n in 0..m.uv_sets {
        components |= 0x100 << n;
    }
    for n in 0..m.color_sets {
        components |= 0x1_0000 << n;
    }
    b.string(m.name).u32(4).u32(m.material_index).u32(components);
    let vec3s = m.vertices as usize * 12;
    for _ in 0..4 {
        b.u32(m.vertices).zeros(vec3s);
    }
    for _ in 0..m.color_sets {
        b.u32(m.vertices).zeros(m.vertices as usize * 16);
    }
    for _ in 0..m.uv_sets {
        b.u32(2).u32(m.vertices).zeros(vec3s);
    }
    b.u32(m.triangles);
    for _ in 0..m.triangles {
        b.u32(0).u32(3).u32(0).u32(1).u32(2);
    }
    b.u32(0); // bones
    b
}

fn property(key: &str, kind: u32, data: Buf) -> Buf {
    let mut b = Buf::default();
    b.string(key).u32(0).u32(0).u32(kind).u32(data.0.len() as u32);
    b.0.extend_from_slice(&data.0);
    b
}

/// A material named `name` whose `$mat.uid` is the name's length.
pub fn material(name: &str) -> Buf {
    let mut colour = Buf::default();
    colour.zeros(12);
    let mut text = Buf::default();
    text.string(name).zeros(1);
    let mut uid = Buf::default();
    uid.0.extend_from_slice(&(name.len() as u64).to_le_bytes());
    let mut b = Buf::default();
    b.u32(3).block(property("$clr.diffuse", 1, colour)).block(property("?mat.name", 3, text)).block(property("$mat.uid", 5, uid));
    b
}

pub const IDENTITY: [f32; 16] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];

/// A node block with a row-major matrix, mesh indices, child nodes and no metadata.
pub fn node(name: &str, matrix: [f32; 16], meshes: &[u32], children: Vec<Buf>) -> Buf {
    let mut b = Buf::default();
    b.string(name).f32s(&matrix);
    b.u32(children.len() as u32).u32(meshes.len() as u32);
    b.u32(meshes.len() as u32);
    meshes.iter().for_each(|&m| {
        b.u32(m);
    });
    b.u32(children.len() as u32);
    for child in children {
        b.block(child);
    }
    b
}

/// The scene block: flags, the root node, meshes, materials and four empty trailing lists.
pub fn scene_from(root: Buf, meshes: Vec<Buf>, materials: &[&str]) -> Buf {
    let mut tail = Buf::default();
    tail.zeros(16);
    scene_with_tail(root, meshes, materials, tail)
}

/// The scene block with `tail` after the materials (the animation, texture, light and camera lists).
pub fn scene_with_tail(root: Buf, meshes: Vec<Buf>, materials: &[&str], tail: Buf) -> Buf {
    let mut s = Buf::default();
    s.u32(8).block(root).u32(meshes.len() as u32);
    for m in meshes {
        s.block(m);
    }
    s.u32(materials.len() as u32);
    for name in materials {
        s.block(material(name));
    }
    s.0.extend_from_slice(&tail.0);
    let mut out = Buf::default();
    out.block(s);
    out
}

/// A scene whose root node places no mesh.
pub fn scene(meshes: &[TestMesh], materials: &[&str]) -> Buf {
    let root = node("RootNode", IDENTITY, &[], Vec::new());
    scene_from(root, meshes.iter().map(mesh).collect(), materials)
}

/// A complete uncompressed file.
pub fn file(meshes: &[TestMesh], materials: &[&str]) -> Vec<u8> {
    let mut out = header(7, false);
    out.0.extend_from_slice(&scene(meshes, materials).0);
    out.0
}

/// A complete compressed file, using a zlib stream of stored DEFLATE blocks.
pub fn compressed_file(meshes: &[TestMesh], materials: &[&str]) -> Vec<u8> {
    let raw = scene(meshes, materials).0;
    let mut out = header(7, true);
    out.u32(raw.len() as u32);
    out.0.extend_from_slice(&[0x78, 0x01]);
    let chunks: Vec<&[u8]> = raw.chunks(0xffff).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        out.0.push((i + 1 == chunks.len()) as u8);
        out.u16(chunk.len() as u16).u16(!(chunk.len() as u16));
        out.0.extend_from_slice(chunk);
    }
    out.0.extend_from_slice(&adler2::adler32_slice(&raw).to_be_bytes()); // Adler-32 of the raw bytes: a conforming zlib reader accepts the stream
    out.0
}
