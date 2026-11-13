//! Synthetic pointer and keyboard input: `ui.click`, `ui.move`, `ui.wheel`, `ui.key`, `ui.release`, `ui.type` and
//! `ui.capsLock`.
//!
//! This module reads and validates the protocol's parameters; the window host queues the input
//! ([`WindowHost::inject_move`], [`WindowHost::inject_click`], [`WindowHost::inject_wheel`],
//! [`WindowHost::inject_key`], [`WindowHost::inject_release`], [`WindowHost::inject_text`]), one step per pass, so a
//! move, a press, a release and a key's down and up land on separate passes as real input does. The host answers the
//! waiting replies once [`WindowHost::injected_idle`] turns true ([`crate::control::ControlChannel::poll_input`]), and
//! withdraws a step queued for a document or focus that changed before the pass consumed it.
//!
//! Held input: `hold: true` on `ui.click` leaves its last press down and on `ui.key` its key, until `ui.release`; a
//! `ui.move` while a button is held drags. Each request names the modifiers held during it; a move or wheel leaves
//! them held (a hover with modifiers across a screenshot), a click or key that holds nothing lets them go.
//!
//! Points are screen points (egui points), or document points with `coordinates: "document"`, which the application
//! maps to the screen through its canvas ([`DocumentMap`]). Document-coordinate `ui.pointer` events are the
//! application's own method: they drive its tool host, not the window.

use craft_behavior::PointerButton;
use craft_keymap::{Key, Mods};
use craft_ui::runtime::windows::{Released, WindowHost};
use egui::{Pos2, Vec2, ViewportId, pos2, vec2};
use serde_json::Value;

use crate::protocol::{BUTTONS, KEY_MODIFIERS, Params};

/// Most clicks one `ui.click` may ask for (`count`).
pub const MAX_CLICKS: u64 = 256;

/// Largest coordinate of a point, either space, and of a document point's place on the screen: far beyond any
/// display, yet small enough that egui's pointer delta between two such points stays finite.
pub const MAX_SCREEN_POINT: f64 = 1e6;

/// The longest hover rest one `ui.move` may ask for (`rest`, seconds).
pub const MAX_REST: f64 = 10.0;

/// The reason [`crate::control::ControlChannel::input_cancelled`] is given when the host withdrew
/// the queued input.
pub const TARGET_CHANGED: &str = "the active document or focus changed while the input was queued";

/// The error of a document point the application cannot place on the screen.
pub const NO_DOCUMENT: &str = "no document canvas to place document coordinates on";

/// The error of a document point whose place on the screen is beyond [`MAX_SCREEN_POINT`].
pub const OFF_SCREEN: &str = "the document point is too far off the screen";

/// The application's map from a document point to the screen point its canvas shows it at; `None` when no canvas
/// shows a document. Applications without documents pass `&|_| None`.
pub type DocumentMap<'a> = &'a dyn Fn(Pos2) -> Option<Pos2>;

/// A point of a request: screen points, or document points the application maps.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum At {
    /// Screen points.
    Screen(Pos2),
    /// Document points.
    Document(Pos2),
}

impl At {
    /// `x` and `y`, each defaulting to 0, in the space `coordinates` names (`screen`, the default, or `document`).
    fn from_params(p: &Params<'_>) -> Result<Self, String> {
        let range = Some((-MAX_SCREEN_POINT, MAX_SCREEN_POINT));
        let x = p.number_in("x", "x", range)?.unwrap_or(0.0) as f32;
        let y = p.number_in("y", "y", range)?.unwrap_or(0.0) as f32;
        Ok(match p.choice("coordinates", "coordinates", &["screen", "document"])?.unwrap_or("screen") {
            "document" => Self::Document(pos2(x, y)),
            _ => Self::Screen(pos2(x, y)),
        })
    }

    /// The screen point, through `map` for a document point; [`NO_DOCUMENT`] when the map has none, and
    /// [`OFF_SCREEN`] when it maps beyond [`MAX_SCREEN_POINT`].
    pub fn resolve(self, map: DocumentMap<'_>) -> Result<Pos2, String> {
        match self {
            Self::Screen(at) => Ok(at),
            Self::Document(at) => {
                let on_screen = map(at).ok_or_else(|| NO_DOCUMENT.to_owned())?;
                let within = |v: f32| v.is_finite() && f64::from(v.abs()) <= MAX_SCREEN_POINT;
                if within(on_screen.x) && within(on_screen.y) { Ok(on_screen) } else { Err(OFF_SCREEN.to_owned()) }
            }
        }
    }
}

/// The point fields every pointer request takes.
const POINT: [&str; 3] = ["x", "y", "coordinates"];
/// The modifier fields ([`crate::protocol::KEY_MODIFIERS`], top-level or under `modifiers`).
const MODIFIERS: [&str; 5] = ["modifiers", "shift", "alt", "command", "ctrl"];

/// The params object with only `fields` besides the point and modifier fields.
fn only(params: &Value, fields: &[&str]) -> Result<Value, String> {
    let all: Vec<&str> = POINT.iter().chain(&MODIFIERS).chain(fields).copied().collect();
    Params(params).only(None, &all)
}

/// `button`, one of [`crate::protocol::BUTTONS`]: `right` and `secondary` are the secondary button, `middle` the
/// middle one, the rest primary.
fn button(p: &Params<'_>) -> Result<Option<PointerButton>, String> {
    Ok(p.choice("button", "button", &BUTTONS)?.map(|name| match name {
        "right" | "secondary" => PointerButton::Secondary,
        "middle" => PointerButton::Middle,
        _ => PointerButton::Primary,
    }))
}

/// `key`, a name `Key::parse` knows: ``"unknown key `{name}`"`` otherwise.
fn key(p: &Params<'_>) -> Result<Option<Key>, String> {
    p.choice("key", "key", &[])?.map(|name| Key::parse(name).ok_or_else(|| format!("unknown key `{name}`"))).transpose()
}

/// A validated `ui.click`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Click {
    /// Target position.
    pub at: At,
    /// The button pressed.
    pub button: PointerButton,
    /// Number of clicks (2 is a double click).
    pub count: u64,
    /// Modifiers held during the click.
    pub mods: Mods,
    /// The last press stays down until `ui.release`.
    pub hold: bool,
}

impl Click {
    /// `{x?, y?, coordinates?, button?, count?, hold?, modifiers…}`, no other fields (`Params::only`). `button`
    /// defaults to `left`. `count` is an unsigned integer, default 1; over [`MAX_CLICKS`] is
    /// ``"`count` must be at most 256 (got {count})"``. Other errors are the `Params` validators'.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let p = only(params, &["button", "count", "hold"])?;
        let p = Params(&p);
        let count = p.uint_field("count")?.unwrap_or(1);
        if count > MAX_CLICKS {
            return Err(format!("`count` must be at most {MAX_CLICKS} (got {count})"));
        }
        Ok(Self {
            at: At::from_params(&p)?,
            button: button(&p)?.unwrap_or(PointerButton::Primary),
            count,
            mods: p.modifiers(&KEY_MODIFIERS)?,
            hold: p.bool_field("hold")?.unwrap_or(false),
        })
    }

    /// Queues a move to the target, then `count` presses and releases of the button, in `viewport`.
    pub fn inject(&self, host: &WindowHost, viewport: ViewportId, map: DocumentMap<'_>) -> Result<(), String> {
        let count = u32::try_from(self.count).unwrap_or(u32::MAX);
        host.inject_click(viewport, self.at.resolve(map)?, self.button, count, self.mods, self.hold);
        Ok(())
    }
}

/// A validated `ui.move`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Move {
    /// Target position.
    pub at: At,
    /// Modifiers held from the move on.
    pub mods: Mods,
    /// Seconds the pointer rests there before the reply.
    pub rest: f64,
}

impl Move {
    /// `{x?, y?, coordinates?, rest?, modifiers…}`, no other fields. `rest` is seconds between 0 and [`MAX_REST`].
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let p = only(params, &["rest"])?;
        let p = Params(&p);
        Ok(Self {
            at: At::from_params(&p)?,
            mods: p.modifiers(&KEY_MODIFIERS)?,
            rest: p.number_in("rest", "rest", Some((0.0, MAX_REST)))?.unwrap_or(0.0),
        })
    }

    /// Queues the move in `viewport`.
    pub fn inject(&self, host: &WindowHost, viewport: ViewportId, map: DocumentMap<'_>) -> Result<(), String> {
        host.inject_move(viewport, self.at.resolve(map)?, self.mods, self.rest);
        Ok(())
    }
}

/// A validated `ui.wheel`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Wheel {
    /// Where the pointer is for the turn.
    pub at: At,
    /// The scroll in points, as egui reports it (positive `y` scrolls towards the content's top).
    pub delta: Vec2,
    /// Modifiers held from the turn on.
    pub mods: Mods,
}

impl Wheel {
    /// `{x?, y?, coordinates?, dx?, dy?, modifiers…}`, no other fields; `dx` and `dy` are points, default 0.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let p = only(params, &["dx", "dy"])?;
        let p = Params(&p);
        let dx = p.number_in("dx", "dx", None)?.unwrap_or(0.0) as f32;
        let dy = p.number_in("dy", "dy", None)?.unwrap_or(0.0) as f32;
        Ok(Self { at: At::from_params(&p)?, delta: vec2(dx, dy), mods: p.modifiers(&KEY_MODIFIERS)? })
    }

    /// Queues the move and the turn in `viewport`.
    pub fn inject(&self, host: &WindowHost, viewport: ViewportId, map: DocumentMap<'_>) -> Result<(), String> {
        host.inject_wheel(viewport, self.at.resolve(map)?, self.delta, self.mods);
        Ok(())
    }
}

/// A validated `ui.key`: a key name (`Enter`, `A`, `F5`) with modifiers.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct KeyPress {
    /// The key pressed.
    pub key: Key,
    /// Modifiers held with it.
    pub mods: Mods,
    /// The key stays down until `ui.release`.
    pub hold: bool,
}

impl KeyPress {
    /// `{key, hold?, modifiers?, shift?, alt?, command?, ctrl?}`, no other fields. A missing key is
    /// ``"missing `key`"``.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let p = Params(params).only(None, &["key", "hold", "modifiers", "shift", "alt", "command", "ctrl"])?;
        let p = Params(&p);
        let Some(key) = key(&p)? else { return Err("missing `key`".into()) };
        Ok(Self { key, mods: p.modifiers(&KEY_MODIFIERS)?, hold: p.bool_field("hold")?.unwrap_or(false) })
    }

    /// Queues the key pressed with its modifiers held, and released unless held, in `viewport`.
    pub fn inject(&self, host: &WindowHost, viewport: ViewportId) {
        host.inject_key(viewport, self.key, self.mods, self.hold);
    }
}

/// A validated `ui.release`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Release {
    /// Where the release happens; where the pointer is when absent.
    pub at: Option<At>,
    /// What is released.
    pub released: Released,
}

impl Release {
    /// `{button?, key?, x?, y?, coordinates?}`, no other fields. With neither `button` nor `key` everything held is
    /// released, the modifiers too; both is ``"`button` and `key` exclude each other"``.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let p = Params(params).only(None, &["button", "key", "x", "y", "coordinates"])?;
        let p = Params(&p);
        let released = match (button(&p)?, key(&p)?) {
            (Some(_), Some(_)) => return Err("`button` and `key` exclude each other".into()),
            (Some(button), None) => Released::Button(button),
            (None, Some(key)) => Released::Key(key),
            (None, None) => Released::All,
        };
        let at = (p.0.get("x").is_some() || p.0.get("y").is_some()).then(|| At::from_params(&p)).transpose()?;
        Ok(Self { at, released })
    }

    /// Queues the release in `viewport`.
    pub fn inject(&self, host: &WindowHost, viewport: ViewportId, map: DocumentMap<'_>) -> Result<(), String> {
        let at = self.at.map(|at| at.resolve(map)).transpose()?;
        host.inject_release(viewport, at, self.released);
        Ok(())
    }
}

/// A validated `ui.capsLock`: `{on}`, a boolean, or `null` to return to the platform's state.
pub fn caps_lock_from_params(params: &Value) -> Result<Option<bool>, String> {
    let Some(object) = params.as_object() else { return Err("params must be an object".into()) };
    if let Some(key) = object.keys().find(|k| *k != "on") {
        return Err(format!("unknown field `{key}` (fields: on)"));
    }
    match object.get("on") {
        None => Err("missing `on`".into()),
        Some(Value::Null) => Ok(None),
        Some(Value::Bool(on)) => Ok(Some(*on)),
        Some(_) => Err("on must be a boolean or null".into()),
    }
}

/// Runs the synthetic input method `method` with `params` in `viewport`; `None` for a method this module does not
/// have. `Ok` means the input is queued and the reply waits for it ([`crate::control::Reply::AfterInput`]);
/// `ui.capsLock` holds from the next pass on.
pub fn input_method(method: &str, params: &Value, host: &WindowHost, viewport: ViewportId, map: DocumentMap<'_>) -> Option<Result<(), String>> {
    Some(match method {
        "ui.click" => Click::from_params(params).and_then(|click| click.inject(host, viewport, map)),
        "ui.move" => Move::from_params(params).and_then(|step| step.inject(host, viewport, map)),
        "ui.wheel" => Wheel::from_params(params).and_then(|wheel| wheel.inject(host, viewport, map)),
        "ui.key" => KeyPress::from_params(params).map(|key| key.inject(host, viewport)),
        "ui.release" => Release::from_params(params).and_then(|release| release.inject(host, viewport, map)),
        "ui.type" => text(params).map(|text| host.inject_text(viewport, &text)),
        "ui.capsLock" => caps_lock_from_params(params).map(|on| host.set_caps_lock(on)),
        _ => return None,
    })
}

/// A validated `ui.type`: `{text}`, a string.
fn text(params: &Value) -> Result<String, String> {
    let p = Params(params).only(None, &["text"])?;
    Params(&p).str_field("text")?.map(str::to_owned).ok_or_else(|| "missing `text`".into())
}
