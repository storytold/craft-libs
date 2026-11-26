//! AI agents in the open window, with no UI toolkit: whether the person lets agents use the app,
//! how the app tells its own `<id>-cli mcp` where it listens, and where that CLI sends each tool
//! call.
//!
//! One switch serves every Craft app: the ArtCraft Launcher's *Settings › AI agents › Let AI agents
//! use Craft apps*, which writes the launcher's app list (`craft_ai_agents`). With the suite's
//! `AI-MCP` flag on, an app lets agents in when that list names it ([`AgentSwitch`]); without the
//! flag, the app's own earlier consent decides ([`agents_allowed`]). Apps have no agents switch of
//! their own.
//!
//! While agents may connect, the app serves its control channel on loopback with a token
//! ([`crate::server`], [`crate::auth`], or its own) and publishes where in its settings folder
//! ([`CONNECTION_FILE`]): the port, its process id and the token file. The app's `<id>-cli mcp`
//! reads that file before every tool call ([`Live`]), so an MCP client's configuration never holds
//! a port or a token. The file holds no secret. A crashed app can leave it behind; nothing then
//! listens on its port ([`probe`]), and the app's next launch removes it
//! ([`remove_stale_connection`]). An app that quits removes its own ([`remove_connection`]).
//!
//! | Part | Names | Builds |
//! |---|---|---|
//! | Settings | [`AgentSettings`], [`WhenAppClosed`], [`agents_allowed`], [`folder`] | every target |
//! | The launcher's switch | [`AgentSwitch`], [`mcp_flag`], [`launcher_allows`], [`POLL`] | desktop, feature `agents` |
//! | The agent folders | [`AgentFolders`], [`AgentSettings::folders`], [`default_folder`], [`FOLDERS_TOOL`], [`folders_answer`], [`folders_tool`] | desktop, feature `agents` |
//! | The connection file | [`Connection`], [`write_connection`], [`read_connection`], [`remove_connection`], [`remove_stale_connection`], [`probe`] | desktop, feature `agents` |
//! | The CLI's side | [`Live`], [`LiveConfig`], [`Route`], [`Window`], [`Consent`], [`HeadlessFolders`], [`find_window`], [`launcher`] | desktop, feature `agents` |
//!
//! `craft_automation::agents` re-exports all of this and adds the egui link host and the Agents
//! page of Preferences, for apps on the shared UI library.
//!
//! # Adding an app
//!
//! The app (its id is the one in the ArtCraft Launcher's catalog, `"gridcraft"`):
//!
//! 1. At start, [`remove_stale_connection`] in its settings folder, then
//!    `AgentSwitch::watch("gridcraft", wake)`, where `wake` asks the UI for a frame.
//! 2. Agents may connect while `settings.allow_agent_control && switch.allows(legacy)`
//!    ([`AgentSettings::allow_agent_control`], [`AgentSwitch::allows`]); check it again when the
//!    launcher's switch wakes the app and when the person changes the setting. When it turns true,
//!    listen on loopback with a token, the token in an owner-only file
//!    ([`crate::auth::App::write_token_file`]), and [`write_connection`] with [`Connection::new`];
//!    when it turns false, and when the app quits, stop listening and [`remove_connection`] with
//!    the app's process id. No restart, no agents switch of its own. There is one connection file
//!    per settings folder: an app that runs more than one process on one folder lets only one of
//!    them accept agents.
//! 3. The folders agents open from and save to are `settings.folders("GridCraft")`
//!    ([`AgentSettings::folders`]): the person's own, otherwise `Documents/ArtCraft/Agents/GridCraft`.
//!    Its MCP server offers [`FOLDERS_TOOL`] (described by [`FOLDERS_TOOL_DESCRIPTION`] or listed
//!    as [`folders_tool`]), answered with [`folders_answer`] for the folders it uses.
//!
//! Its `gridcraft-cli mcp`:
//!
//! 1. A [`LiveConfig`] with the app's settings folder; `consent` reads its saved
//!    [`AgentSettings`] and answers `agents_allowed(mcp_flag(), launcher_allows("gridcraft"), legacy)`
//!    ([`agents_allowed`], [`mcp_flag`], [`launcher_allows`]); `connect` makes the app's client for a
//!    [`Window`] (its address and token); `headless` makes its session without a window for
//!    [`HeadlessFolders`]. [`launcher`] starts the app beside the CLI.
//! 2. Before every tool call, [`Live::resolve`] (from a blocking thread in an async server), then
//!    the call goes to the [`Route`] it returns. `--headless` skips [`Live`] altogether.

// The links above name items of the `agents` feature; without it they have nothing to point at,
// and the docs of the settings alone still build.
#![cfg_attr(not(all(feature = "agents", not(target_arch = "wasm32"))), allow(rustdoc::broken_intra_doc_links))]

mod settings;
pub use settings::{AgentSettings, WhenAppClosed, agents_allowed, folder};

#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
mod connection;
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
mod folders;
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
mod live;
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
mod switch;
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
pub use connection::{
    CONNECTION_FILE, Connection, PROBE_TIMEOUT, TOKEN_FILE, probe, read_connection, remove_connection, remove_stale_connection, write_connection,
};
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
pub use folders::{AgentFolders, DEFAULT_FOLDER_PARENTS, FOLDERS_TOOL, FOLDERS_TOOL_DESCRIPTION, default_folder, folders_answer, folders_tool};
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
pub use live::{
    ConnectFn, Consent, ConsentFn, HeadlessFn, HeadlessFolders, LAUNCH_POLL, LAUNCH_TIMEOUT, LaunchFn, Live, LiveConfig, Route, Window, find_window,
    launcher,
};
#[cfg(all(feature = "agents", not(target_arch = "wasm32")))]
pub use switch::{AgentSwitch, POLL, launcher_allows, mcp_flag};
