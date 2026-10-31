use std::{collections::BTreeMap, fmt};

use crate::{CoreRoles, IdError, Rgba, Role, ThemeId, Variant, builtins};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    BuiltIn,
    User,
}

/// A named theme: one or both variants of the 13 core roles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteTheme {
    pub id: ThemeId,
    pub name: String,
    pub version: u32,
    pub light: Option<CoreRoles>,
    pub dark: Option<CoreRoles>,
    pub origin: Origin,
}

/// One variant as a theme file holds it: role key to colour.
pub type RoleMap = BTreeMap<String, Rgba>;

/// A role a definition left out, filled from built-in Graphite of the same variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MissingRole {
    pub variant: Variant,
    pub role: Role,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefinitionError {
    Id(IdError),
    /// Neither a light nor a dark variant.
    NoVariant,
}

impl SuiteTheme {
    /// A user or imported theme from its file's role maps.
    ///
    /// Missing roles are filled from built-in Graphite of the same variant and reported; unknown
    /// keys are ignored here (the store keeps them on disk). An empty name falls back to the id.
    /// Contrast is not checked: [`crate::contrast::diagnose`] reports it without refusing anything.
    pub fn from_maps(
        id: &str,
        name: &str,
        version: u32,
        light: Option<&RoleMap>,
        dark: Option<&RoleMap>,
    ) -> Result<(SuiteTheme, Vec<MissingRole>), DefinitionError> {
        let id = ThemeId::parse_user(id).map_err(DefinitionError::Id)?;
        if light.is_none() && dark.is_none() {
            return Err(DefinitionError::NoVariant);
        }
        let mut missing = Vec::new();
        let mut fill = |variant: Variant, map: &RoleMap| {
            let mut roles = builtins::graphite::roles(variant);
            for role in Role::ALL {
                match map.get(role.key()) {
                    Some(&c) => roles.set(role, c),
                    None => missing.push(MissingRole { variant, role }),
                }
            }
            roles
        };
        let light = light.map(|m| fill(Variant::Light, m));
        let dark = dark.map(|m| fill(Variant::Dark, m));
        let name = if name.trim().is_empty() { id.as_str().to_owned() } else { name.to_owned() };
        Ok((SuiteTheme { id, name, version, light, dark, origin: Origin::User }, missing))
    }

    pub fn variant(&self, v: Variant) -> Option<&CoreRoles> {
        match v {
            Variant::Light => self.light.as_ref(),
            Variant::Dark => self.dark.as_ref(),
        }
    }

    /// The variants this theme defines, light first.
    pub fn variants(&self) -> impl Iterator<Item = (Variant, &CoreRoles)> {
        Variant::ALL.into_iter().filter_map(|v| self.variant(v).map(|r| (v, r)))
    }
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefinitionError::Id(e) => e.fmt(f),
            DefinitionError::NoVariant => f.write_str("a theme needs a light or a dark variant"),
        }
    }
}

impl std::error::Error for DefinitionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DefinitionError::Id(e) => Some(e),
            DefinitionError::NoVariant => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, u32)]) -> RoleMap {
        pairs.iter().map(|&(k, c)| (k.to_owned(), Rgba::from_rgb_u32(c))).collect()
    }

    #[test]
    fn missing_roles_come_from_graphite_and_are_reported() {
        let dark = map(&[("text", 0xFFFFFF), ("background", 0x000000), ("sparkle", 0x123456)]);
        let (theme, missing) = SuiteTheme::from_maps("night", "Night", 3, None, Some(&dark)).unwrap();
        assert_eq!(theme.origin, Origin::User);
        assert_eq!(theme.version, 3);
        assert!(theme.light.is_none());
        let roles = theme.dark.unwrap();
        assert_eq!(roles.text, Rgba::WHITE);
        assert_eq!(roles.background, Rgba::BLACK);
        assert_eq!(roles.accent, builtins::graphite::DARK.accent);
        assert_eq!(missing.len(), 11);
        assert!(missing.iter().all(|m| m.variant == Variant::Dark && !matches!(m.role, Role::Text | Role::Background)));
    }

    #[test]
    fn definition_errors() {
        let m = map(&[]);
        assert_eq!(SuiteTheme::from_maps("x", "X", 1, Some(&m), None).unwrap_err(), DefinitionError::Id(IdError::Length));
        assert_eq!(SuiteTheme::from_maps("craft.mine", "Mine", 1, Some(&m), None).unwrap_err(), DefinitionError::Id(IdError::Reserved));
        assert_eq!(SuiteTheme::from_maps("mine", "Mine", 1, None, None).unwrap_err(), DefinitionError::NoVariant);
        let (theme, _) = SuiteTheme::from_maps("mine", "  ", 1, Some(&m), None).unwrap();
        assert_eq!(theme.name, "mine");
    }
}
