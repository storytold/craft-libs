//! 3MF, read: the 3MF Consortium's core specification and its materials and properties
//! extension.
//!
//! A 3MF file is an OPC package (a ZIP archive with relationships): [`read`] and [`read_model`]
//! open it with `craft-opc`, find the model part through the package's `3dmodel` relationship
//! (or, without one, the first `.model` part) and read it. [`read_model_part`] reads a model
//! part's XML on its own, for a caller that has the parts some other way.
//!
//! Read: mesh objects (vertices and triangles), component objects and build items with their
//! transforms, the model's unit, base materials (`displaycolor`), colour groups and texture
//! coordinate groups with their textures.
//!
//! - Every build item is a root [`Node`] and every component a child node, each with its
//!   transform; an object placed several times is one [`Mesh`] named by several nodes. A file
//!   without build items places every object once.
//! - A triangle's property group decides its [`Part`]: one per base material, one for vertex
//!   colours, one per texture, one for triangles without a property. Base materials become
//!   materials of that colour (linear); the colour part has a white material; a texture part's
//!   material has the texture as its base colour, with the tile styles as wrap modes.
//! - Colour groups are per corner in the file and per position in the model, so a vertex that
//!   takes different colours in different triangles becomes one position per colour. Without
//!   colours the positions are the file's vertices, in order.
//! - Texture coordinates are a pool indexed per corner; v is flipped to point down.
//! - A texture's image is the package part its `path` names: its bytes are [`Image::data`],
//!   its path [`Image::uri`].
//!
//! Positions stay in the file's unit ([`Model::unit`]) and axes (3MF is Z-up).
//!
//! Not read, and named in a warning when present: components in another model part (the
//! production extension), composite materials and multi-properties. Slices, beam lattices and
//! other extensions are passed over. A triangle that names a vertex that is not there is left
//! out and counted in a warning.

use std::collections::HashMap;

use craft_xml::{Event, Options, to_utf8};

use crate::{
    AlphaMode, Corner, Error, IDENTITY, Image, Limits, Mat4, Material, Mesh, Node, Part, Resolver, Result, Scene, TextureRef, Wrap, count_warning,
    malformed, srgb_to_linear, too_large,
};

const KIND: &str = "3MF";
/// Most parts a package may have.
pub const MAX_ENTRIES: usize = 10_000;

/// The unit of a model's coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unit {
    Micron,
    #[default]
    Millimeter,
    Centimeter,
    Inch,
    Foot,
    Meter,
}

impl Unit {
    pub fn metres(self) -> f64 {
        match self {
            Unit::Micron => 1e-6,
            Unit::Millimeter => 0.001,
            Unit::Centimeter => 0.01,
            Unit::Inch => 0.0254,
            Unit::Foot => 0.3048,
            Unit::Meter => 1.0,
        }
    }
}

/// What a 3MF file holds: the scene, and the unit its positions are in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub scene: Scene,
    pub unit: Unit,
}

/// Is this a ZIP archive (which a 3MF file is)? Other formats are ZIP archives too.
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// Read a 3MF file into a scene. [`read_model`] also gives the unit.
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    Ok(read_model(bytes, limits)?.scene)
}

fn package_error(e: craft_opc::Error) -> Error {
    if e.is_limit() { too_large(KIND, e) } else { malformed(KIND, e) }
}

/// Read a 3MF file. The package may inflate to [`Limits::max_data_bytes`] in all.
pub fn read_model(bytes: &[u8], limits: &Limits) -> Result<Model> {
    let most = limits.max_data_bytes as u64;
    let mut package = craft_opc::LazyPackage::open(bytes, &craft_opc::Limits::new(MAX_ENTRIES, most, most)).map_err(package_error)?;
    let related = package.rels("").map_err(package_error)?.first("3dmodel").filter(|r| !r.external).map(|r| r.target.clone());
    let root = related
        .filter(|r| package.has(r))
        .or_else(|| package.names().into_iter().find(|n| n.ends_with(".model")).map(str::to_string))
        .ok_or_else(|| malformed(KIND, "the package has no model part"))?;
    let xml = package.read(&root).map_err(package_error)?.ok_or_else(|| malformed(KIND, "the package has no model part"))?;
    // A texture that cannot be read leaves its image without data.
    read_model_part(&xml, limits, &mut |path: &str| package.read_optional_limited(path, most).ok().flatten())
}

/// A 3MF transform is twelve numbers, the rows of a 4×3 matrix that a row vector multiplies
/// from the left; anything else is the identity.
fn transform(s: Option<&str>) -> Mat4 {
    let v: Vec<f64> = s.unwrap_or("").split_whitespace().take(13).filter_map(|t| t.parse::<f64>().ok()).filter(|f| f.is_finite()).collect();
    let mut m = IDENTITY;
    if let Ok(v) = <[f64; 12]>::try_from(v) {
        for (i, x) in v.into_iter().enumerate() {
            m[(i / 3) * 4 + i % 3] = x;
        }
    }
    m
}

fn wrap(s: Option<&str>) -> Wrap {
    match s {
        Some("mirror") => Wrap::Mirror,
        Some("clamp" | "none") => Wrap::Clamp,
        _ => Wrap::Repeat,
    }
}

/// `#RRGGBB` or `#RRGGBBAA` as display values in 0..1.
fn color(s: Option<&str>) -> Option<[f32; 4]> {
    let hex = s?.trim().strip_prefix('#')?;
    let byte = |i: usize| hex.get(i * 2..i * 2 + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|b| f32::from(b) / 255.0);
    match hex.len() {
        6 => Some([byte(0)?, byte(1)?, byte(2)?, 1.0]),
        8 => Some([byte(0)?, byte(1)?, byte(2)?, byte(3)?]),
        _ => None,
    }
}

struct Triangle {
    v: [u32; 3],
    pid: Option<u32>,
    p: [Option<u32>; 3],
}

#[derive(Default)]
struct Object {
    name: String,
    vertices: Vec<[f64; 3]>,
    triangles: Vec<Triangle>,
    pid: Option<u32>,
    pindex: Option<u32>,
    components: Vec<(u32, Mat4)>,
}

struct Texture {
    path: String,
    content_type: Option<String>,
    wrap: [Wrap; 2],
}

/// The resource group being read.
#[derive(Clone, Copy)]
enum Group {
    Base(u32),
    Colors(u32),
    Coords(u32),
}

#[derive(Default)]
struct Parsed {
    unit: Unit,
    base: HashMap<u32, Vec<(String, [f32; 4])>>,
    colors: HashMap<u32, Vec<[f32; 4]>>,
    textures: HashMap<u32, Texture>,
    /// Coordinate group id → (texture id, coordinates).
    coords: HashMap<u32, (u32, Vec<[f32; 2]>)>,
    /// In file order.
    objects: Vec<(u32, Object)>,
    build: Vec<(u32, Mat4)>,
    /// Elements that are not read, for the warnings.
    foreign_components: usize,
    mixed_materials: usize,
}

fn parse(xml: &[u8], limits: &Limits) -> Result<Parsed> {
    let text = to_utf8(xml);
    let mut events = Options::local().events(&text);
    let mut m = Parsed::default();
    let mut group: Option<Group> = None;
    let mut object: Option<(u32, Object)> = None;
    // Over the whole file, not each object or group alone.
    let (mut vertices, mut triangles, mut properties, mut components) = (0usize, 0usize, 0usize, 0usize);
    let room = |count: &mut usize, most: usize, what: &str| {
        *count += 1;
        if *count > most { Err(too_large(KIND, format!("more than {most} {what}"))) } else { Ok(()) }
    };
    let mut rooted = false;
    while let Some(event) = events.next().map_err(|e| if e.is_limit() { too_large(KIND, e) } else { malformed(KIND, e) })? {
        let (name, attrs) = match event {
            Event::Start { name, attrs } => (name, attrs),
            Event::End { name } => {
                match name.as_str() {
                    "basematerials" | "colorgroup" | "texture2dgroup" => group = None,
                    "object" => m.objects.extend(object.take()),
                    _ => {}
                }
                continue;
            }
            _ => continue,
        };
        let attr = |key: &str| attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        let id = |key: &str| attr(key).and_then(|s| s.trim().parse::<u32>().ok());
        let number = |key: &str| attr(key).and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0);
        if !rooted {
            if name != "model" {
                return Err(malformed(KIND, "the model part's root element is not `model`"));
            }
            rooted = true;
        }
        match name.as_str() {
            "model" => {
                m.unit = match attr("unit") {
                    Some("micron") => Unit::Micron,
                    Some("centimeter") => Unit::Centimeter,
                    Some("inch") => Unit::Inch,
                    Some("foot") => Unit::Foot,
                    Some("meter") => Unit::Meter,
                    _ => Unit::Millimeter,
                }
            }
            "basematerials" => group = id("id").map(Group::Base),
            "colorgroup" => group = id("id").map(Group::Colors),
            "texture2dgroup" => {
                group = id("id").map(Group::Coords);
                if let (Some(id), Some(texture)) = (id("id"), id("texid")) {
                    m.coords.entry(id).or_insert((texture, vec![]));
                }
            }
            "compositematerials" | "multiproperties" => m.mixed_materials += 1,
            "base" => {
                if let Some(Group::Base(id)) = group {
                    room(&mut properties, limits.max_materials, "base materials")?;
                    let c = color(attr("displaycolor")).unwrap_or([0.8, 0.8, 0.8, 1.0]);
                    m.base.entry(id).or_default().push((attr("name").unwrap_or("").to_string(), c));
                }
            }
            "color" => {
                if let Some(Group::Colors(id)) = group {
                    room(&mut vertices, limits.max_vertices, "vertices, colours and texture coordinates")?;
                    m.colors.entry(id).or_default().push(color(attr("color")).unwrap_or([1.0; 4]));
                }
            }
            "tex2coord" => {
                if let Some(Group::Coords(id)) = group
                    && let Some((_, list)) = m.coords.get_mut(&id)
                {
                    room(&mut vertices, limits.max_vertices, "vertices, colours and texture coordinates")?;
                    // 3MF's v runs up from the bottom of the image.
                    list.push([number("u") as f32, 1.0 - number("v") as f32]);
                }
            }
            "texture2d" => {
                if let (Some(id), Some(path)) = (id("id"), attr("path")) {
                    room(&mut properties, limits.max_materials, "base materials and textures")?;
                    let texture = Texture {
                        path: path.to_string(),
                        content_type: attr("contenttype").map(str::to_string),
                        wrap: [wrap(attr("tilestyleu")), wrap(attr("tilestylev"))],
                    };
                    m.textures.entry(id).or_insert(texture);
                }
            }
            "object" => {
                if m.objects.len() >= limits.max_meshes {
                    return Err(too_large(KIND, format!("more than {} objects", limits.max_meshes)));
                }
                let o = Object { name: attr("name").unwrap_or("").to_string(), pid: id("pid"), pindex: id("pindex"), ..Object::default() };
                object = id("id").map(|id| (id, o));
            }
            "vertex" => {
                if let Some((_, o)) = object.as_mut() {
                    room(&mut vertices, limits.max_vertices, "vertices, colours and texture coordinates")?;
                    o.vertices.push([number("x"), number("y"), number("z")]);
                }
            }
            "triangle" => {
                if let Some((_, o)) = object.as_mut() {
                    room(&mut triangles, limits.max_triangles, "triangles")?;
                    // One without its three vertices is counted as left out when the mesh is built.
                    let v = [id("v1"), id("v2"), id("v3")].map(|v| v.unwrap_or(u32::MAX));
                    o.triangles.push(Triangle { v, pid: id("pid"), p: [id("p1"), id("p2"), id("p3")] });
                }
            }
            "component" => {
                if let (Some((_, o)), Some(target)) = (object.as_mut(), id("objectid")) {
                    if attr("path").is_some() {
                        m.foreign_components += 1;
                    } else {
                        // Each one becomes a node; the node limit is checked when they are placed.
                        room(&mut components, limits.max_nodes, "components")?;
                        o.components.push((target, transform(attr("transform"))));
                    }
                }
            }
            "item" => {
                if let Some(target) = id("objectid") {
                    if m.build.len() >= limits.max_nodes {
                        return Err(too_large(KIND, format!("more than {} build items", limits.max_nodes)));
                    }
                    m.build.push((target, transform(attr("transform"))));
                }
            }
            _ => {}
        }
    }
    if !rooted {
        return Err(malformed(KIND, "the model part has no `model` element"));
    }
    Ok(m)
}

/// What a triangle is drawn with.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Look {
    Plain,
    Base(u32, u32),
    Colors,
    Texture(u32),
}

struct Build<'a, 'r> {
    parsed: &'a Parsed,
    limits: &'a Limits,
    texture: Resolver<'r>,
    scene: Scene,
    materials: HashMap<Look, usize>,
    data_left: usize,
    parts: usize,
    dropped: usize,
}

impl Build<'_, '_> {
    /// The scene's material for `look`, made on first use.
    fn material(&mut self, look: Look) -> Result<Option<usize>> {
        if look == Look::Plain {
            return Ok(None);
        }
        if let Some(i) = self.materials.get(&look) {
            return Ok(Some(*i));
        }
        let m = self.parsed;
        let material = match look {
            Look::Base(group, index) => {
                let (name, c) = m.base.get(&group).and_then(|l| l.get(index as usize)).cloned().unwrap_or((String::new(), [0.8, 0.8, 0.8, 1.0]));
                let base_color = [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2]), c[3]];
                Material { alpha_mode: if c[3] < 0.999 { AlphaMode::Blend } else { AlphaMode::Opaque }, ..Material::plain(name, base_color) }
            }
            Look::Texture(id) => {
                let texture = match m.textures.get(&id) {
                    Some(t) => {
                        let data = (self.texture)(&t.path);
                        self.data_left = self
                            .data_left
                            .checked_sub(data.as_ref().map_or(0, Vec::len))
                            .ok_or_else(|| too_large(KIND, format!("textures of more than {} bytes", self.limits.max_data_bytes)))?;
                        self.scene.external.push(t.path.clone());
                        self.scene.images.push(Image { name: String::new(), uri: Some(t.path.clone()), mime_type: t.content_type.clone(), data });
                        Some(TextureRef { image: self.scene.images.len() - 1, uv_set: 0, wrap: t.wrap })
                    }
                    None => None,
                };
                Material { base_color_texture: texture, ..Material::plain("", [1.0; 4]) }
            }
            Look::Colors | Look::Plain => Material::plain("", [1.0; 4]),
        };
        self.scene.materials.push(material);
        self.materials.insert(look, self.scene.materials.len() - 1);
        Ok(Some(self.scene.materials.len() - 1))
    }

    /// The mesh of an object's triangles; `None` when none of them can be kept.
    fn mesh(&mut self, o: &Object) -> Result<Option<Mesh>> {
        let m = self.parsed;
        // What each triangle is drawn with: its own property group or the object's.
        let look_of = |t: &Triangle| {
            let (pid, p1) = match t.pid {
                Some(pid) => (Some(pid), t.p[0].unwrap_or(0)),
                None => (o.pid, o.pindex.unwrap_or(0)),
            };
            let look = match pid {
                Some(g) if m.base.contains_key(&g) => Look::Base(g, p1),
                Some(g) if m.colors.contains_key(&g) => Look::Colors,
                Some(g) => m.coords.get(&g).map_or(Look::Plain, |c| Look::Texture(c.0)),
                None => Look::Plain,
            };
            (look, pid, [p1, t.p[1].unwrap_or(p1), t.p[2].unwrap_or(p1)])
        };
        let colored = o.triangles.iter().any(|t| look_of(t).0 == Look::Colors);
        let mut mesh = Mesh { name: o.name.clone(), ..Mesh::default() };
        if !colored {
            mesh.positions.clone_from(&o.vertices);
        }
        // With colours: a position per vertex and colour.
        let mut position_of: HashMap<(u32, [u32; 4]), u32> = HashMap::new();
        let mut uvs: Vec<[f32; 2]> = vec![];
        let mut uv_of: HashMap<(u32, u32), u32> = HashMap::new();
        let mut part_of: HashMap<Look, usize> = HashMap::new();
        for t in &o.triangles {
            if t.v.iter().any(|v| *v as usize >= o.vertices.len()) {
                self.dropped += 1;
                continue;
            }
            let (look, pid, p) = look_of(t);
            let at = match part_of.get(&look) {
                Some(at) => *at,
                None => {
                    self.parts += 1;
                    if self.parts > self.limits.max_parts {
                        return Err(too_large(KIND, format!("more than {} parts", self.limits.max_parts)));
                    }
                    mesh.parts.push(Part { material: self.material(look)?, ..Part::default() });
                    part_of.insert(look, mesh.parts.len() - 1);
                    mesh.parts.len() - 1
                }
            };
            for (v, p) in t.v.into_iter().zip(p) {
                let mut corner = Corner::at(v);
                if colored {
                    let c = match look {
                        Look::Colors => pid.and_then(|g| m.colors.get(&g)).and_then(|l| l.get(p as usize)).copied().unwrap_or([1.0; 4]),
                        _ => [1.0; 4],
                    };
                    corner.position = *position_of.entry((v, c.map(f32::to_bits))).or_insert_with(|| {
                        mesh.positions.extend(o.vertices.get(v as usize));
                        mesh.colors.push(c);
                        (mesh.positions.len() - 1) as u32
                    });
                }
                if let (Look::Texture(_), Some(group)) = (look, pid)
                    && let Some(uv) = m.coords.get(&group).and_then(|c| c.1.get(p as usize))
                {
                    corner.uv = *uv_of.entry((group, p)).or_insert_with(|| {
                        uvs.push(*uv);
                        (uvs.len() - 1) as u32
                    });
                }
                if let Some(part) = mesh.parts.get_mut(at) {
                    part.corners.push(corner);
                }
            }
        }
        if !uvs.is_empty() {
            mesh.uv_sets.push(uvs);
        }
        Ok((!mesh.parts.is_empty()).then_some(mesh))
    }
}

/// Read a model part's XML. `texture` is asked for the package parts textures name (by their
/// `path`, as written: a part name such as `/3D/Textures/wood.png`), and may decline.
pub fn read_model_part(xml: &[u8], limits: &Limits, texture: Resolver) -> Result<Model> {
    let parsed = parse(xml, limits)?;
    let mut b = Build {
        parsed: &parsed,
        limits,
        texture,
        scene: Scene::default(),
        materials: HashMap::new(),
        data_left: limits.max_data_bytes,
        parts: 0,
        dropped: 0,
    };
    // Each object's mesh once, however often it is placed. The first object of an id wins.
    let mut objects: HashMap<u32, (&Object, Option<usize>)> = HashMap::new();
    for (id, o) in &parsed.objects {
        if objects.contains_key(id) {
            continue;
        }
        let mesh = match b.mesh(o)? {
            Some(mesh) => {
                b.scene.meshes.push(mesh);
                Some(b.scene.meshes.len() - 1)
            }
            None => None,
        };
        objects.insert(*id, (o, mesh));
    }
    // Build items, or every object once; components below them, in order.
    let items: Vec<(u32, Mat4)> =
        if parsed.build.is_empty() { parsed.objects.iter().map(|(id, _)| (*id, IDENTITY)).collect() } else { parsed.build.clone() };
    let mut missing = 0usize;
    let mut stack: Vec<(u32, Mat4, Option<usize>, usize)> = items.into_iter().rev().map(|(id, t)| (id, t, None, 0)).collect();
    while let Some((id, transform, parent, depth)) = stack.pop() {
        if depth > limits.max_depth {
            return Err(too_large(KIND, format!("components nest deeper than {}", limits.max_depth)));
        }
        let Some((o, mesh)) = objects.get(&id) else {
            missing += 1;
            continue;
        };
        if b.scene.nodes.len() >= limits.max_nodes {
            return Err(too_large(KIND, format!("more than {} placed objects", limits.max_nodes)));
        }
        let node = b.scene.nodes.len();
        b.scene.nodes.push(Node { name: o.name.clone(), transform, mesh: *mesh, ..Node::default() });
        match parent.and_then(|p| b.scene.nodes.get_mut(p)) {
            Some(p) => p.children.push(node),
            None => b.scene.roots.push(node),
        }
        stack.extend(o.components.iter().rev().map(|(target, t)| (*target, *t, Some(node), depth + 1)));
    }
    let mut scene = b.scene;
    count_warning(&mut scene.warnings, b.dropped, "triangle(s) named a vertex that is not there and were left out");
    count_warning(&mut scene.warnings, missing, "build item(s) or component(s) named an object that is not there");
    count_warning(&mut scene.warnings, parsed.foreign_components, "component(s) in another model part were not read");
    count_warning(&mut scene.warnings, parsed.mixed_materials, "composite material or multi-property group(s) were not read");
    Ok(Model { scene, unit: parsed.unit })
}
