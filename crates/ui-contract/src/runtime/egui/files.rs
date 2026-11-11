//! The files the OS hovers over or drops on the running viewport this pass.

use crate::runtime::drag::OsFile;
use egui::Context;

/// What the OS reported for the running pass.
pub(crate) struct Seen {
    /// The files held over the window: a drop is under way.
    pub hovered: Vec<OsFile>,
    /// The files dropped on the window this pass. Their media type is empty: egui reports it only while the files hover,
    /// and the drag step carries it over. A file without a path carries its bytes.
    pub dropped: Vec<OsFile>,
}

/// The hovered and dropped files of the running pass.
pub(crate) fn seen(ctx: &Context) -> Seen {
    ctx.input(|input| Seen {
        hovered: input
            .raw
            .hovered_files
            .iter()
            .map(|file| OsFile {
                name: file.path.as_ref().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
                path: file.path.clone(),
                mime: file.mime.clone(),
                bytes: None,
            })
            .collect(),
        dropped: input
            .raw
            .dropped_files
            .iter()
            .map(|file| {
                let path = file.path();
                let usable = !path.as_os_str().is_empty();
                // The web reads a dropped file's bytes only asynchronously, so a pathless file has none here.
                #[cfg(not(target_arch = "wasm32"))]
                let bytes = if usable { None } else { file.bytes().ok().map(Into::into) };
                #[cfg(target_arch = "wasm32")]
                let bytes = None;
                OsFile {
                    name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
                    path: usable.then(|| path.to_path_buf()),
                    mime: String::new(),
                    bytes,
                }
            })
            .collect(),
    })
}
