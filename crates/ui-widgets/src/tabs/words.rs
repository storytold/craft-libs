//! Built-in words of the tabs family.

craft_ui::words! {
    /// The words of a tab. `{tab}` stands for the tab's label.
    pub struct TabWords {
        /// The name of a tab's close button.
        close: template "tab.close" = "Close {tab}";
        /// The name of a tab with unsaved changes.
        modified: template "tab.modified" = "{tab}, modified";
    }
}

craft_ui::words! {
    /// The words of a tab strip.
    pub struct StripWords {
        /// The name of the button that opens the menu of tabs.
        more: "tabStrip.more" = "More tabs";
    }
}
