//! SettingRule: what a settings document knows about one setting beyond its type.

#[cfg(doc)]
use super::{CustomSettings, setting_label};
use crate::runtime::env::UnitKind;
use std::ops::RangeInclusive;

/// What the document knows about one setting beyond its type: its range or choices, its label, when it shows. A setting
/// without a rule still gets a row, from its value's type.
///
/// ```ignore
/// SettingRule::new("performance.historyStates").int(1..=1000),
/// SettingRule::new("interface.canvasColor").choices(&[("gray", "Gray"), ("black", "Black"), ("custom", "Custom")]),
/// SettingRule::new("interface.canvasCustomColor").color().shown_when("interface.canvasColor", "custom"),
/// SettingRule::new("interface.systemTitleBar").restart(),
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SettingRule {
    /// The dotted path: `"<page>.<key>"`.
    pub key: String,
    /// English label; without one the key's last part is spelled out ([`setting_label`]).
    pub label: Option<String>,
    /// English help text shown with the row; `None` shows none.
    pub help: Option<String>,
    /// The control to use; `None` picks it from the value's type.
    pub kind: Option<RuleKind>,
    /// Shown only while another setting has this value.
    pub shown_when: Option<(String, String)>,
    /// Never shown.
    pub hidden: bool,
    /// Shown only on Linux.
    pub linux_only: bool,
    /// Read at launch: "Applies at next launch." under the control.
    pub restart: bool,
    /// Kept by Reset Section.
    pub keep_on_reset: bool,
    /// The titled group the row goes in; rows without one go in the page's first, untitled group.
    pub group: Option<String>,
    /// A choice whose ids are whole numbers is stored as an integer ([`SettingRule::minutes`]).
    pub(super) integer: bool,
    /// What the ids of a choice count; the row labels the options with it ([`SettingRule::minutes`]).
    pub(super) counted: Option<UnitKind>,
}

/// A rule's control, where the value's type alone isn't enough.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RuleKind {
    /// An integer field limited to the range.
    #[non_exhaustive]
    Int {
        /// The values the field accepts.
        range: RangeInclusive<i64>,
        /// What the number counts, shown after the field.
        unit: Option<UnitKind>,
    },
    /// A number field limited to the range.
    #[non_exhaustive]
    Float {
        /// The values the field accepts.
        range: RangeInclusive<f64>,
        /// How many decimal places the field shows.
        decimals: u8,
        /// What the number counts, shown after the field.
        unit: Option<UnitKind>,
    },
    /// A list of `(id, English label)`; labels translate under `setting.<key>.<id>`.
    Choice(Vec<(String, String)>),
    /// A `#rrggbb` string as a swatch and hex field.
    Color,
    /// The app draws it ([`CustomSettings::row`]).
    Custom,
}

impl SettingRule {
    /// A rule for the setting at the dotted `key` (`"<page>.<key>"`), with no overrides.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: None,
            help: None,
            kind: None,
            shown_when: None,
            hidden: false,
            linux_only: false,
            restart: false,
            keep_on_reset: false,
            group: None,
            integer: false,
            counted: None,
        }
    }

    /// Sets the English label; it translates under `setting.<key>`.
    #[must_use]
    pub fn label(mut self, english: impl Into<String>) -> Self {
        self.label = Some(english.into());
        self
    }

    /// Sets the English help text shown with the row; it translates under `setting.<key>.help`.
    #[must_use]
    pub fn help(mut self, english: impl Into<String>) -> Self {
        self.help = Some(english.into());
        self
    }

    /// An integer field limited to `range`.
    #[must_use]
    pub fn int(mut self, range: RangeInclusive<i64>) -> Self {
        self.kind = Some(RuleKind::Int { range, unit: None });
        self
    }

    /// A number field limited to `range`, showing `decimals` decimal places.
    #[must_use]
    pub fn float(mut self, range: RangeInclusive<f64>, decimals: u8) -> Self {
        self.kind = Some(RuleKind::Float { range, decimals, unit: None });
        self
    }

    /// What an `int` or `float` field counts, shown after it in the user's language (px, %); ignored on other kinds.
    #[must_use]
    pub fn counting(mut self, kind: UnitKind) -> Self {
        if let Some(RuleKind::Int { unit, .. } | RuleKind::Float { unit, .. }) = &mut self.kind {
            *unit = Some(kind);
        }
        self
    }

    /// A choice among `(id, English label)` pairs; labels translate under `setting.<key>.<id>`.
    #[must_use]
    pub fn choices(mut self, options: &[(&str, &str)]) -> Self {
        self.kind = Some(RuleKind::Choice(options.iter().map(|(id, label)| ((*id).to_owned(), (*label).to_owned())).collect()));
        self
    }

    /// A choice of minute counts, stored as the integer count (`10`). The row spells each count as minutes in the
    /// user's language ([`UnitKind::Minutes`]). A settings file that holds the count as text ("10") still shows its
    /// choice; the next change writes an integer, so the app's field reads both.
    #[must_use]
    pub fn minutes(mut self, counts: &[u32]) -> Self {
        let labels: Vec<(String, String)> = counts.iter().map(|n| (n.to_string(), n.to_string())).collect();
        let options: Vec<(&str, &str)> = labels.iter().map(|(id, label)| (id.as_str(), label.as_str())).collect();
        self.integer = true;
        self.counted = Some(UnitKind::Minutes);
        self.choices(&options)
    }

    /// A `#rrggbb` string shown as a swatch and hex field.
    #[must_use]
    pub fn color(mut self) -> Self {
        self.kind = Some(RuleKind::Color);
        self
    }

    /// The app draws the control ([`CustomSettings::row`]).
    #[must_use]
    pub fn custom(mut self) -> Self {
        self.kind = Some(RuleKind::Custom);
        self
    }

    /// Shows the row only while the setting at `key` has `value`.
    #[must_use]
    pub fn shown_when(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.shown_when = Some((key.into(), value.into()));
        self
    }

    /// Never shows the row.
    #[must_use]
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Shows the row only on Linux.
    #[must_use]
    pub fn linux_only(mut self) -> Self {
        self.linux_only = true;
        self
    }

    /// Marks the setting as read at launch: "Applies at next launch." shows under the control.
    #[must_use]
    pub fn restart(mut self) -> Self {
        self.restart = true;
        self
    }

    /// Keeps the setting's value when the user resets its page.
    #[must_use]
    pub fn keep_on_reset(mut self) -> Self {
        self.keep_on_reset = true;
        self
    }

    /// Puts the row in the titled group `english`; the title translates under `settingGroup.<english>`.
    #[must_use]
    pub fn group(mut self, english: impl Into<String>) -> Self {
        self.group = Some(english.into());
        self
    }
}
