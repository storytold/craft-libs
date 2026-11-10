use crate::{CoreRoles, Derived, Mode, SuiteTheme, ThemeId, Variant, derive};

/// A theme and a mode: the suite's selection, or an app's own.
///
/// There is no default: a missing suite selection means the user hasn't picked one, and each app
/// shows its own [`AppDefaults`] until they do.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Selection {
    pub theme: ThemeId,
    pub mode: Mode,
}

/// What an app shows while it follows the suite and the user hasn't picked a suite theme yet.
///
/// Each app states its own, normally [`crate::SUITE_DEFAULT`] with the mode that suits it (the
/// creative apps Dark; office apps may ship Light).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppDefaults {
    pub theme: ThemeId,
    pub mode: Mode,
}

impl AppDefaults {
    pub fn selection(&self) -> Selection {
        Selection { theme: self.theme.clone(), mode: self.mode }
    }
}

/// An app's choice, kept in the app's own preferences.
///
/// No `Default`: an app states what a fresh install and a legacy preference file each become, so
/// a `#[serde(default)]` can't silently move existing users to the suite.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "snake_case"))]
pub enum AppChoice {
    FollowSuite,
    /// "Use a different theme in this app".
    Own(Selection),
}

impl AppChoice {
    /// The selection to resolve: the app's own, else the suite's, else the app's defaults when
    /// the suite has no selection.
    pub fn effective(&self, suite: Option<&Selection>, defaults: &AppDefaults) -> Selection {
        match (self, suite) {
            (AppChoice::Own(own), _) => own.clone(),
            (AppChoice::FollowSuite, Some(suite)) => suite.clone(),
            (AppChoice::FollowSuite, None) => defaults.selection(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Paint the app's own palette in this variant.
    AppDefault {
        variant: Variant,
        why: Option<Fallback>,
    },
    Theme(Resolved),
}

impl Resolution {
    pub fn variant(&self) -> Variant {
        match self {
            Resolution::AppDefault { variant, .. } => *variant,
            Resolution::Theme(r) => r.variant,
        }
    }

    pub fn fallback(&self) -> Option<&Fallback> {
        match self {
            Resolution::AppDefault { why, .. } => why.as_ref(),
            Resolution::Theme(r) => r.note.as_ref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub theme: ThemeId,
    pub variant: Variant,
    pub roles: CoreRoles,
    pub derived: Derived,
    pub note: Option<Fallback>,
}

/// Why the result differs from what the selection asked for. When several apply, the first listed
/// here is reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// No theme has this id (deleted, or from another machine): the app's own palette.
    UnknownTheme(ThemeId),
    /// The theme has only this variant and the other was asked for.
    OnlyVariant(Variant),
    /// The mode follows the system and the OS gave no answer: the app's fallback variant.
    SystemSilent,
}

/// Pure. `system` = OS answer; `fallback` = the app's variant when the OS gives none.
///
/// `craft.app-default` and unknown ids resolve to [`Resolution::AppDefault`], never to another
/// named theme. `lookup` is called at most once, with `sel.theme`.
pub fn resolve(sel: &Selection, system: Option<Variant>, fallback: Variant, lookup: &dyn Fn(&ThemeId) -> Option<SuiteTheme>) -> Resolution {
    let (wanted, silent) = match sel.mode {
        Mode::Light => (Variant::Light, false),
        Mode::Dark => (Variant::Dark, false),
        Mode::System => (system.unwrap_or(fallback), system.is_none()),
    };
    let silent = silent.then_some(Fallback::SystemSilent);
    if sel.theme.is_app_default() {
        return Resolution::AppDefault { variant: wanted, why: silent };
    }
    let Some(theme) = lookup(&sel.theme) else {
        return Resolution::AppDefault { variant: wanted, why: Some(Fallback::UnknownTheme(sel.theme.clone())) };
    };
    let (variant, roles, note) = match (theme.variant(wanted), theme.variants().next()) {
        (Some(roles), _) => (wanted, *roles, silent),
        (None, Some((only, roles))) => (only, *roles, Some(Fallback::OnlyVariant(only))),
        // A SuiteTheme without variants can't come from `from_maps` or the built-ins; treat it
        // like a missing theme rather than invent colours.
        (None, None) => {
            return Resolution::AppDefault { variant: wanted, why: Some(Fallback::UnknownTheme(sel.theme.clone())) };
        }
    };
    Resolution::Theme(Resolved { theme: sel.theme.clone(), variant, roles, derived: derive(&roles, variant), note })
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    #[test]
    fn serde_forms() {
        let own = AppChoice::Own(Selection { theme: ThemeId::app_default(), mode: Mode::Dark });
        let json = serde_json::to_string(&own).unwrap();
        assert_eq!(json, r#"{"own":{"theme":"craft.app-default","mode":"dark"}}"#);
        assert_eq!(serde_json::from_str::<AppChoice>(&json).unwrap(), own);
        assert_eq!(serde_json::to_string(&AppChoice::FollowSuite).unwrap(), r#""follow_suite""#);
        assert!(serde_json::from_str::<Selection>(r#"{"theme":"Bad Id","mode":"dark"}"#).is_err());
        let roles = crate::builtins::graphite::LIGHT;
        let json = serde_json::to_value(roles).unwrap();
        assert_eq!(json["text_muted"], "#5e5e66");
        assert_eq!(serde_json::from_value::<CoreRoles>(json).unwrap(), roles);
    }
}
