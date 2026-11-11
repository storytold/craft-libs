//! `CommonWords`: the words more than one family writes the same way. A family reads them with
//! `env.words::<CommonWords>()` and keeps no copy of its own.

craft_ui::words! {
    /// The words shared by every family: the dialog actions, the "don't show again" choice and the
    /// separator between the steps of a path.
    pub struct CommonWords {
        cancel: "common.cancel" = "Cancel";
        ok: "common.ok" = "OK";
        dont_show_again: "common.dontShowAgain" = "Don't show again";
        /// A caption beside its control; a catalog writes its language's punctuation (French " :").
        caption: template "common.caption" = "{caption}:";
        /// Between the steps of a path (a menu path, a tree picker's selection); a catalog changes it for a
        /// right-to-left script.
        path_separator: "common.pathSeparator" = " › ";
    }
}
