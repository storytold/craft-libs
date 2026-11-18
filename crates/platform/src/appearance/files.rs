//! Motion from the GNOME and KDE settings files, for a desktop whose portal does not answer.

use std::path::Path;

/// Whether the desktop's settings files under `config_home` ask for reduced motion; `None` when they say nothing.
#[allow(dead_code)] // P3(platform-w6): pf.C-O4
pub(crate) fn reduced_motion_from_files(config_home: &Path) -> Option<bool> {
    let _ = config_home;
    todo!("pf.C-O4")
}
