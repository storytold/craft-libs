//! Where a binding works: the whole application, one panel, or a context such as text editing.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// Longest scope name, in bytes.
pub const MAX_SCOPE_NAME: usize = 64;

const APP: &str = "Application";

/// The name of a scope. Never empty, at most [`MAX_SCOPE_NAME`] bytes, without `:` or outer
/// whitespace.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Scope(String);

impl Scope {
    /// `None` for an empty name, one over [`MAX_SCOPE_NAME`] bytes, one with `:` or a control
    /// character, or one with whitespace at either end, which binding text would lose.
    pub fn new(name: impl Into<String>) -> Option<Scope> {
        let name = name.into();
        let ok = !name.is_empty() && name.len() <= MAX_SCOPE_NAME && name.trim() == name && !name.chars().any(|c| c == ':' || c.is_control());
        ok.then_some(Scope(name))
    }

    /// The application scope, `Application`.
    pub fn app() -> Scope {
        Scope(APP.to_string())
    }

    /// Whether this is the application scope.
    pub fn is_app(&self) -> bool {
        self.0 == APP
    }

    /// The scope name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Scope, D::Error> {
        let name = String::deserialize(deserializer)?;
        Scope::new(name).ok_or_else(|| serde::de::Error::custom("bad scope name"))
    }
}

/// A scope named at compile time: the scope of a command set and of a container instance that receives its
/// commands. [`CommandScope::APP`] is the application scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommandScope(&'static str);

impl CommandScope {
    /// The application scope.
    pub const APP: CommandScope = CommandScope(APP);

    /// The scope named `name`; [`CommandScope::scope`] is `None` for a name [`Scope::new`] refuses.
    pub const fn new(name: &'static str) -> CommandScope {
        CommandScope(name)
    }

    /// The scope name.
    pub const fn name(self) -> &'static str {
        self.0
    }

    /// Whether this is the application scope.
    pub fn is_app(self) -> bool {
        self.0 == APP
    }

    /// The matching [`Scope`], `None` when the name is not a valid scope name.
    pub fn scope(self) -> Option<Scope> {
        Scope::new(self.0)
    }
}

/// What kind of scope a name is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScopeKind {
    /// Works whatever has the keyboard.
    Application,
    /// Works while the panel has the keyboard. Names that are not registered count as panels.
    Panel,
    /// Works only while the context is active, in any panel.
    Context,
}

/// The scope registry: contexts, panels and the pairs of panels that share their shortcuts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scopes {
    panels: Vec<String>,
    contexts: Vec<String>,
    shares: Vec<(String, String)>,
    panel_fallthrough: bool,
}

impl Default for Scopes {
    fn default() -> Scopes {
        Scopes { panels: Vec::new(), contexts: Vec::new(), shares: Vec::new(), panel_fallthrough: true }
    }
}

/// What has the keyboard: the focused panel, the active contexts, and whether a text field is
/// being typed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Active<'a> {
    /// The focused panel, if any.
    pub panel: Option<&'a str>,
    /// The active contexts.
    pub contexts: &'a [&'a str],
    /// Whether a text field is being typed in.
    pub typing: bool,
}

impl<'a> Active<'a> {
    /// No panel focused, no context, not typing.
    pub const NONE: Active<'static> = Active { panel: None, contexts: &[], typing: false };

    /// Only `panel` focused: no context, not typing.
    pub const fn panel(panel: &'a str) -> Active<'a> {
        Active { panel: Some(panel), contexts: &[], typing: false }
    }
}

impl Scopes {
    /// An empty registry; panel fallthrough is on.
    pub fn new() -> Scopes {
        Scopes::default()
    }

    /// Registers a panel (used by [`Scopes::panels`]; unknown names rank as panels anyway).
    pub fn with_panel(mut self, name: &str) -> Scopes {
        if !self.panels.iter().any(|p| p == name) {
            self.panels.push(name.to_string());
        }
        self
    }

    /// Registers a context.
    pub fn with_context(mut self, name: &str) -> Scopes {
        if !self.contexts.iter().any(|c| c == name) {
            self.contexts.push(name.to_string());
        }
        self
    }

    /// Two panels that show the same thing and share their shortcuts (EffectCraft's Timeline and
    /// Composition).
    pub fn with_share(mut self, a: &str, b: &str) -> Scopes {
        if !self.shares(a, b) {
            self.shares.push((a.to_string(), b.to_string()));
        }
        self
    }

    /// Whether another panel's shortcut still runs when nothing nearer claims the chord. On by
    /// default.
    pub fn with_panel_fallthrough(mut self, on: bool) -> Scopes {
        self.panel_fallthrough = on;
        self
    }

    /// The registered panels.
    pub fn panels(&self) -> &[String] {
        &self.panels
    }

    /// The registered contexts.
    pub fn contexts(&self) -> &[String] {
        &self.contexts
    }

    /// Whether panel fallthrough is on: see [`Scopes::with_panel_fallthrough`].
    pub fn panel_fallthrough(&self) -> bool {
        self.panel_fallthrough
    }

    /// Whether `scope` is the application or a registered panel, context or sharing panel.
    pub fn knows(&self, scope: &Scope) -> bool {
        let name = scope.as_str();
        scope.is_app()
            || self.panels.iter().any(|p| p == name)
            || self.contexts.iter().any(|c| c == name)
            || self.shares.iter().any(|(a, b)| a == name || b == name)
    }

    /// The kind of `scope`: application, a registered context, else a panel.
    pub fn kind(&self, scope: &Scope) -> ScopeKind {
        self.kind_of(scope.as_str())
    }

    fn kind_of(&self, name: &str) -> ScopeKind {
        if name == APP {
            ScopeKind::Application
        } else if self.contexts.iter().any(|c| c == name) {
            ScopeKind::Context
        } else {
            ScopeKind::Panel
        }
    }

    /// Whether panels `a` and `b` (in either order) share their shortcuts.
    pub fn shares(&self, a: &str, b: &str) -> bool {
        self.shares.iter().any(|(x, y)| (x == a && y == b) || (x == b && y == a))
    }

    /// How strongly a binding in `scope` claims a chord: lower wins, `None` does not run. An active
    /// context 0, the focused panel 1, a panel sharing with it 2, the application 3, any other
    /// panel 4 with fallthrough on. An inactive context never runs.
    // from effectcraft#1266 `scope_rank`
    pub fn rank(&self, scope: &Scope, active: &Active) -> Option<u8> {
        let name = scope.as_str();
        match self.kind_of(name) {
            ScopeKind::Context => active.contexts.contains(&name).then_some(0),
            ScopeKind::Application => Some(3),
            ScopeKind::Panel => match active.panel {
                Some(p) if p == name => Some(1),
                Some(p) if self.shares(p, name) => Some(2),
                _ => self.panel_fallthrough.then_some(4),
            },
        }
    }

    /// [`rank`](Self::rank) for a focus inside nested panels: `outer` lists the panels around the focused one,
    /// which claim a chord after it and before the panels that share with it. An active context 0, the
    /// focused panel 1, an enclosing panel 2, a panel sharing with the focused one 3, the application 4,
    /// any other panel 5 with fallthrough on.
    pub fn rank_nested(&self, scope: &Scope, active: &Active, outer: &[&str]) -> Option<u8> {
        let name = scope.as_str();
        match self.kind_of(name) {
            ScopeKind::Context => active.contexts.contains(&name).then_some(0),
            ScopeKind::Application => Some(4),
            ScopeKind::Panel => match active.panel {
                Some(p) if p == name => Some(1),
                _ if outer.contains(&name) => Some(2),
                Some(p) if self.shares(p, name) => Some(3),
                _ => self.panel_fallthrough.then_some(5),
            },
        }
    }

    /// Whether the same chord in `a` and `b` makes only one of them work: the same scope, two
    /// panels that share, or two different contexts (both can be active at once). Other pairs that
    /// can be active together shadow (`Some(false)`): a context over anything else, a panel over
    /// the application. `None`: never active together (two unrelated panels).
    pub(crate) fn relation(&self, a: &Scope, b: &Scope) -> Option<bool> {
        if a == b {
            return Some(true);
        }
        match (self.kind(a), self.kind(b)) {
            (ScopeKind::Context, ScopeKind::Context) => Some(true),
            (ScopeKind::Context, _) | (_, ScopeKind::Context) => Some(false),
            (ScopeKind::Application, _) | (_, ScopeKind::Application) => Some(false),
            (ScopeKind::Panel, ScopeKind::Panel) => self.shares(a.as_str(), b.as_str()).then_some(true),
        }
    }
}
