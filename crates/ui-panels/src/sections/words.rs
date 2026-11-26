//! Built-in words of the sections family.

craft_ui::words! {
    /// The words of title rows and the footer bar; each id is prefixed by its type.
    pub struct SectionWords {
        close: "sections.titleRowClose" = "Close";
        menu: "sections.titleRowMenu" = "More actions";
        float: "sections.titleRowFloat" = "Float";
        gate: "sections.titleRowGate" = "Enable";
        footer: "sections.footerBarActions" = "Actions";
    }
}

// Reorderable sections.
craft_ui::words! {
    /// The words of the reorderable sections; each id is prefixed by its type.
    pub struct ReorderableWords {
        moved: template "sections.reorderableMoved" = "{title} moved to position {position} of {count}";
    }
}
