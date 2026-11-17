//! Which commands a plug-in may call: the manifest's allowlist and access, the refused list every
//! app shares, and the app's own refused list and exceptions.
//!
//! [`check_call`] runs before every call. It can't know whether a command edits the document:
//! registries differ in what they record (a `history` flag, `mutates`, a journal, nothing). So a
//! plug-in with `read` access is enforced in two places by the app:
//!
//! 1. **Before the call**, when the app's registry knows the command edits:
//!    `check_read_only(manifest, command, true)` refuses it without running it.
//! 2. **After the call**, always: the app compares its document before and after (an undo-history
//!    length, a revision counter, or a pointer to an immutable document), and when it changed,
//!    undoes the call and returns `check_read_only(manifest, command, true)`'s refusal to the plug-in
//!    as the call's error. Run each call as its own nested undo group so this undo removes exactly
//!    that call.
//!
//! Whatever a manifest says, a plug-in never gets a path, the disk, the network, the clock, other
//! documents, the undo history or another plug-in: commands that would give it one are on the
//! refused lists, and parameters that name a file are the app's to refuse (FigCraft refuses
//! `path` on its import commands, for example).

use crate::manifest::{Access, Manifest};

/// Commands no plug-in may call in any app: plug-ins themselves, the undo history, files, the
/// app, preferences and windows. Patterns as in [`pattern_matches`].
pub const ALWAYS_DENIED: &[&str] = &["plugin.*", "plugins.*", "edit.undo", "edit.redo", "file.*", "app.*", "pref.*", "window.*"];

/// Whether `pattern` admits `command`: an exact id, or a prefix ending in `*` (`node.*` admits
/// `node.rename`; `library.add*` admits `library.add` and `library.addAll`). A lone `*` admits
/// everything.
pub fn pattern_matches(pattern: &str, command: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => command.starts_with(prefix),
        None => pattern == command,
    }
}

/// An app's own rules, on top of [`ALWAYS_DENIED`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    /// The app's own refused commands (patterns as in [`pattern_matches`]): its document list,
    /// version history, network sessions, nested batches…
    pub denied: Vec<String>,
    /// Exact command ids allowed although [`Policy::denied`] matches them (FigCraft's `doc.list` and
    /// `version.list`, which only list). Exceptions never override [`ALWAYS_DENIED`].
    pub exceptions: Vec<String>,
}

impl Policy {
    /// A policy from the app's own refused patterns and exceptions.
    pub fn new(denied: &[&str], exceptions: &[&str]) -> Self {
        Policy { denied: denied.iter().map(|s| s.to_string()).collect(), exceptions: exceptions.iter().map(|s| s.to_string()).collect() }
    }

    /// Whether no plug-in may call `command` in this app, whatever its manifest says.
    pub fn refuses(&self, command: &str) -> bool {
        ALWAYS_DENIED.iter().any(|p| pattern_matches(p, command))
            || (self.denied.iter().any(|p| pattern_matches(p, command)) && !self.exceptions.iter().any(|e| e == command))
    }
}

/// Why `manifest`'s plug-in may not call `command` in an app with `policy`, if it may not: a
/// command no plug-in may call, an empty or malformed id, or one outside the manifest's
/// `permissions.commands`. Run it before every call; read access is the app's to enforce too
/// (see the module docs and [`check_read_only`]).
pub fn check_call(manifest: &Manifest, policy: &Policy, command: &str) -> Result<(), String> {
    let shown: String = command.chars().take(80).collect();
    if command.is_empty() || command.len() > 128 || command.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(format!("`{shown}` isn't a command id"));
    }
    if policy.refuses(command) {
        return Err(format!("`{shown}` isn't available to plugins"));
    }
    if !manifest.permissions.allows(command) {
        return Err(format!("`{shown}` isn't in the commands {} asked permission for", manifest.name));
    }
    Ok(())
}

/// Why a plug-in may not run `command` when it `edits` the document, if it may not: its manifest
/// asks only for `read` access. The app calls it before a call when its registry knows the command
/// edits, and after every call with whether the document changed (undoing the call first when it
/// returns an error).
pub fn check_read_only(manifest: &Manifest, command: &str, edits: bool) -> Result<(), String> {
    if edits && manifest.permissions.access == Access::Read {
        let shown: String = command.chars().take(80).collect();
        return Err(format!("`{shown}` edits the document, and {} may only read it", manifest.name));
    }
    Ok(())
}

/// Whether an update from `old` to `new` asks for different permissions or is another kind, so
/// the install confirmation must be shown again.
pub fn changed(old: &Manifest, new: &Manifest) -> bool {
    old.permissions != new.permissions || old.kind != new.kind
}
