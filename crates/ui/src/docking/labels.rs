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
    ///
    /// Every group's drag bar and icon grip, and every column bar, has the same default name.
    /// Opt in to naming them, in the English label and each translation: a `{name}` in
    /// `move_group_label` stands for the logical label of the panel the group shows, and in
    /// `move_column_label` for those of the panels the column's groups show, joined by ", ", so a
    /// screen reader tells the handles apart. The ", " is fixed. Untitled panels are left out;
    /// with no title at all the placeholder goes with the brackets or quotes around it, a
    /// separator right after it and one of the spaces beside it ("Move the tab group"). Groups
    /// showing panels of the same title still share a name:
    ///
    /// ```
    /// let style = craft_ui::docking::DockStyle::default().localized(|key| match key {
    ///     "dock.moveGroup" => Some("Move the {name} tab group".to_owned()),
    ///     "dock.moveColumn" => Some("Move the {name} panel column".to_owned()),
    ///     _ => None,
    /// });
    /// # assert_eq!(style.move_group_label, "Move the {name} tab group");
    /// ```
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

/// `label` with every `{name}` replaced by `name()`, which only runs when there is one. A blank
/// name leaves each placeholder out with what only made sense around it (see [`unnamed`]); a
/// label with no letter or digit left gives `fallback`, so no handle is announced with a hole in
/// its name or without one.
pub(super) fn named(label: &str, fallback: &str, name: impl FnOnce() -> String) -> String {
    if !label.contains(NAME) {
        return label.to_owned();
    }
    let name = name();
    if !name.trim().is_empty() {
        return label.replace(NAME, name.trim());
    }
    let bare = unnamed(label);
    if bare.chars().any(char::is_alphanumeric) { bare } else { fallback.to_owned() }
}

/// `label` without its placeholders. Each goes with the brackets or quotes around it and the
/// spaces inside them, and with a separator right after it. Where both sides were spaces, one
/// stays; where neither was, none is added. Separators and spaces at the ends go too.
fn unnamed(label: &str) -> String {
    let mut text = label.to_owned();
    // Every pass removes a placeholder, so the text gets shorter each time.
    while let Some((before, after)) = text.split_once(NAME) {
        let (mut head, mut tail) = (before, after);
        let (open, close) = (head.trim_end(), tail.trim_start());
        if let (Some(left), Some(right)) = (open.chars().next_back(), close.chars().next())
            && PAIRS.contains(&(left, right))
        {
            head = open.strip_suffix(left).unwrap_or(open);
            tail = close.strip_prefix(right).unwrap_or(close);
        }
        let next = tail.trim_start();
        if let Some(separator) = next.chars().next().filter(|c| SEPARATORS.contains(c))
            && !head.trim().is_empty()
        {
            tail = next.strip_prefix(separator).unwrap_or(next);
        }
        text = if head.ends_with(char::is_whitespace) && tail.starts_with(char::is_whitespace) {
            format!("{} {}", head.trim_end(), tail.trim_start())
        } else {
            format!("{head}{tail}")
        };
    }
    text.trim_matches(|c: char| c.is_whitespace() || SEPARATORS.contains(&c) || DASHES.contains(&c)).to_owned()
}

/// Brackets and quotes, opening and closing, that can wrap the placeholder.
const PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('<', '>'),
    ('"', '"'),
    ('\'', '\''),
    ('«', '»'),
    ('»', '«'),
    ('‹', '›'),
    ('›', '‹'),
    ('“', '”'),
    ('„', '“'),
    ('”', '”'),
    ('‘', '’'),
    ('‚', '‘'),
    ('’', '’'),
    ('「', '」'),
    ('『', '』'),
    ('（', '）'),
    ('［', '］'),
    ('【', '】'),
    ('〈', '〉'),
    ('《', '》'),
];

/// Separators that go with a placeholder they follow, or that are left at an end.
const SEPARATORS: &[char] = &[',', ';', ':', '、', '，', '；', '：'];

/// Dashes left at an end.
const DASHES: &[char] = &['-', '–', '—'];

/// The default [`DockStyle::move_group_label`].
pub(super) const MOVE_GROUP: &str = "Move tab group";
/// The default [`DockStyle::move_column_label`].
pub(super) const MOVE_COLUMN: &str = "Move panel column";

/// What [`DockStyle::move_group_label`] and [`DockStyle::move_column_label`] put the moved
/// group's or column's name in place of.
const NAME: &str = "{name}";
