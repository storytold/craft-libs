//! Built-in words of the presets family.

craft_ui::words! {
    /// The words of the presets family: the browser, its footer, context menu, panel menu and the file entries. Each
    /// id is `<type>.<name>`. `{n}` stands for a number, `{name}` for a preset's or group's name, `{file}` for
    /// a file name and `{reason}` for one of the `reason_*` words.
    pub struct PresetWords {
        grid_view: "presetBrowser.gridView" = "Grid View";
        list_view: "presetBrowser.listView" = "List View";
        thumb_size: "presetBrowser.thumbSize" = "Thumbnail size";
        footer: "presetFooter.name" = "Preset actions";
        view: "presetBrowser.view" = "View";
        menu: "presetBrowser.menu" = "Preset menu";
        search: "presetBrowser.search" = "Search";
        clear: "presetBrowser.clear" = "Clear";
        name: "namedPresetRow.name" = "Name";
        rename: "presetBrowser.rename" = "Rename…";
        rename_group: "presetBrowser.renameGroup" = "Rename Group…";
        delete: "presetBrowser.delete" = "Delete";
        delete_group: "presetBrowser.deleteGroup" = "Delete Group";
        move_to: "presetBrowser.moveTo" = "Move to";
        new_preset: "presetFooter.new" = "New";
        new_group: "presetFooter.newGroup" = "New Group";
        delete_button: "presetFooter.delete" = "Delete";
        count: count "presetFooter.count" = "{n} preset" | "{n} presets";
        empty: "presetBrowser.empty" = "No presets";
        no_match: "presetBrowser.noMatch" = "No presets match";
        group_count: count "presetBrowser.groupCount" = "{n} preset" | "{n} presets";
        loaded: template "presetFiles.loaded" = "Loaded {file}";
        saved: template "presetFiles.saved" = "Saved {file}";
        read_failed: template "presetFiles.readFailed" = "Could not read {file}: {reason}";
        write_failed: template "presetFiles.writeFailed" = "Could not save {file}: {reason}";
        reason_not_found: "presetFiles.reasonNotFound" = "the file was not found";
        reason_denied: "presetFiles.reasonDenied" = "access was denied";
        reason_unsupported: "presetFiles.reasonUnsupported" = "this is not available here";
        reason_other: "presetFiles.reasonOther" = "the file system reported an error";

        custom: "presetPicker.custom" = "Custom";
        mixed: "presetPicker.mixed" = "Mixed";
        built_in: template "namedPresetRow.builtIn" = "{name} (built-in)";
        name_taken: "namedPresetRow.nameTaken" = "A preset with this name already exists.";
        name_invalid: "namedPresetRow.nameInvalid" = "This name can't be used.";
        more: "namedPresetRow.more" = "Manage presets";
    }
}
