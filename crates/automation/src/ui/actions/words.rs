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
        keyboard_shortcuts: "actions.keyboardShortcuts" = "Keyboard Shortcuts";
        /// The shortcut menu's "no shortcut" row.
        none: "actions.none" = "None";
        shift: "actions.shift" = "Shift";
        /// The modifier check row on macOS.
        command: "actions.command" = "Command";
        /// The modifier check row elsewhere.
        ctrl: "actions.ctrl" = "Ctrl";
        color: "actions.color" = "Color";
    }
}
