//! Built-in words of the shell family.

craft_ui::words! {
    /// The words of the shell family: the names of the window's regions (`appShell`), the start screen
    /// (`startScreen`), the caption buttons (`captionButton`) and the title bar (`titleBar`). The labels of the recent files' and pinned
    /// folders' commands are the commands'.
    pub struct ShellWords {
        window: "appShell.window" = "Window";
        document: "appShell.document" = "Document";
        document_tabs: "appShell.documentTabs" = "Document Tabs";
        options: "appShell.options" = "Options";
        tools: "appShell.tools" = "Tools";
        status: "appShell.status" = "Status";
        screen_mode: "appShell.screenMode" = "Screen Mode";

        new: "startScreen.new" = "New…";
        open: "startScreen.open" = "Open…";
        recent: "startScreen.recent" = "Recent";
        no_recent: "startScreen.noRecent" = "Files you open show here.";
        view: "startScreen.view" = "View";
        grid_view: "startScreen.gridView" = "Grid";
        list_view: "startScreen.listView" = "List";
        sort: "startScreen.sort" = "Sort";
        sort_opened: "startScreen.sortOpened" = "Last Opened";
        sort_name: "startScreen.sortName" = "Name";
        sort_created: "startScreen.sortCreated" = "Created";
        pinned: "startScreen.pinned" = "Pinned Folders";
        no_pinned: "startScreen.noPinned" = "Pin the folders you open often.";
        pin_folder: "startScreen.pinFolder" = "Pin Folder…";
        templates: "startScreen.templates" = "Templates";
        recent_menu: "startScreen.recentMenu" = "Recent file";
        pinned_menu: "startScreen.pinnedMenu" = "Pinned folder";

        /// The minimize button.
        minimize: "captionButton.minimize" = "Minimize";
        /// The maximize button.
        maximize: "captionButton.maximize" = "Maximize";
        /// The maximize button while the window is maximized.
        restore: "captionButton.restore" = "Restore";
        /// The close button.
        close: "captionButton.close" = "Close";

        /// The name of the title bar.
        title_bar: "titleBar.name" = "Title Bar";
        /// The description of a title that shows the mark of unsaved changes.
        unsaved: "titleBar.unsaved" = "Unsaved changes";
        /// A document title with the mark of unsaved changes.
        unsaved_title: template "titleBar.unsavedTitle" = "{title} *";
    }
}
