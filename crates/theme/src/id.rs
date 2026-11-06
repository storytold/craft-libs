use std::{fmt, sync::Arc};

/// The id of the theme that stands for each app's own palette.
pub const APP_DEFAULT: &str = "craft.app-default";
/// The suite's default theme: the id apps normally put in their [`crate::AppDefaults`].
pub const SUITE_DEFAULT: &str = "craft.artcraft";
/// Ids with this prefix belong to the built-in themes and [`APP_DEFAULT`].
pub const RESERVED_PREFIX: &str = "craft.";

const MAX_LEN: usize = 64;

/// A theme's stable identity: `[a-z0-9][a-z0-9._-]{1,63}`.
///
/// Renaming a theme keeps its id. Selections and per-app choices refer to themes only by id.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThemeId(Arc<str>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdError {
    /// Shorter than 2 or longer than 64 bytes.
    Length,
    /// The first character is not `a-z` or `0-9`.
    Start,
    /// A character outside `a-z 0-9 . _ -`.
    Char(char),
    /// `craft.` ids are the built-ins'; a user or imported theme can't take one.
    Reserved,
    /// The first dot-separated segment is a Windows device name (`con`, `nul`, `aux`, `prn`,
    /// `com1`-`com9`, `lpt1`-`lpt9`), which can't be a file name there.
    DeviceName,
}

/// Windows refuses these as file names, with any extension.
const DEVICE_NAMES: [&str; 22] = [
    "con", "nul", "aux", "prn", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5",
    "lpt6", "lpt7", "lpt8", "lpt9",
];

impl ThemeId {
    /// Checks the form only; [`IdError::Reserved`] comes from [`ThemeId::parse_user`].
    pub fn parse(s: &str) -> Result<Self, IdError> {
        if !(2..=MAX_LEN).contains(&s.len()) {
            return Err(IdError::Length);
        }
        let mut chars = s.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return Err(IdError::Start);
        }
        if let Some(c) = chars.find(|&c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))) {
            return Err(IdError::Char(c));
        }
        Ok(ThemeId(Arc::from(s)))
    }

    /// [`ThemeId::parse`], refusing the reserved `craft.` prefix and Windows device names: for
    /// themes users create or import, whose ids become file names.
    pub fn parse_user(s: &str) -> Result<Self, IdError> {
        let id = Self::parse(s)?;
        let first = id.0.split('.').next().unwrap_or_default();
        if id.is_builtin() {
            Err(IdError::Reserved)
        } else if DEVICE_NAMES.iter().any(|d| first.eq_ignore_ascii_case(d)) {
            Err(IdError::DeviceName)
        } else {
            Ok(id)
        }
    }

    /// For the crate's own constants, which are valid by construction (a test checks each).
    pub(crate) fn from_static(s: &'static str) -> Self {
        ThemeId(Arc::from(s))
    }

    pub fn app_default() -> Self {
        Self::from_static(APP_DEFAULT)
    }

    pub fn suite_default() -> Self {
        Self::from_static(SUITE_DEFAULT)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True for every id in the reserved `craft.` namespace, including [`APP_DEFAULT`].
    pub fn is_builtin(&self) -> bool {
        self.0.starts_with(RESERVED_PREFIX)
    }

    pub fn is_app_default(&self) -> bool {
        &*self.0 == APP_DEFAULT
    }
}

impl fmt::Debug for ThemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for ThemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdError::Length => write!(f, "a theme id has 2 to {MAX_LEN} characters"),
            IdError::Start => f.write_str("a theme id starts with a-z or 0-9"),
            IdError::Char(c) => write!(f, "a theme id can't contain {c:?}; use a-z, 0-9, '.', '_' or '-'"),
            IdError::Reserved => write!(f, "theme ids starting with {RESERVED_PREFIX:?} are reserved for built-in themes"),
            IdError::DeviceName => f.write_str("a theme id can't start with a Windows device name such as \"con\" or \"lpt1\""),
        }
    }
}

impl std::error::Error for IdError {}

#[cfg(feature = "serde")]
impl serde::Serialize for ThemeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ThemeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        ThemeId::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_forms() {
        for ok in ["ab", "0x", "my-theme", "a.b_c-d", "studio.night-2", &"a".repeat(64)] {
            assert_eq!(ThemeId::parse(ok).map(|id| id.as_str().to_owned()), Ok(ok.to_owned()), "{ok}");
        }
        assert_eq!(ThemeId::parse(""), Err(IdError::Length));
        assert_eq!(ThemeId::parse("a"), Err(IdError::Length));
        assert_eq!(ThemeId::parse(&"a".repeat(65)), Err(IdError::Length));
        assert_eq!(ThemeId::parse("-ab"), Err(IdError::Start));
        assert_eq!(ThemeId::parse(".ab"), Err(IdError::Start));
        assert_eq!(ThemeId::parse("Ab"), Err(IdError::Start));
        assert_eq!(ThemeId::parse("aB"), Err(IdError::Char('B')));
        assert_eq!(ThemeId::parse("a b"), Err(IdError::Char(' ')));
        assert_eq!(ThemeId::parse("a/b"), Err(IdError::Char('/')));
        assert_eq!(ThemeId::parse("aé"), Err(IdError::Char('é')));
    }

    #[test]
    fn craft_prefix_is_reserved() {
        for constant in [APP_DEFAULT, SUITE_DEFAULT] {
            let id = ThemeId::parse(constant).unwrap();
            assert!(id.is_builtin());
            assert_eq!(ThemeId::parse_user(constant), Err(IdError::Reserved));
        }
        assert!(ThemeId::app_default().is_app_default());
        assert_eq!(ThemeId::suite_default().as_str(), SUITE_DEFAULT);
        assert!(!ThemeId::parse("crafty").unwrap().is_builtin());
        assert!(ThemeId::parse_user("crafty.night").is_ok());
    }

    #[test]
    fn windows_device_names_are_refused_for_user_ids() {
        for bad in ["con", "nul", "aux", "prn", "com1", "com9", "lpt1", "lpt9", "con.dark", "nul.x", "aux.my-theme"] {
            assert_eq!(ThemeId::parse_user(bad), Err(IdError::DeviceName), "{bad}");
            assert!(ThemeId::parse(bad).is_ok(), "{bad}");
        }
        for ok in ["console", "con-dark", "com0", "com10", "lpt", "my.con", "nullish", "auxiliary"] {
            assert!(ThemeId::parse_user(ok).is_ok(), "{ok}");
        }
    }
}
