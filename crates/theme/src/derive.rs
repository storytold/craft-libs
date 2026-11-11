//! Colour derivation: seeds to ramps, ramps to tokens, the background greys anchored on the
//! panel (state variants on their pinned bases), pins, then the contrast pass.

use std::collections::{BTreeMap, BTreeSet};

use super::color::{Oklab, Oklch, Rgba};
use super::contrast::{contrast_ratio, ensure_contrast, relative_luminance};
use super::ramp::{self, Mode, Ramp, RampKind};
use super::theme::{Contrast, PinLayer, Pins, ThemeDef};
use super::tokens::{ColorToken, ColorTokens, Status, TokenRef};
use super::values::{self, Bg, ColorSource as Source, PairRule, RULES};
use crate::AppId;

/// A contrast pair with its measured ratio and its minimum.
/// [`Resolved::warnings`](crate::Resolved::warnings) lists those below their minimum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContrastCheck {
    /// The foreground token or field.
    pub fg: TokenRef,
    /// The background token or field. Overlay pairs name `overlay.halo` and report the lower of
    /// the ratios over black and over white.
    pub bg: TokenRef,
    /// The measured ratio.
    pub ratio: f64,
    /// The required ratio.
    pub minimum: f64,
}

impl ContrastCheck {
    /// Whether the ratio reaches the minimum.
    pub fn passes(&self) -> bool {
        self.ratio >= self.minimum
    }
}

/// Most times a foreground is moved while fixing it against several backgrounds.
const MAX_ROUNDS: usize = 8;

/// The colour ramps of one theme in one mode, which the colour tokens take their steps from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ramps {
    /// From the neutral seed.
    pub neutral: Ramp,
    /// From the accent seed.
    pub accent: Ramp,
    /// From each status seed.
    pub status: Status<Ramp>,
}

impl Ramps {
    /// The ramp `source` takes its steps from, the step it uses and, for a mix, the step it
    /// moves toward; `None` for sources that use no ramp.
    pub fn steps_of(&self, source: Source) -> Option<(&Ramp, usize, Option<usize>)> {
        Some(match source {
            Source::Neutral(n) | Source::Anchored(n) | Source::StateVariant { step: n, .. } | Source::NeutralAlpha(n, _) => (&self.neutral, n, None),
            Source::Accent(n) => (&self.accent, n, None),
            Source::AccentMix { from, to, .. } => (&self.accent, from, Some(to)),
            Source::Status(kind, n) => (self.status.get(kind), n, None),
            Source::StatusMix { kind, from, to, .. } => (self.status.get(kind), from, Some(to)),
            Source::Fixed(_)
            | Source::Oklch(..)
            | Source::Same(_)
            | Source::Faded(..)
            | Source::SolidFrom(_)
            | Source::ShiftedFrom { .. }
            | Source::OnSolid(_)
            | Source::Lighting(_) => return None,
        })
    }
}

/// What an anchored token keeps its curve distance from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Anchoring {
    /// `surface.panel`, or the pinned base of a [`StateVariant`](Source::StateVariant).
    pub on: ColorToken,
    /// The twelve neutral steps at their curve distance from it (in its hue and chroma when it
    /// is pinned); the token's target is the step its source names.
    pub steps: Ramp,
}

/// How the contrast pass last moved a token that serves as a background, and the foreground it
/// moved for. [`explain`](crate::explain) reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PassMove {
    /// An anchored grey pulled back toward its anchor, no further than this foreground needed.
    Pulled(ColorToken),
    /// A state fill moved away from this foreground with high contrast, where the foreground
    /// alone couldn't reach the minimum.
    Relaxed(ColorToken),
    /// A solid fill moved away from this text, which is white or black.
    Solid(ColorToken),
}

/// The intermediate values of [`colors`], which [`explain`](crate::explain) reports.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ColorSteps {
    pub ramps: Ramps,
    /// What each token's source gives without its own pin (see [`derived_values`]).
    pub derived: ColorTokens,
    /// Each pinned token and the layer whose pin it takes.
    pub pins: BTreeMap<ColorToken, PinLayer>,
    /// After pins, before the contrast pass.
    pub before_pass: ColorTokens,
    /// Each anchored token and what it keeps its curve distance from.
    pub anchors: BTreeMap<ColorToken, Anchoring>,
    /// Each token the contrast pass moved as a background, with the move that set its final
    /// value.
    pub moves: BTreeMap<ColorToken, PassMove>,
    /// Every pair of [`RULES`] on `before_pass`.
    pub pairs_before: Vec<ContrastCheck>,
    /// The same pairs on the final colours.
    pub pairs: Vec<ContrastCheck>,
}

impl ColorSteps {
    /// The pairs below their minimum.
    pub fn warnings(&self) -> impl Iterator<Item = ContrastCheck> + '_ {
        self.pairs.iter().filter(|m| !m.passes()).copied()
    }
}

/// Derives every colour of `theme` in `mode`: ramp defaults, anchored greys moved to their
/// anchors' values, pins, then the contrast pass, which may pull anchored greys back toward their
/// anchors. Returns the colours and the steps on the way, with every pair measured on the result.
pub(super) fn colors(theme: &ThemeDef, app: Option<&AppId>, mode: Mode, contrast: Contrast) -> (ColorTokens, ColorSteps) {
    let seeds = &theme.seeds_for(mode);
    let ramps = Ramps {
        neutral: Ramp::from_seed(seeds.neutral, mode, RampKind::Neutral),
        accent: Ramp::from_seed(seeds.accent, mode, RampKind::Chromatic),
        status: seeds.status.map(|&s| Ramp::from_seed(s, mode, RampKind::Chromatic)),
    };
    let high = contrast == Contrast::High;

    let mut c = ColorTokens::splat(Rgba::BLACK);
    for &t in ColorToken::ALL {
        c.set(t, direct(&ramps, values::source(t, mode)).unwrap_or(Rgba::BLACK));
    }
    let pins: Vec<_> = pins(theme, app, mode, high).collect();
    let pin = |t: ColorToken| pins.iter().rev().find(|(_, p, _)| *p == TokenRef::Color(t)).map(|&(_, _, v)| v);
    let anchors = Anchors::new(&ramps, seeds.neutral, mode, pin);
    for &t in ColorToken::ALL {
        if let Some((a, n, _)) = anchors.of(t) {
            c.set(t, a.target(n));
        }
    }
    let unpinned = c;
    let mut layers = BTreeMap::new();
    for (layer, t, v) in pins {
        if let TokenRef::Color(t) = t {
            c.set(t, v);
            layers.insert(t, layer);
        }
    }
    let pinned: BTreeSet<ColorToken> = layers.keys().copied().collect();
    // Tokens defined by other tokens follow their pinned values; aliases follow the contrast
    // pass too. On-solid text, solids taken from other tokens and aliases that are a pair's
    // foreground are the pass's own choice, so they are not re-derived afterwards.
    follow(&mut c, &unpinned, &pinned, mode, true);
    if high {
        // A boundary that has to be identifiable is drawn opaque.
        c.set(ColorToken::AccentBorder, over(c.accent.border, c.surface.panel));
    }
    let before_pass = c;
    let pass = Pass { pinned: &pinned, ramp: &unpinned, high, mode, anchors: &anchors };
    let mut moves = Moves::new();
    for (i, rule) in RULES.iter().enumerate() {
        fix_rule(&mut c, &pass, rule, &RULES[..=i], &mut moves);
    }
    follow(&mut c, &unpinned, &pinned, mode, false);
    // An unpinned outline is the accent the pass settled on, which later rules may still move.
    if !pinned.contains(&ColorToken::AccentBorder) {
        c.set(ColorToken::AccentBorder, c.accent.solid);
    }
    moves.retain(|&t, _| c.get(t) != before_pass.get(t));
    let steps = ColorSteps {
        ramps,
        derived: derived_values(&unpinned, &before_pass, &pinned, mode),
        pins: layers,
        before_pass,
        anchors: anchors.report(),
        moves,
        pairs_before: measure(&before_pass, RULES, high),
        pairs: measure(&c, RULES, high),
    };
    (c, steps)
}

/// What each token's source gives without the token's own pin, from the recorded steps: the
/// ramp value of a direct source; for a token defined by other tokens, its value before the
/// contrast pass when it is not pinned (it followed its targets' pins), else what its source
/// gives on the unpinned values. `unpinned` holds the direct sources' values.
fn derived_values(unpinned: &ColorTokens, before_pass: &ColorTokens, pinned: &BTreeSet<ColorToken>, mode: Mode) -> ColorTokens {
    let mut out = *unpinned;
    for &t in ColorToken::ALL {
        let value = match values::source(t, mode) {
            Source::Same(_) | Source::Faded(..) | Source::SolidFrom(_) | Source::ShiftedFrom { .. } | Source::OnSolid(_) | Source::Lighting(_)
                if !pinned.contains(&t) =>
            {
                before_pass.get(t)
            }
            Source::Same(other) | Source::SolidFrom(other) => unpinned.get(other),
            Source::Faded(other, a) => unpinned.get(other).with_alpha(a),
            Source::ShiftedFrom { base, from, to, away_from } => {
                shifted(unpinned.get(base), unpinned.get(from), unpinned.get(to), unpinned.get(away_from))
            }
            // Lit like the anchored greys: from the panel's own value, pinned or not.
            Source::Lighting(amount) => lighting(before_pass.surface.panel, amount),
            Source::OnSolid(solids) => best_extreme(unpinned, solids),
            _ => continue,
        };
        out.set(t, value);
    }
    out
}

/// The mode's pins, then the app's, then, when `high`, the theme's and the app's high-contrast
/// pins, each with its layer; later ones win.
fn pins<'a>(theme: &'a ThemeDef, app: Option<&AppId>, mode: Mode, high: bool) -> impl Iterator<Item = (PinLayer, TokenRef, Rgba)> + 'a {
    let app = app.and_then(|a| theme.apps.get(a));
    let layers = [
        Some((PinLayer::Theme, &theme.pins)),
        app.map(|o| (PinLayer::App, &o.pins)),
        high.then_some((PinLayer::ThemeHighContrast, &theme.high_contrast)),
        app.filter(|_| high).map(|o| (PinLayer::AppHighContrast, &o.high_contrast)),
    ];
    layers.into_iter().flatten().flat_map(move |(layer, p)| {
        let p: &Pins = p.get(mode);
        p.colors.iter().map(move |(&t, &v)| (layer, t, v))
    })
}

/// The colours the anchored greys keep their curve distance from: `surface.panel`, and each
/// state base the theme pins.
#[derive(Clone, Debug)]
struct Anchors {
    mode: Mode,
    panel: Anchor,
    bases: BTreeMap<ColorToken, Anchor>,
}

impl Anchors {
    /// The anchors of `mode`, with `pin` giving a token's pinned value. A pin equal to the value
    /// its token has without it moves nothing, so its anchor stays the unpinned one.
    fn new(ramps: &Ramps, seed: Rgba, mode: Mode, pin: impl Fn(ColorToken) -> Option<Rgba>) -> Anchors {
        let step = values::panel_step(mode);
        let curve = Anchor { mode, step, shift: 0.0, tint: Tint::Seed(seed) };
        let panel_value = pin(ColorToken::SurfacePanel);
        let panel = panel_value.filter(|&p| p != ramps.neutral.step(step)).map_or(curve, |p| Anchor::on(p, step, mode));
        let backdrop = panel_value.unwrap_or_else(|| ramps.neutral.step(step));
        let mut bases = BTreeMap::new();
        for &t in ColorToken::ALL {
            if let Source::StateVariant { base, .. } = values::source(t, mode)
                && let Source::Anchored(step) = values::source(base, mode)
                && let Some(p) = pin(base).filter(|&p| p != panel.target(step))
            {
                // A translucent base presents itself over the panel, as the contrast pass measures it.
                bases.insert(base, Anchor::on(over(p, backdrop), step, mode));
            }
        }
        Anchors { mode, panel, bases }
    }

    /// The anchor of an anchored token, the step it takes, and the token it is anchored on;
    /// `None` for other tokens.
    fn of(&self, t: ColorToken) -> Option<(&Anchor, usize, ColorToken)> {
        match values::source(t, self.mode) {
            Source::Anchored(n) => Some((&self.panel, n, ColorToken::SurfacePanel)),
            Source::StateVariant { base, step } => {
                Some(self.bases.get(&base).map_or((&self.panel, step, ColorToken::SurfacePanel), |a| (a, step, base)))
            }
            _ => None,
        }
    }

    /// Every anchored token with its anchor's token and steps.
    fn report(&self) -> BTreeMap<ColorToken, Anchoring> {
        let steps = |a: &Anchor| Ramp(std::array::from_fn(|i| a.target(i + 1)));
        ColorToken::ALL.iter().filter_map(|&t| self.of(t).map(|(a, _, on)| (t, Anchoring { on, steps: steps(a) }))).collect()
    }
}

/// One anchor: a colour standing for neutral step `step`, which anchored steps keep their curve
/// distance from.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    mode: Mode,
    step: usize,
    /// How far the anchor's lightness lies from its step on the curve; 0 for the unpinned panel,
    /// which leaves the anchored steps as the ramp has them.
    shift: f64,
    tint: Tint,
}

/// The hue and chroma of anchored steps.
#[derive(Clone, Copy, Debug)]
enum Tint {
    /// The neutral ramp's: the seed's hue, its capped chroma scaled per step.
    Seed(Rgba),
    /// A pinned anchor's hue and chroma, the same at every step: the pin defines the grey family.
    /// Scaling it per step as the ramp does would multiply a light panel's chroma by up to six at
    /// the line steps.
    Pinned { c: f64, h: f64 },
}

impl Anchor {
    /// The anchor of a pinned `color` standing for `step`.
    fn on(color: Rgba, step: usize, mode: Mode) -> Anchor {
        let o = color.to_oklch();
        Anchor { mode, step, shift: o.l - ramp::curve(mode)[step - 1], tint: Tint::Pinned { c: o.c, h: o.h } }
    }

    /// Anchored step `n` at its curve distance from the anchor.
    fn target(&self, n: usize) -> Rgba {
        self.at(n, ramp::curve(self.mode)[n.clamp(1, 12) - 1] + self.shift)
    }

    /// The anchor's lightness: where a pull stops.
    fn home(&self) -> f64 {
        ramp::curve(self.mode)[self.step - 1] + self.shift
    }

    /// Anchored step `n` with lightness `l`, clamped to 0..=1.
    fn at(&self, n: usize, l: f64) -> Rgba {
        match self.tint {
            Tint::Seed(seed) => ramp::neutral_at(seed, n, l),
            Tint::Pinned { c, h } => Oklch { l: l.clamp(0.0, 1.0), c, h }.to_rgba(),
        }
    }
}

/// What the contrast pass may move.
struct Pass<'a> {
    pinned: &'a BTreeSet<ColorToken>,
    /// Every token's ramp default, before pins and the pass.
    ramp: &'a ColorTokens,
    high: bool,
    mode: Mode,
    anchors: &'a Anchors,
}

impl Pass<'_> {
    /// An unpinned anchored background, which the pass may pull toward its anchor, with the
    /// anchor and its step.
    fn pullable(&self, bg: Bg) -> Option<(ColorToken, &Anchor, usize)> {
        let Bg::Token(t) = bg else { return None };
        if self.pinned.contains(&t) {
            return None;
        }
        self.anchors.of(t).map(|(a, n, _)| (t, a, n))
    }
}

/// The moves of the contrast pass so far.
type Moves = BTreeMap<ColorToken, PassMove>;

/// One rule being fixed: what the pass may move, the rule's backgrounds and minimum, and the
/// rules fixed so far, this one included.
struct Step<'a> {
    pass: &'a Pass<'a>,
    bgs: &'a [Bg],
    min: f64,
    done: &'a [PairRule],
}

impl Step<'_> {
    /// Whether setting `t` to `value` makes a pair of the rules fixed so far fail on `t` that
    /// passes now. A move for one pair never breaks another on the same background, so a later
    /// move can't undo an earlier rule.
    fn breaks(&self, c: &ColorTokens, t: ColorToken, value: Rgba) -> bool {
        let mut moved = *c;
        moved.set(t, value);
        let bg = Bg::Token(t);
        self.done.iter().filter(|r| r.bgs.contains(&bg)).any(|r| {
            let min = minimum(r, self.pass.high);
            r.fgs.iter().any(|&g| ratio(c.get(g), bg_color(c, bg)) >= min && ratio(moved.get(g), bg_color(&moved, bg)) < min)
        })
    }
}

/// Bisection steps when pulling an anchored background toward the panel; finer than a channel step.
const PULL_STEPS: usize = 24;

/// The value of a source that needs no other token, or `None` for one that does. An anchored
/// source gives its ramp step; [`Anchors`] moves it.
fn direct(r: &Ramps, src: Source) -> Option<Rgba> {
    Some(match src {
        Source::Neutral(n) | Source::Anchored(n) | Source::StateVariant { step: n, .. } => r.neutral.step(n),
        Source::NeutralAlpha(n, a) => r.neutral.step(n).with_alpha(values::alpha(a)),
        Source::Accent(n) => r.accent.step(n),
        Source::AccentMix { from, to, t } => mix(r.accent.step(from), r.accent.step(to), t),
        Source::Status(kind, n) => r.status.get(kind).step(n),
        Source::StatusMix { kind, from, to, t } => mix(r.status.get(kind).step(from), r.status.get(kind).step(to), t),
        Source::Fixed(c) => c,
        Source::Oklch(l, c, h) => super::color::Oklch { l, c, h }.to_rgba(),
        Source::Same(_) | Source::Faded(..) | Source::SolidFrom(_) | Source::ShiftedFrom { .. } | Source::OnSolid(_) | Source::Lighting(_) => {
            return None;
        }
    })
}

/// Sets every unpinned token whose source refers to other tokens; on-solid text, solids taken
/// from other tokens and aliases that are a pair's foreground only when `on_solid`.
fn follow(c: &mut ColorTokens, ramp: &ColorTokens, pinned: &BTreeSet<ColorToken>, mode: Mode, on_solid: bool) {
    for &t in ColorToken::ALL {
        if pinned.contains(&t) {
            continue;
        }
        match values::source(t, mode) {
            Source::Same(other) if on_solid || !is_foreground(t) => c.set(t, c.get(other)),
            Source::Faded(other, a) => c.set(t, c.get(other).with_alpha(a)),
            Source::Lighting(amount) => c.set(t, lighting(c.surface.panel, amount)),
            Source::SolidFrom(other) if on_solid => c.set(t, c.get(other)),
            Source::ShiftedFrom { base, from, to, away_from } if on_solid => {
                c.set(t, shifted(c.get(base), ramp.get(from), ramp.get(to), c.get(away_from)))
            }
            Source::OnSolid(solids) if on_solid => c.set(t, best_extreme(c, solids)),
            _ => {}
        }
    }
}

/// Whether `t` is the foreground of a pair: an alias that is one starts from its target and the
/// pass may then move it away (`overlay.handle_stroke` from `overlay.stroke`). Read from
/// [`RULES`], so a new rule changes which aliases follow this way;
/// `semantic_tests::the_aliases_the_contrast_pass_moves_are_pinned` lists them.
pub(crate) fn is_foreground(t: ColorToken) -> bool {
    RULES.iter().any(|rule| rule.fgs.contains(&t))
}

/// `panel` with its OKLCH lightness moved by `amount`, clamped to 0..=1, hue and chroma kept.
fn lighting(panel: Rgba, amount: f64) -> Rgba {
    let o = panel.to_oklch();
    Oklch { l: (o.l + amount).clamp(0.0, 1.0), ..o }.to_rgba()
}

/// `base` with its OKLCH lightness moved by the distance between `from` and `to`, away from
/// `text`: darker under lighter text, lighter under darker. Clamped to 0..=1, hue and chroma kept.
fn shifted(base: Rgba, from: Rgba, to: Rgba, text: Rgba) -> Rgba {
    let o = base.to_oklch();
    let step = (to.to_oklch().l - from.to_oklch().l).abs();
    let sign = if relative_luminance(text) >= relative_luminance(base) { -1.0 } else { 1.0 };
    Oklch { l: (o.l + sign * step).clamp(0.0, 1.0), ..o }.to_rgba().with_alpha(base.a())
}

/// White or black, whichever has the higher lowest ratio on `solids`.
fn best_extreme(c: &ColorTokens, solids: &[ColorToken]) -> Rgba {
    let worst = |e: Rgba| solids.iter().map(|&s| contrast_ratio(e, bg_color(c, Bg::Token(s)))).fold(f64::INFINITY, f64::min);
    if worst(Rgba::WHITE) >= worst(Rgba::BLACK) { Rgba::WHITE } else { Rgba::BLACK }
}

/// Linear interpolation in OKLab, opaque.
pub(crate) fn mix(a: Rgba, b: Rgba, t: f64) -> Rgba {
    let (a, b) = (a.to_oklab(), b.to_oklab());
    let lerp = |x: f64, y: f64| x + (y - x) * t;
    Oklab { l: lerp(a.l, b.l), a: lerp(a.a, b.a), b: lerp(a.b, b.b) }.to_oklch().to_rgba()
}

/// `fg` alpha-composited over `bg` (sRGB channel blend, as renderers draw it); the result is
/// opaque when `bg` is.
pub fn over(fg: Rgba, bg: Rgba) -> Rgba {
    let a = f64::from(fg.a()) / 255.0;
    let ch = |i: usize| (f64::from(fg.0[i]) * a + f64::from(bg.0[i]) * (1.0 - a)).round() as u8;
    Rgba([ch(0), ch(1), ch(2), bg.a()])
}

/// The colour a background presents: a translucent token over `surface.panel`, as the
/// component pass measures it. Surfaces are opaque, so they present themselves.
fn bg_color(c: &ColorTokens, bg: Bg) -> Rgba {
    match bg {
        Bg::Token(t) => over(c.get(t), c.surface.panel),
        Bg::HaloOver(backdrop) => over(c.get(ColorToken::OverlayHalo), backdrop),
    }
}

/// The token a background reports as.
fn bg_token(bg: Bg) -> ColorToken {
    match bg {
        Bg::Token(t) => t,
        Bg::HaloOver(_) => ColorToken::OverlayHalo,
    }
}

/// The ratio of `fg` on `bg`, compositing a translucent `fg` first.
fn ratio(fg: Rgba, bg: Rgba) -> f64 {
    contrast_ratio(over(fg, bg), bg)
}

/// The lowest ratio of `fg` over `bgs`, and the background that gives it.
fn worst(c: &ColorTokens, fg: Rgba, bgs: &[Bg]) -> Option<(f64, Rgba)> {
    bgs.iter().map(|&b| bg_color(c, b)).map(|bg| (ratio(fg, bg), bg)).min_by(|x, y| x.0.total_cmp(&y.0))
}

/// The lowest ratio of `fg` over `bgs`; infinite when there are none.
fn worst_ratio(c: &ColorTokens, fg: Rgba, bgs: &[Bg]) -> f64 {
    worst(c, fg, bgs).map_or(f64::INFINITY, |(r, _)| r)
}

fn minimum(rule: &PairRule, high: bool) -> f64 {
    if high { rule.high } else { rule.standard }
}

/// Applies one rule. A pinned foreground is left alone in standard contrast and fixed in high;
/// the anchored backgrounds it fails on are pulled toward their anchors either way.
fn fix_rule(c: &mut ColorTokens, pass: &Pass, rule: &PairRule, done: &[PairRule], moves: &mut Moves) {
    let (pinned, high) = (pass.pinned, pass.high);
    // An unpinned solid taken from another token starts from that token's value as the rules
    // fixed so far left it.
    for &b in rule.bgs {
        if let Bg::Token(t) = b
            && !pinned.contains(&t)
        {
            match values::source(t, pass.mode) {
                Source::SolidFrom(other) => c.set(t, c.get(other)),
                Source::ShiftedFrom { base, from, to, away_from } => {
                    c.set(t, shifted(c.get(base), pass.ramp.get(from), pass.ramp.get(to), c.get(away_from)));
                }
                _ => {}
            }
        }
    }
    let s = Step { pass, bgs: rule.bgs, min: minimum(rule, high), done };
    // A text rule may move a state fill in high contrast when the text alone can't reach `min`.
    let moves_fills = high && !rule.on_solid;
    for round in 0..2 {
        for &fg in rule.fgs {
            if pinned.contains(&fg) && !high {
                // Pinned text stays and the pairs it misses are reported. The anchored greys
                // under it still move; the solids under it stay where the theme's ramps put them
                // (PhotoCraft's white on its accent must not darken the hover and pressed
                // accents), and move in high contrast.
                if !rule.on_solid {
                    pull(c, &s, fg, moves);
                }
                continue;
            }
            if rule.on_solid {
                fix_on_solid(c, &s, fg, moves);
            } else {
                fix_text(c, &s, fg, moves);
                // After the pull: the pull keeps an anchored fill between its target and its
                // anchor, and this fallback moves it further, past the anchor if it must. Both
                // refuse a move that breaks a pair fixed so far, so neither undoes the other.
                if moves_fills {
                    relax_state_fills(c, &s, fg, moves);
                }
            }
        }
        // The second round re-fixes foregrounds that a moved fill pushed below the minimum.
        if round == 0 && !moves_fills {
            break;
        }
    }
}

/// Fixes `fg` on the step's backgrounds, moving the text first so the theme's greys stay where
/// they are, then pulling the anchored backgrounds it still fails on toward their anchors. When
/// that leaves pairs failing, it also tries fixing the text only on the backgrounds that can't
/// be pulled and pulling the rest, and keeps whichever leaves fewer pairs failing. Text between a
/// pinned fill and an anchored one needs that: the colour best on both may pass on neither,
/// while the colour that passes on the pinned fill passes on the other once it is pulled.
fn fix_text(c: &mut ColorTokens, s: &Step, fg: ColorToken, moves: &mut Moves) {
    let failing = |c: &ColorTokens| s.bgs.iter().filter(|&&b| ratio(c.get(fg), bg_color(c, b)) < s.min).count();
    let trial = |fixed: &[Bg]| {
        let (mut t, mut m) = (*c, Moves::new());
        fix_fg(&mut t, fg, fixed, s.min);
        pull(&mut t, s, fg, &mut m);
        (failing(&t), t, m)
    };
    let mut best = trial(s.bgs);
    if best.0 > 0 {
        let fixed: Vec<Bg> = s.bgs.iter().copied().filter(|&b| s.pass.pullable(b).is_none()).collect();
        let other = trial(&fixed);
        if other.0 < best.0 {
            best = other;
        }
    }
    *c = best.1;
    moves.extend(best.2);
}

/// Pulls each unpinned anchored background that `fg` fails on toward its anchor's lightness,
/// along the anchor's hue and chroma, just far enough. A background stays where it is when `fg`
/// fails on it even at the anchor's lightness (the pair would fail on the anchor too), or when
/// the pull would make another pair of the rules fixed so far fail on it.
fn pull(c: &mut ColorTokens, s: &Step, fg: ColorToken, moves: &mut Moves) {
    let f = c.get(fg);
    for &bg in s.bgs {
        let Some((t, a, n)) = s.pass.pullable(bg) else { continue };
        if ratio(f, bg_color(c, bg)) >= s.min {
            continue;
        }
        let (from, to) = (c.get(t).to_oklch().l, a.home());
        let at = |x: f64| a.at(n, from + (to - from) * x);
        let passes = |x: f64| ratio(f, at(x)) >= s.min;
        if !passes(1.0) {
            continue;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..PULL_STEPS {
            let mid = (lo + hi) / 2.0;
            if passes(mid) { hi = mid } else { lo = mid }
        }
        let value = at(hi);
        if !s.breaks(c, t, value) {
            c.set(t, value);
            moves.insert(t, PassMove::Pulled(fg));
        }
    }
}

/// Moves each state fill among the step's backgrounds that `fg` still fails on, away from `fg`,
/// just far enough, unless that breaks another pair fixed so far. A translucent fill that moves
/// becomes opaque.
fn relax_state_fills(c: &mut ColorTokens, s: &Step, fg: ColorToken, moves: &mut Moves) {
    use ColorToken::{StateHover, StatePressed, StateSelected, StateSelectedHover};
    for &bg in s.bgs {
        let Bg::Token(t @ (StateHover | StatePressed | StateSelected | StateSelectedHover)) = bg else { continue };
        let (f, b) = (c.get(fg), bg_color(c, bg));
        if ratio(f, b) < s.min {
            let moved = ensure_contrast(b, f.with_alpha(255), s.min);
            if !s.breaks(c, t, moved) {
                c.set(t, moved);
                moves.insert(t, PassMove::Relaxed(fg));
            }
        }
    }
}

/// Moves each solid among the step's backgrounds that the text `fg` fails on away from it, just
/// far enough; pinned solids only with high contrast. Moved solids are opaque.
fn move_solids(c: &mut ColorTokens, s: &Step, fg: ColorToken, moves: &mut Moves) {
    let text = c.get(fg);
    for &b in s.bgs {
        let (solid, bg) = (bg_token(b), bg_color(c, b));
        if (s.pass.high || !s.pass.pinned.contains(&solid)) && ratio(text, bg) < s.min {
            c.set(solid, ensure_contrast(bg, text.with_alpha(255), s.min));
            moves.insert(solid, PassMove::Solid(fg));
        }
    }
}

/// Moves `fg`'s lightness until it passes on every background, fixing against the hardest one
/// each round. A translucent `fg` that has to move becomes opaque so the move is exact.
///
/// When no colour passes everywhere, `fg` ends as the colour with the best lowest ratio among
/// its value before the pass, each round's value, white and black, preferring the value before
/// the pass on ties. A candidate with a lower ratio on the surfaces among `bgs` than the value
/// before the pass is never chosen: text that reads on the surfaces is not traded for a fill.
fn fix_fg(c: &mut ColorTokens, fg: ColorToken, bgs: &[Bg], min: f64) {
    let before = c.get(fg);
    let mut tried = vec![before];
    for _ in 0..MAX_ROUNDS {
        let f = c.get(fg);
        let Some((r, bg)) = worst(c, f, bgs) else { return };
        if r >= min {
            return;
        }
        let moved = ensure_contrast(f.with_alpha(255), bg, min);
        if moved == f {
            break;
        }
        c.set(fg, moved);
        tried.push(moved);
    }
    if worst_ratio(c, c.get(fg), bgs) >= min {
        return;
    }
    // Backgrounds on opposite sides of the foreground (the halo over black and over white) can
    // leave the hue-keeping move short; white or black then reaches the minimum when any colour does.
    let best = if worst_ratio(c, Rgba::WHITE, bgs) >= worst_ratio(c, Rgba::BLACK, bgs) { Rgba::WHITE } else { Rgba::BLACK };
    if worst_ratio(c, best, bgs) >= min {
        c.set(fg, best);
        return;
    }
    let surfaces: Vec<Bg> = bgs.iter().copied().filter(|&b| matches!(b, Bg::Token(t) if t.is_opaque_surface())).collect();
    let floor = worst_ratio(c, before, &surfaces);
    let mut keep = (before, worst_ratio(c, before, bgs));
    for &candidate in tried[1..].iter().chain(&[Rgba::WHITE, Rgba::BLACK]) {
        let r = worst_ratio(c, candidate, bgs);
        if r > keep.1 && worst_ratio(c, candidate, &surfaces) >= floor {
            keep = (candidate, r);
        }
    }
    c.set(fg, keep.0);
}

/// Keeps text on solid fills that already passes; otherwise picks white or black. When an
/// extreme can't reach the minimum on a fill, the fill moves away from it (if it may move). Of
/// the two extremes, keeps the one that leaves the fewest failing pairs among the rules fixed so
/// far, preferring the derived default (the one with more contrast on the step-9 solids).
fn fix_on_solid(c: &mut ColorTokens, s: &Step, fg: ColorToken, moves: &mut Moves) {
    let tokens: Vec<_> = s.bgs.iter().map(|&b| bg_token(b)).collect();
    let current = c.get(fg);
    if worst(c, current, s.bgs).is_none_or(|(r, _)| r >= s.min) {
        return;
    }
    let preferred = if current == Rgba::WHITE || current == Rgba::BLACK { current } else { best_extreme(c, &tokens) };
    let other = if preferred == Rgba::WHITE { Rgba::BLACK } else { Rgba::WHITE };
    let mut best: Option<(usize, ColorTokens, Moves)> = None;
    for extreme in [preferred, other] {
        let (mut trial, mut m) = (*c, moves.clone());
        trial.set(fg, extreme);
        move_solids(&mut trial, s, fg, &mut m);
        let fails = measure(&trial, s.done, s.pass.high).iter().filter(|m| !m.passes()).count();
        if best.as_ref().is_none_or(|(f, ..)| fails < *f) {
            best = Some((fails, trial, m));
        }
    }
    if let Some((_, trial, m)) = best {
        *c = trial;
        *moves = m;
    }
}

/// Every pair of `rules`, one per foreground and background token. Both halo backdrops report
/// as one pair, with the lower ratio.
fn measure(c: &ColorTokens, rules: &[PairRule], high: bool) -> Vec<ContrastCheck> {
    let mut out: Vec<ContrastCheck> = Vec::new();
    for rule in rules {
        let minimum = minimum(rule, high);
        for &fg in rule.fgs {
            let (f, start) = (c.get(fg), out.len());
            for &bg in rule.bgs {
                let r = ratio(f, bg_color(c, bg));
                let bg = TokenRef::Color(bg_token(bg));
                match out.iter_mut().skip(start).find(|m| m.bg == bg) {
                    Some(m) => m.ratio = m.ratio.min(r),
                    None => out.push(ContrastCheck { fg: TokenRef::Color(fg), bg, ratio: r, minimum }),
                }
            }
        }
    }
    out
}
