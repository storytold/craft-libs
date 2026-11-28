//! Built-in words of the menu family.

craft_ui::words! {
    /// The words of the menu family: the menu bar's, then the dock rows the dock commands do not label.
    pub struct MenuWords {
        bar_more: "menu.barMore" = "…";
        bar_all: "menu.barAll" = "Menu";
        bar_companion: "menu.barCompanion" = "Menu bar";
        bar_search: "menu.barSearch" = "Search menus";
        bar_no_matches: "menu.barNoMatches" = "No matching commands";
        edit: "menu.edit" = "Edit";
        panel_move_to: "menu.panelMoveTo" = "Move Panel To";
        panel_auto_collapse: "menu.panelAutoCollapse" = "Auto-Collapse Iconic Panels";
        panel_minimize: "menu.panelMinimize" = "Minimize";
        panel_expand: "menu.panelExpand" = "Expand";
        panel_collapse_to_icons: "menu.panelCollapseToIcons" = "Collapse to Icons";
        panel_expand_panels: "menu.panelExpandPanels" = "Expand Panels";
        /// The Close row of a menu outside a group's command scope, such as the context menu of a collapsed column's
        /// icon: the same words as the dock's Close and Close Tab Group commands, which only run through a scope.
        panel_close: "menu.panelClose" = "Close";
        panel_close_group: "menu.panelCloseGroup" = "Close Tab Group";
    }
}
