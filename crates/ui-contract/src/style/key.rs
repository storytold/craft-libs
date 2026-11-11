//! Keys: the instance inputs a style depends on (`StyleKey`), the environment it depends on (`EnvKey`), and the
//! ink text and icons read on a surface.

use crate::runtime::env::Scale;
use craft_host::target::Os;
use craft_i18n::Direction;
use std::hash::{Hash, Hasher};

/// A `'static` key of the instance inputs a style depends on: unique per component type, so two components that
/// share a `Style` type have separate cache entries. A component with no inputs declares a unit struct.
pub trait StyleKey: Copy + Eq + Hash + Send + Sync + 'static {}

/// The text and icon colour that reads on a surface, resolved at step 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Ink {
    /// On panels, raised and sunken surfaces: the normal text colour.
    #[default]
    OnPanel,
    /// On strong fills and scrims: the colour that reads on them.
    OnDark,
    /// Accent-coloured text on panels and sunken surfaces: a prominent readout.
    Accent,
}

/// The key of everything a style depends on besides its own key: tokens, direction, scale, the OS, the window's
/// active state and the style-affecting app values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct EnvKey(u64);

impl EnvKey {
    /// The key of these inputs. `tokens` is `UiTokens::key` and `values` the hash of the style-affecting app values.
    pub fn new(tokens: u64, direction: Direction, scale: Scale, os: Os, active: bool, values: u64) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tokens.hash(&mut hasher);
        direction.hash(&mut hasher);
        scale.0.to_bits().hash(&mut hasher);
        os.hash(&mut hasher);
        active.hash(&mut hasher);
        values.hash(&mut hasher);
        EnvKey(hasher.finish())
    }

    /// This key with the line height per point of the interface stack's leading face folded in: control heights follow it.
    pub(crate) fn with_line(self, line_ratio: f32) -> Self {
        self.folded(u64::from(line_ratio.to_bits()))
    }

    /// This key with one more input folded in: a face's styles depend on the host's ink as well.
    pub(crate) fn folded(self, input: u64) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.0.hash(&mut hasher);
        input.hash(&mut hasher);
        EnvKey(hasher.finish())
    }
}
