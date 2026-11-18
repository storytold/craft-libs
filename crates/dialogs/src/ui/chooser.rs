//! [`FileChooser`]: the in-app file chooser a pick falls back to when no native dialog can open.

mod accept;

use craft_browse::places::FolderCache;
use craft_host::PickRequest;
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use egui::Ui;

use crate::Answer;

/// The words of the chooser.
#[derive(Clone, Debug, Default)]
pub struct ChooserWords;

/// The fallback file chooser: a path field, a breadcrumb bar over the folder roots, a list of the
/// folder's entries, name and type rows for a Save, and the line that says why the native dialog
/// is unavailable. It reads folders only from the [`FolderCache`] its caller gives it.
pub struct FileChooser {
    request: PickRequest,
    cause: Option<String>,
    folders: FolderCache,
}

impl FileChooser {
    /// A chooser for `request`. `cause` is the text of the `Picked::Unavailable` that opened it;
    /// `folders` is the caller's folder source.
    pub fn new(request: PickRequest, cause: Option<String>, folders: FolderCache) -> Self {
        let _ = (&request, &cause, &folders);
        todo!("sh.K13")
    }
}

impl StackDialog for FileChooser {
    type Result = Answer;

    fn id(&self) -> DialogId {
        dialog_id("fileChooser")
    }

    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<Answer>> {
        let _ = (&mut *ui, stack, &self.request, &self.cause, &self.folders);
        todo!("sh.K13")
    }
}
