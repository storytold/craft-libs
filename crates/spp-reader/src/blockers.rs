//! [`Blocker`]: one thing in a project file that the app can't evaluate yet, with a stable key.
//!
//! Every part of a layer that isn't computable names its blocker, so an import can say exactly what keeps each
//! layer locked, a corpus can be ranked by what blocks the most (the importer's blocker census), and a locked layer knows which
//! keys would unlock it once support for them is added (an importer's own support catalog decides which can be). The key is stable across imports and
//! projects; the text is for people and keeps the role it was met in (a mask generator, a procedural fill …).
//!
//! | Kind | Key | What blocks |
//! |---|---|---|
//! | [`BlockerKind::Graph`] | `graph:<graph>` | a compiled graph (generator, procedural fill or texture, filter) |
//! | [`BlockerKind::BrushAlpha`] | `brush-alpha:<name>` | strokes painted with a brush alpha we don't have |
//! | [`BlockerKind::Brush`] | `brush:<what>` | strokes our brush can't paint: particles, 3D paths, stencils, projection … |
//! | [`BlockerKind::Strokes`] | `strokes:<what>` | strokes that were not replayed (stroke replay is off) |
//! | [`BlockerKind::Feature`] | `feature:<id>` | a project feature the import doesn't model yet |
//! | [`BlockerKind::Class`] | `class:<class>` | a layer, effect or source class the reader doesn't know |

use std::fmt;

/// What kind of thing blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockerKind {
    Graph,
    BrushAlpha,
    Brush,
    Strokes,
    Feature,
    Class,
}

impl BlockerKind {
    pub const ALL: [BlockerKind; 6] =
        [BlockerKind::Graph, BlockerKind::BrushAlpha, BlockerKind::Brush, BlockerKind::Strokes, BlockerKind::Feature, BlockerKind::Class];

    /// The key prefix.
    pub fn id(self) -> &'static str {
        match self {
            BlockerKind::Graph => "graph",
            BlockerKind::BrushAlpha => "brush-alpha",
            BlockerKind::Brush => "brush",
            BlockerKind::Strokes => "strokes",
            BlockerKind::Feature => "feature",
            BlockerKind::Class => "class",
        }
    }

    pub fn from_id(id: &str) -> Option<BlockerKind> {
        BlockerKind::ALL.into_iter().find(|k| k.id() == id)
    }

    /// What people call it, plural, for group headings.
    pub fn label(self) -> &'static str {
        match self {
            BlockerKind::Graph => "Compiled graphs",
            BlockerKind::BrushAlpha => "Brush alphas",
            BlockerKind::Brush => "Brushes",
            BlockerKind::Strokes => "Strokes not replayed",
            BlockerKind::Feature => "Project features",
            BlockerKind::Class => "Unknown classes",
        }
    }
}

/// The stable identity of a blocker: its kind and a name, written `kind:name`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockerKey {
    pub kind: BlockerKind,
    pub name: String,
}

impl BlockerKey {
    pub fn new(kind: BlockerKind, name: impl Into<String>) -> BlockerKey {
        BlockerKey { kind, name: name.into() }
    }

    /// Parses `kind:name` (the name may contain colons).
    pub fn parse(key: &str) -> Option<BlockerKey> {
        let (kind, name) = key.split_once(':')?;
        Some(BlockerKey::new(BlockerKind::from_id(kind)?, name))
    }
}

impl fmt::Display for BlockerKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.kind.id(), self.name)
    }
}

/// One blocker met in a project: its key and how it was met.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Blocker {
    pub key: BlockerKey,
    /// For people, e.g. `mask generator "mg_dirt"`.
    pub text: String,
}

impl Blocker {
    pub fn new(key: BlockerKey, text: impl Into<String>) -> Blocker {
        Blocker { key, text: text.into() }
    }

    /// A compiled graph (`graph` without the `graph:` prefix), met as `text`.
    pub fn graph(graph: &str, text: impl Into<String>) -> Blocker {
        Blocker::new(BlockerKey::new(BlockerKind::Graph, graph), text)
    }

    /// A project feature not modelled yet (`id` is kebab-case), met as `text`.
    pub fn feature(id: &str, text: impl Into<String>) -> Blocker {
        Blocker::new(BlockerKey::new(BlockerKind::Feature, id), text)
    }

    /// A project class the reader doesn't know, met as `text`.
    pub fn class(class: &str, text: impl Into<String>) -> Blocker {
        Blocker::new(BlockerKey::new(BlockerKind::Class, class), text)
    }

    /// The same blocker met through something else: `context` is put in front of the text, the key stays.
    pub fn through(&self, context: &str) -> Blocker {
        Blocker::new(self.key.clone(), format!("{context}: {}", self.text))
    }
}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_keep_colons_in_names() {
        for kind in BlockerKind::ALL {
            let key = BlockerKey::new(kind, "a:b");
            assert_eq!(BlockerKey::parse(&key.to_string()), Some(key));
        }
        assert_eq!(BlockerKey::parse("graph:mg_dirt").unwrap().name, "mg_dirt");
        assert_eq!(BlockerKey::parse("nonsense:x"), None);
        assert_eq!(BlockerKey::parse("no colon"), None);
    }

    #[test]
    fn a_blocker_met_through_something_else_keeps_its_key() {
        let b = Blocker::graph("mg_dirt", "mask generator \"mg_dirt\"");
        let t = b.through("fill from an anchor");
        assert_eq!(t.key, b.key);
        assert_eq!(t.text, "fill from an anchor: mask generator \"mg_dirt\"");
    }
}
