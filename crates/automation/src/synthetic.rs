//! Synthetic pointer and keyboard input: `ui.click`, `ui.move`, `ui.key`, `ui.type`.
//!
//! This module reads and validates the protocol's parameters; the window host queues the input
//! ([`WindowHost::inject_move`], [`WindowHost::inject_click`], [`WindowHost::inject_key`],
//! [`WindowHost::inject_text`]), one step per pass, so a move, a press, a release and a key's down
//! and up land on separate passes as real input does. The host answers the waiting replies once
//! [`WindowHost::injected_idle`] turns true ([`crate::control::ControlChannel::poll_input`]), and
//! withdraws a step queued for a document or focus that changed before the pass consumed it.
//! Screen points are egui points. Document-coordinate `ui.pointer` events stay in the app: they
//! drive its tool host, not the window.

use craft_behavior::PointerButton;
use craft_keymap::{Key, Mods};
use craft_ui::runtime::windows::WindowHost;
use egui::{Pos2, ViewportId, pos2};
use serde_json::Value;

use crate::protocol::{BUTTONS, KEY_MODIFIERS, Params};

/// Most clicks one `ui.click` may ask for (`count`).
pub const MAX_CLICKS: u64 = 256;

/// The reason [`crate::control::ControlChannel::input_cancelled`] is given when the host withdrew
/// the queued input.
pub const TARGET_CHANGED: &str = "the active document or focus changed while the input was queued";

/// `x` and `y` in screen points, each defaulting to 0.
fn coordinates(p: &Params<'_>) -> Result<(f32, f32), String> {
    let x = p.number_in("x", "x", None)?.unwrap_or(0.0) as f32;
    let y = p.number_in("y", "y", None)?.unwrap_or(0.0) as f32;
    Ok((x, y))
}

/// A validated `ui.click`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Click {
    /// Target position in points.
    pub at: Pos2,
    /// The button pressed.
    pub button: PointerButton,
    /// Number of clicks (2 is a double click).
    pub count: u64,
}

impl Click {
    /// `{x?, y?, button?, count?}`, no other fields (`Params::only`). `x` and `y` are numbers,
    /// default 0. `button` is one of [`crate::protocol::BUTTONS`], default `left`; `right` and
    /// `secondary` are the secondary button, `middle` the middle one, the rest primary. `count`
    /// is an unsigned integer, default 1; over [`MAX_CLICKS`] is
    /// ``"`count` must be at most 256 (got {count})"``. Other errors are the `Params` validators'.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let p = Params(params).only(None, &["x", "y", "button", "count"])?;
        let p = Params(&p);
        let (x, y) = coordinates(&p)?;
        let button = match p.choice("button", "button", &BUTTONS)?.unwrap_or("left") {
            "right" | "secondary" => PointerButton::Secondary,
            "middle" => PointerButton::Middle,
            _ => PointerButton::Primary,
        };
        let count = p.uint_field("count")?.unwrap_or(1);
        if count > MAX_CLICKS {
            return Err(format!("`count` must be at most {MAX_CLICKS} (got {count})"));
        }
        Ok(Self { at: pos2(x, y), button, count })
    }

    /// Queues a move to the target, then `count` presses and releases of the button, in `viewport`.
    pub fn inject(&self, host: &WindowHost, viewport: ViewportId) {
        host.inject_click(viewport, self.at, self.button, u32::try_from(self.count).unwrap_or(u32::MAX));
    }
}

/// A validated `ui.move`: `{x?, y?}` only, numbers, default 0.
pub fn move_from_params(params: &Value) -> Result<Pos2, String> {
    let p = Params(params).only(None, &["x", "y"])?;
    let (x, y) = coordinates(&Params(&p))?;
    Ok(pos2(x, y))
}

/// A validated `ui.key`: a key name (`Enter`, `A`, `F5`) with modifiers.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct KeyPress {
    /// The key pressed.
    pub key: Key,
    /// Modifiers held with it.
    pub mods: Mods,
}

impl KeyPress {
    /// `{key, modifiers?, shift?, alt?, command?, ctrl?}`, no other fields. The modifier flags may
    /// be top-level or grouped under `modifiers` ([`crate::protocol::KEY_MODIFIERS`]). A missing
    /// key is ``"missing `key`"``; a name `Key::parse` doesn't know is ``"unknown key `{name}`"``.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let p = Params(params).only(None, &["key", "modifiers", "shift", "alt", "command", "ctrl"])?;
        let p = Params(&p);
        let Some(name) = p.choice("key", "key", &[])? else { return Err("missing `key`".into()) };
        let key = Key::parse(name).ok_or_else(|| format!("unknown key `{name}`"))?;
        Ok(Self { key, mods: p.modifiers(&KEY_MODIFIERS)? })
    }

    /// Queues the key pressed and released with its modifiers held, in `viewport`.
    pub fn inject(&self, host: &WindowHost, viewport: ViewportId) {
        host.inject_key(viewport, self.key, self.mods);
    }
}
