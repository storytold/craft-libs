//! `ActionsWords`: every user-visible word of the Actions panel (`actions.*`).

craft_ui::words! {
    /// The Actions panel's words, English by default (Photoshop's and PhotoCraft's). Patterns with
    /// `{row}`, `{name}`, `{shortcut}`, `{enabled}` and `{steps}` are whole phrases, so a language puts
    /// the value where its grammar wants it. Step labels are the host's
    /// ([`ActionsHost::step_label`](super::ActionsHost::step_label)).
    #[non_exhaustive]
    pub struct ActionsWords {
        /// The tree's accessible name.
        name: "actions.name" = "Actions";
        /// The empty list's text.
        empty: "actions.empty" = "Record a sequence of edits, then play it on any document.";
        /// A recording action's accessible name.
        recording_suffix: template "actions.recordingSuffix" = "{name}, recording";
        /// The recording dot's name.
        recording: "actions.recording" = "Recording";
        /// The include cell's accessible name: `{row}` is the row's label.
        include: "actions.include" = "Include in playback: {row}";
        include_tooltip: "actions.includeTooltip" = "Include in playback";
        /// The dialog cell's accessible name: `{row}` is the row's label.
        dialog: "actions.dialog" = "Show dialog during playback: {row}";
        dialog_tooltip: "actions.dialogTooltip" = "Show dialog during playback";
        /// An action row's trailing text with some steps off: the counted steps and how many are on.
        partial_steps: template "actions.partialSteps" = "{enabled}/{steps}";
        /// An action row's trailing text with a shortcut.
        shortcut_steps: template "actions.shortcutSteps" = "{shortcut} · {steps}";
        /// "{n} step" / "{n} steps": an action row's step count.
        steps: count "actions.steps" = "{n} step" | "{n} steps";
        /// "{n} action" / "{n} actions": a set header's count.
        action_count: count "actions.actions" = "{n} action" | "{n} actions";
        stop: "actions.stop" = "Stop playing/recording";
        record: "actions.record" = "Begin recording";
        play: "actions.play" = "Play selection";
        new_set: "actions.newSet" = "Create new set";
        new_action: "actions.newAction" = "Create new action";
        delete: "actions.delete" = "Delete";
        button_mode: "actions.buttonMode" = "Button Mode";
        /// The panel menu's submenu-free title of the row menu.
        row_menu: "actions.rowMenu" = "Action";
        save_set: "actions.saveSet" = "Save Set…";
        menu_new_action: "actions.menuNewAction" = "New Action…";
        menu_new_set: "actions.menuNewSet" = "New Set…";
        duplicate_set: "actions.duplicateSet" = "Duplicate Set";
        delete_set: "actions.deleteSet" = "Delete Set";
        set_options: "actions.setOptions" = "Set Options…";
        menu_play: "actions.menuPlay" = "Play";
        menu_record: "actions.menuRecord" = "Record";
        duplicate: "actions.duplicate" = "Duplicate";
        action_options: "actions.actionOptions" = "Action Options…";
        batch: "actions.batch" = "Batch…";
        keyboard_shortcuts: "actions.keyboardShortcuts" = "Keyboard Shortcuts";
        /// The shortcut menu's "no shortcut" row.
        none: "actions.none" = "None";
        shift: "actions.shift" = "Shift";
        /// The modifier check row on macOS.
        command: "actions.command" = "Command";
        /// The modifier check row elsewhere.
        ctrl: "actions.ctrl" = "Ctrl";
        color: "actions.color" = "Color";
        /// The title of the Clear All Actions question.
        clear_all_title: "actions.clearAllTitle" = "Clear All Actions";
        clear_all_question: "actions.clearAllQuestion" = "Clear all actions?";
        /// The title of the Reset Actions question.
        reset_title: "actions.resetTitle" = "Reset Actions";
        reset_question: "actions.resetQuestion" = "Replace the current actions?";
        /// Reset's answer that keeps the current actions and adds the starting sets.
        append: "actions.append" = "Append";
        /// The title of the Replace Actions question.
        replace_title: "actions.replaceTitle" = "Replace Actions";
        /// `{count}` is the number of actions in the file, `{file}` its name.
        replace_question: template "actions.replaceQuestion" = "Replace all actions with {count} actions from {file}?";
    }
}

craft_ui::words! {
    /// The Modern Actions editor's words; command and settings labels come from its host.
    pub struct ActionEditorWords {
        name: "actionEditor.name" = "Name";
        description: "actionEditor.description" = "Description";
        shortcut: "actionEditor.shortcut" = "Keyboard shortcut";
        icon: "actionEditor.icon" = "Icon";
        default_icon: "actionEditor.defaultIcon" = "Default icon";
        color: "actionEditor.color" = "Color";
        steps: "actionEditor.steps" = "Steps";
        include: "actionEditor.include" = "Include in playback: {row}";
        dialog: "actionEditor.dialog" = "Show dialog during playback: {row}";
        duplicate_step: "actionEditor.duplicateStep" = "Duplicate step";
        delete_step: "actionEditor.deleteStep" = "Delete step";
        preview: "actionEditor.preview" = "Play action";
        preview_from: "actionEditor.previewFrom" = "Play from selected step";
        capture: "actionEditor.capture" = "Record";
        stop_capture: "actionEditor.stopCapture" = "Stop recording";
        edit_step: "actionEditor.editStep" = "Edit step settings";
        insert_stop: "actionEditor.insertStop" = "Insert Stop";
        save: "actionEditor.save" = "Save";
        discard: "actionEditor.discard" = "Discard";
        leave: "actionEditor.leave" = "Back to Actions";
        none: "actionEditor.none" = "None";
        red: "actionEditor.red" = "Red";
        orange: "actionEditor.orange" = "Orange";
        yellow: "actionEditor.yellow" = "Yellow";
        green: "actionEditor.green" = "Green";
        teal: "actionEditor.teal" = "Teal";
        light_blue: "actionEditor.lightBlue" = "Light blue";
        blue: "actionEditor.blue" = "Blue";
        magenta: "actionEditor.magenta" = "Magenta";
        purple: "actionEditor.purple" = "Purple";
        lavender: "actionEditor.lavender" = "Lavender";
        gray: "actionEditor.gray" = "Gray";
        icon_click: "actionEditor.iconClick" = "Click";
        icon_crop: "actionEditor.iconCrop" = "Crop";
        icon_merge: "actionEditor.iconMerge" = "Merge layers";
        icon_adjustment: "actionEditor.iconAdjustment" = "Adjustment";
        icon_hue: "actionEditor.iconHue" = "Hue and saturation";
        icon_exposure: "actionEditor.iconExposure" = "Exposure";
        icon_lookup: "actionEditor.iconLookup" = "Color lookup";
        icon_monochrome: "actionEditor.iconMonochrome" = "Black and white";
        icon_remove: "actionEditor.iconRemove" = "Remove";
        icon_dodge: "actionEditor.iconDodge" = "Dodge";
        icon_burn: "actionEditor.iconBurn" = "Burn";
        icon_erase: "actionEditor.iconErase" = "Erase";
        icon_effects: "actionEditor.iconEffects" = "Layer effects";
        icon_generated_effects: "actionEditor.iconGeneratedEffects" = "Generated effects";
        icon_sharpen: "actionEditor.iconSharpen" = "Sharpen";
        icon_object_selection: "actionEditor.iconObjectSelection" = "Object selection";
        icon_wand: "actionEditor.iconWand" = "Magic wand";
        icon_generate: "actionEditor.iconGenerate" = "Generate";
        icon_text_color: "actionEditor.iconTextColor" = "Text color";
        icon_text: "actionEditor.iconText" = "Text";
        icon_download: "actionEditor.iconDownload" = "Download";
        icon_layers: "actionEditor.iconLayers" = "Layers";
        icon_brush: "actionEditor.iconBrush" = "Brush";
        icon_subject: "actionEditor.iconSubject" = "Select subject";
        icon_background: "actionEditor.iconBackground" = "Remove background";
        icon_blur: "actionEditor.iconBlur" = "Blur";
        icon_harmonize: "actionEditor.iconHarmonize" = "Harmonize layers";
        icon_clarity: "actionEditor.iconClarity" = "Clarity";
        icon_styles: "actionEditor.iconStyles" = "Generated styles";
        icon_grid: "actionEditor.iconGrid" = "Show grid";
    }
}
