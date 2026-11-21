//! Panels and data plane contributions become surfaces the app shows or lists.
//!
//! A `command` or `menu` goes to the command registry and a `hook` goes to `Hooks`. A `panel`, an
//! `effect`, a `filter`, an `object-filter` and a `live-effect` need an app side registry the host
//! cannot reach (egui panels, the effect list), so the host hands each one to a [`SurfaceSink`] the
//! app provides. The sink gets a [`Surface`] when a generation loads or reloads, and `remove` when
//! it unloads, so a new plugin appears without a restart.
//!
//! A [`Surface`] calls through the [`Router`], like a registered command. After a reload the same
//! surface runs generation N+1. The reload still calls `remove` and `add`, so a changed label or a
//! changed parameter list reaches the app.
//!
//! Rules this module keeps:
//!
//! - A `panel` needs `ui.contribute` (default deny). Without the grant it is left out and listed in
//!   [`SurfaceRegistration::skipped`].
//! - `effect` and `filter` need no permission. They only compute on buffers.
//! - `object-filter` and `live-effect` run through the VectorCraft v1 shim only. A craft-abi 1 module
//!   that declares one is left out, with the reason.
//! - A `format` needs `fs.picked` (default deny). The host only ever gives it the bytes of a file the
//!   user opened or chose to save, never a path. [`crate::contrib::format::FormatBook`] is the sink
//!   an app uses to run them.
//! - `audio-effect` is left out: the host has no audio block runner yet. The reason says so.

use std::sync::Arc;

use craft_command::Owner;
use craft_plugin_abi::Permission;
use craft_plugin_abi::manifest::{Contribution, Kind, Manifest};
use serde_json::Value;

use crate::invoke::{CallError, CallOpts, Generation, Router};
use crate::process::{ImageJob, ProcessOpts, ProcessReport};

/// One panel or data plane contribution of a loaded generation.
#[derive(Clone)]
pub struct Surface {
    pub owner: Owner,
    pub contribution: Arc<Contribution>,
    /// Set for a v1 plugin, which has no permissions and runs with its original limits.
    pub legacy: bool,
    router: Arc<Router>,
}

impl Surface {
    fn generation(&self) -> Result<Arc<Generation>, CallError> {
        self.router.current(&self.owner.plugin_id).ok_or(CallError::NotLoaded)
    }

    /// Send a panel event (`{"event": "open"|"close"|"action", ...}`) and return the answer:
    /// `{"tree": ...}` or `{}`. Only for a `panel`.
    pub fn panel_event(&self, event: &Value) -> Result<Value, CallError> {
        if self.contribution.kind != Kind::Panel {
            return Err(CallError::NotCallable(format!("{} is a {}, not a panel", self.contribution.id, self.contribution.kind.name())));
        }
        self.generation()?.call(&self.contribution, event, &[], 0)
    }

    /// Send a `format` request (`craft_plugin_abi::format`) with the file or the pixels as `blob`.
    /// Only for a `format`. The permission check is the caller's: see [`Surface::grants`].
    pub fn format_call(&self, request: &Value, blob: &[u8], opts: CallOpts) -> Result<Value, CallError> {
        if self.contribution.kind != Kind::Format {
            return Err(CallError::NotCallable(format!("{} is a {}, not a format", self.contribution.id, self.contribution.kind.name())));
        }
        self.generation()?.call_with(&self.contribution, request, blob, 0, opts)
    }

    /// The permissions the user granted to the generation that runs now.
    pub fn grants(&self) -> Result<Vec<Permission>, CallError> {
        Ok(self.generation()?.grants().to_vec())
    }

    /// Run the contribution over an image. See [`Generation::process_image`].
    pub fn process_image(&self, job: &mut ImageJob<'_>, params: &Value, opts: &ProcessOpts) -> Result<ProcessReport, CallError> {
        self.generation()?.process_image(&self.contribution, job, params, opts)
    }

    /// Run a VectorCraft v1 filter or live effect. See [`Generation::run_objects`].
    #[cfg(feature = "legacy-v1")]
    pub fn run_objects(&self, input: &[u8], params: &Value) -> Result<Vec<u8>, CallError> {
        self.generation()?.run_objects(&self.contribution, input, params)
    }
}

/// What an app implements to receive surfaces.
pub trait SurfaceSink: Send + Sync {
    /// A contribution is live. The same `owner` and contribution id may arrive again after a
    /// reload, preceded by one [`SurfaceSink::remove`].
    fn add(&self, surface: Surface);
    /// Every surface of `owner` is gone: the generation reloaded, was disabled, or was removed.
    fn remove(&self, owner: &Owner);
}

/// A contribution the host left out, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedSurface {
    pub contribution: String,
    pub reason: String,
}

/// What [`register_surfaces`] did with one generation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceRegistration {
    /// Contribution ids handed to the sink.
    pub registered: Vec<String>,
    pub skipped: Vec<SkippedSurface>,
}

enum Verdict {
    /// Not a surface kind. Commands, menus and hooks have their own registration.
    NotSurface,
    Take,
    Skip(String),
}

fn judge(c: &Contribution, legacy: bool, grants: &[Permission]) -> Verdict {
    match &c.kind {
        Kind::Panel if !grants.contains(&Permission::UiContribute) => Verdict::Skip("missing permission ui.contribute".into()),
        Kind::Panel | Kind::Effect | Kind::Filter => Verdict::Take,
        Kind::ObjectFilter | Kind::LiveEffect if legacy => Verdict::Take,
        Kind::ObjectFilter | Kind::LiveEffect => Verdict::Skip(format!("{} runs through the VectorCraft v1 shim only", c.kind.name())),
        Kind::Format if !grants.contains(&Permission::FsPicked) => Verdict::Skip("missing permission fs.picked".into()),
        Kind::Format => Verdict::Take,
        Kind::AudioEffect => Verdict::Skip("this host has no audio block runner yet".into()),
        Kind::Command | Kind::Menu | Kind::Hook | Kind::Unknown(_) => Verdict::NotSurface,
    }
}

/// Hand every surface of `manifest` that `grants` allow to `sink`.
pub fn register_surfaces(
    sink: &dyn SurfaceSink,
    router: &Arc<Router>,
    owner: &Owner,
    manifest: &Manifest,
    grants: &[Permission],
) -> SurfaceRegistration {
    let legacy = manifest.legacy.is_some();
    let mut out = SurfaceRegistration::default();
    for c in &manifest.contributions {
        match judge(c, legacy, grants) {
            Verdict::NotSurface => {}
            Verdict::Skip(reason) => out.skipped.push(SkippedSurface { contribution: c.id.clone(), reason }),
            Verdict::Take => {
                sink.add(Surface { owner: owner.clone(), contribution: Arc::new(c.clone()), legacy, router: Arc::clone(router) });
                out.registered.push(c.id.clone());
            }
        }
    }
    out
}
