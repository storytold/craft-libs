//! The fonts registered with Windows in the registry: one value per face, holding a file name in
//! the Windows font folder or a full path. Programs that install fonts into their own folders
//! register them with full paths, which only the registry lists.

use std::path::PathBuf;

/// The full paths the registry lists for the machine's and the current user's fonts, sorted and
/// without repeats. Bare file names (files in the Windows font folder) are left out: the scan
/// reads that folder. Empty elsewhere than on Windows.
pub(crate) fn registered_font_files() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        const MAX_REGISTERED: usize = 1 << 16;
        const KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";
        use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
        let mut files: Vec<PathBuf> = [LOCAL_MACHINE, CURRENT_USER]
            .into_iter()
            .filter_map(|root| root.open(KEY).ok())
            .flat_map(|key| key.values().map(|vs| vs.filter_map(|(_, v)| String::try_from(v).ok()).collect::<Vec<_>>()).unwrap_or_default())
            .take(MAX_REGISTERED)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();
        files.sort();
        files.dedup();
        files
    }
    #[cfg(not(windows))]
    Vec::new()
}
