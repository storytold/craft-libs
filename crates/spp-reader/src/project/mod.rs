//! One opened project file: [`ProjectFile`] and the listings it offers.

mod cache;
mod error;
mod file;
mod layout;
pub mod paths;
mod resources;

pub use cache::CacheEntryRef;
pub use error::ProjectError;
pub use file::ProjectFile;
pub use layout::{DatasetRef, Layout};
pub use resources::{ResourceKind, ResourceRef};
