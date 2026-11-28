//! Part keys and part references: the one name for a part in focus, style, results, sub-nodes and decision records.

/// The key of a part, declared as a const by the component that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PartKey(&'static str);

impl PartKey {
    /// A key named `name`; the name is unique within one component.
    pub const fn new(name: &'static str) -> PartKey {
        PartKey(name)
    }

    /// A cell of a sheet row: `CELL.at(column)`. The item step writes the cell's `SELECTED` and `LEAD` flags into
    /// the parts of the row item that carry it.
    pub const CELL: PartKey = PartKey::new("cell");

    /// The name the key was declared with.
    pub const fn name(self) -> &'static str {
        self.0
    }

    /// One of a repeated part: `POINT.at(id)`. `i` is a stable sub-key, an entity id where the repeats
    /// are data and the position only for fixed repeats.
    pub const fn at(self, i: u64) -> PartRef {
        PartRef { key: self, index: Some(i) }
    }
}

/// A part's key with an optional index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PartRef {
    /// The part's key.
    pub key: PartKey,
    /// The repeat the reference names, `None` for a part that occurs once.
    pub index: Option<u64>,
}

impl From<PartKey> for PartRef {
    fn from(key: PartKey) -> Self {
        PartRef { key, index: None }
    }
}

impl From<(PartKey, u64)> for PartRef {
    fn from((key, i): (PartKey, u64)) -> Self {
        PartRef { key, index: Some(i) }
    }
}
