//! Why each colour has its value: the steps of [`resolve`] for one theme and appearance, with
//! every contrast pair the rules measure. The gallery's token browser shows them.

use super::app_sizes::{AppSizes, SizeValue, base_path};
use super::color::Rgba;
use super::derive::{Anchoring, ContrastCheck, PassMove, Ramps};
use super::resolve::{self, Resolved};
use super::sizes::{Preset, Slot, leaves};
use super::theme::{Appearance, PinLayer, Size, ThemeDef};
use super::tokens::{ColorToken, StatusKind, TokenRef};
use super::values::{self, ColorSource};
use crate::AppId;

/// The steps of [`crate::resolve()`] for one theme and appearance.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Explained {
    /// Equal to `resolve(theme, appearance, app)`.
    pub resolved: Resolved,
    /// The ramps of the mode used ([`Resolved::variant_used`]).
    pub ramps: Ramps,
    /// One per [`ColorToken::ALL`], in that order.
    pub colors: Vec<ColorOrigin>,
    /// Every pair the contrast rules measure, failing or not, in rule order.
    pub pairs: Vec<PairReport>,
    /// The base size, spacing, radius and type values in the order of a size set file.
    pub sizes: Vec<SizeOrigin>,
}

/// Which layer of the size model set a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SizeLayer {
    /// A base value from the size set.
    SizeSet,
    /// A base value the app overrides.
    App,
}

impl SizeLayer {
    /// Whether the app layer set the value.
    pub fn from_app(self) -> bool {
        self == SizeLayer::App
    }
}

/// How a size, spacing, radius or type value got its value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SizeOrigin {
    /// The path an app's `sizes.json` names it by, such as `size.control.md`.
    pub path: String,
    /// The layer that set it.
    pub layer: SizeLayer,
    /// The final value, after the theme's shape and Text size.
    pub value: SizeValue,
    /// The least and greatest number an app may set.
    pub range: (f32, f32),
}

/// How a colour token got its value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorOrigin {
    /// The token.
    pub token: ColorToken,
    /// Its derivation rule in the mode used.
    pub source: ColorSource,
    /// The value `source` gives without this token's pin, before the contrast pass. A token
    /// defined by other tokens takes their pinned values when it is not pinned itself, so the
    /// value explains what it shows, and their unpinned values when it is.
    pub derived: Rgba,
    /// The layer whose pin the token takes, if it is pinned.
    pub pin: Option<PinLayer>,
    /// The value before the contrast pass, when the pass changed it. Tokens defined by other
    /// tokens count as changed when they follow a token the pass moved.
    pub contrast_move: Option<Rgba>,
    /// For an anchored token ([`Anchored`](ColorSource::Anchored),
    /// [`StateVariant`](ColorSource::StateVariant)), what it keeps its curve distance from.
    pub anchoring: Option<Anchoring>,
    /// How the contrast pass last moved the token as a background, which set its final value;
    /// `contrast_move` then holds the value before the pass.
    pub pass_move: Option<PassMove>,
    /// The final value.
    pub value: Rgba,
}

impl ColorOrigin {
    /// The derivation rule in words, with the contrast pass's move when there is one: "neutral
    /// 4, anchored on surface.panel, pulled toward it for text.muted".
    pub fn source_words(&self) -> String {
        let rule = source_words(self.source, self.anchoring.map(|a| a.on));
        match self.pass_move {
            Some(PassMove::Pulled(fg)) => format!("{rule}, pulled toward it for {}", fg.path()),
            Some(PassMove::Relaxed(fg)) => format!("{rule}, moved away from {} for high contrast", fg.path()),
            Some(PassMove::Solid(fg)) => format!("{rule}, moved away from {}", fg.path()),
            None => rule,
        }
    }
}

/// A derivation rule in words, e.g. "neutral 10", "neutral 4, anchored on surface.panel" or
/// "accent 9 → 10 at 0.5". `anchored_on` names what an anchored token keeps its distance from.
fn source_words(source: ColorSource, anchored_on: Option<ColorToken>) -> String {
    let status = |kind: StatusKind| match kind {
        StatusKind::Good => "good",
        StatusKind::Warn => "warn",
        StatusKind::Bad => "bad",
        StatusKind::Info => "info",
    };
    match source {
        ColorSource::Neutral(n) => format!("neutral {n}"),
        ColorSource::Anchored(n) => format!("neutral {n}, anchored on {}", anchored_on.unwrap_or(ColorToken::SurfacePanel).path()),
        ColorSource::StateVariant { base, step } => match anchored_on {
            Some(on) => format!("neutral {step}, anchored on {}", on.path()),
            None => format!("neutral {step}, anchored on {} when pinned, else {}", base.path(), ColorToken::SurfacePanel.path()),
        },
        ColorSource::Lighting(amount) if amount >= 0.0 => format!("{} lit by {amount}", ColorToken::SurfacePanel.path()),
        ColorSource::Lighting(amount) => format!("{} shaded by {}", ColorToken::SurfacePanel.path(), -amount),
        ColorSource::NeutralAlpha(n, a) => format!("neutral {n} at alpha {a}"),
        ColorSource::Faded(t, a) => format!("= {} at alpha {a}", t.path()),
        ColorSource::Accent(n) => format!("accent {n}"),
        ColorSource::AccentMix { from, to, t } => format!("accent {from} → {to} at {t}"),
        ColorSource::Status(kind, n) => format!("status {} {n}", status(kind)),
        ColorSource::StatusMix { kind, from, to, t } => format!("status {} {from} → {to} at {t}", status(kind)),
        ColorSource::Fixed(c) => format!("fixed {}", c.to_hex()),
        ColorSource::Oklch(l, c, h) => format!("oklch {l} {c} {h}"),
        ColorSource::Same(t) | ColorSource::SolidFrom(t) => format!("= {}", t.path()),
        ColorSource::ShiftedFrom { base, from, to, away_from } => {
            format!("= {} moved away from {} as far as {} lies from {}", base.path(), away_from.path(), to.path(), from.path())
        }
        ColorSource::OnSolid([only]) => format!("white or black on {}", only.path()),
        ColorSource::OnSolid(solids) => match solids.first() {
            Some(first) => format!("white or black on {} and {} more solids", first.path(), solids.len() - 1),
            None => "white or black".to_owned(),
        },
    }
}

/// One measured contrast pair. Translucent backgrounds are measured over `surface.panel`;
/// pairs on `overlay.halo` report the lower of the ratios over black and over white.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PairReport {
    /// The pair on the final colours.
    pub pair: ContrastCheck,
    /// The ratio before any contrast pass.
    pub ratio_before: f64,
    /// Whether the contrast pass moved the foreground or the background.
    pub moved_by_pass: bool,
    /// Whether the pair failed before the passes and passes after them.
    pub fixed_by_pass: bool,
}

impl PairReport {
    /// Whether `token` is this pair's foreground or background.
    pub fn involves(&self, token: TokenRef) -> bool {
        self.pair.fg == token || self.pair.bg == token
    }
}

impl Explained {
    /// The origin of `token`.
    pub fn color(&self, token: ColorToken) -> Option<&ColorOrigin> {
        self.colors.iter().find(|o| o.token == token)
    }

    /// Every pair `token` takes part in, as foreground or background.
    pub fn pairs_of(&self, token: TokenRef) -> impl Iterator<Item = &PairReport> + '_ {
        self.pairs.iter().filter(move |p| p.involves(token))
    }
}

/// Resolves `theme` for `appearance` as [`crate::resolve()`] does, and reports how every colour got its
/// value and every contrast pair the rules measure.
pub fn explain(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>) -> Explained {
    explain_sized(theme, appearance, app, &AppSizes::default())
}

/// [`explain`] for the app's size overrides, as [`resolve_sized`](crate::resolve_sized) applies
/// them: `resolved` equals `resolve_sized(theme, appearance, app, sizes)`, and `sizes` reports the
/// layer of every size value.
pub fn explain_sized(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>, sizes: &AppSizes) -> Explained {
    let (resolved, steps) = resolve::resolve_with_steps(theme, appearance, app, sizes);
    let mode = resolved.variant_used;
    let final_colors = &resolved.tokens.color;
    let changed = |before: Rgba, after: Rgba| (before != after).then_some(before);

    let colors: Vec<ColorOrigin> = ColorToken::ALL
        .iter()
        .map(|&token| {
            let value = final_colors.get(token);
            ColorOrigin {
                token,
                source: values::source(token, mode),
                derived: steps.color.derived.get(token),
                pin: steps.color.pins.get(&token).copied(),
                contrast_move: changed(steps.color.before_pass.get(token), value),
                anchoring: steps.color.anchors.get(&token).copied(),
                pass_move: steps.color.moves.get(&token).copied(),
                value,
            }
        })
        .collect();

    let moved = |t: TokenRef| match t {
        TokenRef::Color(t) => colors.iter().any(|o| o.token == t && o.contrast_move.is_some()),
        TokenRef::Component(_) => false,
    };
    let before: Vec<&ContrastCheck> = steps.color.pairs_before.iter().collect();
    let report = |pair: &ContrastCheck| {
        let ratio_before = before.iter().find(|b| b.fg == pair.fg && b.bg == pair.bg).map_or(pair.ratio, |b| b.ratio);
        PairReport {
            pair: *pair,
            ratio_before,
            moved_by_pass: moved(pair.fg) || moved(pair.bg),
            fixed_by_pass: ratio_before < pair.minimum && pair.passes(),
        }
    };
    let mut pairs: Vec<PairReport> = steps.color.pairs.iter().map(report).collect();
    // The variant table's own fill and ink pairs are measured, not derived: no colour moves for them, and a
    // shortfall is a finding about the theme.
    for (fg, bg) in crate::variant::variant_pairs() {
        let (fg, bg) = (TokenRef::Color(fg), TokenRef::Color(bg));
        if pairs.iter().any(|p| p.pair.fg == fg && p.pair.bg == bg) {
            continue;
        }
        let (TokenRef::Color(f), TokenRef::Color(b)) = (fg, bg) else { continue };
        let ratio = crate::contrast::contrast_ratio(final_colors.get(f), final_colors.get(b));
        let minimum = match appearance.contrast {
            crate::Contrast::Standard => values::TEXT_MIN.0,
            crate::Contrast::High => values::TEXT_MIN.1,
        };
        pairs.push(PairReport { pair: ContrastCheck { fg, bg, ratio, minimum }, ratio_before: ratio, moved_by_pass: false, fixed_by_pass: false });
    }

    let sizes = size_origins(&resolved, appearance.size, sizes);
    Explained { resolved, ramps: steps.color.ramps, colors, pairs, sizes }
}

/// The layer of every base value in `resolved`.
fn size_origins(resolved: &Resolved, size: Size, app: &AppSizes) -> Vec<SizeOrigin> {
    let merged = app.merged(size);
    let tokens = &resolved.tokens;
    let mut base = Preset {
        size: tokens.size,
        space: tokens.space,
        radius: tokens.shape.radius,
        edge: tokens.shape.edge,
        rows: tokens.shape.rows,
        stroke: tokens.stroke,
        typography: tokens.typography,
    };
    let mut out = Vec::new();
    for leaf in leaves(&mut base) {
        let Some(path) = leaf.path.strip_prefix("base.").filter(|p| base_path(p)) else { continue };
        let value = match leaf.slot {
            Slot::Num(x) | Slot::Length(x) => SizeValue::Number(*x),
            _ => continue,
        };
        // A theme's own corner radii win over the app's, so an override that did not take is the set's.
        let set_by_app = merged.get(path).is_some_and(|o| !path.starts_with("radius.") || *o == value);
        let layer = if set_by_app { SizeLayer::App } else { SizeLayer::SizeSet };
        out.push(SizeOrigin { path: path.to_owned(), layer, value, range: (leaf.lo, leaf.hi) });
    }
    out
}
