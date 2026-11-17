/// Why a font file can't be read or a shape source can't be compiled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShxError {
    /// Not an SHX file (unknown signature).
    NotShx,
    /// The file ends inside its header or index.
    Truncated,
    /// The file is larger, or has more of something, than CADCraft reads.
    TooLarge,
    /// The file is well-formed but holds no shape.
    Empty,
    /// Shape source error: line and message.
    Source(usize, String),
}

impl std::fmt::Display for ShxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShxError::NotShx => f.write_str("not an SHX shape or font file"),
            ShxError::Truncated => f.write_str("the SHX file is truncated"),
            ShxError::TooLarge => f.write_str("the SHX file is too large"),
            ShxError::Empty => f.write_str("the SHX file has no shapes"),
            ShxError::Source(line, msg) => write!(f, "line {line}: {msg}"),
        }
    }
}

impl std::error::Error for ShxError {}
