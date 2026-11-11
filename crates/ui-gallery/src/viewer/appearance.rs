//! The choices of the options bar: the named options of each root value, the theme list and the Variants axes.

use craft_i18n::Direction;
use craft_theme::{ContrastChoice, ModeChoice, MotionChoice, Size, TextSize, ThemeDef, ThemeId, builtins};
use craft_ui::contract::Entry;
use craft_ui::runtime::env::Scale;

/// One option of a choice: the value, the name it is saved under and the label the options bar shows.
#[non_exhaustive]
pub struct Choice<T: 'static> {
    /// The value.
    pub value: T,
    /// The saved name.
    pub name: &'static str,
    /// The text the options bar shows.
    pub label: &'static str,
}

/// The options of one appearance field, in the order the options bar lists them.
#[non_exhaustive]
pub struct Choices<T: 'static>(pub &'static [Choice<T>]);

impl<T: Copy + PartialEq + 'static> Choices<T> {
    /// The value saved as `name`.
    pub fn parse(&self, name: &str) -> Option<T> {
        self.0.iter().find(|choice| choice.name == name).map(|choice| choice.value)
    }

    /// The name `value` is saved under.
    pub fn name(&self, value: T) -> &'static str {
        self.0.iter().find(|choice| choice.value == value).map_or("", |choice| choice.name)
    }

    /// The text the options bar shows for `value`.
    pub fn label(&self, value: T) -> &'static str {
        self.0.iter().find(|choice| choice.value == value).map_or("", |choice| choice.label)
    }

    /// The entries of a select or a segmented row over the options.
    pub fn entries(&self) -> Vec<Entry<T>> {
        self.0.iter().map(|choice| Entry::new(choice.value, choice.label)).collect()
    }
}

macro_rules! choices {
    ($name:ident: $ty:ty = [$(($value:expr, $saved:literal, $label:literal)),+ $(,)?]) => {
        /// The options of the field.
        pub const $name: Choices<$ty> = Choices(&[$(Choice { value: $value, name: $saved, label: $label }),+]);
    };
}

choices!(MODES: ModeChoice = [
    (ModeChoice::Light, "light", "Light"),
    (ModeChoice::Dark, "dark", "Dark"),
    (ModeChoice::System, "system", "System"),
]);
choices!(CONTRASTS: ContrastChoice = [
    (ContrastChoice::Standard, "standard", "Standard"),
    (ContrastChoice::High, "high", "High"),
    (ContrastChoice::System, "system", "System"),
]);
choices!(TEXT_SIZES: TextSize = [
    (TextSize::Tiny, "tiny", "Tiny"),
    (TextSize::Small, "small", "Small"),
    (TextSize::Medium, "medium", "Medium"),
    (TextSize::Large, "large", "Large"),
]);
choices!(DENSITIES: Size = [(Size::Compact, "compact", "Compact"), (Size::Comfortable, "comfortable", "Comfortable")]);
choices!(MOTIONS: MotionChoice = [
    (MotionChoice::Full, "full", "Off"),
    (MotionChoice::Reduced, "reduced", "On"),
    (MotionChoice::System, "system", "System"),
]);
choices!(SCALES: Scale = [
    (Scale::new(1.0), "100", "100%"),
    (Scale::new(1.25), "125", "125%"),
    (Scale::new(1.5), "150", "150%"),
    (Scale::new(2.0), "200", "200%"),
]);
choices!(DIRECTIONS: Direction = [(Direction::Ltr, "ltr", "LTR"), (Direction::Rtl, "rtl", "RTL")]);

/// What a Variants view repeats the story over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VariantAxis {
    /// Every built-in theme.
    Theme,
    /// Light and dark.
    Mode,
    /// Standard and high contrast.
    Contrast,
    /// Compact and comfortable density.
    Size,
    /// Left to right and right to left.
    Direction,
    /// Small and medium controls.
    ControlSize,
    /// Every direction at every control size.
    DirectionAndControlSize,
}

impl VariantAxis {
    /// Every axis, in menu order.
    pub const ALL: [VariantAxis; 7] = [
        VariantAxis::Theme,
        VariantAxis::Mode,
        VariantAxis::Contrast,
        VariantAxis::Size,
        VariantAxis::Direction,
        VariantAxis::ControlSize,
        VariantAxis::DirectionAndControlSize,
    ];

    /// The Variants menu's item.
    pub fn menu_label(self) -> &'static str {
        match self {
            VariantAxis::Theme => "Every theme",
            VariantAxis::Mode => "Every mode (light, dark)",
            VariantAxis::Contrast => "Every contrast (standard, high)",
            VariantAxis::Size => "Every size (compact, comfortable)",
            VariantAxis::Direction => "Every direction (LTR, RTL)",
            VariantAxis::ControlSize => "Every control size (Sm, Md)",
            VariantAxis::DirectionAndControlSize => "Every direction and control size",
        }
    }
}
/// The built-in theme saved as `name`.
pub fn theme_named(name: &str) -> Option<ThemeId> {
    builtins().iter().find(|theme| theme.id.as_str() == name).map(|theme| theme.id.clone())
}

/// The built-in theme `id` names; the suite default when `id` names none.
pub fn theme_of(id: &ThemeId) -> &'static ThemeDef {
    let themes = builtins();
    themes.iter().find(|theme| theme.id == *id).or_else(|| themes.iter().find(|theme| theme.id == ThemeId::suite_default())).unwrap_or(&themes[0])
}

/// The entries of the Theme select: every built-in theme under its name.
pub fn theme_entries() -> Vec<Entry<ThemeId>> {
    builtins().iter().map(|theme| Entry::new(theme.id.clone(), theme.name.clone())).collect()
}
