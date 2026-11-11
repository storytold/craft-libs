//! The words of the Brush Preset picker (`brushPresets.*`).

craft_ui::words! {
    /// The Brush Preset picker's words, English by default. Templates take `{name}` and `{size}`.
    pub struct BrushPresetWords {
        /// The picker's popover and its closed field, for AccessKit.
        picker: "brushPresets.picker" = "Brush Preset picker";
        /// The Size row's label and field name.
        size: "brushPresets.size" = "Size";
        /// The Hardness slider's label.
        hardness: "brushPresets.hardness" = "Hardness";
        /// The Brush Settings button's name and tooltip.
        settings: "brushPresets.settings" = "Brush Settings…";
        /// The New Brush Preset button's name and tooltip.
        new_preset: "brushPresets.newPreset" = "New Brush Preset";
        /// The gear button's name and tooltip.
        options: "brushPresets.options" = "Brush Preset Options";
        /// Gear menu: New Brush Preset….
        menu_new: "brushPresets.menuNew" = "New Brush Preset…";
        /// Gear menu: rename the current preset.
        menu_rename: "brushPresets.menuRename" = "Rename Brush…";
        /// Gear menu: delete the current preset.
        menu_delete: "brushPresets.menuDelete" = "Delete Brush";
        /// Gear menu: the name part box.
        part_name: "brushPresets.partName" = "Brush Name";
        /// Gear menu: the stroke part box.
        part_stroke: "brushPresets.partStroke" = "Brush Stroke";
        /// Gear menu: the tip part box.
        part_tip: "brushPresets.partTip" = "Brush Tip";
        /// Gear menu: Import Brushes….
        menu_import: "brushPresets.menuImport" = "Import Brushes…";
        /// A preset's accessible name: "{name}, {size}".
        preset_size: template "brushPresets.presetSize" = "{name}, {size}";
    }
}
