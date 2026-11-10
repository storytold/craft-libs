use crate::Rgba;

/// One of a theme's two colour sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "snake_case"))]
pub enum Variant {
    Light,
    Dark,
}

impl Variant {
    pub const ALL: [Variant; 2] = [Variant::Light, Variant::Dark];
}

/// Which variant the user asked for: the OS's answer, or a fixed one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "snake_case"))]
pub enum Mode {
    #[default]
    System,
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::System, Mode::Light, Mode::Dark];
}

/// The 13 core colour roles every suite theme defines per variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "snake_case"))]
pub enum Role {
    /// Behind panels: the window, pasteboard or canvas surround.
    Background,
    /// Panels, sidebars and controls at rest.
    Surface,
    /// Cards, popups and menus above a surface.
    Raised,
    Text,
    /// Secondary labels.
    TextMuted,
    /// Decorative dividers; control edges use the derived `control_edge`.
    Line,
    Accent,
    /// Text and icons on an accent fill.
    OnAccent,
    /// Selected rows and items, with `text` on top.
    Selection,
    /// Keyboard focus rings.
    Focus,
    Good,
    Warn,
    Bad,
}

impl Role {
    pub const ALL: [Role; 13] = [
        Role::Background,
        Role::Surface,
        Role::Raised,
        Role::Text,
        Role::TextMuted,
        Role::Line,
        Role::Accent,
        Role::OnAccent,
        Role::Selection,
        Role::Focus,
        Role::Good,
        Role::Warn,
        Role::Bad,
    ];

    /// The role's key in theme files, equal to its [`CoreRoles`] field name.
    pub fn key(self) -> &'static str {
        match self {
            Role::Background => "background",
            Role::Surface => "surface",
            Role::Raised => "raised",
            Role::Text => "text",
            Role::TextMuted => "text_muted",
            Role::Line => "line",
            Role::Accent => "accent",
            Role::OnAccent => "on_accent",
            Role::Selection => "selection",
            Role::Focus => "focus",
            Role::Good => "good",
            Role::Warn => "warn",
            Role::Bad => "bad",
        }
    }

    pub fn from_key(k: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.key() == k)
    }
}

/// One variant's colours for the 13 core roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CoreRoles {
    pub background: Rgba,
    pub surface: Rgba,
    pub raised: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub line: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
    pub selection: Rgba,
    pub focus: Rgba,
    pub good: Rgba,
    pub warn: Rgba,
    pub bad: Rgba,
}

impl CoreRoles {
    pub fn get(&self, r: Role) -> Rgba {
        *self.slot(r)
    }

    pub fn set(&mut self, r: Role, c: Rgba) {
        *self.slot_mut(r) = c;
    }

    fn slot(&self, r: Role) -> &Rgba {
        match r {
            Role::Background => &self.background,
            Role::Surface => &self.surface,
            Role::Raised => &self.raised,
            Role::Text => &self.text,
            Role::TextMuted => &self.text_muted,
            Role::Line => &self.line,
            Role::Accent => &self.accent,
            Role::OnAccent => &self.on_accent,
            Role::Selection => &self.selection,
            Role::Focus => &self.focus,
            Role::Good => &self.good,
            Role::Warn => &self.warn,
            Role::Bad => &self.bad,
        }
    }

    fn slot_mut(&mut self, r: Role) -> &mut Rgba {
        match r {
            Role::Background => &mut self.background,
            Role::Surface => &mut self.surface,
            Role::Raised => &mut self.raised,
            Role::Text => &mut self.text,
            Role::TextMuted => &mut self.text_muted,
            Role::Line => &mut self.line,
            Role::Accent => &mut self.accent,
            Role::OnAccent => &mut self.on_accent,
            Role::Selection => &mut self.selection,
            Role::Focus => &mut self.focus,
            Role::Good => &mut self.good,
            Role::Warn => &mut self.warn,
            Role::Bad => &mut self.bad,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_keys_round_trip_and_address_distinct_fields() {
        let mut roles = crate::builtins::graphite::LIGHT;
        for (i, role) in Role::ALL.into_iter().enumerate() {
            assert_eq!(Role::from_key(role.key()), Some(role));
            roles.set(role, Rgba::rgb(i as u8, 0, 0));
        }
        for (i, role) in Role::ALL.into_iter().enumerate() {
            assert_eq!(roles.get(role), Rgba::rgb(i as u8, 0, 0));
        }
        assert_eq!(Role::from_key("Text"), None);
    }
}
