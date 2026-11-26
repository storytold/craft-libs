//! AI agents in the open window: whether the person lets agents use the app, and how an agent's
//! client finds the app's control server without being told.
//!
//! One switch serves every Craft app: the ArtCraft Launcher's *Settings › AI agents › Let AI agents
//! use Craft apps*, which writes the launcher's app list (`craft_ai_agents`). With the suite's
//! `AI-MCP` flag on, an app lets agents in when that list names it ([`AgentSwitch`]); without the
//! flag, the app's own earlier consent decides ([`agents_allowed`]).
//!
//! While agents may connect, the app serves its control channel on loopback ([`AgentLink`], over
//! [`crate::server`]) and publishes where in its settings folder ([`CONNECTION_FILE`]): the port,
//! its process id and the token file ([`TOKEN_FILE`], created `0600` on Unix). The app's
//! `<id>-cli mcp` reads that file on every tool call ([`Live`]), then the token from the file it
//! names, so an MCP client's configuration never holds a port or a token.
//!
//! The file holds no secret. A crashed (or, on macOS, quit) app can leave it behind; nothing then
//! listens on its port ([`probe`]), and the app's next launch removes it
//! ([`remove_stale_connection`]). Until then another program could take that port, so a client
//! checks that [`Connection::pid`] is still running before it sends anything derived from the
//! token, and authenticates with the challenge handshake ([`crate::security::ServerHandshake`]).
//!
//! Everything but the link host is `craft_control::agents`, re-exported here: apps that don't use
//! the shared UI library use it from there, with the same files and formats.
//!
//! | Part | Names | Builds |
//! |---|---|---|
//! | Settings | [`AgentSettings`], [`WhenAppClosed`], [`agents_allowed`], [`folder`] | every target |
//! | The launcher's switch | [`AgentSwitch`], [`mcp_flag`], [`launcher_allows`], [`POLL`] | desktop, feature `agents` |
//! | The connection file | [`Connection`], [`write_connection`], [`read_connection`], [`remove_connection`], [`remove_stale_connection`], [`probe`] | desktop, feature `agents` |
//! | The CLI's side | [`Live`], [`LiveConfig`], [`Route`], [`Window`], [`Consent`], [`HeadlessFolders`], [`find_window`], [`launcher`] | desktop, feature `agents` |
//! | The link host | [`AgentLink`], [`AgentLinkStatus`], [`LinkProblem`], [`LinkUpdate`] | desktop, features `agents` and `ui` |
//!
//! The Agents page of Preferences is [`crate::ui::agents`].

// The links above name items of the `agents` and `ui` features; without them they have nothing to
// point at, and the docs of the settings alone still build.
#![cfg_attr(not(all(feature = "agents", feature = "ui", not(target_arch = "wasm32"))), allow(rustdoc::broken_intra_doc_links))]

pub use craft_control::agents::{AgentSettings, WhenAppClosed, agents_allowed, folder};

#[cfg(all(feature = "agents", feature = "ui", not(target_arch = "wasm32")))]
mod link;
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
pub use craft_control::agents::{
    AgentSwitch, CONNECTION_FILE, ConnectFn, Connection, Consent, ConsentFn, HeadlessFn, HeadlessFolders, LAUNCH_POLL, LAUNCH_TIMEOUT, LaunchFn,
    Live, LiveConfig, POLL, PROBE_TIMEOUT, Route, TOKEN_FILE, Window, find_window, launcher, launcher_allows, mcp_flag, probe, read_connection,
    remove_connection, remove_stale_connection, write_connection,
};
#[cfg(all(feature = "agents", feature = "ui", not(target_arch = "wasm32")))]
pub use link::{AgentLink, AgentLinkStatus, LinkProblem, LinkUpdate, RETRY};
