//! Built-in words of the preferences family.

craft_ui::words! {
    /// The words of the preferences window: its title, search, page list and buttons that are not shared words. OK and
    /// Cancel are `CommonWords`.
    pub struct PreferencesWords {
        title: "preferences.title" = "Preferences";
        search_label: "preferences.searchLabel" = "Search settings";
        search_placeholder: "preferences.searchPlaceholder" = "Search";
        clear_search: "preferences.clearSearch" = "Clear search";
        pages_label: "preferences.pagesLabel" = "Pages";
        no_matches: "preferences.noMatches" = "No matching settings";
        empty_page: "preferences.emptyPage" = "These settings aren't available yet.";
        apply: "preferences.apply" = "Apply";
        reset_section: "preferences.resetSection" = "Reset Section";
        hex_label: "preferences.hexLabel" = "Hex color";
        default_hint: "preferences.defaultHint" = "Default";
        cannot_ok: "preferences.cannotOk" = "These settings can't be applied yet.";
    }
}

craft_ui::words! {
    /// The words of a settings document: the note under a setting read at launch and the refusal of Apply.
    /// `{error}` is why Apply refused the values.
    pub struct SettingsDocumentWords {
        restart: "settingsDocument.restart" = "Applies at next launch.";
        invalid: template "settingsDocument.invalid" = "These settings can't be applied: {error}";
    }
}
