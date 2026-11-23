//! `CommonWords`: the words more than one family writes the same way. A family reads them with
//! `env.words::<CommonWords>()` and keeps no copy of its own.

craft_ui::words! {
    /// The words shared by every family: the dialog actions, the "don't show again" choice and the
    /// separator between the steps of a path.
    pub struct CommonWords {
        cancel: "common.cancel" = "Cancel";
        ok: "common.ok" = "OK";
        dont_show_again: "common.dontShowAgain" = "Don't show again";
        /// The tooltip of a label's reset dot.
        reset_to_default: "common.resetToDefault" = "Reset to default";
        /// The tooltip of a label's reset dot when the default is named; `{value}` is the default as the control words it.
        reset_to_value: template "common.resetToValue" = "Reset to default: {value}";
        /// A caption beside its control; a catalog writes its language's punctuation (French " :").
        caption: template "common.caption" = "{caption}:";
        /// Between the steps of a path (a menu path, a tree picker's selection); a catalog changes it for a
        /// right-to-left script.
        path_separator: "common.pathSeparator" = " › ";
    }
}
