//! A typed model of the Figma REST API file JSON: the response of `GET /v1/files/:key` (a
//! `document` with `CANVAS` pages, plus `components`, `componentSets` and `styles` metadata) and
//! of `GET /v1/files/:key/nodes` (a `nodes` map of such entries). Written from the published REST
//! API reference only.
//!
//! This is the neutral model the `.fig` and `.sketch` readers map onto, and the shape an
//! application's importer takes. It is the file's own shape, named as the API names it; it knows
//! nothing of any application's document.
//!
//! # Nothing is lost
//!
//! Every record keeps the fields this model has no name for in `extra`, in the JSON they came
//! in, and writes them back. So a file read and written keeps properties the API added after this
//! model was written, and the documented extensions ([`ext`]). Enumerations keep values they don't
//! know (`Other`). Two things do change on a round trip: a field that was `null` is left out, and
//! a whole number written as `10` comes back as `10.0`.
//!
//! # Hostile input
//!
//! [`File::from_value`] and [`File::from_slice`] refuse a tree nested deeper than [`MAX_DEPTH`]
//! or holding more than the caller's node limit before any typed value is built. The layer tree
//! is converted level by level, without recursion, in both directions; only the JSON tree itself
//! (text to [`serde_json::Value`], and dropping one) is walked by recursion, in small frames. A field of the
//! wrong type fails the read (the error names it): a reader that must skip bad fields one by one
//! reads the [`serde_json::Value`] instead. Nothing panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod build;
pub mod ext;
mod model;
#[cfg(test)]
mod tests;

pub use build::{MAX_WARNINGS, Warnings, style_name};
pub use model::{
    ArcData, BlendMode, Color, ColorStop, Component, ComponentSet, Effect, EffectType, ExportSetting, File, FileNode, FileNodes, LayoutConstraint,
    LayoutGrid, Node, NodeProps, NodeType, Paint, PaintType, Path, Rectangle, Style, StyleType, Transform, TypeStyle, Vector,
};

use serde_json::Value;

/// Deepest JSON nesting read: about 500 layers inside one another (each layer is two levels: its
/// `children` array and itself).
pub const MAX_DEPTH: usize = 1024;
/// Most nodes [`File::from_slice`] reads.
pub const DEFAULT_MAX_NODES: usize = 200_000;

/// Why a file JSON could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Not JSON, or a field of the wrong type.
    #[error("file JSON: {0}")]
    Json(String),
    /// Nested deeper than [`MAX_DEPTH`].
    #[error("file JSON: nested deeper than {MAX_DEPTH} levels")]
    TooDeep,
    /// More nodes than the limit given.
    #[error("file JSON: more than {0} layers")]
    TooManyNodes(usize),
}

/// Checks `v` against [`MAX_DEPTH`] and `max_nodes` (objects with a `children` array or an `id`
/// and a `type` count as nodes) without recursion.
pub fn check(v: &Value, max_nodes: usize) -> Result<(), Error> {
    let mut nodes = 0usize;
    let mut pending = vec![(v, 0usize)];
    while let Some((value, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        match value {
            Value::Object(fields) => {
                if fields.contains_key("children") || (fields.contains_key("id") && fields.contains_key("type")) {
                    nodes += 1;
                    if nodes > max_nodes {
                        return Err(Error::TooManyNodes(max_nodes));
                    }
                }
                pending.extend(fields.values().map(|f| (f, depth + 1)));
            }
            Value::Array(items) => pending.extend(items.iter().map(|i| (i, depth + 1))),
            _ => {}
        }
    }
    Ok(())
}

fn json(e: serde_json::Error) -> Error {
    Error::Json(e.to_string())
}

/// Takes `document` out of the JSON of a file or of a `nodes` entry and builds it level by level,
/// so that serde converts only the shallow rest.
fn take_document(v: &mut Value) -> Result<Option<Node>, Error> {
    match v.as_object_mut().and_then(|o| o.remove("document")) {
        None | Some(Value::Null) => Ok(None),
        Some(doc) => Node::build(doc).map(Some).map_err(json),
    }
}

/// Puts `document` into the JSON of a file or of a `nodes` entry that was written without it.
fn put_document(v: &mut Value, document: Option<&Node>) -> Result<(), Error> {
    if let (Some(doc), Value::Object(o)) = (document, v) {
        o.insert("document".into(), doc.unbuild().map_err(json)?);
    }
    Ok(())
}

impl File {
    /// A file from its JSON text, within [`MAX_DEPTH`] and [`DEFAULT_MAX_NODES`]. (serde_json
    /// itself refuses text nested past 128 levels; [`File::from_value`] takes deeper trees.)
    pub fn from_slice(bytes: &[u8]) -> Result<File, Error> {
        File::from_value(serde_json::from_slice(bytes).map_err(json)?, DEFAULT_MAX_NODES)
    }

    /// A file from its JSON, within [`MAX_DEPTH`] and `max_nodes`.
    pub fn from_value(mut v: Value, max_nodes: usize) -> Result<File, Error> {
        check(&v, max_nodes)?;
        let document = take_document(&mut v)?;
        let mut file: File = serde_json::from_value(v).map_err(json)?;
        file.document = document;
        Ok(file)
    }

    /// The file as JSON. Unlike the file's `Serialize`, this does not recurse through the layers.
    pub fn to_value(&self) -> Result<Value, Error> {
        let shallow = File { document: None, extra: self.extra.clone(), components: self.components.clone(), ..self.metadata() };
        let mut v = serde_json::to_value(shallow).map_err(json)?;
        put_document(&mut v, self.document.as_ref())?;
        Ok(v)
    }

    /// The file's own fields, without the document, the component maps and the unnamed fields.
    fn metadata(&self) -> File {
        File {
            name: self.name.clone(),
            last_modified: self.last_modified.clone(),
            thumbnail_url: self.thumbnail_url.clone(),
            version: self.version.clone(),
            schema_version: self.schema_version,
            component_sets: self.component_sets.clone(),
            styles: self.styles.clone(),
            ..File::default()
        }
    }
}

impl FileNodes {
    /// The response of `GET /v1/files/:key/nodes`, within [`MAX_DEPTH`] and `max_nodes`.
    pub fn from_value(mut v: Value, max_nodes: usize) -> Result<FileNodes, Error> {
        check(&v, max_nodes)?;
        let mut documents = Vec::new();
        if let Some(Value::Object(entries)) = v.get_mut("nodes") {
            for (id, entry) in entries.iter_mut() {
                if let Some(doc) = take_document(entry)? {
                    documents.push((id.clone(), doc));
                }
            }
        }
        let mut read: FileNodes = serde_json::from_value(v).map_err(json)?;
        for (id, doc) in documents {
            if let Some(Some(entry)) = read.nodes.as_mut().and_then(|n| n.get_mut(&id)) {
                entry.document = Some(doc);
            }
        }
        Ok(read)
    }
}

impl Node {
    /// A node (and its subtree) from its JSON, within [`MAX_DEPTH`] and `max_nodes`.
    pub fn from_value(v: Value, max_nodes: usize) -> Result<Node, Error> {
        check(&v, max_nodes)?;
        Node::build(v).map_err(json)
    }

    /// The node as JSON.
    pub fn to_value(&self) -> Result<Value, Error> {
        self.unbuild().map_err(json)
    }

    /// The node and every node under it, parents first, without recursion.
    pub fn walk(&self) -> impl Iterator<Item = &Node> {
        let mut pending = vec![self];
        std::iter::from_fn(move || {
            let node = pending.pop()?;
            pending.extend(node.children.iter().flatten().rev());
            Some(node)
        })
    }
}
