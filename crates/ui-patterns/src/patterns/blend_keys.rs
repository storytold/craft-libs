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

use craft_ui::contract::{CommandSet, Entry};
use craft_ui::runtime::commands::{CommandRef, FirePhase, Fired};
use craft_ui::runtime::env::Label;

/// Photoshop's Shift+Alt letter positions: the 26 lettered blend modes. Darker Color, Lighter Color, Subtract and
/// Divide have no letter; list them with [`BlendKeyEntry::unlettered`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "blendMode")]
#[non_exhaustive]
pub enum BlendShortcut {
    /// Shift+Alt+N.
    #[command(modal = "declared", label = "Normal", default = "Shift+Alt+N")]
    Normal,
    /// Shift+Alt+I.
    #[command(modal = "declared", label = "Dissolve", default = "Shift+Alt+I")]
    Dissolve,
    /// Shift+Alt+Q: painting tools only.
    #[command(modal = "declared", label = "Behind", default = "Shift+Alt+Q")]
    Behind,
    /// Shift+Alt+R: painting tools only (paints transparency).
    #[command(modal = "declared", label = "Clear", default = "Shift+Alt+R")]
    Clear,
    /// Shift+Alt+P: groups only.
    #[command(modal = "declared", label = "Pass Through", default = "Shift+Alt+P")]
    PassThrough,
    /// Shift+Alt+K.
    #[command(modal = "declared", label = "Darken", default = "Shift+Alt+K")]
    Darken,
    /// Shift+Alt+M.
    #[command(modal = "declared", label = "Multiply", default = "Shift+Alt+M")]
    Multiply,
    /// Shift+Alt+B.
    #[command(modal = "declared", label = "Color Burn", default = "Shift+Alt+B")]
    ColorBurn,
    /// Shift+Alt+A.
    #[command(modal = "declared", label = "Linear Burn", default = "Shift+Alt+A")]
    LinearBurn,
    /// Shift+Alt+G.
    #[command(modal = "declared", label = "Lighten", default = "Shift+Alt+G")]
    Lighten,
    /// Shift+Alt+S.
    #[command(modal = "declared", label = "Screen", default = "Shift+Alt+S")]
    Screen,
    /// Shift+Alt+D.
    #[command(modal = "declared", label = "Color Dodge", default = "Shift+Alt+D")]
    ColorDodge,
    /// Shift+Alt+W.
    #[command(modal = "declared", label = "Linear Dodge", default = "Shift+Alt+W")]
    LinearDodge,
    /// Shift+Alt+O.
    #[command(modal = "declared", label = "Overlay", default = "Shift+Alt+O")]
    Overlay,
    /// Shift+Alt+F.
    #[command(modal = "declared", label = "Soft Light", default = "Shift+Alt+F")]
    SoftLight,
    /// Shift+Alt+H.
    #[command(modal = "declared", label = "Hard Light", default = "Shift+Alt+H")]
    HardLight,
    /// Shift+Alt+V.
    #[command(modal = "declared", label = "Vivid Light", default = "Shift+Alt+V")]
    VividLight,
    /// Shift+Alt+J.
    #[command(modal = "declared", label = "Linear Light", default = "Shift+Alt+J")]
    LinearLight,
    /// Shift+Alt+Z.
    #[command(modal = "declared", label = "Pin Light", default = "Shift+Alt+Z")]
    PinLight,
    /// Shift+Alt+L.
    #[command(modal = "declared", label = "Hard Mix", default = "Shift+Alt+L")]
    HardMix,
    /// Shift+Alt+E.
    #[command(modal = "declared", label = "Difference", default = "Shift+Alt+E")]
    Difference,
    /// Shift+Alt+X.
    #[command(modal = "declared", label = "Exclusion", default = "Shift+Alt+X")]
    Exclusion,
    /// Shift+Alt+U.
    #[command(modal = "declared", label = "Hue", default = "Shift+Alt+U")]
    Hue,
    /// Shift+Alt+T.
    #[command(modal = "declared", label = "Saturation", default = "Shift+Alt+T")]
    Saturation,
    /// Shift+Alt+C.
    #[command(modal = "declared", label = "Color", default = "Shift+Alt+C")]
    Color,
    /// Shift+Alt+Y.
    #[command(modal = "declared", label = "Luminosity", default = "Shift+Alt+Y")]
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

    /// Whether Photoshop's blend menu starts a new group at this mode: Darken, Lighten, Overlay, Difference and
    /// Hue. Behind, Clear and Pass Through belong to the first group.
    pub const fn starts_group(self) -> bool {
        matches!(self, BlendShortcut::Darken | BlendShortcut::Lighten | BlendShortcut::Overlay | BlendShortcut::Difference | BlendShortcut::Hue)
    }
}

/// The entries of a blend mode list, with a separator above each group head ([`BlendShortcut::starts_group`]) and
/// none first. An unlettered mode never heads a group. `label` gives each mode's text. One rule for every blend
/// list: the Layers panel, tool modes, parameter dialogs and Layer Style.
pub fn blend_entries<K: Clone>(modes: &[BlendKeyEntry<K>], label: impl Fn(&BlendKeyEntry<K>) -> Label) -> Vec<Entry<K>> {
    let mut entries = Vec::with_capacity(modes.len() + 5);
    for (at, mode) in modes.iter().enumerate() {
        if at > 0 && mode.shortcut.is_some_and(BlendShortcut::starts_group) {
            entries.push(Entry::separator());
        }
        entries.push(Entry::new(mode.key.clone(), label(mode)));
    }
    entries
}

/// Shift+= and Shift+-: the next and the previous mode of the target's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "blendMode")]
#[non_exhaustive]
pub enum BlendStep {
    /// Shift+=: the next mode, wrapping at the end.
    #[command(modal = "declared", label = "Next Blend Mode", default = "Shift+=")]
    Next,
    /// Shift+-: the previous mode, wrapping at the start.
    #[command(modal = "declared", label = "Previous Blend Mode", default = "Shift+-")]
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
/// - An entry [`enabled`](Self::enabled) rules out is never picked: a step goes on past it the same way, staying put
///   when no other entry is enabled, and its letter picks nothing (the command is still taken).
/// - A key release picks nothing; holding a step key repeats the step.
/// - No AccessKit node: the app's blend control announces the new mode. Keys are not directional, so nothing mirrors
///   in a right-to-left interface.
#[must_use = "read the fired command with `fired`"]
pub struct BlendKeys<'a, K> {
    entries: &'a [BlendKeyEntry<K>],
    current: Option<&'a K>,
    enabled: Option<&'a dyn Fn(&K) -> bool>,
}

impl<'a, K: Clone + PartialEq> BlendKeys<'a, K> {
    /// The keys over `entries`, in menu order.
    pub fn new(entries: &'a [BlendKeyEntry<K>]) -> Self {
        Self { entries, current: None, enabled: None }
    }

    /// Which entries the keys may pick: a greyed mode is skipped. Every entry when not set.
    pub fn enabled(mut self, enabled: &'a dyn Fn(&K) -> bool) -> Self {
        self.enabled = Some(enabled);
        self
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
        let enabled = |key: &K| self.enabled.is_none_or(|enabled| enabled(key));
        if let Some(step) = BlendStep::from_ref(command) {
            let step = match step {
                BlendStep::Next => 1,
                BlendStep::Previous => -1,
            };
            return stepped(self.entries, self.current, step, &enabled).map(|entry| BlendKeyPick::Stepped(entry.key.clone()));
        }
        let shortcut = BlendShortcut::from_ref(command)?;
        self.entries
            .iter()
            .find(|entry| entry.shortcut == Some(shortcut))
            .filter(|entry| enabled(&entry.key))
            .map(|entry| BlendKeyPick::Lettered(entry.key.clone()))
    }
}

/// The first enabled entry `step` places after (or before, when negative) `current` in `entries`, wrapping; an
/// unlisted or absent `current` steps from the top (`step > 0`) or the bottom. `None` for an empty list, or when no
/// entry other than `current` is enabled.
fn stepped<'e, K: PartialEq>(
    entries: &'e [BlendKeyEntry<K>],
    current: Option<&K>,
    step: i32,
    enabled: &dyn Fn(&K) -> bool,
) -> Option<&'e BlendKeyEntry<K>> {
    // pc: crates/ui-egui/src/input/blend_keys.rs:18 `stepped` (`rem_euclid`; unlisted: 0 or n − 1); fade_modes.rs
    // `step_past` for greyed modes.
    let n = i32::try_from(entries.len()).ok().filter(|n| *n > 0)?;
    let at = current.and_then(|c| entries.iter().position(|e| e.key == *c)).and_then(|i| i32::try_from(i).ok());
    let first = match at {
        Some(i) => (i + step).rem_euclid(n),
        None if step > 0 => 0,
        None => n - 1,
    };
    (0..n)
        .map(|k| (first + k * step.signum()).rem_euclid(n))
        .take_while(|i| Some(*i) != at)
        .filter_map(|i| entries.get(usize::try_from(i).ok()?))
        .find(|entry| enabled(&entry.key))
}
