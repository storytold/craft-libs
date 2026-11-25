//! The ArtCraft Launcher's switch for one app, behind the suite's `AI-MCP` flag.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use super::agents_allowed;

/// How often [`AgentSwitch::watch`] reads the launcher's app list.
pub const POLL: Duration = Duration::from_secs(2);

/// The suite's `AI-MCP` flag (`ArtCraft/flags.toml` or `CRAFT_FLAGS`). Reads the flags file;
/// what could not be read is logged.
pub fn mcp_flag() -> bool {
    let flags = craft_flags::load();
    for problem in &flags.problems {
        log::warn!("flags: {problem}");
    }
    flags.enabled(&craft_flags::AI_MCP)
}

/// The launcher's switch (Settings › AI agents › *Let AI agents use Craft apps*) lets agents use
/// the app `app_id`: the `AI-MCP` flag is on and the launcher's app list names the app. Reads two
/// small files; ask from a timer or a command, not every frame ([`AgentSwitch`] does that).
pub fn launcher_allows(app_id: &str) -> bool {
    mcp_flag() && craft_ai_agents::allowed(app_id)
}

/// The launcher's switch for one app, watched on a thread so a frame can ask without touching the
/// disk. The `AI-MCP` flag is read once, at [`Self::watch`]; the launcher's list every [`POLL`].
/// Clones share one watch; the thread ends after the last clone is dropped.
#[derive(Clone, Debug)]
pub struct AgentSwitch {
    mcp_flag: bool,
    on: Arc<AtomicBool>,
}

impl AgentSwitch {
    /// Watches the launcher's list for `app_id`; a change logs `agents on/off` and calls `wake` (the
    /// app asks for a frame there).
    /// Without the `AI-MCP` flag nothing is watched: the launcher's list means nothing then.
    pub fn watch(app_id: &'static str, wake: impl Fn() + Send + 'static) -> Self {
        let flag = mcp_flag();
        if !flag {
            return Self::fixed(false, false);
        }
        Self::watch_with(flag, move || craft_ai_agents::allowed(app_id), POLL, wake)
    }

    /// A switch that never changes: tests, the web, an app without agents.
    pub fn fixed(mcp_flag: bool, launcher: bool) -> Self {
        Self { mcp_flag, on: Arc::new(AtomicBool::new(launcher)) }
    }

    /// [`Self::watch`] with the flag given, `probe` asked once now and then every `poll`.
    #[doc(hidden)]
    pub fn watch_with(mcp_flag: bool, probe: impl Fn() -> bool + Send + 'static, poll: Duration, wake: impl Fn() + Send + 'static) -> Self {
        let on = Arc::new(AtomicBool::new(probe()));
        let seen: Weak<AtomicBool> = Arc::downgrade(&on);
        let started = std::thread::Builder::new().name("launcher-agents".into()).spawn(move || {
            loop {
                std::thread::sleep(poll);
                let now = probe();
                let Some(seen) = seen.upgrade() else { return };
                if seen.swap(now, Ordering::Relaxed) != now {
                    log::info!("ArtCraft Launcher: agents {}", if now { "on" } else { "off" });
                    wake();
                }
            }
        });
        // Without a thread a later "off" in the launcher would never be seen: agents stay out.
        if let Err(e) = started {
            log::warn!("couldn't watch the ArtCraft Launcher's agents switch, so agents stay off: {e}");
            on.store(false, Ordering::Relaxed);
        }
        Self { mcp_flag, on }
    }

    /// The `AI-MCP` flag as it was when the watch started.
    pub fn mcp_flag(&self) -> bool {
        self.mcp_flag
    }

    /// Whether the launcher's list named the app at the last poll.
    pub fn launcher(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// Whether agents may use the open window ([`agents_allowed`]); `legacy` is the app's own
    /// earlier consent, which counts only without the flag.
    pub fn allows(&self, legacy: bool) -> bool {
        agents_allowed(self.mcp_flag, self.launcher(), legacy)
    }
}
