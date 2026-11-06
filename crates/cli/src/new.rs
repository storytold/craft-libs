//! `craft new`: write a plugin package from the templates in `templates/`.
//!
//! The package is a Cargo crate (`cdylib` + `rlib`) with `craft.toml`, `src/lib.rs`, a
//! `.cargo/config.toml` that sets the wasm memory maximum the gate demands, and the SDK as a path
//! dependency. A `Cargo.lock` is copied from the SDK's workspace, so the plugin builds against the
//! dependency versions the suite already tested.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::error::{CliError, Result, io_at};

const CARGO_TOML: &str = include_str!("../templates/common/Cargo.toml.tmpl");
const CARGO_CONFIG: &str = include_str!("../templates/common/cargo-config.toml");
const GITIGNORE: &str = include_str!("../templates/common/gitignore");
const CRAFT_TOML: &str = include_str!("../templates/common/craft.toml");
const LIB_RS: &str = include_str!("../templates/common/lib.rs");

/// The contribution kinds a template can produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplateKind {
    Command,
    Hook,
    Effect,
    Panel,
}

struct Fragment {
    toml: &'static str,
    handlers: &'static str,
    tests: &'static str,
    ops: &'static [&'static str],
    permissions: &'static [&'static str],
}

impl TemplateKind {
    pub const ALL: [TemplateKind; 4] = [TemplateKind::Command, TemplateKind::Hook, TemplateKind::Effect, TemplateKind::Panel];

    pub fn name(self) -> &'static str {
        match self {
            TemplateKind::Command => "command",
            TemplateKind::Hook => "hook",
            TemplateKind::Effect => "effect",
            TemplateKind::Panel => "panel",
        }
    }

    fn fragment(self) -> Fragment {
        match self {
            TemplateKind::Command => Fragment {
                toml: include_str!("../templates/kinds/command.toml"),
                handlers: include_str!("../templates/kinds/command.rs"),
                tests: include_str!("../templates/kinds/command.test.rs"),
                ops: &["host.log"],
                permissions: &[],
            },
            TemplateKind::Hook => Fragment {
                toml: include_str!("../templates/kinds/hook.toml"),
                handlers: include_str!("../templates/kinds/hook.rs"),
                tests: include_str!("../templates/kinds/hook.test.rs"),
                ops: &["host.log"],
                permissions: &["commands.observe"],
            },
            TemplateKind::Effect => Fragment {
                toml: include_str!("../templates/kinds/effect.toml"),
                handlers: include_str!("../templates/kinds/effect.rs"),
                tests: include_str!("../templates/kinds/effect.test.rs"),
                ops: &[],
                permissions: &[],
            },
            TemplateKind::Panel => Fragment {
                toml: include_str!("../templates/kinds/panel.toml"),
                handlers: include_str!("../templates/kinds/panel.rs"),
                tests: include_str!("../templates/kinds/panel.test.rs"),
                ops: &["host.log", "state.get", "state.put"],
                permissions: &["ui.contribute", "state.local"],
            },
        }
    }
}

impl fmt::Display for TemplateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for TemplateKind {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self> {
        TemplateKind::ALL
            .into_iter()
            .find(|k| k.name() == s)
            .ok_or_else(|| CliError::new(format!("unknown template kind {s:?}; the kinds are command, hook, effect, panel")))
    }
}

/// Parse `command,panel`.
pub fn parse_kinds(list: &str) -> Result<Vec<TemplateKind>> {
    let mut kinds = Vec::new();
    for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let kind: TemplateKind = part.parse()?;
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    if kinds.is_empty() {
        return Err(CliError::new("--kind needs at least one kind: command, hook, effect, panel"));
    }
    Ok(kinds)
}

/// Replace every `{{key}}`. A marker with no value is an error, so a template typo cannot ship.
fn render(template: &str, values: &[(&str, &str)]) -> Result<String> {
    let mut out = template.to_owned();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    if let Some(at) = out.find("{{") {
        let tail: String = out[at..].chars().take(24).collect();
        return Err(CliError::new(format!("template marker with no value: {tail}")));
    }
    Ok(out)
}

/// A package name is a crate name that reads well as a file name.
pub fn check_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && name.len() <= 40
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(CliError::new(format!("plugin name {name:?} must be lowercase letters, digits and '-', start with a letter, at most 40 characters")))
    }
}

pub struct NewOptions {
    pub name: String,
    /// Parent directory. The package goes to `<parent>/<name>`.
    pub parent: PathBuf,
    pub kinds: Vec<TemplateKind>,
    /// Manifest id. Default `dev.local.<name>`.
    pub plugin_id: Option<String>,
    /// Path to `crates/plugin-sdk`. Default: the checkout this `craft` was built from.
    pub sdk_path: Option<PathBuf>,
}

/// The SDK of the craft-libs checkout this binary was built from. This default is for developers who
/// run `cargo run -p craft-cli` or a binary they built in the checkout. The path is fixed at build
/// time, so a copied or installed `craft` must be given `--sdk-path`.
fn default_sdk_path() -> Result<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugin-sdk");
    path.canonicalize().map_err(|_| {
        CliError::new(format!(
            "the SDK is not at {} (the craft-libs checkout this craft was built from moved or is gone); pass --sdk-path <crates/plugin-sdk>",
            path.display()
        ))
    })
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_at(dir, e))?;
    }
    std::fs::write(path, text).map_err(|e| io_at(path, e))
}

fn toml_list(items: impl IntoIterator<Item = String>) -> String {
    items.into_iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ")
}

/// Create the package. Returns its directory. Fails if the directory exists.
pub fn create(opts: &NewOptions) -> Result<PathBuf> {
    check_name(&opts.name)?;
    let dir = opts.parent.join(&opts.name);
    if dir.exists() {
        return Err(CliError::new(format!("{} already exists; pick another name or remove it", dir.display())));
    }
    let sdk = match &opts.sdk_path {
        Some(p) => p.canonicalize().map_err(|e| io_at(p, e))?,
        None => default_sdk_path()?,
    };
    if !sdk.join("Cargo.toml").is_file() {
        return Err(CliError::new(format!("{} has no Cargo.toml; --sdk-path must be the crates/plugin-sdk directory", sdk.display())));
    }
    let plugin_id = opts.plugin_id.clone().unwrap_or_else(|| format!("dev.local.{}", opts.name));

    let mut ops: Vec<&str> = Vec::new();
    let mut permissions: Vec<&str> = Vec::new();
    let (mut contributions, mut handlers, mut tests) = (String::new(), String::new(), String::new());
    for kind in &opts.kinds {
        let f = kind.fragment();
        for op in f.ops {
            if !ops.contains(op) {
                ops.push(op);
            }
        }
        for p in f.permissions {
            if !permissions.contains(p) {
                permissions.push(p);
            }
        }
        contributions.push_str(f.toml);
        handlers.push_str(f.handlers);
        tests.push_str(f.tests);
    }
    let permissions_toml = if permissions.is_empty() {
        String::new()
    } else {
        format!(
            "\n[permissions]\n# Shown to the user at install. Anything not listed is denied.\nrequired = [{}]\n",
            toml_list(permissions.iter().map(|s| (*s).to_owned()))
        )
    };

    let sdk_text = sdk.to_string_lossy().into_owned();
    write(&dir.join("Cargo.toml"), &render(CARGO_TOML, &[("name", &opts.name), ("sdk_path", &sdk_text)])?)?;
    write(&dir.join(".cargo/config.toml"), CARGO_CONFIG)?;
    write(&dir.join(".gitignore"), GITIGNORE)?;
    write(
        &dir.join("craft.toml"),
        &render(
            CRAFT_TOML,
            &[
                ("plugin_id", &plugin_id),
                ("name", &opts.name),
                ("ops", &toml_list(ops.iter().map(|s| (*s).to_owned()))),
                ("permissions", &permissions_toml),
                ("contributions", &contributions),
            ],
        )?,
    )?;
    write(&dir.join("src/lib.rs"), &render(LIB_RS, &[("name", &opts.name), ("handlers", &handlers), ("tests", &tests)])?)?;

    // Pin dependencies to the versions the suite built with. cargo prunes what the plugin does not use.
    if let Some(lock) = sdk.ancestors().map(|a| a.join("Cargo.lock")).find(|p| p.is_file()) {
        std::fs::copy(&lock, dir.join("Cargo.lock")).map_err(|e| io_at(&lock, e))?;
    }
    Ok(dir)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn render_replaces_every_marker_and_fails_on_a_leftover() {
        assert_eq!(render("a {{x}} b {{x}} {{y}}", &[("x", "1"), ("y", "2")]).unwrap(), "a 1 b 1 2");
        let err = render("a {{x}} {{typo}}", &[("x", "1")]).unwrap_err().to_string();
        assert!(err.contains("{{typo}}"), "{err}");
    }

    #[test]
    fn every_embedded_template_renders_with_the_values_create_gives_it() {
        // `create` supplies exactly these keys. A marker added to a template without a value fails here.
        let common = [
            ("name", "n"),
            ("sdk_path", "/s"),
            ("plugin_id", "i"),
            ("ops", ""),
            ("permissions", ""),
            ("contributions", ""),
            ("handlers", ""),
            ("tests", ""),
        ];
        for template in [CARGO_TOML, CRAFT_TOML, LIB_RS] {
            render(template, &common).unwrap();
        }
        for kind in TemplateKind::ALL {
            let f = kind.fragment();
            for part in [f.toml, f.handlers, f.tests] {
                assert!(!part.contains("{{"), "{kind} fragment has a marker");
            }
        }
    }

    #[test]
    fn names_follow_the_rule() {
        for ok in ["hello", "a", "my-plugin-2"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Hello", "9a", "a_b", "a-", "-a", "a b", &"x".repeat(41)] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn kinds_parse_dedupe_and_reject_unknowns() {
        assert_eq!(parse_kinds("command, panel,command").unwrap(), [TemplateKind::Command, TemplateKind::Panel]);
        assert!(parse_kinds("").is_err());
        assert!(parse_kinds("command,widget").is_err());
    }

    #[test]
    fn each_kind_declares_the_ops_and_permissions_its_handlers_use() {
        // A handler that calls an op the manifest does not list gets E_DENIED at run time, so the template must list it.
        for kind in TemplateKind::ALL {
            let f = kind.fragment();
            for (needle, op) in [("host::info", "host.log"), ("host::error", "host.log"), ("state::get", "state.get"), ("state::put", "state.put")] {
                if f.handlers.contains(needle) {
                    assert!(f.ops.contains(&op), "{kind}: uses {needle} but does not list {op}");
                }
            }
        }
        assert!(TemplateKind::Hook.fragment().permissions.contains(&"commands.observe"));
        assert!(TemplateKind::Panel.fragment().permissions.contains(&"state.local"));
    }
}
