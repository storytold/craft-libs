//! Built-in words of the docking family.

craft_ui::words! {
    /// The words of the dock's handles and chrome.
    pub struct DockWords {
        resize: "splitter.resize" = "Resize panels";
        /// The name of the dock area for assistive technology.
        panels: "dockRoot.panels" = "Panels";
        resize_window: "floatingGroup.resize" = "Resize panel window";
        /// The name of a group's drag bar and top border. `{name}` stands for the label of the panel the group shows.
        move_group: template "dockGroup.move" = "Move tab group";
        /// The name of a column bar. `{name}` stands for the labels of the panels the column's groups show, joined
        /// by ", ".
        move_column: template "dockColumn.move" = "Move panel column";
        collapse_column: "dockColumn.collapse" = "Collapse column to icons";
        expand_column: "dockColumn.expand" = "Expand column";
        move_window: "floatingGroup.move" = "Move panel window";
        close_window: "floatingGroup.close" = "Close panel window";
        /// The tooltip and accessible name of the panel-menu button.
        panel_menu: "dockGroup.panelMenu" = "Panel Menu";

        /// The name of the workspace switcher's select.
        choose_workspace: "workspaceSwitcher.choose" = "Choose a workspace";
        /// The name of the switcher's command menu.
        workspace_menu: "workspaceSwitcher.menu" = "Workspace";

        // PaneArea
        /// The name of a pane's menu.
        pane_menu: "paneArea.menu" = "Pane options";
    }
}
