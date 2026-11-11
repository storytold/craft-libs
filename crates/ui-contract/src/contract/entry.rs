//! `Entry<T>` (11.4): one row of a value menu or a choice list. Split-button menus, segmented controls, number
//! presets and select options share it, so each shows the same label, icon, checked and disabled states.

use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use craft_theme::Srgba;

/// The label of a separator, which has none.
static NO_LABEL: Label = Label::Empty;

/// A row of entries: an item that reports its value when picked, or a separator between groups of items.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry<T> {
    row: Row<T>,
}

#[derive(Clone, Debug, PartialEq)]
enum Row<T> {
    Item(Item<T>),
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
struct Item<T> {
    value: T,
    label: Label,
    icon: Option<IconSpec<'static>>,
    enabled: bool,
    reason: Option<Label>,
    checked: bool,
}

impl<T> Entry<T> {
    /// An entry reporting `value` when picked, shown as `label`: enabled and unchecked, without an icon.
    pub fn new(value: T, label: impl Into<Label>) -> Self {
        Entry { row: Row::Item(Item { value, label: label.into(), icon: None, enabled: true, reason: None, checked: false }) }
    }

    /// A rule between groups of entries. It is not a row: it takes no pick and no lead.
    pub fn separator() -> Self {
        Entry { row: Row::Separator }
    }

    /// An icon before the label. A separator has none.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<IconSpec<'static>>) -> Self {
        if let Row::Item(item) = &mut self.row {
            item.icon = Some(icon.into());
        }
        self
    }

    /// A swatch of `color` before the label, in the icon's place (a guide colour, a matte colour). A separator has none.
    #[must_use]
    pub fn swatch(self, color: Srgba) -> Self {
        self.icon(IconSpec::swatch(color))
    }

    /// Whether the entry can be picked; a disabled entry shows dimmed and never takes the lead.
    #[must_use]
    pub fn enabled(mut self, on: bool) -> Self {
        if let Row::Item(item) = &mut self.row {
            item.enabled = on;
        }
        self
    }

    /// Disables the entry and says why: the row shows `reason` as its tooltip and its accessible description while the
    /// entry is disabled. A separator has none.
    #[must_use]
    pub fn disabled_reason(mut self, reason: impl Into<Label>) -> Self {
        if let Row::Item(item) = &mut self.row {
            item.enabled = false;
            item.reason = Some(reason.into());
        }
        self
    }

    /// Whether the entry shows as the current choice.
    #[must_use]
    pub fn checked(mut self, on: bool) -> Self {
        if let Row::Item(item) = &mut self.row {
            item.checked = on;
        }
        self
    }

    /// The value a pick reports; `None` for a separator.
    pub fn value(&self) -> Option<&T> {
        match &self.row {
            Row::Item(item) => Some(&item.value),
            Row::Separator => None,
        }
    }

    /// The label; empty for a separator.
    pub fn label(&self) -> &Label {
        match &self.row {
            Row::Item(item) => &item.label,
            Row::Separator => &NO_LABEL,
        }
    }

    /// The icon before the label.
    pub fn icon_spec(&self) -> Option<&IconSpec<'static>> {
        match &self.row {
            Row::Item(item) => item.icon.as_ref(),
            Row::Separator => None,
        }
    }

    /// Whether the entry can be picked; false for a separator.
    pub fn is_enabled(&self) -> bool {
        matches!(&self.row, Row::Item(item) if item.enabled)
    }

    /// Why the entry is disabled, while it is.
    pub fn why_disabled(&self) -> Option<&Label> {
        match &self.row {
            Row::Item(item) if !item.enabled => item.reason.as_ref(),
            _ => None,
        }
    }

    /// Whether the entry shows as the current choice.
    pub fn is_checked(&self) -> bool {
        matches!(&self.row, Row::Item(item) if item.checked)
    }

    /// Whether the entry is a separator.
    pub fn is_separator(&self) -> bool {
        matches!(self.row, Row::Separator)
    }
}
