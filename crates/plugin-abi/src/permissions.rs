//! Permission names from `ABI.md` section 5. Default deny: a plugin holds only what its manifest lists.

use std::fmt;
use std::str::FromStr;

/// Doc-read scope in `doc.read[:metadata|pixels]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DocScope {
    Metadata,
    Pixels,
}

/// One parsed permission.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    /// `commands.run` or `commands.run:<glob>`. `None` means every command.
    CommandsRun(Option<String>),
    CommandsObserve,
    CommandsIntercept,
    /// `doc.read` or `doc.read:<scope>`. `None` means every scope.
    DocRead(Option<DocScope>),
    DocWrite,
    UiContribute,
    StateLocal,
    /// `link.call:<app>`. Lets the plugin send the read-only methods of [`LINK_CALL_METHODS`] to the app.
    LinkCall(String),
    /// `link.exec:<app>:<command-glob>`. Lets the plugin run the commands of `app` that match the
    /// glob, with the rights of that app. Needs `link.call:<app>` too.
    LinkExec {
        app: String,
        glob: String,
    },
    FsPicked,
    /// `net.fetch:<origin>`. The origin starts with `https://`.
    NetFetch(String),
    Clock,
    Random,
    /// Reserved for post-v1 `frames.*` ops. Parses so manifests can ask for it.
    FramesRead,
}

/// The permission name without its argument. Ops map to a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    CommandsRun,
    CommandsObserve,
    CommandsIntercept,
    DocRead,
    DocWrite,
    UiContribute,
    StateLocal,
    LinkCall,
    LinkExec,
    FsPicked,
    NetFetch,
    Clock,
    Random,
    FramesRead,
}

/// How loudly the install prompt shows a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptLevel {
    Low,
    Normal,
    High,
    /// Asked when the plugin first uses it, not at install.
    OnFirstUse,
}

/// A permission string that failed to parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionError {
    pub input: String,
    pub reason: &'static str,
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid permission {:?}: {}", self.input, self.reason)
    }
}

impl std::error::Error for PermissionError {}

fn is_glob_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '*' | '-')
}

fn is_app_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Permission {
    pub fn kind(&self) -> PermissionKind {
        match self {
            Permission::CommandsRun(_) => PermissionKind::CommandsRun,
            Permission::CommandsObserve => PermissionKind::CommandsObserve,
            Permission::CommandsIntercept => PermissionKind::CommandsIntercept,
            Permission::DocRead(_) => PermissionKind::DocRead,
            Permission::DocWrite => PermissionKind::DocWrite,
            Permission::UiContribute => PermissionKind::UiContribute,
            Permission::StateLocal => PermissionKind::StateLocal,
            Permission::LinkCall(_) => PermissionKind::LinkCall,
            Permission::LinkExec { .. } => PermissionKind::LinkExec,
            Permission::FsPicked => PermissionKind::FsPicked,
            Permission::NetFetch(_) => PermissionKind::NetFetch,
            Permission::Clock => PermissionKind::Clock,
            Permission::Random => PermissionKind::Random,
            Permission::FramesRead => PermissionKind::FramesRead,
        }
    }

    pub fn prompt_level(&self) -> PromptLevel {
        match self {
            Permission::CommandsRun(_) | Permission::CommandsObserve | Permission::DocRead(_) | Permission::FramesRead => PromptLevel::Normal,
            Permission::CommandsIntercept
            | Permission::DocWrite
            | Permission::LinkCall(_)
            | Permission::LinkExec { .. }
            | Permission::NetFetch(_) => PromptLevel::High,
            Permission::UiContribute | Permission::StateLocal | Permission::Clock | Permission::Random => PromptLevel::Low,
            Permission::FsPicked => PromptLevel::OnFirstUse,
        }
    }

    /// True when this grant lets the plugin run `command_id`. Only `commands.run` grants commands.
    /// No glob reaches a host command id (see [`plugin_may_name_command`]), not even a bare `commands.run`.
    pub fn allows_command(&self, command_id: &str) -> bool {
        if !plugin_may_name_command(command_id) {
            return false;
        }
        match self {
            Permission::CommandsRun(None) => true,
            Permission::CommandsRun(Some(glob)) => glob_match(glob, command_id),
            _ => false,
        }
    }

    /// True when this grant lets the plugin call `app` through Craft Link.
    pub fn allows_link(&self, app: &str) -> bool {
        matches!(self, Permission::LinkCall(a) if a == app)
    }

    /// True when this grant lets the plugin run `command_id` in `app` through `link.exec`.
    /// The same host-command rule as [`Permission::allows_command`] applies.
    pub fn allows_link_exec(&self, app: &str, command_id: &str) -> bool {
        matches!(self, Permission::LinkExec { app: a, glob } if a == app && plugin_may_name_command(command_id) && glob_match(glob, command_id))
    }

    /// True when this grant lets the plugin read at `scope`. A bare `doc.read` covers both scopes.
    pub fn allows_doc_read(&self, scope: DocScope) -> bool {
        matches!(self, Permission::DocRead(None)) || matches!(self, Permission::DocRead(Some(s)) if *s == scope)
    }

    /// True when this grant lets the plugin fetch from `origin` (exact match on the granted origin).
    pub fn allows_origin(&self, origin: &str) -> bool {
        matches!(self, Permission::NetFetch(o) if o == origin)
    }
}

/// Command ids that belong to the host application and never to a plugin. Today this is the
/// plugin-management family `plugin.app.<app>.<verb>` (install, grant, enable, remove, revoke).
/// Compared without regard to ASCII case.
pub const HOST_COMMAND_PREFIXES: [&str; 1] = ["plugin.app."];

/// Longest command id a plugin may name.
pub const MAX_COMMAND_ID_BYTES: usize = 256;

/// Methods that `link.call:<app>` may forward. Every one is read-only or only moves focus, and
/// none runs a command, writes a path or touches plugin management. The list is closed: a method
/// not named here is refused.
pub const LINK_CALL_METHODS: [&str; 4] = ["link.resolve", "asset.describe", "link.refresh", "link.focus"];

/// The one method `link.exec:<app>:<glob>` adds. The host rebuilds its params from the command id
/// and the command params, so no other key reaches the peer.
pub const LINK_EXEC_METHOD: &str = "engine.execute";

/// True when `method` is one that `link.call:<app>` alone may forward. Exact, case-sensitive match.
pub fn is_link_call_method(method: &str) -> bool {
    LINK_CALL_METHODS.contains(&method)
}

/// True when `id` starts with a host command prefix, ignoring ASCII case.
pub fn is_host_command_id(id: &str) -> bool {
    HOST_COMMAND_PREFIXES.iter().any(|p| id.as_bytes().get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p.as_bytes())))
}

/// The one rule every plugin route applies before it runs a command id: `cmd.execute`, `cmd.list`
/// and `link.exec`. The id must be non-empty printable ASCII (no spaces, controls, or lookalike
/// Unicode that a peer might fold to a host id) and must not start with a host command prefix.
pub fn plugin_may_name_command(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_COMMAND_ID_BYTES && id.bytes().all(|b| b.is_ascii_graphic()) && !is_host_command_id(id)
}

/// Glob match where `*` matches any run of characters (including none). No other metacharacters.
pub fn glob_match(glob: &str, text: &str) -> bool {
    let g: Vec<char> = glob.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut gi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if gi < g.len() && g[gi] == '*' {
            star = Some((gi, ti));
            gi += 1;
        } else if gi < g.len() && g[gi] == t[ti] {
            gi += 1;
            ti += 1;
        } else if let Some((sg, st)) = star {
            gi = sg + 1;
            ti = st + 1;
            star = Some((sg, st + 1));
        } else {
            return false;
        }
    }
    while gi < g.len() && g[gi] == '*' {
        gi += 1;
    }
    gi == g.len()
}

impl FromStr for Permission {
    type Err = PermissionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |reason| PermissionError { input: s.to_owned(), reason };
        let (name, arg) = match s.split_once(':') {
            Some((n, a)) => (n, Some(a)),
            None => (s, None),
        };
        match (name, arg) {
            ("commands.run", None) => Ok(Permission::CommandsRun(None)),
            ("commands.run", Some(g)) if !g.is_empty() && g.chars().all(is_glob_char) => Ok(Permission::CommandsRun(Some(g.to_owned()))),
            ("commands.run", Some(_)) => Err(err("glob must be non-empty and use [A-Za-z0-9._*-]")),
            ("commands.observe", None) => Ok(Permission::CommandsObserve),
            ("commands.intercept", None) => Ok(Permission::CommandsIntercept),
            ("doc.read", None) => Ok(Permission::DocRead(None)),
            ("doc.read", Some("metadata")) => Ok(Permission::DocRead(Some(DocScope::Metadata))),
            ("doc.read", Some("pixels")) => Ok(Permission::DocRead(Some(DocScope::Pixels))),
            ("doc.read", Some(_)) => Err(err("doc.read scope must be metadata or pixels")),
            ("doc.write", None) => Ok(Permission::DocWrite),
            ("ui.contribute", None) => Ok(Permission::UiContribute),
            ("state.local", None) => Ok(Permission::StateLocal),
            ("link.call", Some(app)) if is_app_name(app) => Ok(Permission::LinkCall(app.to_owned())),
            ("link.call", _) => Err(err("link.call needs an app name [a-z][a-z0-9-]*")),
            ("link.exec", Some(rest)) => match rest.split_once(':') {
                Some((app, glob)) if is_app_name(app) && !glob.is_empty() && glob.chars().all(is_glob_char) => {
                    Ok(Permission::LinkExec { app: app.to_owned(), glob: glob.to_owned() })
                }
                _ => Err(err("link.exec needs <app>:<command-glob>, glob in [A-Za-z0-9._*-]")),
            },
            ("link.exec", None) => Err(err("link.exec needs <app>:<command-glob>")),
            ("fs.picked", None) => Ok(Permission::FsPicked),
            ("net.fetch", Some(origin)) => match origin.strip_prefix("https://") {
                Some(rest) if !rest.is_empty() && !origin.chars().any(char::is_whitespace) => Ok(Permission::NetFetch(origin.to_owned())),
                _ => Err(err("net.fetch needs an https:// origin without whitespace")),
            },
            ("net.fetch", None) => Err(err("net.fetch needs an origin")),
            ("clock", None) => Ok(Permission::Clock),
            ("random", None) => Ok(Permission::Random),
            ("frames.read", None) => Ok(Permission::FramesRead),
            (
                "commands.observe" | "commands.intercept" | "doc.write" | "ui.contribute" | "state.local" | "fs.picked" | "clock" | "random"
                | "frames.read",
                Some(_),
            ) => Err(err("this permission takes no argument")),
            _ => Err(err("unknown permission name")),
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Permission::CommandsRun(None) => f.write_str("commands.run"),
            Permission::CommandsRun(Some(g)) => write!(f, "commands.run:{g}"),
            Permission::CommandsObserve => f.write_str("commands.observe"),
            Permission::CommandsIntercept => f.write_str("commands.intercept"),
            Permission::DocRead(None) => f.write_str("doc.read"),
            Permission::DocRead(Some(DocScope::Metadata)) => f.write_str("doc.read:metadata"),
            Permission::DocRead(Some(DocScope::Pixels)) => f.write_str("doc.read:pixels"),
            Permission::DocWrite => f.write_str("doc.write"),
            Permission::UiContribute => f.write_str("ui.contribute"),
            Permission::StateLocal => f.write_str("state.local"),
            Permission::LinkCall(a) => write!(f, "link.call:{a}"),
            Permission::LinkExec { app, glob } => write!(f, "link.exec:{app}:{glob}"),
            Permission::FsPicked => f.write_str("fs.picked"),
            Permission::NetFetch(o) => write!(f, "net.fetch:{o}"),
            Permission::Clock => f.write_str("clock"),
            Permission::Random => f.write_str("random"),
            Permission::FramesRead => f.write_str("frames.read"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const ALL: &[&str] = &[
        "commands.run",
        "commands.run:effect.*",
        "commands.observe",
        "commands.intercept",
        "doc.read",
        "doc.read:metadata",
        "doc.read:pixels",
        "doc.write",
        "ui.contribute",
        "state.local",
        "link.call:filmcraft",
        "link.exec:filmcraft:effect.*",
        "fs.picked",
        "net.fetch:https://example.org",
        "clock",
        "random",
        "frames.read",
    ];

    #[test]
    fn every_listed_name_round_trips() {
        for s in ALL {
            let p: Permission = s.parse().unwrap();
            assert_eq!(p.to_string(), *s);
        }
    }

    #[test]
    fn rejects_unknown_and_malformed() {
        for s in [
            "",
            "commands",
            "commands.exec",
            "doc.read:all",
            "doc.write:x",
            "clock:now",
            "link.call",
            "link.call:",
            "link.call:Film",
            "link.call:1app",
            "link.exec",
            "link.exec:",
            "link.exec:filmcraft",
            "link.exec:filmcraft:",
            "link.exec::effect.*",
            "link.exec:Film:effect.*",
            "link.exec:filmcraft:a b",
            "link.exec:filmcraft:a/b",
            "link.exec:filmcraft:\u{ff10}*",
            "net.fetch",
            "net.fetch:http://example.org",
            "net.fetch:https://",
            "net.fetch:https://a b",
            "commands.run:",
            "commands.run:a b",
            "commands.run:a/b",
            "COMMANDS.RUN",
        ] {
            assert!(s.parse::<Permission>().is_err(), "{s:?} must be rejected");
        }
    }

    #[test]
    fn command_globs_scope_the_grant() {
        let p: Permission = "commands.run:effect.*".parse().unwrap();
        assert!(p.allows_command("effect.add"));
        assert!(p.allows_command("effect."));
        assert!(!p.allows_command("project.save"));
        assert!(!p.allows_command("xeffect.add"));
        let any: Permission = "commands.run".parse().unwrap();
        assert!(any.allows_command("anything"));
        assert!(!Permission::CommandsObserve.allows_command("effect.add"));
    }

    #[test]
    fn glob_handles_backtracking_and_edges() {
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "aXbYbZc"));
        assert!(glob_match("a*b", "abab"));
        assert!(!glob_match("a*b", "abac"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exactly"));
        assert!(!glob_match("", "x"));
        assert!(glob_match("", ""));
    }

    #[test]
    fn scoped_grants_check_their_argument() {
        let link: Permission = "link.call:filmcraft".parse().unwrap();
        assert!(link.allows_link("filmcraft"));
        assert!(!link.allows_link("photocraft"));
        let meta: Permission = "doc.read:metadata".parse().unwrap();
        assert!(meta.allows_doc_read(DocScope::Metadata));
        assert!(!meta.allows_doc_read(DocScope::Pixels));
        assert!(Permission::DocRead(None).allows_doc_read(DocScope::Pixels));
        let net: Permission = "net.fetch:https://example.org".parse().unwrap();
        assert!(net.allows_origin("https://example.org"));
        assert!(!net.allows_origin("https://evil.example.org"));
    }

    #[test]
    fn prompt_levels_match_the_abi_table() {
        let level = |s: &str| s.parse::<Permission>().unwrap().prompt_level();
        assert_eq!(level("commands.intercept"), PromptLevel::High);
        assert_eq!(level("doc.write"), PromptLevel::High);
        assert_eq!(level("link.call:a"), PromptLevel::High);
        assert_eq!(level("link.exec:a:x.*"), PromptLevel::High);
        assert_eq!(level("net.fetch:https://x"), PromptLevel::High);
        assert_eq!(level("commands.run"), PromptLevel::Normal);
        assert_eq!(level("commands.observe"), PromptLevel::Normal);
        assert_eq!(level("doc.read"), PromptLevel::Normal);
        assert_eq!(level("ui.contribute"), PromptLevel::Low);
        assert_eq!(level("state.local"), PromptLevel::Low);
        assert_eq!(level("clock"), PromptLevel::Low);
        assert_eq!(level("random"), PromptLevel::Low);
        assert_eq!(level("fs.picked"), PromptLevel::OnFirstUse);
    }

    #[test]
    fn no_command_grant_reaches_a_host_command_id() {
        let any: Permission = "commands.run".parse().unwrap();
        let star: Permission = "commands.run:*".parse().unwrap();
        let host: Permission = "commands.run:plugin.app.*".parse().unwrap();
        let exec: Permission = "link.exec:filmcraft:*".parse().unwrap();
        for id in ["plugin.app.filmcraft.install", "plugin.app.filmcraft.grant", "PLUGIN.APP.filmcraft.install", "Plugin.App.x", "plugin.app."] {
            assert!(!any.allows_command(id), "{id}");
            assert!(!star.allows_command(id), "{id}");
            assert!(!host.allows_command(id), "{id}");
            assert!(!exec.allows_link_exec("filmcraft", id), "{id}");
        }
        // Near misses are ordinary ids.
        for id in ["plugin.apps.x", "plugin.app", "plugin.ap.x", "xplugin.app.x", "plugin.org.app.x"] {
            assert!(any.allows_command(id), "{id}");
            assert!(exec.allows_link_exec("filmcraft", id), "{id}");
        }
    }

    #[test]
    fn odd_command_ids_are_refused_on_every_route() {
        let any: Permission = "commands.run".parse().unwrap();
        let exec: Permission = "link.exec:filmcraft:*".parse().unwrap();
        for id in [
            "",
            " ",
            " plugin.app.filmcraft.install",
            "plugin.app.filmcraft.install ",
            "plugin.app.filmcraft.install\n",
            "plugin.app.filmcraft.install\0",
            "plugin.\u{ff41}pp.filmcraft.install",
            "plug\u{131}n.app.x",
            "\u{212a}ey",
            "a\u{200b}b",
        ] {
            assert!(!any.allows_command(id), "{id:?}");
            assert!(!exec.allows_link_exec("filmcraft", id), "{id:?}");
        }
        assert!(!any.allows_command(&"a".repeat(MAX_COMMAND_ID_BYTES + 1)));
        assert!(any.allows_command(&"a".repeat(MAX_COMMAND_ID_BYTES)));
        // The prefix test must not panic on a multi-byte char that straddles the prefix length.
        assert!(!is_host_command_id("plugin.ap\u{e9}"));
    }

    #[test]
    fn link_exec_is_scoped_to_its_app_and_glob() {
        let p: Permission = "link.exec:filmcraft:effect.*".parse().unwrap();
        assert!(p.allows_link_exec("filmcraft", "effect.add"));
        assert!(!p.allows_link_exec("effectcraft", "effect.add"));
        assert!(!p.allows_link_exec("filmcraft", "project.save"));
        assert!(!p.allows_link_exec("filmcraft", "xeffect.add"));
        // `link.exec` grants nothing else.
        assert!(!p.allows_command("effect.add"));
        assert!(!p.allows_link("filmcraft"));
        let call: Permission = "link.call:filmcraft".parse().unwrap();
        assert!(!call.allows_link_exec("filmcraft", "effect.add"));
        // A glob has no escape syntax. Only `*` is special.
        assert!("link.exec:filmcraft:a\\*".parse::<Permission>().is_err());
        assert!("link.exec:filmcraft:a[b]".parse::<Permission>().is_err());
        assert!("link.exec:filmcraft:a?".parse::<Permission>().is_err());
    }

    #[test]
    fn link_call_methods_are_a_closed_exact_list() {
        for m in LINK_CALL_METHODS {
            assert!(is_link_call_method(m));
        }
        for m in [
            "engine.execute",
            "engine.commands",
            "ui.menu.invoke",
            "ui.screenshot",
            "render.frame",
            "app.quit",
            "link.open",
            "link.attach",
            "link.close",
            "perf.stats",
            "plugin.app.filmcraft.install",
            "Link.Resolve",
            "link.resolve ",
            "link.resolve\0",
            "link.",
            "link",
            "",
        ] {
            assert!(!is_link_call_method(m), "{m:?}");
        }
        assert!(!is_link_call_method(LINK_EXEC_METHOD));
    }
}
