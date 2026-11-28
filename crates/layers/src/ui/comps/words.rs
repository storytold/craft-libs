//! The words of the Layer Comps panel.

craft_ui::words! {
    /// The words of the Layer Comps panel's rows, footer and row menu. Patterns with `{row}` are whole phrases, so a
    /// language puts the name where its grammar wants it.
    pub struct LayerCompsWords {
        last_state: "layerComps.lastDocumentState" = "Last Document State";
        apply_cell: "layerComps.applyCell" = "Apply {row}";
        apply: "layerComps.apply" = "Apply";
        name: "layerComps.name" = "Name";
        visibility: "layerComps.visibility" = "Visibility";
        position: "layerComps.position" = "Position";
        appearance: "layerComps.appearance" = "Appearance (Layer Style)";
        visibility_cell: "layerComps.visibilityCell" = "Visibility of {row}";
        position_cell: "layerComps.positionCell" = "Position of {row}";
        appearance_cell: "layerComps.appearanceCell" = "Appearance of {row}";
        missing: count "layerComps.missing" = "{n} layer recorded by this comp was deleted. Update the comp, or clear the warning." | "{n} layers recorded by this comp were deleted. Update the comp, or clear the warning.";
        row_menu: "layerComps.rowMenu" = "Layer Comp";
        move_up: "layerComps.moveUp" = "Move Up";
        move_down: "layerComps.moveDown" = "Move Down";
        previous: "layerComps.previous" = "Apply previous selected layer comp";
        next: "layerComps.next" = "Apply next selected layer comp";
        update: "layerComps.update" = "Update layer comp (visibility, position and appearance)";
        new_comp: "layerComps.new" = "Create new layer comp";
        delete: "layerComps.delete" = "Delete layer comp";
        hint: "layerComps.hint" = "Capture the visibility, position and style of every layer with + below, then switch between versions.";
    }
}
