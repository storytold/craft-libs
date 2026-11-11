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
    stepped: bool,
}

impl<T> Entry<T> {
    /// An entry reporting `value` when picked, shown as `label`: enabled and unchecked, without an icon.
    pub fn new(value: T, label: impl Into<Label>) -> Self {
        Entry { row: Row::Item(Item { value, label: label.into(), icon: None, enabled: true, reason: None, checked: false, stepped: true }) }
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

    /// Whether a step over the closed control (an arrow, Home, End, the wheel, an AccessKit step) stops on the entry.
    /// A row only the open list offers is not stepped: an action such as "Load…", or a choice that is costly to reach in
    /// passing (a file read from disk at each step).
    #[must_use]
    pub fn stepped(mut self, on: bool) -> Self {
        if let Row::Item(item) = &mut self.row {
            item.stepped = on;
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

    /// Whether a step over the closed control stops on the entry: an enabled, stepped item.
    pub fn is_stepped(&self) -> bool {
        matches!(&self.row, Row::Item(item) if item.enabled && item.stepped)
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

/// The positions of the entries a step over a closed control stops on ([`Entry::is_stepped`]), in order.
pub fn step_stops<T>(entries: &[Entry<T>]) -> Vec<usize> {
    (0..entries.len()).filter(|&at| entries[at].is_stepped()).collect()
}

/// The stop one step from the entry at `from` (a position in the entries), toward the end when `forward`. A step holds
/// at the first and last stop. A value that is none of the stops (mixed, or an entry that is not stepped) has no
/// place: a step forward lands on the first stop and a step back on the last. Every control that steps over entries
/// steps with this, and reports such a value without a number, so both steps are offered.
pub fn step_stop(stops: &[usize], from: Option<usize>, forward: bool) -> Option<usize> {
    let last = stops.len().checked_sub(1)?;
    let to = match (from.and_then(|from| stops.iter().position(|&stop| stop == from)), forward) {
        (Some(at), true) => (at + 1).min(last),
        (Some(at), false) => at.saturating_sub(1),
        (None, true) => 0,
        (None, false) => last,
    };
    Some(stops[to])
}
