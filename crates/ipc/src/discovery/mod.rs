//! Discovery records for Craft apps on this machine.

mod dir;
mod record;

pub use dir::RuntimeDir;
pub use record::{Discovery, DocInfo, Endpoint, InstanceRecord, Protocol, valid_app_name};

pub(crate) use dir::ensure_private_dir;
#[cfg(unix)]
pub(crate) use dir::{current_euid, enforce_mode};
