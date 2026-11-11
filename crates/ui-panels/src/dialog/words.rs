//! Built-in words of the dialog family.

craft_ui::words! {
    /// The words of the dialog family: the standard prompts, the unsaved-changes question and the recovery dialog.
    /// Cancel, OK and "Don't show again" are the common words. The unsaved-changes question asks Yes, No and Cancel
    /// on Windows and Linux and Save, Don't Save and Cancel on macOS.
    pub struct DialogWords {
        confirm_delete_title: "prompt.confirmDeleteTitle" = "Delete";
        confirm_delete_one: template "prompt.confirmDeleteOne" = "Delete {name}?";
        confirm_delete_many: template "prompt.confirmDeleteMany" = "Delete {things}? {names}";
        items: count "prompt.items" = "{n} item" | "{n} items";
        delete: "prompt.delete" = "Delete";
        damaged_title: "prompt.damagedTitle" = "Damaged file";
        damaged_message: template "prompt.damagedMessage" = "“{name}” has damaged data. Open it anyway?";
        open_anyway: "prompt.openAnyway" = "Open Anyway";
        changed_on_disk_title: "prompt.changedOnDiskTitle" = "File changed";
        changed_on_disk_message: template "prompt.changedOnDiskMessage" = "“{name}” was changed by another app after it was opened or saved here.";
        changed_on_disk_detail: "prompt.changedOnDiskDetail" = "Replace it with this document, or cancel and use Save As to keep both.";
        replace: "prompt.replace" = "Replace";
        replace_file_title: "prompt.replaceFileTitle" = "Replace file";
        replace_file_message: template "prompt.replaceFileMessage" = "“{name}” already exists. Replace it?";
        replace_file_detail: template "prompt.replaceFileDetail" = "The name in the save dialog was “{answered}”.";
        trash_mask_title: "prompt.trashMaskTitle" = "Delete mask";
        trash_mask_message: "prompt.trashMaskMessage" = "Apply the mask to the layer before removing it?";
        trash_vector_mask_title: "prompt.trashVectorMaskTitle" = "Delete vector mask";
        trash_vector_mask_message: "prompt.trashVectorMaskMessage" = "Delete the vector mask?";
        /// The title of a custom prompt the app gave none.
        custom_title: "prompt.customTitle" = "Confirm";
        apply: "prompt.apply" = "Apply";
        discard: "prompt.discard" = "Delete";
        unsaved_title: "unsaved.title" = "Unsaved changes";
        unsaved_closing: template "unsaved.closing" = "Do you want to save the changes you made to “{name}” before closing?";
        unsaved_quitting: template "unsaved.quitting" = "Do you want to save the changes you made to “{name}” before quitting?";
        unsaved_reverting: template "unsaved.reverting" = "Revert “{name}” to the last saved version? Your changes will be lost.";
        unsaved_save: "unsaved.save" = "Save";
        unsaved_dont_save: "unsaved.dontSave" = "Don't Save";
        unsaved_yes: "unsaved.yes" = "Yes";
        unsaved_no: "unsaved.no" = "No";
        unsaved_revert: "unsaved.revert" = "Revert";
        unsaved_apply_to_all: "unsaved.applyToAll" = "Apply to All";
        recovery_title: "recovery.title" = "Recover Unsaved Work";
        /// `{name}` is the document and `{age}` when it was autosaved ("5 minutes ago").
        recovery_message: template "recovery.message" = "“{name}” has unsaved changes from an earlier run, autosaved {age}.";
        recovery_recover: "recovery.recover" = "Recover";
        recovery_discard: "recovery.discard" = "Discard";
        // full-window parts
        full_window_title_zoom: template "fullWindowTitle.zoomed" = "{name} ({subject}, {zoom})";
        full_window_title_subject: template "fullWindowTitle.subject" = "{name} ({subject})";
        full_window_title_zoom_only: template "fullWindowTitle.zoomOnly" = "{name} ({zoom})";
        full_window_canvas: "fullWindowCanvas.name" = "Image";
        full_window_zoom: "fullWindowZoomBar.zoom" = "Zoom";
        full_window_fit_in_view: "fullWindowZoomBar.fitInView" = "Fit in View";
        full_window_updating: "fullWindowStatus.updating" = "Updating preview";
        full_window_failed: template "fullWindowStatus.failed" = "Preview failed: {error}";
        full_window_lost: "fullWindowStatus.lost" = "Preview unavailable";
        // end of full-window parts
    }
}
