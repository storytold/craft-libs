//! Built-in words of the dock family.

craft_ui::words! {
    /// The words of New Workspace and Delete Workspace beside the common OK and Cancel.
    pub struct WorkspaceDialogWords {
        new_title: "newWorkspaceDialog.title" = "New Workspace";
        name: "newWorkspaceDialog.name" = "Name";
        built_in: "newWorkspaceDialog.builtIn" = "A built-in workspace has this name.";
        replaces: "newWorkspaceDialog.replaces" = "A saved workspace has this name. OK replaces it.";
        capture: "newWorkspaceDialog.capture" = "Capture";
        capture_note: "newWorkspaceDialog.captureNote" = "Panel locations are always saved in the workspace.";
        shortcuts: "newWorkspaceDialog.shortcuts" = "Keyboard Shortcuts";
        menus: "newWorkspaceDialog.menus" = "Menus";
        toolbar: "newWorkspaceDialog.toolbar" = "Toolbar";
        delete_title: "deleteWorkspaceDialog.title" = "Delete Workspace";
        workspace: "deleteWorkspaceDialog.workspace" = "Workspace";
        current: "deleteWorkspaceDialog.current" = "The current workspace can't be deleted.";
    }
}
