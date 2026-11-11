//! A key binding: its chord or sequence text, parse errors and serde form.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::*;

/// A trigger in a scope. `scope: None` is the command's home scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    /// What is pressed.
    pub trigger: Trigger,
    /// The scope the binding works in; `None` for the command's home scope.
    pub scope: Option<Scope>,
}

/// Why binding text was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BindingError {
    /// Longer than [`MAX_BINDING_TEXT`].
    TooLong,
    /// The part before `": "` is not a scope name.
    Scope,
    /// The trigger part did not parse.
    Trigger(ParseError),
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::TooLong => write!(f, "binding text over {MAX_BINDING_TEXT} bytes"),
            BindingError::Scope => f.write_str("bad scope name"),
            BindingError::Trigger(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for BindingError {}

impl Binding {
    /// A binding in the command's home scope.
    pub fn new(trigger: impl Into<Trigger>) -> Binding {
        Binding { trigger: trigger.into(), scope: None }
    }

    /// A binding of `trigger` in `scope`.
    pub fn in_scope(trigger: impl Into<Trigger>, scope: Scope) -> Binding {
        Binding { trigger: trigger.into(), scope: Some(scope) }
    }

    /// Reads `<trigger>` or `<Scope>: <trigger>`. The whole text is tried as a trigger first, so
    /// `Cmd+: Cmd+:` is the Colon chord pressed twice; otherwise it is split at the first `": "`.
    pub fn parse(text: &str) -> Result<Binding, BindingError> {
        if text.len() > MAX_BINDING_TEXT {
            return Err(BindingError::TooLong);
        }
        let whole = match Trigger::parse(text.trim()) {
            Ok(trigger) => return Ok(Binding::new(trigger)),
            Err(e) => e,
        };
        match text.split_once(": ") {
            Some((scope, trigger)) => {
                let scope = Scope::new(scope.trim()).ok_or(BindingError::Scope)?;
                let trigger = Trigger::parse(trigger.trim()).map_err(BindingError::Trigger)?;
                Ok(Binding::in_scope(trigger, scope))
            }
            None => Err(BindingError::Trigger(whole)),
        }
    }

    /// The binding text for a command whose home scope is `home`: the scope is written only when
    /// it differs.
    pub fn text(&self, home: &Scope) -> String {
        match &self.scope {
            Some(s) if s != home => format!("{s}: {}", self.trigger),
            _ => self.trigger.to_string(),
        }
    }

    /// The scope, `home` when unset.
    pub fn scope_or<'a>(&'a self, home: &'a Scope) -> &'a Scope {
        self.scope.as_ref().unwrap_or(home)
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scope {
            Some(s) => write!(f, "{s}: {}", self.trigger),
            None => write!(f, "{}", self.trigger),
        }
    }
}

impl Serialize for Binding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Binding, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Binding::parse(&text).map_err(serde::de::Error::custom)
    }
}
