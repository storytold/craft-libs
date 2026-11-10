use crate::container::bytes::OutOfBounds;
use std::fmt;

#[derive(Debug)]
pub enum Hdf5Error {
    Io(std::io::Error),
    /// The file does not start with the HDF5 signature.
    NotHdf5,
    /// Valid HDF5, but uses a feature this reader does not implement (e.g. a filter or a newer object header).
    Unsupported(String),
    /// A structure is inconsistent (bad signature, out-of-range address, …).
    Corrupt(String),
    /// No object at this path.
    NotFound(String),
    /// The path names a group, not a dataset.
    NotADataset(String),
}

impl fmt::Display for Hdf5Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hdf5Error::Io(e) => write!(f, "I/O error: {e}"),
            Hdf5Error::NotHdf5 => write!(f, "not an HDF5 file"),
            Hdf5Error::Unsupported(what) => write!(f, "unsupported HDF5 feature: {what}"),
            Hdf5Error::Corrupt(what) => write!(f, "corrupt HDF5 file: {what}"),
            Hdf5Error::NotFound(path) => write!(f, "no HDF5 object at '{path}'"),
            Hdf5Error::NotADataset(path) => write!(f, "'{path}' is a group, not a dataset"),
        }
    }
}

impl std::error::Error for Hdf5Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Hdf5Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Hdf5Error {
    fn from(e: std::io::Error) -> Self {
        Hdf5Error::Io(e)
    }
}

impl From<OutOfBounds> for Hdf5Error {
    fn from(e: OutOfBounds) -> Self {
        Hdf5Error::Corrupt(format!("structure is truncated: {e}"))
    }
}
