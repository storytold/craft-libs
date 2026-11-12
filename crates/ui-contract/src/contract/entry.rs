//! `Entry<T>` (11.4): one row of a value menu or a choice list. Split-button menus, segmented controls, number
//! presets and select options share it, so each shows the same label, icon, checked and disabled states.

use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use craft_theme::Srgba;

/// The label of a separator, which has none.
static NO_LABEL: Label = Label::Empty;

/// A row of entries: an item that reports its value when picked, a heading over a group of items, or a separator
/// between groups of items.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry<T> {
    row: Row<T>,
}

#[derive(Clone, Debug, PartialEq)]
enum Row<T> {
    Item(Item<T>),
    Heading(Label),
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
struct Item<T> {
    value: T,
    label: Label,
    closed: Option<Label>,
    icon: Option<IconSpec<'static>>,
    enabled: bool,
    reason: Option<Label>,
    checked: bool,
    stepped: bool,
}

impl<T> Entry<T> {
    /// An entry reporting `value` when picked, shown as `label`: enabled and unchecked, without an icon.
    pub fn new(value: T, label: impl Into<Label>) -> Self {
        Entry {
            row: Row::Item(Item { value, label: label.into(), closed: None, icon: None, enabled: true, reason: None, checked: false, stepped: true }),
        }
    }

    /// A rule between groups of entries. It is not a row: it takes no pick and no lead.
    pub fn separator() -> Self {
        Entry { row: Row::Separator }
    }

    /// A caption over the entries after it, up to the next heading or separator, that names their group for assistive
    /// technology too. Like a separator it takes no pick and no lead, and steps and type-ahead pass over it.
    pub fn heading(label: impl Into<Label>) -> Self {
        Entry { row: Row::Heading(label.into()) }
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

    /// The empty swatch of a colour the theme decides, in the icon's place, so the label lines up with the labels of
    /// entries that have swatches. A separator has none.
    #[must_use]
    pub fn empty_swatch(self) -> Self {
        self.icon(IconSpec::empty_swatch())
    }

    /// The text a closed control shows for the entry in place of its label (a tab leader's fill without its number).
    /// The list, type-ahead and the accessible value keep the label. A heading or separator has none.
    #[must_use]
    pub fn closed_label(mut self, text: impl Into<Label>) -> Self {
        if let Row::Item(item) = &mut self.row {
            item.closed = Some(text.into());
        }
        self
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

    /// The value a pick reports; `None` for a heading or a separator.
    pub fn value(&self) -> Option<&T> {
        match &self.row {
            Row::Item(item) => Some(&item.value),
            Row::Heading(_) | Row::Separator => None,
        }
    }

    /// The label: a heading's caption; empty for a separator.
    pub fn label(&self) -> &Label {
        match &self.row {
            Row::Item(item) => &item.label,
            Row::Heading(label) => label,
            Row::Separator => &NO_LABEL,
        }
    }

    /// The text a closed control shows for the entry: its closed label, else its label.
    pub fn face_label(&self) -> &Label {
        match &self.row {
            Row::Item(Item { closed: Some(closed), .. }) => closed,
            _ => self.label(),
        }
    }

    /// The icon before the label.
    pub fn icon_spec(&self) -> Option<&IconSpec<'static>> {
        match &self.row {
            Row::Item(item) => item.icon.as_ref(),
            Row::Heading(_) | Row::Separator => None,
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

    /// Whether the entry is a heading.
    pub fn is_heading(&self) -> bool {
        matches!(self.row, Row::Heading(_))
    }

    /// Whether the entry is an item, which reports a value: neither a heading nor a separator.
    pub fn is_item(&self) -> bool {
        matches!(self.row, Row::Item(_))
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
