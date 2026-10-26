//! One keyed hook for every label in [`DockStyle`].

use super::DockStyle;

impl DockStyle {
    /// Replace every `*_label` field with `translate(key)`, keeping the English default where it
    /// returns `None`. The defaults of menu items are in title case; those of controls, which
    /// only screen readers announce, in sentence case. An application translates the docking
    /// chrome in one place:
    ///
    /// ```
    /// fn dock_style(ui: &egui::Ui) -> craft_ui::docking::DockStyle {
    ///     craft_ui::docking::DockStyle::from_ui(ui).localized(|key| match key {
    ///         "dock.close" => Some("Fermer le panneau".to_owned()),
    ///         _ => None,
    ///     })
    /// }
    /// ```
    ///
    /// | Key | Field | English default |
    /// |---|---|---|
    /// | `dock.float` | `float_label` | Float Panel |
    /// | `dock.close` | `close_label` | Close Panel |
    /// | `dock.movePanelTo` | `move_panel_to_label` | Move Panel to Group |
    /// | `dock.panels` | `panels_label` | Panels |
    /// | `dock.resize` | `resize_label` | Resize panels |
    /// | `dock.resizeWindow` | `resize_window_label` | Resize panel window |
    /// | `dock.moveGroup` | `move_group_label` | Move tab group |
    /// | `dock.closeGroup` | `close_group_label` | Close Tab Group |
    /// | `dock.minimize` | `minimize_label` | Minimize |
    /// | `dock.expand` | `expand_label` | Expand |
    /// | `dock.floatGroup` | `float_group_label` | Float Tab Group |
    /// | `dock.moveGroupTo` | `move_group_to_label` | Move Group To |
    /// | `dock.moveColumn` | `move_column_label` | Move panel column |
    /// | `dock.collapseToIcons` | `collapse_to_icons_label` | Collapse to Icons |
    /// | `dock.expandPanels` | `expand_panels_label` | Expand Panels |
    /// | `dock.collapseColumn` | `collapse_column_label` | Collapse column to icons |
    /// | `dock.expandColumn` | `expand_column_label` | Expand column |
    /// | `dock.moveWindow` | `move_window_label` | Move panel window |
    /// | `dock.closeWindow` | `close_window_label` | Close panel window |
    /// | `dock.closeOthers` | `close_others_label` | Close Other Panels in Group |
    /// | `dock.maximize` | `maximize_label` | Maximize Frame |
    /// | `dock.restoreFrame` | `restore_frame_label` | Restore Frame Size |
    /// | `dock.autoCollapse` | `auto_collapse_label` | Auto-Collapse Iconic Panels |
    /// | `dock.floatHint` | `float_hint_label` | Hold {key} to undock |
    #[must_use]
    pub fn localized(mut self, translate: impl Fn(&str) -> Option<String>) -> Self {
        for (key, label) in self.labels_mut() {
            if let Some(text) = translate(key) {
                *label = text;
            }
        }
        self
    }

    fn labels_mut(&mut self) -> [(&'static str, &mut String); 24] {
        [
            ("dock.float", &mut self.float_label),
            ("dock.close", &mut self.close_label),
            ("dock.movePanelTo", &mut self.move_panel_to_label),
            ("dock.panels", &mut self.panels_label),
            ("dock.resize", &mut self.resize_label),
            ("dock.resizeWindow", &mut self.resize_window_label),
            ("dock.moveGroup", &mut self.move_group_label),
            ("dock.closeGroup", &mut self.close_group_label),
            ("dock.minimize", &mut self.minimize_label),
            ("dock.expand", &mut self.expand_label),
            ("dock.floatGroup", &mut self.float_group_label),
            ("dock.moveGroupTo", &mut self.move_group_to_label),
            ("dock.moveColumn", &mut self.move_column_label),
            ("dock.collapseToIcons", &mut self.collapse_to_icons_label),
            ("dock.expandPanels", &mut self.expand_panels_label),
            ("dock.collapseColumn", &mut self.collapse_column_label),
            ("dock.expandColumn", &mut self.expand_column_label),
            ("dock.moveWindow", &mut self.move_window_label),
            ("dock.closeWindow", &mut self.close_window_label),
            ("dock.closeOthers", &mut self.close_others_label),
            ("dock.maximize", &mut self.maximize_label),
            ("dock.restoreFrame", &mut self.restore_frame_label),
            ("dock.autoCollapse", &mut self.auto_collapse_label),
            ("dock.floatHint", &mut self.float_hint_label),
        ]
    }
}
