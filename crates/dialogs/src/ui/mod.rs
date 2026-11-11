//! The file service's UI parts: [`PathField`] and [`FileListPicker`], and the words of the service's native boxes.

mod file_list;
mod path_field;
#[cfg(feature = "screen-testing")]
mod stories;

pub use file_list::{FileList, FileListMode, FileListPicker, FileListPickerOutput, FileListWords};
pub use path_field::{PathField, PathFieldOutput, PathFieldWords};

craft_ui::words! {
    /// The Replace question of a Save the file panel did not ask about. The service shows it as a native message
    /// box, so the app hands these words to the panels with [`ReplacePromptWords::words`] at startup and when the
    /// language changes. `{name}` stands for the file name.
    pub struct ReplacePromptWords {
        title: "fileService.replaceTitle" = "Replace";
        question: template "fileService.replaceQuestion" = "“{name}” already exists. Replace it?";
        replace: "fileService.replace" = "Replace";
        cancel: "fileService.replaceCancel" = "Cancel";
    }
}

craft_ui::words! {
    /// What a feature tells the user when `Picked::Unavailable` says no file dialog can open. `{cause}` stands for the
    /// technical cause the answer carries.
    pub struct UnavailableWords {
        message: template "fileService.unavailable" = "No file dialog can open: {cause}. Install xdg-desktop-portal with a backend (xdg-desktop-portal-gtk, -kde or -gnome) and libdbus, or zenity. Meanwhile, drop a file onto the window or paste a copied image.";
    }
}

impl UnavailableWords {
    /// The sentence for `cause`, the text of a `Picked::Unavailable`.
    pub fn sentence(&self, cause: &str) -> String {
        self.message.fill(&[("cause", cause)])
    }
}

impl ReplacePromptWords {
    /// The words for [`FilePanels::replace_words`](crate::service::FilePanels::replace_words).
    pub fn words(&self) -> crate::service::ReplaceWords {
        crate::service::ReplaceWords {
            title: self.title.as_str().to_owned(),
            question: self.question.as_str().to_owned(),
            replace: self.replace.as_str().to_owned(),
            cancel: self.cancel.as_str().to_owned(),
        }
    }
}
