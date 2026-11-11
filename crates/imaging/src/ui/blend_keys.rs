//! The blend mode keys, as in Photoshop: Shift+ and Shift- step through the target's modes, Shift+Alt plus a
//! letter picks one ([`BlendKeys`]).
//!
//! The app lists the modes its target accepts ([`BlendKeyEntry`]: a painting tool's own Mode menu, or the layer modes)
//! with its own key type. The keys are the [`BlendShortcut`] and [`BlendStep`] command sets; the app routes the fired
//! commands here before the single-letter tool keys and gets back the entry to set. Which target a tool has, the
//! engine command and its undo step stay the app's.
//!
//! ```ignore
//! let entries = [BlendKeyEntry::new(Mode::Normal, BlendShortcut::Normal), BlendKeyEntry::new(Mode::Multiply, BlendShortcut::Multiply)];
//! if let Some(pick) = BlendKeys::new(&entries).current(&layer.blend).fired(&fired) {
//!     app.set_blend(pick.into_key()); // one undo step
//! }
//! ```

use craft_ui::contract::CommandSet;
use craft_ui::runtime::commands::{CommandRef, FirePhase, Fired};

/// Photoshop's Shift+Alt letter positions: the 26 lettered blend modes. Darker Color, Lighter Color, Subtract and
/// Divide have no letter; list them with [`BlendKeyEntry::unlettered`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "blendMode")]
#[non_exhaustive]
pub enum BlendShortcut {
    /// Shift+Alt+N.
    #[command(label = "Normal", default = "Shift+Alt+N")]
    Normal,
    /// Shift+Alt+I.
    #[command(label = "Dissolve", default = "Shift+Alt+I")]
    Dissolve,
    /// Shift+Alt+Q: painting tools only.
    #[command(label = "Behind", default = "Shift+Alt+Q")]
    Behind,
    /// Shift+Alt+R: painting tools only (paints transparency).
    #[command(label = "Clear", default = "Shift+Alt+R")]
    Clear,
    /// Shift+Alt+P: groups only.
    #[command(label = "Pass Through", default = "Shift+Alt+P")]
    PassThrough,
    /// Shift+Alt+K.
    #[command(label = "Darken", default = "Shift+Alt+K")]
    Darken,
    /// Shift+Alt+M.
    #[command(label = "Multiply", default = "Shift+Alt+M")]
    Multiply,
    /// Shift+Alt+B.
    #[command(label = "Color Burn", default = "Shift+Alt+B")]
    ColorBurn,
    /// Shift+Alt+A.
    #[command(label = "Linear Burn", default = "Shift+Alt+A")]
    LinearBurn,
    /// Shift+Alt+G.
    #[command(label = "Lighten", default = "Shift+Alt+G")]
    Lighten,
    /// Shift+Alt+S.
    #[command(label = "Screen", default = "Shift+Alt+S")]
    Screen,
    /// Shift+Alt+D.
    #[command(label = "Color Dodge", default = "Shift+Alt+D")]
    ColorDodge,
    /// Shift+Alt+W.
    #[command(label = "Linear Dodge", default = "Shift+Alt+W")]
    LinearDodge,
    /// Shift+Alt+O.
    #[command(label = "Overlay", default = "Shift+Alt+O")]
    Overlay,
    /// Shift+Alt+F.
    #[command(label = "Soft Light", default = "Shift+Alt+F")]
    SoftLight,
    /// Shift+Alt+H.
    #[command(label = "Hard Light", default = "Shift+Alt+H")]
    HardLight,
    /// Shift+Alt+V.
    #[command(label = "Vivid Light", default = "Shift+Alt+V")]
    VividLight,
    /// Shift+Alt+J.
    #[command(label = "Linear Light", default = "Shift+Alt+J")]
    LinearLight,
    /// Shift+Alt+Z.
    #[command(label = "Pin Light", default = "Shift+Alt+Z")]
    PinLight,
    /// Shift+Alt+L.
    #[command(label = "Hard Mix", default = "Shift+Alt+L")]
    HardMix,
    /// Shift+Alt+E.
    #[command(label = "Difference", default = "Shift+Alt+E")]
    Difference,
    /// Shift+Alt+X.
    #[command(label = "Exclusion", default = "Shift+Alt+X")]
    Exclusion,
    /// Shift+Alt+U.
    #[command(label = "Hue", default = "Shift+Alt+U")]
    Hue,
    /// Shift+Alt+T.
    #[command(label = "Saturation", default = "Shift+Alt+T")]
    Saturation,
    /// Shift+Alt+C.
    #[command(label = "Color", default = "Shift+Alt+C")]
    Color,
    /// Shift+Alt+Y.
    #[command(label = "Luminosity", default = "Shift+Alt+Y")]
    Luminosity,
}

impl BlendShortcut {
    /// Every position, in Photoshop's menu order (as the variants).
    pub const ALL: [BlendShortcut; 26] = [
        BlendShortcut::Normal,
        BlendShortcut::Dissolve,
        BlendShortcut::Behind,
        BlendShortcut::Clear,
        BlendShortcut::PassThrough,
        BlendShortcut::Darken,
        BlendShortcut::Multiply,
        BlendShortcut::ColorBurn,
        BlendShortcut::LinearBurn,
        BlendShortcut::Lighten,
        BlendShortcut::Screen,
        BlendShortcut::ColorDodge,
        BlendShortcut::LinearDodge,
        BlendShortcut::Overlay,
        BlendShortcut::SoftLight,
        BlendShortcut::HardLight,
        BlendShortcut::VividLight,
        BlendShortcut::LinearLight,
        BlendShortcut::PinLight,
        BlendShortcut::HardMix,
        BlendShortcut::Difference,
        BlendShortcut::Exclusion,
        BlendShortcut::Hue,
        BlendShortcut::Saturation,
        BlendShortcut::Color,
        BlendShortcut::Luminosity,
    ];
}

/// Shift+= and Shift+-: the next and the previous mode of the target's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "blendMode")]
#[non_exhaustive]
pub enum BlendStep {
    /// Shift+=: the next mode, wrapping at the end.
    #[command(label = "Next Blend Mode", default = "Shift+=")]
    Next,
    /// Shift+-: the previous mode, wrapping at the start.
    #[command(label = "Previous Blend Mode", default = "Shift+-")]
    Previous,
}

/// One mode the target accepts, in the order of its menu: the app's key of the mode and its Shift+Alt+
/// letter position, if it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BlendKeyEntry<K> {
    /// The app's key of the blend mode.
    pub key: K,
    /// The mode's Shift+Alt+ letter position, if it has one.
    pub shortcut: Option<BlendShortcut>,
}

impl<K> BlendKeyEntry<K> {
    /// The mode `key` at letter position `shortcut`.
    pub fn new(key: K, shortcut: BlendShortcut) -> Self {
        Self { key, shortcut: Some(shortcut) }
    }

    /// A mode without a letter (Darker Color, Lighter Color, Subtract, Divide): Shift+= and Shift+- reach
    /// it, no letter does.
    pub fn unlettered(key: K) -> Self {
        Self { key, shortcut: None }
    }

    /// Behind (Shift+Alt+Q), for a painting tool that offers it. Without such an entry Shift+Alt+Q is consumed
    /// and does nothing.
    pub fn behind(key: K) -> Self {
        Self::new(key, BlendShortcut::Behind)
    }

    /// Clear (Shift+Alt+R), for a painting tool that offers it (PhotoCraft's Brush, Pencil and Paint
    /// Bucket list it after Dissolve). Without such an entry Shift+Alt+R is consumed and does nothing.
    pub fn clear(key: K) -> Self {
        Self::new(key, BlendShortcut::Clear)
    }
}

/// What a blend mode key picked: the entry's key, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlendKeyPick<K> {
    /// Shift+= or Shift+- stepped to this entry.
    Stepped(K),
    /// A Shift+Alt+ letter picked this entry.
    Lettered(K),
}

impl<K> BlendKeyPick<K> {
    /// The picked entry's key.
    pub fn key(&self) -> &K {
        match self {
            BlendKeyPick::Stepped(key) | BlendKeyPick::Lettered(key) => key,
        }
    }

    /// The picked entry's key, taken.
    pub fn into_key(self) -> K {
        match self {
            BlendKeyPick::Stepped(key) | BlendKeyPick::Lettered(key) => key,
        }
    }
}

/// The blend mode keys over the app's list of `entries`.
///
/// - [`BlendStep`] steps to the next or previous entry, wrapping at either end; a [`current`](Self::current) that
///   isn't listed (or none) steps from the top (next) or the bottom (previous).
/// - A [`BlendShortcut`] picks the entry whose position it is. A position whose mode isn't listed (Clear or Behind for
///   layers, Pass Through for a painting tool, Overlay for the Healing Brush) picks nothing: the command is taken, so
///   the letter does not reach the single-letter tool keys.
/// - A key release picks nothing; holding a step key repeats the step.
/// - No AccessKit node: the app's blend control announces the new mode. Keys are not directional, so nothing mirrors
///   in a right-to-left interface.
#[must_use = "read the fired command with `fired`"]
pub struct BlendKeys<'a, K> {
    entries: &'a [BlendKeyEntry<K>],
    current: Option<&'a K>,
}

impl<'a, K: Clone + PartialEq> BlendKeys<'a, K> {
    /// The keys over `entries`, in menu order.
    pub fn new(entries: &'a [BlendKeyEntry<K>]) -> Self {
        Self { entries, current: None }
    }

    /// The target's mode now, where the steps start from.
    pub fn current(mut self, current: &'a K) -> Self {
        self.current = Some(current);
        self
    }

    /// The entry a fired [`BlendStep`] or [`BlendShortcut`] picks; `None` for any other command, a release, or a
    /// position the list lacks.
    pub fn fired(self, fired: &Fired) -> Option<BlendKeyPick<K>> {
        self.command(&fired.command, fired.phase)
    }

    /// [`fired`](Self::fired) for a command and the phase it fired in.
    pub fn command(self, command: &CommandRef, phase: FirePhase) -> Option<BlendKeyPick<K>> {
        if phase == FirePhase::Release {
            return None;
        }
        if let Some(step) = BlendStep::from_ref(command) {
            let step = match step {
                BlendStep::Next => 1,
                BlendStep::Previous => -1,
            };
            return stepped(self.entries, self.current, step).map(|entry| BlendKeyPick::Stepped(entry.key.clone()));
        }
        let shortcut = BlendShortcut::from_ref(command)?;
        self.entries.iter().find(|entry| entry.shortcut == Some(shortcut)).map(|entry| BlendKeyPick::Lettered(entry.key.clone()))
    }
}

/// The entry `step` places after (or before, when negative) `current` in `entries`, wrapping; an
/// unlisted or absent `current` steps from the top (`step > 0`) or the bottom. `None` for an
/// empty list.
fn stepped<'e, K: PartialEq>(entries: &'e [BlendKeyEntry<K>], current: Option<&K>, step: i32) -> Option<&'e BlendKeyEntry<K>> {
    // pc: crates/ui-egui/src/input/blend_keys.rs:18 `stepped` (`rem_euclid`; unlisted: 0 or n − 1).
    let n = i32::try_from(entries.len()).ok().filter(|n| *n > 0)?;
    let at = current.and_then(|c| entries.iter().position(|e| e.key == *c)).and_then(|i| i32::try_from(i).ok());
    let next = match at {
        Some(i) => (i + step).rem_euclid(n),
        None if step > 0 => 0,
        None => n - 1,
    };
    entries.get(usize::try_from(next).ok()?)
}
