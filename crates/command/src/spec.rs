//! Static command rows and the allocation-free view used by enumeration.

/// One built-in command. The nine fields match the per-app `CommandSpec` row.
///
/// `enabled` is `fn(&S)`. FilmCraft and EffectCraft already apply named params
/// in `enabled_for` / `execute` by standing the named targets in for the
/// selection. Putting `&Value` on this function pointer would change every
/// enablement function in both apps.
pub struct CommandSpec<S, E> {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static [&'static str],
    pub shortcut: Option<&'static str>,
    /// Further default shortcuts. Menus and tooltips show `shortcut` only.
    pub alt_shortcuts: &'static [&'static str],
    pub params: &'static str,
    pub enabled: fn(&S) -> Result<(), String>,
    pub run: fn(&mut S, &serde_json::Value) -> Result<serde_json::Value, E>,
    pub journal: bool,
}

/// Menu path borrowed from a static row or from a dynamic command's `Arc`s.
///
/// `for_each` builds this view without allocating.
pub enum Menu<'a> {
    Static(&'a [&'static str]),
    Shared(&'a [std::sync::Arc<str>]),
}

/// Borrowed command metadata. `dynamic` is false for built-in rows.
pub struct CommandView<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub shortcut: Option<&'a str>,
    /// Empty for a dynamic command.
    pub alt_shortcuts: &'a [&'static str],
    pub params: &'a str,
    pub journal: bool,
    pub dynamic: bool,
    pub menu: Menu<'a>,
}
