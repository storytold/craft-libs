//! An app's size overrides: layer 2 (base values) of the size model. The app ships them as a JSON block read by [`AppSizes::parse`] and hands them to the
//! resolver in [`AppDefaults`](crate::AppDefaults).

use std::collections::BTreeMap;

use super::format::{self, FormatErrors};
use super::sizes::{Leaf, Preset, Slot, leaves};
use super::theme::{Size, TextSizeSteps};

/// The value of one override: a length in logical pixels; a corner radius of 999 gives pills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeValue {
    /// A length.
    Number(f32),
}

/// Overrides by path, such as `size.control.md`.
pub type SizeMap = BTreeMap<String, SizeValue>;

/// An app's size overrides. A path is a base value (`size.*`, `space.*`, `radius.*`,
/// `stroke.*`, `typography.<role>.size` and `.line_height`), which every size derived from it
/// follows. A size set's own map wins over `all`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppSizes {
    /// For every size set.
    pub all: SizeMap,
    /// For Compact.
    pub compact: SizeMap,
    /// For Comfortable.
    pub comfortable: SizeMap,
    /// The factors of the text-size steps on the type roles. A runtime value of the app: it is
    /// not part of `sizes.json`, so [`AppSizes::parse`] gives the default.
    pub text_steps: TextSizeSteps,
}

/// The base paths an app may override: sizes, spacing, corner radii, strokes and type sizes, not the
/// edge or fonts.
pub(crate) fn base_path(path: &str) -> bool {
    ["size.", "space.", "radius.", "stroke."].iter().any(|p| path.starts_with(p))
        || path.strip_prefix("typography.").is_some_and(|rest| rest.ends_with(".size") || rest.ends_with(".line_height"))
}

/// The path of `path` in a size set file, or `None` when it names no override.
pub(crate) fn target(path: &str) -> Option<String> {
    if !base_path(path) {
        return None;
    }
    let mut base = Preset::default();
    let file_path = format!("base.{path}");
    leaves(&mut base).iter().any(|l| l.path == file_path).then_some(file_path)
}

impl AppSizes {
    /// Reads and checks a `sizes.json` block: `{"all": {...}, "compact": {...}, "comfortable":
    /// {...}}`, each a map from path to a number or switch. Rejects an unknown path, a value of
    /// the wrong kind and one out of range, naming the path; apps call it in a test so a bad
    /// file fails their CI.
    pub fn parse(bytes: &[u8]) -> Result<AppSizes, FormatErrors> {
        format::parse_app_sizes(bytes)
    }

    /// [`AppSizes::parse`] for a file the app embeds: a file that fails is logged, one line per
    /// error, and ignored, so the size set applies as it is.
    pub fn parse_or_default(bytes: &[u8]) -> AppSizes {
        Self::parse(bytes).unwrap_or_else(|errors| {
            for error in &errors.0 {
                log::error!("sizes.json ignored: {error}");
            }
            AppSizes::default()
        })
    }

    /// The block as `sizes.json` text, which [`AppSizes::parse`] reads back.
    pub fn to_json(&self) -> String {
        format::write_app_sizes(self)
    }

    /// Whether no path is overridden.
    pub fn is_empty(&self) -> bool {
        self.all.is_empty() && self.compact.is_empty() && self.comfortable.is_empty()
    }

    /// The overrides for `size`: `all`, with the set's own map over it.
    pub(crate) fn merged(&self, size: Size) -> SizeMap {
        let own = match size {
            Size::Compact => &self.compact,
            Size::Comfortable => &self.comfortable,
        };
        let mut out = self.all.clone();
        out.extend(own.iter().map(|(k, v)| (k.clone(), *v)));
        out
    }

    /// Layer 2: the base overrides for `size` on `base`, before anything derives from it.
    pub(crate) fn apply_base(&self, size: Size, base: &mut Preset) {
        let merged = self.merged(size);
        if merged.is_empty() {
            return;
        }
        for Leaf { path, slot, .. } in leaves(base) {
            let Some(value) = path.strip_prefix("base.").and_then(|p| merged.get(p)) else { continue };
            let SizeValue::Number(n) = value;
            if let Slot::Num(x) | Slot::Length(x) = slot {
                *x = *n;
            }
        }
    }
}
