//! Wavefront OBJ geometry and MTL materials, read and written.
//!
//! OBJ read: `v x y z` with optional vertex colours (`v x y z r g b`), `vt`, `vn`, `f` with
//! `v`, `v/vt`, `v//vn` and `v/vt/vn` references (negative ones count back from the end),
//! polygons of any size kept whole, `o` objects (one [`Mesh`] each, holding only the vertices
//! it uses), `g` groups, `usemtl` and `s` smoothing groups (a [`Part`] per combination),
//! `mtllib` (asked of the resolver), `\` line continuations and `#` comments. Lines (`l`),
//! points (`p`) and free-form geometry are passed over. A vertex or face line that cannot be
//! read is left out and counted in one warning; the numbering of later vertices is kept.
//!
//! MTL materials map onto metallic-roughness: `Kd` → base colour, `d`/`Tr` → alpha (blended
//! when below 1), `Ke` → emissive, `Pr`/`Pm` (the PBR extension) → roughness/metallic,
//! otherwise roughness from the Phong exponent `Ns` (`sqrt(2 / (Ns + 2))`) and metallic 0;
//! `map_Kd`, `map_Ke`, `map_Bump`/`bump`/`norm` name base colour, emissive and normal images.
//! An image is only named ([`Image::uri`]); reading and decoding it is the caller's.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::{
    AlphaMode, Corner, Error, Image, Limits, Material, Mesh, NONE, Part, Resolver, Result, Scene, TextureRef, count_warning, is_mirror,
    linear_to_srgb, malformed, srgb_to_linear, too_large, transform_normal, transform_point,
};

const KIND: &str = "OBJ";

/// The last component of a referenced path, treating `\` as a separator too: a path from a
/// machine of another kind (`C:\textures\checker.png`) still names `checker.png`. `None` when
/// the name carries no path at all.
pub fn sibling_name(name: &str) -> Option<&str> {
    let file = name.rsplit(['/', '\\']).next()?;
    (!file.is_empty() && file.len() < name.len()).then_some(file)
}

/// The image a `map_*` line names: the rest of the line after its options
/// (`-bm 1 -o 0.5 0.5 my file.png`). Options are passed over by their argument counts, so a
/// name with spaces is one name; after an option this reader does not know, the name is the
/// final token.
fn map_name(args: &str) -> &str {
    fn split(s: &str) -> (&str, &str) {
        s.split_once(char::is_whitespace).map_or((s, ""), |(a, b)| (a, b.trim_start()))
    }
    let mut rest = args.trim();
    loop {
        let (token, after) = split(rest);
        if after.is_empty() {
            break;
        }
        // How many arguments the option takes, and from which one they may be left out:
        // `-o`, `-s` and `-t` take one to three numbers.
        let (count, optional_from) = match token {
            "-blendu" | "-blendv" | "-bm" | "-boost" | "-cc" | "-clamp" | "-imfchan" | "-texres" | "-type" => (1, 1),
            "-mm" => (2, 2),
            "-o" | "-s" | "-t" => (3, 1),
            t if t.strip_prefix('-').is_some_and(|o| o.starts_with(|c: char| c.is_ascii_alphabetic())) => {
                return args.split_whitespace().last().unwrap_or(args);
            }
            _ => break,
        };
        rest = after;
        for i in 0..count {
            let (arg, after) = split(rest);
            // The last token is always the name.
            if after.is_empty() || (i >= optional_from && arg.parse::<f32>().is_err()) {
                break;
            }
            rest = after;
        }
    }
    rest.trim_end()
}

/// Materials of an MTL library and the images they name. `TextureRef::image` indexes the
/// returned images.
pub fn read_mtl(bytes: &[u8], limits: &Limits) -> Result<(Vec<Material>, Vec<Image>)> {
    let text = String::from_utf8_lossy(bytes);
    let mut out: Vec<Material> = vec![];
    let mut images: Vec<Image> = vec![];
    let mut pr_seen = false;
    for line in text.lines() {
        let line = line.trim();
        let (key, rest) = line.split_once(char::is_whitespace).map(|(k, r)| (k, r.trim())).unwrap_or((line, ""));
        if key == "newmtl" {
            if out.len() >= limits.max_materials {
                return Err(too_large("MTL", format!("more than {} materials", limits.max_materials)));
            }
            pr_seen = false;
            out.push(Material::plain(rest, [1.0; 4]));
            continue;
        }
        let Some(m) = out.last_mut() else { continue };
        mtl_line(m, key, rest, &mut pr_seen, &mut images, limits)?;
    }
    for m in &mut out {
        if m.base_color[3] < 0.999 {
            m.alpha_mode = AlphaMode::Blend;
        }
    }
    Ok((out, images))
}

fn mtl_line(m: &mut Material, key: &str, rest: &str, pr_seen: &mut bool, images: &mut Vec<Image>, limits: &Limits) -> Result<()> {
    let finite = |x: &str| x.parse::<f32>().ok().filter(|v| v.is_finite());
    let f3 = || -> [f32; 3] {
        let v: Vec<f32> = rest.split_whitespace().take(3).filter_map(finite).collect();
        match v[..] {
            [a, b, c] => [a, b, c],
            [a, ..] => [a; 3],
            [] => [0.0; 3],
        }
    };
    let f1 = || rest.split_whitespace().next().and_then(finite);
    let mut tex = || -> Result<Option<TextureRef>> {
        let name = map_name(rest);
        if name.is_empty() {
            return Ok(None);
        }
        let image = match images.iter().position(|i| i.uri.as_deref() == Some(name)) {
            Some(i) => i,
            None if images.len() >= limits.max_materials => return Err(too_large("MTL", format!("more than {} images", limits.max_materials))),
            None => {
                images.push(Image { name: name.to_string(), uri: Some(name.to_string()), ..Image::default() });
                images.len() - 1
            }
        };
        Ok(Some(TextureRef { image, ..TextureRef::default() }))
    };
    match key {
        "Kd" => {
            let c = f3().map(srgb_to_linear);
            m.base_color = [c[0], c[1], c[2], m.base_color[3]];
        }
        "Ke" => m.emissive = f3().map(srgb_to_linear),
        "d" => m.base_color[3] = f1().unwrap_or(1.0).clamp(0.0, 1.0),
        "Tr" => m.base_color[3] = (1.0 - f1().unwrap_or(0.0)).clamp(0.0, 1.0),
        "Ns" if !*pr_seen => m.roughness = (2.0 / (f1().unwrap_or(0.0).max(0.0) + 2.0)).sqrt(),
        "Pr" => {
            m.roughness = f1().unwrap_or(0.5).clamp(0.0, 1.0);
            *pr_seen = true;
        }
        "Pm" => m.metallic = f1().unwrap_or(0.0).clamp(0.0, 1.0),
        "map_Kd" => m.base_color_texture = tex()?,
        "map_Ke" => {
            m.emissive_texture = tex()?;
            if m.emissive == [0.0; 3] {
                m.emissive = [1.0; 3];
            }
        }
        "map_Bump" | "map_bump" | "bump" | "norm" => m.normal_texture = tex()?,
        _ => {}
    }
    Ok(())
}

/// A 1-based (or negative, counting back) OBJ reference as an index into `len` entries.
fn index(token: &str, len: usize) -> Option<u32> {
    let value: i64 = token.parse().ok()?;
    let len = i64::try_from(len).ok()?;
    let value = if value < 0 { len.checked_add(value)? } else { value.checked_sub(1)? };
    (0..len).contains(&value).then(|| u32::try_from(value).ok()).flatten()
}

#[derive(Default)]
struct Object {
    name: String,
    parts: Vec<Part>,
    slots: HashMap<(String, Option<usize>, u32), usize>,
}

struct Reader<'a> {
    limits: &'a Limits,
    v: Vec<[f64; 3]>,
    colors: Vec<[f32; 4]>,
    vt: Vec<[f32; 2]>,
    vn: Vec<[f64; 3]>,
    objects: Vec<Object>,
    scene: Scene,
    material_ids: HashMap<String, usize>,
    /// The `mtllib` names already read: exporters repeat the line for every object.
    libraries: HashSet<String>,
    material: Option<usize>,
    group: String,
    smoothing: u32,
    /// The part faces go to until the object, group, material or smoothing changes.
    slot: Option<usize>,
    triangles: usize,
    parts: usize,
    bad_lines: usize,
    data_bytes: usize,
}

const NAN3: [f64; 3] = [f64::NAN; 3];

/// `N` finite numbers from the next `N` tokens.
fn numbers<'a, const N: usize>(mut it: impl Iterator<Item = &'a str>) -> Option<[f64; N]> {
    let mut out = [0.0; N];
    for v in &mut out {
        *v = it.next()?.parse::<f64>().ok().filter(|v| v.is_finite())?;
    }
    Some(out)
}

impl Reader<'_> {
    fn pool_room(&self, len: usize, what: &str) -> Result<()> {
        if len >= self.limits.max_vertices {
            return Err(too_large(KIND, format!("more than {} {what}", self.limits.max_vertices)));
        }
        Ok(())
    }

    fn vertex(&mut self, rest: &[&str]) -> Result<()> {
        self.pool_room(self.v.len(), "vertices")?;
        let p = numbers::<3>(rest.iter().copied());
        // Keep the numbering: later faces count vertices by position.
        self.v.push(p.unwrap_or(NAN3));
        self.bad_lines += usize::from(p.is_none());
        // `v x y z r g b`: vertices without a colour are white.
        if let (true, Some([r, g, b])) = (rest.len() >= 6, numbers::<3>(rest.iter().skip(3).copied())) {
            self.colors.resize(self.v.len() - 1, [1.0; 4]);
            self.colors.push([r as f32, g as f32, b as f32, 1.0]);
        }
        Ok(())
    }

    fn start_object(&mut self, name: String) -> Result<()> {
        self.slot = None;
        let full = self.objects.len() >= self.limits.max_meshes;
        match self.objects.last_mut() {
            // An empty object takes the name instead of starting another.
            Some(o) if o.parts.is_empty() => o.name = name,
            _ if full => return Err(too_large(KIND, format!("more than {} objects", self.limits.max_meshes))),
            _ => self.objects.push(Object { name, ..Object::default() }),
        }
        Ok(())
    }

    fn use_material(&mut self, name: &str) -> Result<()> {
        self.slot = None;
        self.material = match self.material_ids.get(name) {
            Some(i) => Some(*i),
            None if self.scene.materials.len() >= self.limits.max_materials => {
                return Err(too_large(KIND, format!("more than {} materials", self.limits.max_materials)));
            }
            None => {
                // Not in any library read: a named grey default.
                self.scene.materials.push(Material::plain(name, [0.8, 0.8, 0.8, 1.0]));
                self.material_ids.insert(name.to_string(), self.scene.materials.len() - 1);
                Some(self.scene.materials.len() - 1)
            }
        };
        Ok(())
    }

    fn library(&mut self, name: &str, resolve: Resolver) -> Result<()> {
        // A library named again is already read: reading it again would add its materials and
        // images a second time and count its bytes against the limit once per line.
        if name.is_empty() || !self.libraries.insert(name.to_string()) {
            return Ok(());
        }
        if self.scene.external.len() >= self.limits.max_materials {
            return Err(too_large(KIND, format!("more than {} material libraries", self.limits.max_materials)));
        }
        self.scene.external.push(name.to_string());
        // The name as written (it may hold spaces), then each of several names, then the file
        // name of a path from another machine.
        let Some(bytes) = std::iter::once(name).chain(name.split_whitespace()).chain(sibling_name(name)).find_map(resolve) else {
            self.scene.warnings.push(format!("material library `{name}` was not supplied"));
            return Ok(());
        };
        self.data_bytes = self.data_bytes.saturating_add(bytes.len());
        if self.data_bytes > self.limits.max_data_bytes {
            return Err(too_large(KIND, format!("material libraries of more than {} bytes", self.limits.max_data_bytes)));
        }
        let (materials, images) = read_mtl(&bytes, self.limits)?;
        let base = self.scene.images.len();
        if base.saturating_add(images.len()) > self.limits.max_materials
            || self.scene.materials.len().saturating_add(materials.len()) > self.limits.max_materials
        {
            return Err(too_large(KIND, format!("more than {} materials or images", self.limits.max_materials)));
        }
        self.scene.external.extend(images.iter().filter_map(|i| i.uri.clone()));
        self.scene.images.extend(images);
        for mut m in materials {
            for t in [&mut m.base_color_texture, &mut m.emissive_texture, &mut m.normal_texture].into_iter().flatten() {
                t.image += base;
            }
            // The first definition of a name wins, as for a reader that looks names up in order.
            self.material_ids.entry(m.name.clone()).or_insert(self.scene.materials.len());
            self.scene.materials.push(m);
        }
        Ok(())
    }

    fn corner(&self, token: &str) -> Option<Corner> {
        let mut fields = token.split('/');
        let position = index(fields.next()?, self.v.len())?;
        self.v.get(position as usize).filter(|p| p[0].is_finite())?;
        let uv = fields.next().and_then(|t| index(t, self.vt.len())).filter(|i| self.vt.get(*i as usize).is_some_and(|t| t[0].is_finite()));
        let normal = fields.next().and_then(|t| index(t, self.vn.len())).filter(|i| self.vn.get(*i as usize).is_some_and(|n| n[0].is_finite()));
        Some(Corner { position, normal: normal.unwrap_or(NONE), uv: uv.unwrap_or(NONE) })
    }

    fn part(&mut self) -> Result<Option<&mut Part>> {
        if self.objects.is_empty() {
            self.objects.push(Object::default());
        }
        let Some(object) = self.objects.last_mut() else { return Ok(None) };
        let slot = match self.slot {
            Some(s) => s,
            None => match object.slots.get(&(self.group.clone(), self.material, self.smoothing)) {
                Some(s) => *s,
                None => {
                    if self.parts >= self.limits.max_parts {
                        return Err(too_large(KIND, format!("more than {} groups", self.limits.max_parts)));
                    }
                    self.parts += 1;
                    object.parts.push(Part { name: self.group.clone(), material: self.material, smoothing: self.smoothing, ..Part::default() });
                    object.slots.insert((self.group.clone(), self.material, self.smoothing), object.parts.len() - 1);
                    object.parts.len() - 1
                }
            },
        };
        self.slot = Some(slot);
        Ok(object.parts.get_mut(slot))
    }

    fn face(&mut self, rest: &[&str]) -> Result<()> {
        if rest.len() > self.limits.max_face_corners {
            return Err(too_large(KIND, format!("a face of more than {} corners", self.limits.max_face_corners)));
        }
        let corners: Option<Vec<Corner>> = rest.iter().map(|t| self.corner(t)).collect();
        let Some(corners) = corners.filter(|c| c.len() >= 3) else {
            self.bad_lines += 1;
            return Ok(());
        };
        self.triangles = self.triangles.saturating_add(corners.len() - 2);
        if self.triangles > self.limits.max_triangles {
            return Err(too_large(KIND, format!("more than {} triangles", self.limits.max_triangles)));
        }
        let Some(part) = self.part()? else { return Ok(()) };
        // Sizes are only kept once a face is not a triangle.
        if corners.len() != 3 && part.face_sizes.is_empty() {
            part.face_sizes = vec![3; part.corners.len() / 3];
        }
        if corners.len() != 3 || !part.face_sizes.is_empty() {
            part.face_sizes.push(corners.len() as u32);
        }
        part.corners.extend(corners);
        Ok(())
    }

    fn line(&mut self, line: &str, resolve: Resolver) -> Result<()> {
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { return Ok(()) };
        let after = line.get(key.len()..).unwrap_or("").trim();
        match key {
            "v" => self.vertex(&it.collect::<Vec<_>>())?,
            "vt" => {
                self.pool_room(self.vt.len(), "texture coordinates")?;
                // `vt u` alone is a 1D coordinate; OBJ's v axis points up the image.
                let uv = numbers::<2>(it.chain(["0"]));
                self.vt.push(uv.map_or([f32::NAN; 2], |[u, v]| [u as f32, 1.0 - v as f32]));
                self.bad_lines += usize::from(uv.is_none());
            }
            "vn" => {
                self.pool_room(self.vn.len(), "normals")?;
                let n = numbers::<3>(it);
                self.vn.push(n.unwrap_or(NAN3));
                self.bad_lines += usize::from(n.is_none());
            }
            "s" => {
                self.slot = None;
                self.smoothing = match it.next() {
                    Some("off" | "0") | None => 0,
                    Some("on") => 1,
                    Some(v) => v.parse::<u32>().unwrap_or(0),
                };
            }
            "o" => self.start_object(after.to_string())?,
            "g" => {
                self.slot = None;
                self.group = after.to_string();
            }
            "usemtl" => self.use_material(after)?,
            "mtllib" => self.library(after, resolve)?,
            "f" => self.face(&it.collect::<Vec<_>>())?,
            _ => {}
        }
        Ok(())
    }

    /// Each object as a mesh holding only the vertices it uses.
    fn finish(mut self) -> Result<Scene> {
        let colored = !self.colors.is_empty();
        self.colors.resize(self.v.len(), [1.0; 4]);
        let mut meshes = Vec::new();
        for object in std::mem::take(&mut self.objects) {
            if object.parts.is_empty() {
                continue;
            }
            let mut mesh = Mesh { name: object.name, parts: object.parts, ..Mesh::default() };
            // The file's order is kept within each pool.
            let used = |of: fn(&Corner) -> u32| {
                let mut u: Vec<u32> = mesh.parts.iter().flat_map(|p| p.corners.iter().map(of)).filter(|i| *i != NONE).collect();
                u.sort_unstable();
                u.dedup();
                u
            };
            let (pu, nu, tu) = (used(|c| c.position), used(|c| c.normal), used(|c| c.uv));
            let place = |u: &[u32], i: u32| if i == NONE { NONE } else { u.binary_search(&i).map_or(NONE, |k| k as u32) };
            for c in mesh.parts.iter_mut().flat_map(|p| p.corners.iter_mut()) {
                *c = Corner { position: place(&pu, c.position), normal: place(&nu, c.normal), uv: place(&tu, c.uv) };
            }
            mesh.positions = pu.iter().map(|i| self.v.get(*i as usize).copied().unwrap_or_default()).collect();
            if colored {
                mesh.colors = pu.iter().map(|i| self.colors.get(*i as usize).copied().unwrap_or([1.0; 4])).collect();
            }
            mesh.normals = nu.iter().map(|i| self.vn.get(*i as usize).copied().unwrap_or_default()).collect();
            let uvs: Vec<[f32; 2]> = tu.iter().map(|i| self.vt.get(*i as usize).copied().unwrap_or_default()).collect();
            if !uvs.is_empty() {
                mesh.uv_sets.push(uvs);
            }
            meshes.push(mesh);
        }
        if meshes.is_empty() {
            return Err(malformed(KIND, "no faces"));
        }
        let mut scene = Scene::from_meshes(meshes, std::mem::take(&mut self.scene.materials));
        scene.images = std::mem::take(&mut self.scene.images);
        scene.external = std::mem::take(&mut self.scene.external);
        scene.warnings = std::mem::take(&mut self.scene.warnings);
        count_warning(&mut scene.warnings, self.bad_lines, "vertex or face line(s) could not be read and were left out");
        Ok(scene)
    }
}

/// Read an OBJ file. Its material libraries are asked of `resolve` by the name the file
/// writes; every name asked, and every image a library names, is listed in
/// [`Scene::external`].
pub fn read(bytes: &[u8], limits: &Limits, resolve: Resolver) -> Result<Scene> {
    let text = String::from_utf8_lossy(bytes);
    let mut r = Reader {
        limits,
        v: vec![],
        colors: vec![],
        vt: vec![],
        vn: vec![],
        objects: vec![],
        scene: Scene::default(),
        material_ids: HashMap::new(),
        libraries: HashSet::new(),
        material: None,
        group: String::new(),
        smoothing: 0,
        slot: None,
        triangles: 0,
        parts: 0,
        bad_lines: 0,
        data_bytes: 0,
    };
    // A line ending in `\` continues on the next.
    let mut joined = String::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("");
        if let Some(head) = line.trim_end().strip_suffix('\\') {
            joined.push_str(head);
            joined.push(' ');
            continue;
        }
        if joined.is_empty() {
            r.line(line.trim(), resolve)?;
        } else {
            joined.push_str(line);
            r.line(joined.trim(), resolve)?;
            joined.clear();
        }
    }
    r.line(joined.trim(), resolve)?;
    r.finish()
}

/// A name as one OBJ token: whitespace and control characters become `_`.
fn token(name: &str) -> String {
    let t: String = name.trim().chars().map(|c| if c.is_whitespace() || c.is_control() { '_' } else { c }).collect();
    if t.is_empty() { "material".into() } else { t }
}

/// The name each material is written with: its [`token`], made unique with `_2`, `_3`… where
/// two would be the same, also ignoring case (the names may become file names).
pub fn material_names(scene: &Scene) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    scene
        .materials
        .iter()
        .map(|m| {
            let base = token(&m.name);
            let mut unique = base.clone();
            let mut n = 1u32;
            while !seen.insert(unique.to_lowercase()) {
                n = n.saturating_add(1);
                unique = format!("{base}_{n}");
            }
            unique
        })
        .collect()
}

/// The scene as OBJ text: every mesh placed by its nodes as an object (`o`), with its vertex
/// colours, first UV set and normals, a group (`g`), material (`usemtl`) and smoothing group
/// (`s`) per part, and polygons kept whole. `mtllib` is the material library's file name, to
/// be written next to the OBJ from [`write_mtl`]; `None` writes no `mtllib` line.
pub fn write(scene: &Scene, mtllib: Option<&str>, limits: &Limits) -> Result<String> {
    let mut text = String::new();
    if let Some(lib) = mtllib.filter(|_| !scene.materials.is_empty()) {
        let _ = writeln!(text, "mtllib {}", lib.replace(['\n', '\r'], " "));
    }
    let materials = material_names(scene);
    // OBJ indices run on across objects.
    let (mut vbase, mut tbase, mut nbase) = (1u64, 1u64, 1u64);
    let mut smoothing = 0;
    for (index, world) in scene.instances(limits)? {
        let Some(mesh) = scene.meshes.get(index) else { continue };
        mesh.validate()?;
        if !mesh.name.trim().is_empty() {
            let _ = writeln!(text, "o {}", mesh.name.trim().replace(|c: char| c.is_control(), " "));
        }
        let colors = mesh.colors.len() == mesh.positions.len();
        let mirror = is_mirror(&world);
        for (i, p) in mesh.positions.iter().enumerate() {
            let p = transform_point(&world, *p);
            let _ = write!(text, "v {} {} {}", p[0], p[1], p[2]);
            if let Some(c) = mesh.colors.get(i).filter(|_| colors) {
                let _ = write!(text, " {} {} {}", c[0], c[1], c[2]);
            }
            text.push('\n');
        }
        let uvs = mesh.uv_sets.first().map_or(&[][..], Vec::as_slice);
        for t in uvs {
            let _ = writeln!(text, "vt {} {}", t[0], 1.0 - t[1]);
        }
        for n in &mesh.normals {
            let n = transform_normal(&world, *n);
            let _ = writeln!(text, "vn {} {} {}", n[0], n[1], n[2]);
        }
        for part in &mesh.parts {
            if !part.name.trim().is_empty() {
                let _ = writeln!(text, "g {}", part.name.trim().replace(|c: char| c.is_control(), " "));
            }
            if let Some(m) = part.material.and_then(|m| materials.get(m)) {
                let _ = writeln!(text, "usemtl {m}");
            }
            if part.smoothing != smoothing {
                smoothing = part.smoothing;
                let _ = if smoothing == 0 { writeln!(text, "s off") } else { writeln!(text, "s {smoothing}") };
            }
            for face in part.faces() {
                text.push('f');
                // A mirrored placement turns the winding; turn it back so the face faces out.
                let mut face = face.to_vec();
                if mirror {
                    face.reverse();
                }
                for c in face {
                    let v = vbase + u64::from(c.position);
                    let _ = match (c.uv != NONE, c.normal != NONE) {
                        (true, true) => write!(text, " {v}/{}/{}", tbase + u64::from(c.uv), nbase + u64::from(c.normal)),
                        (true, false) => write!(text, " {v}/{}", tbase + u64::from(c.uv)),
                        (false, true) => write!(text, " {v}//{}", nbase + u64::from(c.normal)),
                        (false, false) => write!(text, " {v}"),
                    };
                }
                text.push('\n');
            }
        }
        vbase += mesh.positions.len() as u64;
        tbase += uvs.len() as u64;
        nbase += mesh.normals.len() as u64;
    }
    if vbase == 1 {
        return Err(Error::Malformed("OBJ: the scene has no placed mesh to write".into()));
    }
    Ok(text)
}

/// The scene's materials as an MTL library, named as [`write`] names them: `Kd`, `d`, `Ke`,
/// `Ns` (for readers without the PBR extension) with `Pr` and `Pm`, and `map_Kd`, `map_Ke`
/// and `norm` for images that have a [`Image::uri`].
pub fn write_mtl(scene: &Scene) -> String {
    let mut mtl = String::new();
    for (m, name) in scene.materials.iter().zip(material_names(scene)) {
        let kd = [m.base_color[0], m.base_color[1], m.base_color[2]].map(linear_to_srgb);
        let ke = m.emissive.map(linear_to_srgb);
        let r = m.roughness.clamp(0.0, 1.0);
        let ns = if r > 0.0 { (2.0 / (r * r) - 2.0).clamp(0.0, 1000.0) } else { 1000.0 };
        let _ = writeln!(
            mtl,
            "newmtl {name}\nKd {} {} {}\nd {}\nKe {} {} {}\nNs {ns}\nPr {r}\nPm {}",
            kd[0], kd[1], kd[2], m.base_color[3], ke[0], ke[1], ke[2], m.metallic
        );
        for (key, tex) in [("map_Kd", m.base_color_texture), ("map_Ke", m.emissive_texture), ("norm", m.normal_texture)] {
            if let Some(uri) = tex.and_then(|t| scene.images.get(t.image)).and_then(|i| i.uri.as_deref()) {
                let _ = writeln!(mtl, "{key} {}", uri.replace(['\n', '\r'], " "));
            }
        }
        mtl.push('\n');
    }
    mtl
}
