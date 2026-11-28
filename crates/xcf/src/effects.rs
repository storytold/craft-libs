//! Bounded, inert XCF20+ effect envelopes and ordered typed arguments.
//!
//! No operation is executed, no profile interpreted, and no linked resource opened.
//! Record and argument slices borrow the caller's validated backing bytes. Use one
//! [`Budget`](crate::effects::Budget) across all owners and repeated references: failed work is never refunded.
//! Configuration forms are data, not parsed GEGL graphs or native commands.

mod config;
mod envelope;
mod inventory;
pub use inventory::LayerInventory;
#[cfg(test)]
mod tests;

/// Typed optional-resource refusal, without extending the legacy crate error enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Limit {
    #[error("effect retained storage allowance exhausted")]
    Storage,
    #[error("effect decode work allowance exhausted")]
    Work,
}

/// Essential framing/cancellation use the existing shared error. Optional quota
/// refusal stays distinguishable so consumers cannot misreport it as damaged data.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Source(#[from] crate::Error),
    #[error(transparent)]
    Limit(Limit),
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(message: impl Into<String>) -> Error {
    crate::invalid(message).into()
}

/// Caller-owned per-document admission, separate from destination pixel storage.
/// Successful records keep storage charges. Handled failed reads refund only
/// temporary storage; cancellation aborts without that optional-loss refund.
#[derive(Debug)]
pub struct Budget {
    retained: usize,
    work: envelope::DecodeWork,
}

impl Budget {
    pub fn new(retained_bytes: usize, work_bytes: usize) -> Self {
        Self { retained: retained_bytes, work: envelope::DecodeWork::new(work_bytes) }
    }
    pub fn remaining_storage(&self) -> usize {
        self.retained
    }
    pub fn remaining_work(&self) -> usize {
        self.work.remaining()
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(32 << 20, 128 << 20)
    }
}

/// Source ownership is supplied by the caller's drawable traversal, never guessed
/// from an operation name or a report entry. It has no native-layer identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub kind: OwnerKind,
    pub offset: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OwnerKind {
    Layer,
    Channel,
    Selection,
    LayerMask,
    EffectMask,
}

#[derive(Debug, PartialEq)]
pub struct OwnedEffect<'a> {
    pub owner: Owner,
    pub record: StoredEffect<'a>,
}

/// A view over one backing XCF; no pixels or auxiliary mask hierarchies are read.
#[derive(Clone, Copy)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    version: u32,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8], version: u32) -> Self {
        Self { bytes, version }
    }

    /// Preserve on-disk pointer order and full wire identities, including offsets
    /// which cannot address this host. Decode validity belongs to `read`.
    pub fn pointers(&self, offset: u64, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<Vec<u64>> {
        if !keep_going() {
            return Err(crate::Error::Cancelled.into());
        }
        let start = usize::try_from(offset).map_err(|_| invalid("effect pointer list does not fit in memory"))?;
        let mut cursor = crate::Cursor { b: self.bytes, pos: start, wide: self.version >= 11 };
        let before = budget.retained;
        let result = envelope::read_pointers_with_work(&mut cursor, &mut budget.retained, keep_going, &mut budget.work);
        if result.as_ref().is_err_and(|error| !matches!(error, Error::Source(crate::Error::Cancelled))) {
            budget.retained = before;
        }
        result
    }

    /// Decode one envelope without following its mask or executing its operation.
    /// Unknown arguments/properties retain borrowed bytes and order. Malformed
    /// optional values carry bounded diagnostics; envelope truncation is an error.
    pub fn read(&self, owner: Owner, offset: u64, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<OwnedEffect<'a>> {
        if !keep_going() {
            return Err(crate::Error::Cancelled.into());
        }
        let start = usize::try_from(offset).map_err(|_| invalid("effect offset does not fit in memory"))?;
        let cursor = crate::Cursor { b: self.bytes, pos: 0, wide: self.version >= 11 };
        let before = budget.retained;
        let result = envelope::decode_with_work(&cursor, start, self.version, &mut budget.retained, &mut budget.work, keep_going);
        if result.as_ref().is_err_and(|error| !matches!(error, Error::Source(crate::Error::Cancelled))) {
            budget.retained = before;
        }
        result.map(|record| OwnedEffect { owner, record })
    }
}

/// One parsed effect; file order and argument order remain explicit.
#[derive(Debug, PartialEq)]
pub struct StoredEffect<'a> {
    // Parser-owned provenance prevents mutable public fields from redirecting a
    // later auxiliary mask read to another source or unrelated record bytes.
    pub(crate) source_span: SourceSpan<'a>,
    pub name: &'a str,
    /// None is the null wire form; Some("") is an explicitly empty icon.
    pub icon: Option<&'a str>,
    pub operation: &'a str,
    pub version: Option<&'a str>,
    pub arguments: Vec<Argument<'a>>,
    pub visible: bool,
    pub opacity: f32,
    pub mode: i32,
    pub blend_space: i32,
    pub composite_space: i32,
    pub composite_mode: i32,
    pub clip: bool,
    pub region: u32,
    /// Original source identity, retained even when outside the backing file. Zero means no mask.
    pub mask_offset: u64,
    pub record_offset: usize,
    pub raw: &'a [u8],
    pub unknown_properties: Vec<SourceProperty<'a>>,
    pub errors: Vec<String>,
}

pub(crate) struct SourceSpan<'a> {
    bytes: &'a [u8],
    version: u32,
    start: usize,
    end: usize,
    mask: u64,
}

impl std::fmt::Debug for SourceSpan<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceSpan")
            .field("source_bytes", &self.bytes.len())
            .field("version", &self.version)
            .field("start", &self.start)
            .field("end", &self.end)
            .field("mask", &self.mask)
            .finish()
    }
}
impl PartialEq for SourceSpan<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.start == other.start && self.end == other.end && self.mask == other.mask
    }
}

impl<'a> StoredEffect<'a> {
    pub(crate) fn source_mask(&self, bytes: &[u8], version: u32) -> crate::Result<Option<(u64, std::ops::Range<usize>)>> {
        let span = &self.source_span;
        let raw = span.bytes.get(span.start..span.end).ok_or_else(|| crate::invalid("effect source range is invalid"))?;
        if !std::ptr::eq(span.bytes, bytes)
            || span.version != version
            || self.record_offset != span.start
            || self.mask_offset != span.mask
            || !std::ptr::eq(raw, self.raw)
        {
            return Err(crate::invalid("effect mask does not belong to this source record"));
        }
        Ok((span.mask != 0).then_some((span.mask, span.start..span.end)))
    }
}

#[derive(Debug, PartialEq)]
pub struct SourceProperty<'a> {
    pub offset: usize,
    pub kind: u32,
    pub bytes: &'a [u8],
}

#[derive(Debug, PartialEq)]
pub struct Argument<'a> {
    pub name: &'a str,
    pub value: ArgumentValue<'a>,
}

#[derive(Debug, PartialEq)]
pub enum ArgumentValue<'a> {
    Int(i32),
    UInt(u32),
    Bool(bool),
    Float(f32),
    /// A zero-length wire string; distinct from a one-NUL empty string.
    NullString,
    String(&'a str),
    Enum(i32),
    Config(Config<'a>),
    Color(Option<ColorLiteral<'a>>),
    Unknown {
        kind: u32,
        bytes: &'a [u8],
    },
    Invalid {
        kind: u32,
        bytes: &'a [u8],
        message: String,
    },
}

/// Typed source color bytes and their own profile. No implicit document ICC.
#[derive(Debug, PartialEq)]
pub struct ColorLiteral<'a> {
    pub encoding: &'a str,
    pub pixels: &'a [u8],
    pub profile: &'a [u8],
}

/// A bounded configuration syntax tree. It contains no commands or evaluation API.
#[derive(Debug, PartialEq)]
pub struct Config<'a> {
    pub raw: &'a str,
    pub forms: Vec<ConfigExpr<'a>>,
}

#[derive(Debug, PartialEq)]
pub enum ConfigExpr<'a> {
    Symbol(&'a str),
    String(String),
    /// GimpConfig quotes may carry escaped binary color/profile data. These are
    /// bytes, not source text or executable instructions.
    Bytes(Vec<u8>),
    Number(f64),
    List(Vec<ConfigExpr<'a>>),
}

impl<'a> ConfigExpr<'a> {
    /// Return only a parsed symbol, leaving other configuration value kinds distinguishable.
    pub fn as_symbol(&self) -> Option<&'a str> {
        if let Self::Symbol(value) = self { Some(value) } else { None }
    }
    /// Return only a numeric configuration atom without coercing strings or symbols.
    pub fn as_number(&self) -> Option<f64> {
        if let Self::Number(value) = self { Some(*value) } else { None }
    }
    /// Borrow an ordered configuration list without flattening repeated source groups.
    pub fn as_list(&self) -> Option<&[Self]> {
        if let Self::List(value) = self { Some(value) } else { None }
    }
    /// Read the leading symbol of a configuration list when both shapes are valid.
    pub fn head(&self) -> Option<&'a str> {
        self.as_list()?.first()?.as_symbol()
    }
}

impl<'a> Config<'a> {
    /// Repeated groups remain ordered, including channel/curve/levels groups.
    pub fn fields(&self, name: &str) -> impl Iterator<Item = &[ConfigExpr<'a>]> {
        self.forms.iter().filter_map(move |form| {
            let list = form.as_list()?;
            if list.first()?.as_symbol() == Some(name) { list.get(1..) } else { None }
        })
    }
}
