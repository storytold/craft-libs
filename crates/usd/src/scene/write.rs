//! Writers for neutral [`UsdLayer`] scenes: `.usda` text and `.usdc` Crate binary (0.8.0).

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::codec::{self, ty};
use crate::compress;
use crate::integers::{self, Width};
use crate::scene::{MAX_DECODED_BYTES, Result, SceneError, UsdAttr, UsdLayer, UsdPrim, UsdValue};

const VERSION: [u8; 3] = [0, 8, 0];
const SPEC_ATTRIBUTE: i64 = 1;
const SPEC_PRIM: i64 = 6;
const SPEC_PSEUDO_ROOT: i64 = 7;
const SPEC_RELATIONSHIP: i64 = 8;
/// List-op header bits.
const LIST_EXPLICIT: u8 = 0b11;
const LIST_PREPENDED: u8 = 1 << 5;

fn rep(t: u8, flags: u64, payload: u64) -> u64 {
    (u64::from(t) << 48) | flags | (payload & codec::PAYLOAD_MASK)
}

fn compress_block(input: &[u8]) -> Result<Vec<u8>> {
    compress::compress(input, MAX_DECODED_BYTES).map_err(|e| SceneError::Malformed(e.to_string()))
}

fn encode_ints(values: &[i64]) -> Result<Vec<u8>> {
    integers::encode(values, Width::I32, MAX_DECODED_BYTES).map_err(|e| SceneError::Malformed(e.to_string()))
}

/// Text of a value as `.usda` writes it.
fn usda_value(s: &mut String, v: &UsdValue) {
    let tuple = |s: &mut String, c: &[f32]| {
        s.push('(');
        for (i, x) in c.iter().enumerate() {
            let _ = write!(s, "{}{x}", if i > 0 { ", " } else { "" });
        }
        s.push(')');
    };
    let list = |s: &mut String, n: usize, item: &mut dyn FnMut(&mut String, usize)| {
        s.push('[');
        for i in 0..n {
            if i > 0 {
                s.push_str(", ");
            }
            item(s, i);
        }
        s.push(']');
    };
    match v {
        UsdValue::Token(t) | UsdValue::Str(t) => {
            let _ = write!(s, "\"{t}\"");
        }
        UsdValue::Int(i) => {
            let _ = write!(s, "{i}");
        }
        UsdValue::Double(d) => {
            let _ = write!(s, "{d}");
        }
        UsdValue::Asset(a) => {
            let _ = write!(s, "@{a}@");
        }
        UsdValue::Target(p) => {
            let _ = write!(s, "<{p}>");
        }
        UsdValue::Float4(c) => tuple(s, c),
        UsdValue::Ints(v) => list(s, v.len(), &mut |s, i| {
            let _ = write!(s, "{}", v.get(i).copied().unwrap_or_default());
        }),
        UsdValue::Vec2fs(v) => list(s, v.len(), &mut |s, i| tuple(s, v.get(i).map_or(&[][..], |x| &x[..]))),
        UsdValue::Vec3fs(v) => list(s, v.len(), &mut |s, i| tuple(s, v.get(i).map_or(&[][..], |x| &x[..]))),
    }
}

fn usda_prim(s: &mut String, p: &UsdPrim, depth: usize) {
    let pad = "    ".repeat(depth);
    let _ = write!(s, "\n{pad}def {} \"{}\"", p.type_name, p.name);
    if p.binding_api {
        let _ = write!(s, " (\n{pad}    prepend apiSchemas = [\"MaterialBindingAPI\"]\n{pad})");
    }
    let _ = writeln!(s, "\n{pad}{{");
    for a in &p.attrs {
        s.push_str(&pad);
        s.push_str("    ");
        if a.type_name == "rel" {
            let _ = write!(s, "rel {}", a.name);
        } else {
            let _ = write!(s, "{}{} {}", if a.uniform { "uniform " } else { "" }, a.type_name, a.name);
        }
        if let Some(c) = &a.connect {
            let _ = write!(s, ".connect = <{c}>");
        } else if let Some(v) = &a.value {
            s.push_str(" = ");
            usda_value(s, v);
        }
        if let Some(i) = &a.interpolation {
            let _ = write!(s, " (\n{pad}        interpolation = \"{i}\"\n{pad}    )");
        }
        s.push('\n');
    }
    for c in &p.children {
        usda_prim(s, c, depth + 1);
    }
    let _ = writeln!(s, "{pad}}}");
}

/// Format `layer` as `.usda` text.
pub fn usda(layer: &UsdLayer) -> String {
    let mut s = format!("#usda 1.0\n(\n    defaultPrim = \"{}\"\n", layer.root.name);
    if let Some(doc) = &layer.doc {
        let _ = writeln!(s, "    doc = \"{doc}\"");
    }
    let _ = writeln!(s, "    metersPerUnit = {}\n    upAxis = \"{}\"\n)", layer.meters_per_unit, layer.up_axis);
    usda_prim(&mut s, &layer.root, 0);
    s
}

struct Node {
    token: u32,
    prop: bool,
    children: Vec<usize>,
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    tokens: Vec<String>,
    token_ids: HashMap<String, u32>,
    /// String table: token index per string.
    strings: Vec<u32>,
    fields: Vec<(u32, u64)>,
    field_ids: HashMap<(u32, u64), u32>,
    fieldsets: Vec<i64>,
    fieldset_ids: HashMap<Vec<u32>, u32>,
    /// Path tree (index 0 is the root).
    paths: Vec<Node>,
    path_ids: HashMap<String, usize>,
    specs: Vec<[i64; 3]>,
}

impl Writer {
    /// The bootstrap header and the root path; values follow.
    fn new() -> Writer {
        let mut w = Writer::default();
        // Token 0 must never name a property (properties store their token negated).
        w.token("Xform");
        w.paths.push(Node { token: 0, prop: false, children: Vec::new() });
        w.path_ids.insert("/".into(), 0);
        w.out.extend_from_slice(codec::MAGIC);
        w.out.extend_from_slice(&VERSION);
        w.out.resize(codec::BOOTSTRAP_LEN, 0);
        w
    }

    fn token(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.token_ids.get(s) {
            return i;
        }
        let i = self.tokens.len() as u32;
        self.tokens.push(s.into());
        self.token_ids.insert(s.into(), i);
        i
    }

    /// Index of `s` in the string table (which holds token indices).
    fn string(&mut self, s: &str) -> u32 {
        let t = self.token(s);
        match self.strings.iter().position(|x| *x == t) {
            Some(i) => i as u32,
            None => {
                self.strings.push(t);
                (self.strings.len() - 1) as u32
            }
        }
    }

    fn add_path(&mut self, parent: usize, parent_text: &str, name: &str, prop: bool) -> (usize, String) {
        let text = match (prop, parent_text) {
            (true, _) => format!("{parent_text}.{name}"),
            (false, "/") => format!("/{name}"),
            _ => format!("{parent_text}/{name}"),
        };
        let id = self.paths.len();
        let token = self.token(name);
        self.paths.push(Node { token, prop, children: Vec::new() });
        if let Some(p) = self.paths.get_mut(parent) {
            p.children.push(id);
        }
        self.path_ids.insert(text.clone(), id);
        (id, text)
    }

    /// Pre-order path nodes of `p` (attributes before child prims).
    fn add_prim_paths(&mut self, parent: usize, parent_text: &str, p: &UsdPrim) {
        let (id, text) = self.add_path(parent, parent_text, &p.name, false);
        for a in &p.attrs {
            self.add_path(id, &text, &a.name, true);
        }
        for c in &p.children {
            self.add_prim_paths(id, &text, c);
        }
    }

    fn path_id(&self, path: &str) -> Result<u32> {
        self.path_ids.get(path).map(|&i| i as u32).ok_or_else(|| SceneError::Invalid(format!("USD export: unknown path {path}")))
    }

    fn offset(&self) -> u64 {
        self.out.len() as u64
    }

    fn u32s(&mut self, v: impl Iterator<Item = u32>) {
        for x in v {
            self.out.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn list_op(&mut self, t: u8, header: u8, items: &[u32]) -> u64 {
        let at = self.offset();
        self.out.push(header);
        self.out.extend_from_slice(&(items.len() as u64).to_le_bytes());
        self.u32s(items.iter().copied());
        rep(t, 0, at)
    }

    fn token_vector(&mut self, names: &[&str]) -> u64 {
        let ids: Vec<u32> = names.iter().map(|n| self.token(n)).collect();
        let at = self.offset();
        self.out.extend_from_slice(&(ids.len() as u64).to_le_bytes());
        self.u32s(ids.into_iter());
        rep(ty::TOKEN_VECTOR, 0, at)
    }

    /// An array of `n` elements whose little-endian bytes are `bytes`.
    fn raw_array(&mut self, t: u8, n: usize, bytes: impl Iterator<Item = u8>) -> u64 {
        let at = self.offset();
        self.out.extend_from_slice(&(n as u64).to_le_bytes());
        self.out.extend(bytes);
        rep(t, codec::IS_ARRAY, at)
    }

    fn value(&mut self, v: &UsdValue) -> Result<u64> {
        let floats = |v: &mut dyn Iterator<Item = f32>| v.flat_map(f32::to_le_bytes).collect::<Vec<u8>>();
        Ok(match v {
            UsdValue::Token(t) => rep(ty::TOKEN, codec::IS_INLINED, u64::from(self.token(t))),
            UsdValue::Asset(a) => rep(ty::ASSET_PATH, codec::IS_INLINED, u64::from(self.token(a))),
            UsdValue::Str(s) => rep(ty::STRING, codec::IS_INLINED, u64::from(self.string(s))),
            UsdValue::Int(i) => rep(ty::INT, codec::IS_INLINED, u64::from(*i as u32)),
            UsdValue::Double(d) => {
                let at = self.offset();
                self.out.extend_from_slice(&d.to_le_bytes());
                rep(ty::DOUBLE, 0, at)
            }
            UsdValue::Float4(c) => {
                let at = self.offset();
                self.out.extend(floats(&mut c.iter().copied()));
                rep(ty::VEC4F, 0, at)
            }
            UsdValue::Ints(v) if v.len() >= codec::MIN_COMPRESSED_ARRAY => {
                let at = self.offset();
                self.out.extend_from_slice(&(v.len() as u64).to_le_bytes());
                self.ints(&v.iter().map(|&i| i64::from(i)).collect::<Vec<_>>())?;
                rep(ty::INT, codec::IS_ARRAY | codec::IS_COMPRESSED, at)
            }
            UsdValue::Ints(v) => self.raw_array(ty::INT, v.len(), v.iter().flat_map(|i| i.to_le_bytes())),
            UsdValue::Vec2fs(v) => {
                let bytes = floats(&mut v.iter().flatten().copied());
                self.raw_array(ty::VEC2F, v.len(), bytes.into_iter())
            }
            UsdValue::Vec3fs(v) => {
                let bytes = floats(&mut v.iter().flatten().copied());
                self.raw_array(ty::VEC3F, v.len(), bytes.into_iter())
            }
            UsdValue::Target(p) => {
                let id = self.path_id(p)?;
                self.list_op(ty::PATH_LIST_OP, LIST_EXPLICIT, &[id])
            }
        })
    }

    /// Compressed integers: their compressed size, then the LZ4-framed coding.
    fn ints(&mut self, v: &[i64]) -> Result<()> {
        let comp = compress_block(&encode_ints(v)?)?;
        self.out.extend_from_slice(&(comp.len() as u64).to_le_bytes());
        self.out.extend_from_slice(&comp);
        Ok(())
    }

    fn spec(&mut self, path: usize, kind: i64, fields: Vec<(&str, u64)>) {
        let ids: Vec<u32> = fields
            .into_iter()
            .map(|(name, value)| {
                let key = (self.token(name), value);
                let next = self.fields.len() as u32;
                *self.field_ids.entry(key).or_insert_with(|| {
                    self.fields.push(key);
                    next
                })
            })
            .collect();
        let set = match self.fieldset_ids.get(&ids) {
            Some(&s) => s,
            None => {
                let s = self.fieldsets.len() as u32;
                self.fieldsets.extend(ids.iter().map(|&i| i64::from(i)));
                self.fieldsets.push(i64::from(u32::MAX));
                self.fieldset_ids.insert(ids, s);
                s
            }
        };
        self.specs.push([path as i64, i64::from(set), kind]);
    }

    fn prim_specs(&mut self, text: &str, p: &UsdPrim) -> Result<()> {
        let id = *self.path_ids.get(text).ok_or_else(|| SceneError::Invalid(format!("USD export: unknown path {text}")))?;
        let mut fields = vec![
            ("specifier", rep(ty::SPECIFIER, codec::IS_INLINED, 0)),
            ("typeName", rep(ty::TOKEN, codec::IS_INLINED, u64::from(self.token(&p.type_name)))),
        ];
        if p.binding_api {
            let api = self.token("MaterialBindingAPI");
            fields.push(("apiSchemas", self.list_op(ty::TOKEN_LIST_OP, LIST_PREPENDED, &[api])));
        }
        if !p.attrs.is_empty() {
            let names: Vec<&str> = p.attrs.iter().map(|a| a.name.as_str()).collect();
            fields.push(("properties", self.token_vector(&names)));
        }
        if !p.children.is_empty() {
            let names: Vec<&str> = p.children.iter().map(|c| c.name.as_str()).collect();
            fields.push(("primChildren", self.token_vector(&names)));
        }
        self.spec(id, SPEC_PRIM, fields);
        for a in &p.attrs {
            self.attr_spec(&format!("{text}.{}", a.name), a)?;
        }
        for c in &p.children {
            self.prim_specs(&format!("{text}/{}", c.name), c)?;
        }
        Ok(())
    }

    fn attr_spec(&mut self, text: &str, a: &UsdAttr) -> Result<()> {
        let id = *self.path_ids.get(text).ok_or_else(|| SceneError::Invalid(format!("USD export: unknown path {text}")))?;
        let rel = a.type_name == "rel";
        let mut fields = vec![
            ("custom", rep(ty::BOOL, codec::IS_INLINED, 0)),
            ("variability", rep(ty::VARIABILITY, codec::IS_INLINED, u64::from(rel || a.uniform))),
        ];
        if !rel {
            fields.push(("typeName", rep(ty::TOKEN, codec::IS_INLINED, u64::from(self.token(&a.type_name)))));
        }
        if let Some(c) = &a.connect {
            let id = self.path_id(c)?;
            fields.push(("connectionPaths", self.list_op(ty::PATH_LIST_OP, LIST_EXPLICIT, &[id])));
        }
        if let Some(v) = &a.value {
            fields.push((if rel { "targetPaths" } else { "default" }, self.value(v)?));
        }
        if let Some(i) = &a.interpolation {
            fields.push(("interpolation", rep(ty::TOKEN, codec::IS_INLINED, u64::from(self.token(i)))));
        }
        self.spec(id, if rel { SPEC_RELATIONSHIP } else { SPEC_ATTRIBUTE }, fields);
        Ok(())
    }

    /// (path index, element token, jump) per node, in pre-order.
    fn path_table(&self) -> [Vec<i64>; 3] {
        let n = self.paths.len();
        let mut order = Vec::with_capacity(n);
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            order.push(i);
            stack.extend(self.paths.get(i).into_iter().flat_map(|p| p.children.iter().rev()));
        }
        // Subtree sizes: the next sibling follows a node's whole subtree.
        let mut size = vec![1i64; n];
        for &i in order.iter().rev() {
            let s: i64 = self.paths.get(i).map_or(0, |p| p.children.iter().filter_map(|c| size.get(*c)).sum());
            if let Some(x) = size.get_mut(i) {
                *x += s;
            }
        }
        let mut has_sibling = vec![false; n];
        for p in &self.paths {
            for w in p.children.windows(2) {
                if let Some(s) = w.first().and_then(|&c| has_sibling.get_mut(c)) {
                    *s = true;
                }
            }
        }
        let mut table = [Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n)];
        for i in order {
            let Some(p) = self.paths.get(i) else { continue };
            let jump = match (!p.children.is_empty(), has_sibling.get(i).copied().unwrap_or(false)) {
                (true, true) => size.get(i).copied().unwrap_or(1),
                (true, false) => -1,
                (false, true) => 0,
                (false, false) => -2,
            };
            let token = i64::from(p.token);
            table[0].push(i as i64);
            table[1].push(if p.prop { -token } else { token });
            table[2].push(jump);
        }
        table
    }

    fn section(&mut self, toc: &mut Vec<(&'static str, u64, u64)>, name: &'static str, start: u64) {
        toc.push((name, start, self.offset() - start));
    }

    fn u64(&mut self, v: usize) {
        self.out.extend_from_slice(&(v as u64).to_le_bytes());
    }

    fn sections(&mut self) -> Result<()> {
        let mut toc = Vec::new();
        let start = self.offset();
        let mut raw = Vec::new();
        for t in &self.tokens {
            raw.extend_from_slice(t.as_bytes());
            raw.push(0);
        }
        let comp = compress_block(&raw)?;
        self.u64(self.tokens.len());
        self.u64(raw.len());
        self.u64(comp.len());
        self.out.extend_from_slice(&comp);
        self.section(&mut toc, "TOKENS", start);

        let start = self.offset();
        let strings = std::mem::take(&mut self.strings);
        self.u64(strings.len());
        self.u32s(strings.into_iter());
        self.section(&mut toc, "STRINGS", start);

        let start = self.offset();
        let fields = std::mem::take(&mut self.fields);
        self.u64(fields.len());
        self.ints(&fields.iter().map(|f| i64::from(f.0)).collect::<Vec<_>>())?;
        let reps: Vec<u8> = fields.iter().flat_map(|f| f.1.to_le_bytes()).collect();
        let comp = compress_block(&reps)?;
        self.u64(comp.len());
        self.out.extend_from_slice(&comp);
        self.section(&mut toc, "FIELDS", start);

        let start = self.offset();
        let sets = std::mem::take(&mut self.fieldsets);
        self.u64(sets.len());
        self.ints(&sets)?;
        self.section(&mut toc, "FIELDSETS", start);

        let start = self.offset();
        let table = self.path_table();
        self.u64(self.paths.len());
        self.u64(self.paths.len());
        for column in &table {
            self.ints(column)?;
        }
        self.section(&mut toc, "PATHS", start);

        let start = self.offset();
        let specs = std::mem::take(&mut self.specs);
        self.u64(specs.len());
        for k in 0..3 {
            self.ints(&specs.iter().map(|s| s.get(k).copied().unwrap_or_default()).collect::<Vec<_>>())?;
        }
        self.section(&mut toc, "SPECS", start);

        let toc_at = self.offset();
        self.u64(toc.len());
        for (name, start, size) in toc {
            let mut n = [0u8; 16];
            for (d, s) in n.iter_mut().zip(name.bytes()) {
                *d = s;
            }
            self.out.extend_from_slice(&n);
            self.out.extend_from_slice(&start.to_le_bytes());
            self.out.extend_from_slice(&size.to_le_bytes());
        }
        if let Some(slot) = self.out.get_mut(16..24) {
            slot.copy_from_slice(&toc_at.to_le_bytes());
        }
        Ok(())
    }

    fn root_spec(&mut self, default_prim: &str, meters_per_unit: f64, up_axis: &str) -> Result<()> {
        let fields = vec![
            ("defaultPrim", rep(ty::TOKEN, codec::IS_INLINED, u64::from(self.token(default_prim)))),
            ("metersPerUnit", self.value(&UsdValue::Double(meters_per_unit))?),
            ("upAxis", rep(ty::TOKEN, codec::IS_INLINED, u64::from(self.token(up_axis)))),
            ("primChildren", self.token_vector(&[default_prim])),
        ];
        self.spec(0, SPEC_PSEUDO_ROOT, fields);
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<u8>> {
        self.sections()?;
        Ok(self.out)
    }
}

/// Encode `layer` as a `.usdc` Crate binary.
pub fn usdc(layer: &UsdLayer) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    let root = &layer.root;
    w.add_prim_paths(0, "/", root);
    w.root_spec(&root.name, layer.meters_per_unit, &layer.up_axis)?;
    w.prim_specs(&format!("/{}", root.name), root)?;
    w.finish()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{SPEC_ATTRIBUTE, SPEC_PRIM, Writer, rep, usda, usdc};
    use crate::codec::{self, ty};
    use crate::scene::{Specifier, UsdAttr, UsdLayer, UsdPrim, UsdValue, Val, read_layer};

    impl Writer {
        fn raw(&mut self, bytes: &[u8]) -> u64 {
            let at = self.offset();
            self.out.extend_from_slice(bytes);
            at
        }

        fn prop_path(&mut self, parent: (usize, &str), name: &str) -> usize {
            self.add_path(parent.0, parent.1, name, true).0
        }
    }

    fn le(values: &[f64]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn usda_and_usdc_round_trip_neutral_layer() {
        let mut root = UsdPrim::new("World", "Xform");
        let mut mesh = UsdPrim::new("Tri", "Mesh");
        mesh.binding_api = true;
        mesh.attrs = vec![
            UsdAttr::new("points", "point3f[]", Some(UsdValue::Vec3fs(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]))),
            UsdAttr::new("faceVertexCounts", "int[]", Some(UsdValue::Ints(vec![3]))),
            UsdAttr::new("faceVertexIndices", "int[]", Some(UsdValue::Ints(vec![0, 1, 2]))),
            UsdAttr::new("primvars:st", "texCoord2f[]", Some(UsdValue::Vec2fs(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]])))
                .with_interpolation("vertex"),
            UsdAttr::rel("material:binding", "/World/Mat".into()),
        ];
        root.children.push(mesh);
        let mut mat = UsdPrim::new("Mat", "Material");
        mat.attrs.push(UsdAttr::connected("outputs:surface", "token", "/World/Mat/PBR.outputs:surface".into()));
        let mut pbr = UsdPrim::new("PBR", "Shader");
        pbr.attrs.push(UsdAttr::new("outputs:surface", "token", None));
        mat.children.push(pbr);
        root.children.push(mat);

        let mut layer = UsdLayer::new(root);
        layer.doc = Some("Test doc".into());
        layer.meters_per_unit = 0.01;
        layer.up_axis = "Z".into();

        let text = usda(&layer);
        assert!(text.contains("doc = \"Test doc\""));
        assert!(text.contains("metersPerUnit = 0.01"));
        assert!(text.contains("upAxis = \"Z\""));

        for bytes in [text.as_bytes(), &usdc(&layer).unwrap()] {
            let parsed = read_layer(bytes).unwrap();
            assert_eq!(parsed.meters_per_unit, Some(0.01));
            assert_eq!(parsed.up_axis.as_deref(), Some("Z"));
            let tri = parsed.prims.iter().find(|p| p.path == "/World/Tri").unwrap();
            assert_eq!(tri.type_name, "Mesh");
            assert_eq!(tri.specifier, Specifier::Def);
            assert_eq!(tri.props.get("primvars:st").and_then(|p| p.interpolation.as_deref()), Some("vertex"));
            assert_eq!(tri.props.get("material:binding").and_then(|p| p.value.as_ref()), Some(&Val::Path(vec!["/World/Mat".into()])));
        }
    }

    /// A Crate file assembled spec by spec with a class prim referenced by a def prim
    /// (with customData on the reference), a time-sampled out-of-line double3 translate,
    /// a token[] xformOpOrder, and an inlined diagonal matrix.
    #[test]
    fn crafted_crate_with_references_time_samples_and_inlined_values() {
        let mut w = Writer::new();
        let (wp, wt) = w.add_path(0, "/", "W", false);
        let (pp, pt) = w.add_path(0, "/", "P", false);
        let (mp, mt) = w.add_path(pp, &pt, "M", false);
        let ops = ["xformOp:translate", "xformOp:transform", "xformOpOrder"].map(|n| w.prop_path((wp, &wt), n));
        let attrs = ["points", "faceVertexCounts", "faceVertexIndices"].map(|n| w.prop_path((mp, &mt), n));
        w.root_spec("W", 1.0, "Y").unwrap();
        let empty = w.token("");
        let key = w.token("note");
        w.strings = vec![empty, key];
        let def = |t: &str, w: &mut Writer| {
            vec![("specifier", rep(ty::SPECIFIER, codec::IS_INLINED, 0)), ("typeName", rep(ty::TOKEN, codec::IS_INLINED, u64::from(w.token(t))))]
        };

        // /W: def Xform (prepend references = </P> with customData = {note: 7}).
        let mut fields = def("Xform", &mut w);
        let mut r = vec![1u8 << 5];
        r.extend_from_slice(&1u64.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes());
        r.extend_from_slice(&(pp as u32).to_le_bytes());
        r.extend(le(&[0.0, 1.0]));
        r.extend_from_slice(&1u64.to_le_bytes());
        r.extend_from_slice(&1u32.to_le_bytes());
        r.extend_from_slice(&8i64.to_le_bytes());
        r.extend_from_slice(&rep(ty::INT, codec::IS_INLINED, 7).to_le_bytes());
        fields.push(("references", rep(ty::REFERENCE_LIST_OP, 0, w.raw(&r))));
        w.spec(wp, SPEC_PRIM, fields);

        // translate.timeSamples = {0: (1, 0, 0), 10: (5, 0, 0)}, out-of-line double3 values.
        let at = w.offset();
        let mut t = 32i64.to_le_bytes().to_vec();
        t.extend_from_slice(&2u64.to_le_bytes());
        t.extend(le(&[0.0, 10.0]));
        t.extend_from_slice(&rep(ty::DOUBLE_VECTOR, 0, at + 8).to_le_bytes());
        let values_at = at + 40 + 8;
        t.extend_from_slice(&56i64.to_le_bytes());
        t.extend(le(&[1.0, 0.0, 0.0, 5.0, 0.0, 0.0]));
        t.extend_from_slice(&2u64.to_le_bytes());
        t.extend_from_slice(&rep(23, 0, values_at).to_le_bytes());
        t.extend_from_slice(&rep(23, 0, values_at + 24).to_le_bytes());
        let samples = rep(ty::TIME_SAMPLES, 0, w.raw(&t));
        let double3 = rep(ty::TOKEN, codec::IS_INLINED, u64::from(w.token("double3")));
        w.spec(ops[0], SPEC_ATTRIBUTE, vec![("typeName", double3), ("timeSamples", samples)]);
        // transform: diag(2, 2, 2, 1), inlined.
        w.spec(ops[1], SPEC_ATTRIBUTE, vec![("default", rep(ty::MATRIX4D, codec::IS_INLINED, 0x0102_0202))]);
        let order: Vec<u32> = ["xformOp:translate", "xformOp:transform"].iter().map(|n| w.token(n)).collect();
        let mut o = 2u64.to_le_bytes().to_vec();
        o.extend(order.iter().flat_map(|t| t.to_le_bytes()));
        let order = rep(ty::TOKEN, codec::IS_ARRAY, w.raw(&o));
        w.spec(ops[2], SPEC_ATTRIBUTE, vec![("default", order)]);

        // /P: class Xform holding def Mesh M.
        let mut fields = def("Xform", &mut w);
        fields[0].1 = rep(ty::SPECIFIER, codec::IS_INLINED, 2);
        w.spec(pp, SPEC_PRIM, fields);
        let fields = def("Mesh", &mut w);
        w.spec(mp, SPEC_PRIM, fields);
        let points = w.value(&UsdValue::Vec3fs(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])).unwrap();
        let counts = w.value(&UsdValue::Ints(vec![3])).unwrap();
        let indices = w.value(&UsdValue::Ints(vec![0, 1, 2])).unwrap();
        for (path, value) in attrs.into_iter().zip([points, counts, indices]) {
            w.spec(path, SPEC_ATTRIBUTE, vec![("default", value)]);
        }
        let bytes = w.finish().unwrap();

        let layer = read_layer(&bytes).unwrap();
        let w_prim = layer.prims.iter().find(|p| p.path == "/W").unwrap();
        assert_eq!(w_prim.references, vec!["/P"]);
        assert_eq!(w_prim.props.get("xformOp:translate").and_then(|p| p.value.as_ref()), Some(&Val::Num(vec![1.0, 0.0, 0.0])));
        assert_eq!(
            w_prim.props.get("xformOp:transform").and_then(|p| p.value.as_ref()),
            Some(&Val::Num(vec![2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0]))
        );
        let p_prim = layer.prims.iter().find(|p| p.path == "/P").unwrap();
        assert_eq!(p_prim.specifier, Specifier::Class);
    }
}
