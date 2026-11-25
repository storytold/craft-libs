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
        /// The access name of the options bar grip.
        move_bar: "appShell.moveBar" = "Move bar";
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
        /// The button at the Recent heading's end that empties the list.
        clear_recent: "startScreen.clearRecent" = "Clear";
        sort_opened: "startScreen.sortOpened" = "Last Opened";
        sort_name: "startScreen.sortName" = "Name";
        sort_created: "startScreen.sortCreated" = "Created";
        pinned: "startScreen.pinned" = "Pinned Folders";
        no_pinned: "startScreen.noPinned" = "Pin the folders you open often.";
        /// The note on a pinned folder the app is still listing.
        looking_for_files: "startScreen.lookingForFiles" = "Looking for files…";
        /// The note on a pinned folder the app cannot list.
        folder_unavailable: "startScreen.folderUnavailable" = "Folder unavailable";
        /// The note on a pinned folder that holds no files.
        folder_empty: "startScreen.folderEmpty" = "No files";
        /// The row under a pinned folder's first files that lists the rest.
        show_more: "startScreen.showMore" = "Show more";
        /// The row under an expanded pinned folder's files that lists only the first again.
        show_less: "startScreen.showLess" = "Show less";
        /// The note on a pinned folder the app kept only the newest files of.
        newest_of: template "startScreen.newestOf" = "The newest {shown} of {total}";
        /// A pinned folder's Unpin button.
        unpin: "startScreen.unpin" = "Unpin";
        /// The accessible name of a pinned folder's Unpin part.
        unpin_folder: template "startScreen.unpinFolder" = "Unpin {name}";

        pin_folder: "startScreen.pinFolder" = "Pin Folder…";
        templates: "startScreen.templates" = "Templates";
        recent_menu: "startScreen.recentMenu" = "Recent file";
        pinned_menu: "startScreen.pinnedMenu" = "Pinned folder";
        /// The accessible name of the start screen's rail of links.
        start_links: "startScreen.links" = "Links";
        /// The title line of the opening card.
        opening: template "openingCard.title" = "Opening {name}…";
        /// The accessible name of a recent row's revealed remove part.
        remove_recent: template "startScreen.removeRecent" = "Remove {name} from Recent";

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
