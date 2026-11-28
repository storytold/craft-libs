//! The agent settings an app saves in its preferences, and who decides that agents may connect.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The agent settings of an app's Preferences (its `integrations` section in PhotoCraft). Saved in
/// camelCase: `allowAgentControl`, `controlPort`, `readRoot`, `writeRoot`, `whenAppClosed`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct AgentSettings {
    /// Off refuses every control server, `--control` included.
    pub allow_agent_control: bool,
    /// The loopback port agents connect to; 0 is any free port, published in the connection file.
    pub control_port: u32,
    /// The folder agents may open from; empty is none. Applied at once while agents are connected.
    pub read_root: String,
    /// The folder agents may save to; empty is none. Applied at once while agents are connected.
    pub write_root: String,
    /// What the app's `<id>-cli mcp` does while agents may use the app but it is closed.
    pub when_app_closed: WhenAppClosed,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            allow_agent_control: true,
            control_port: 0,
            read_root: String::new(),
            write_root: String::new(),
            when_app_closed: WhenAppClosed::Launch,
        }
    }
}

impl AgentSettings {
    /// [`Self::read_root`] as a path ([`folder`]).
    pub fn read_folder(&self) -> Option<PathBuf> {
        folder(&self.read_root)
    }

    /// [`Self::write_root`] as a path ([`folder`]).
    pub fn write_folder(&self) -> Option<PathBuf> {
        folder(&self.write_root)
    }
}

/// A folder setting as a path, trimmed; empty or blank is `None`.
pub fn folder(pref: &str) -> Option<PathBuf> {
    let pref = pref.trim();
    (!pref.is_empty()).then(|| PathBuf::from(pref))
}

/// What an app's `<id>-cli mcp` does when an agent calls a tool while the app is closed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhenAppClosed {
    /// Open the app and wait for it.
    #[default]
    Launch,
    /// Run the tool without a window.
    Headless,
    /// Answer with an error that asks to open the app.
    Error,
}

impl WhenAppClosed {
    /// The saved names, in [`Self::ALL`] order.
    pub const NAMES: &'static [&'static str] = &["launch", "headless", "error"];
    /// Every choice, in the order a menu lists them.
    pub const ALL: [Self; 3] = [Self::Launch, Self::Headless, Self::Error];

    /// The saved name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Headless => "headless",
            Self::Error => "error",
        }
    }

    /// The choice a saved name means; `None` for any other text.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// Whether agents may use the open window. With the suite's `AI-MCP` flag only the launcher's
/// switch decides: one switch for every Craft app, none in the app. Without the flag the app's own
/// earlier consent decides (`legacy`; PhotoCraft's is a Control Port saved before the switch).
/// Allow Agent Control off still refuses agents either way ([`AgentSettings::allow_agent_control`]).
pub fn agents_allowed(mcp_flag: bool, launcher: bool, legacy: bool) -> bool {
    if mcp_flag { launcher } else { legacy }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    // The saved names read back as the same choice, and nothing else reads as one.
    #[test]
    fn when_app_closed_names_round_trip() {
        for choice in WhenAppClosed::ALL {
            assert_eq!(WhenAppClosed::parse(choice.name()), Some(choice));
            assert_eq!(serde_json::to_value(choice).unwrap(), serde_json::json!(choice.name()));
        }
        assert_eq!(WhenAppClosed::NAMES, WhenAppClosed::ALL.map(WhenAppClosed::name));
        assert_eq!(WhenAppClosed::parse("Launch"), None);
        let settings: AgentSettings = serde_json::from_str(r#"{"whenAppClosed": "headless", "controlPort": 7878}"#).unwrap();
        assert_eq!((settings.when_app_closed, settings.control_port, settings.allow_agent_control), (WhenAppClosed::Headless, 7878, true));
    }

    // A folder setting is trimmed; empty or blank means no folder.
    #[test]
    fn folder_preferences_are_trimmed() {
        assert_eq!(folder(""), None);
        assert_eq!(folder("   "), None);
        assert_eq!(folder(" /work/project "), Some(PathBuf::from("/work/project")));
        let settings = AgentSettings { read_root: " /in ".into(), ..AgentSettings::default() };
        assert_eq!((settings.read_folder(), settings.write_folder()), (Some(PathBuf::from("/in")), None));
    }
}
