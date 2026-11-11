//! Colour harmonies (F04.16): groups of colours built from a base colour by turning its hue and
//! scaling its saturation and brightness, the variation grid round each, and five-colour themes.
//! VectorCraft's Color Guide and Color Themes math, on encoded sRGB through HSB.
//!
//! ```no_run
//! use craft_color::harmony::{Guide, GuideOptions, Harmony};
//! use craft_theme::Srgba;
//! let base = Srgba::rgb(0.24, 0.39, 0.87);
//! let triad = Harmony::Triad.apply(base);              // base first, then two partners
//! let guide = Guide::new(base, Harmony::Triad, &GuideOptions::default());
//! ```

use craft_theme::{Hsb, Srgba};

/// A harmony rule: a group of colours built from a base colour, base first. VectorCraft's 21
/// rules, in its menu order ([`Harmony::ALL`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Harmony {
    #[default]
    #[allow(missing_docs)]
    Complementary,
    #[allow(missing_docs)]
    Complementary2,
    #[allow(missing_docs)]
    SplitComplementary,
    #[allow(missing_docs)]
    LeftComplement,
    #[allow(missing_docs)]
    RightComplement,
    #[allow(missing_docs)]
    Analogous,
    #[allow(missing_docs)]
    Analogous2,
    #[allow(missing_docs)]
    Monochromatic,
    #[allow(missing_docs)]
    Shades,
    #[allow(missing_docs)]
    Triad,
    #[allow(missing_docs)]
    Triad2,
    #[allow(missing_docs)]
    Triad3,
    #[allow(missing_docs)]
    Tetrad,
    #[allow(missing_docs)]
    Tetrad2,
    #[allow(missing_docs)]
    Tetrad3,
    #[allow(missing_docs)]
    Compound,
    #[allow(missing_docs)]
    Compound2,
    #[allow(missing_docs)]
    HighContrast,
    #[allow(missing_docs)]
    HighContrast2,
    #[allow(missing_docs)]
    HighContrast3,
    #[allow(missing_docs)]
    Pentagram,
}

/// One colour of a rule: the base turned by `turn` degrees of hue, its saturation scaled by `sat`
/// and its brightness scaled or flipped to the contrasting one.
#[derive(Clone, Copy)]
struct Tone {
    turn: f32,
    sat: f32,
    bright: Bright,
}

#[derive(Clone, Copy)]
enum Bright {
    Times(f32),
    /// Dark for a light base, light for a dark one ([`contrast`]).
    Contrast,
}

const fn sv(turn: f32, sat: f32, bright: f32) -> Tone {
    Tone { turn, sat, bright: Bright::Times(bright) }
}
/// The base's hue turned by `turn`.
const fn t(turn: f32) -> Tone {
    sv(turn, 1.0, 1.0)
}
/// A softer (half as saturated) partner.
const fn soft(turn: f32) -> Tone {
    sv(turn, 0.5, 1.0)
}
/// A deeper (darker) partner.
const fn deep(turn: f32) -> Tone {
    sv(turn, 1.0, 0.6)
}
/// A partner of contrasting brightness.
const fn hc(turn: f32) -> Tone {
    Tone { turn, sat: 1.0, bright: Bright::Contrast }
}

/// The brightness that contrasts with `v`.
fn contrast(v: f32) -> f32 {
    if v >= 0.5 { v * 0.3 } else { 0.7 + 0.3 * v }
}

fn lab(c: Srgba) -> [f64; 3] {
    let l = c.to_lab();
    [l.l, l.a, l.b].map(f64::from)
}

fn hsb(h: f32, s: f32, b: f32) -> Srgba {
    Srgba::from_hsb(Hsb { h, s, b })
}

impl Harmony {
    /// Every rule, in the menu's order.
    pub const ALL: [Harmony; 21] = [
        Harmony::Complementary,
        Harmony::Complementary2,
        Harmony::SplitComplementary,
        Harmony::LeftComplement,
        Harmony::RightComplement,
        Harmony::Analogous,
        Harmony::Analogous2,
        Harmony::Monochromatic,
        Harmony::Shades,
        Harmony::Triad,
        Harmony::Triad2,
        Harmony::Triad3,
        Harmony::Tetrad,
        Harmony::Tetrad2,
        Harmony::Tetrad3,
        Harmony::Compound,
        Harmony::Compound2,
        Harmony::HighContrast,
        Harmony::HighContrast2,
        Harmony::HighContrast3,
        Harmony::Pentagram,
    ];

    /// The rule's id, camelCase as its serialised name ("splitComplementary", "highContrast2"): the
    /// key of its name among the UI's words (`harmony.rule.<id>`).
    pub fn id(self) -> &'static str {
        match self {
            Harmony::Complementary => "complementary",
            Harmony::Complementary2 => "complementary2",
            Harmony::SplitComplementary => "splitComplementary",
            Harmony::LeftComplement => "leftComplement",
            Harmony::RightComplement => "rightComplement",
            Harmony::Analogous => "analogous",
            Harmony::Analogous2 => "analogous2",
            Harmony::Monochromatic => "monochromatic",
            Harmony::Shades => "shades",
            Harmony::Triad => "triad",
            Harmony::Triad2 => "triad2",
            Harmony::Triad3 => "triad3",
            Harmony::Tetrad => "tetrad",
            Harmony::Tetrad2 => "tetrad2",
            Harmony::Tetrad3 => "tetrad3",
            Harmony::Compound => "compound",
            Harmony::Compound2 => "compound2",
            Harmony::HighContrast => "highContrast",
            Harmony::HighContrast2 => "highContrast2",
            Harmony::HighContrast3 => "highContrast3",
            Harmony::Pentagram => "pentagram",
        }
    }

    /// The rule's colours, the base first. Each partner is the base with its hue turned, its
    /// saturation scaled and its brightness scaled or flipped to the contrasting one (dark for a
    /// light base, light for a dark one). Alpha is the base's.
    // inv-vectorcraft: crates/color/src/harmony.rs:178 `apply`, with the `tones` table at :36–70
    // (`t`, `soft`, `deep`, `hc`) and `contrast` at :72; `keep_model` is not needed (sRGB only).
    pub fn apply(self, base: Srgba) -> Vec<Srgba> {
        let Hsb { h, s, b: v } = base.to_hsb();
        std::iter::once(base)
            .chain(self.tones()[1..].iter().map(|tone| {
                let b = match tone.bright {
                    Bright::Times(f) => v * f,
                    Bright::Contrast => contrast(v),
                };
                hsb(h + tone.turn, (s * tone.sat).clamp(0.0, 1.0), b.clamp(0.0, 1.0)).with_alpha(base.a)
            }))
            .collect()
    }

    fn tones(self) -> &'static [Tone] {
        match self {
            Harmony::Complementary => const { &[t(0.0), t(180.0)] },
            Harmony::Complementary2 => const { &[t(0.0), soft(0.0), t(180.0), soft(180.0)] },
            Harmony::SplitComplementary => const { &[t(0.0), t(150.0), t(210.0)] },
            Harmony::LeftComplement => const { &[t(0.0), t(-30.0), t(150.0)] },
            Harmony::RightComplement => const { &[t(0.0), t(30.0), t(210.0)] },
            Harmony::Analogous => const { &[t(0.0), t(30.0), t(-30.0)] },
            Harmony::Analogous2 => const { &[t(0.0), t(30.0), t(60.0), t(-30.0), t(-60.0)] },
            Harmony::Monochromatic => const { &[t(0.0), sv(0.0, 0.5, 1.0), sv(0.0, 1.0, 0.6), sv(0.0, 0.25, 1.0), sv(0.0, 1.0, 0.35)] },
            Harmony::Shades => const { &[t(0.0), sv(0.0, 1.0, 0.8), sv(0.0, 1.0, 0.6), sv(0.0, 1.0, 0.4), sv(0.0, 1.0, 0.2)] },
            Harmony::Triad => const { &[t(0.0), t(120.0), t(240.0)] },
            Harmony::Triad2 => const { &[t(0.0), t(120.0), soft(120.0), t(240.0), soft(240.0)] },
            Harmony::Triad3 => const { &[t(0.0), t(120.0), deep(120.0), t(240.0), deep(240.0)] },
            Harmony::Tetrad => const { &[t(0.0), t(60.0), t(180.0), t(240.0)] },
            Harmony::Tetrad2 => const { &[t(0.0), t(90.0), t(180.0), t(270.0)] },
            Harmony::Tetrad3 => const { &[t(0.0), t(30.0), t(180.0), t(210.0)] },
            Harmony::Compound => const { &[t(0.0), t(40.0), t(160.0), t(200.0)] },
            Harmony::Compound2 => const { &[t(0.0), t(-40.0), t(160.0), t(200.0)] },
            Harmony::HighContrast => const { &[t(0.0), hc(180.0)] },
            Harmony::HighContrast2 => const { &[t(0.0), hc(0.0), t(180.0), hc(180.0)] },
            Harmony::HighContrast3 => const { &[t(0.0), hc(120.0), hc(240.0)] },
            Harmony::Pentagram => const { &[t(0.0), t(72.0), t(144.0), t(216.0), t(288.0)] },
        }
    }

    /// A [`THEME_SIZE`]-colour theme from `base`: the rule's colours, each followed by lighter or
    /// darker variations that fill the remaining places (the rule's first colours get them
    /// first), base first.
    // inv-vectorcraft: crates/color/src/harmony.rs:194 `theme`.
    pub fn theme(self, base: Srgba) -> Vec<Srgba> {
        let colors = self.apply(base);
        let n = colors.len();
        let mut rows: Vec<Vec<Srgba>> = colors.iter().map(|c| vec![*c]).collect();
        let options = GuideOptions::default();
        for k in 0..THEME_SIZE.saturating_sub(n) {
            let (i, pass) = (k % n, k / n);
            // Alternate lighter and darker along the rule and from one pass to the next.
            let step = if (i + pass) % 2 == 0 { 3 } else { -3 };
            rows[i].push(variation(colors[i], &options, step));
        }
        rows.into_iter().flatten().take(THEME_SIZE).collect()
    }
}

/// The number of colours in a colour theme.
pub const THEME_SIZE: usize = 5;

/// Moves colour `i` of a harmony on the hue and saturation wheel to `hsb` (hue in degrees,
/// saturation and brightness 0–1). With `linked` the other colours keep their relation to it:
/// they turn by the same hue, scale their saturation alike and shift their brightness by as
/// much. An index past the end changes nothing.
// inv-vectorcraft: crates/color/src/harmony.rs:216 `move_on_wheel`.
pub fn move_on_wheel(colors: &mut [Srgba], i: usize, target: [f32; 3], linked: bool) {
    let Some(&old) = colors.get(i) else { return };
    let Hsb { h: h0, s: s0, b: v0 } = old.to_hsb();
    let (dh, ratio, dv) = (target[0] - h0, if s0 > 0.01 { target[1] / s0 } else { 1.0 }, target[2] - v0);
    for (j, c) in colors.iter_mut().enumerate() {
        let new = if j == i {
            target
        } else if linked {
            let Hsb { h, s, b } = c.to_hsb();
            [h + dh, (s * ratio).clamp(0.0, 1.0), (b + dv).clamp(0.0, 1.0)]
        } else {
            continue;
        };
        *c = hsb(new[0], new[1], new[2]).with_alpha(c.a);
    }
}

/// What the variation grid varies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Variation {
    /// Towards black (left) and white (right): Show Tints/Shades.
    #[default]
    TintsShades,
    /// Towards cool and warm: Show Warm/Cool.
    WarmCool,
    /// Towards grey and full saturation: Show Vivid/Muted.
    VividMuted,
}

impl Variation {
    /// The three variations in menu order.
    pub const ALL: [Variation; 3] = [Variation::TintsShades, Variation::WarmCool, Variation::VividMuted];

    /// The grid's ends, English (words keys): ("Shades", "Tints"), ("Cool", "Warm"),
    /// ("Muted", "Vivid").
    pub fn sides(self) -> (&'static str, &'static str) {
        match self {
            Variation::TintsShades => ("Shades", "Tints"),
            Variation::WarmCool => ("Cool", "Warm"),
            Variation::VividMuted => ("Muted", "Vivid"),
        }
    }
}

/// The variation grid's options: what it varies, steps on each side and how far they reach.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GuideOptions {
    /// Which axis the grid varies along.
    pub variation: Variation,
    /// Variations on each side of a harmony colour, 1 to [`GuideOptions::MAX_STEPS`].
    pub steps: u32,
    /// How far the outermost variations go, 0–100 (Less … More): 10 % to 90 % of the way.
    pub amount: f32,
}

impl Default for GuideOptions {
    fn default() -> Self {
        GuideOptions { variation: Variation::TintsShades, steps: 4, amount: 50.0 }
    }
}

impl GuideOptions {
    /// Largest accepted value of `steps`.
    pub const MAX_STEPS: u32 = 20;

    /// The options with `steps` and `amount` in their ranges.
    pub fn clamped(self) -> GuideOptions {
        GuideOptions { steps: self.steps.clamp(1, Self::MAX_STEPS), amount: self.amount.clamp(0.0, 100.0), ..self }
    }
}

/// The variation of `color` at `step` (−steps to +steps; negative towards shades, cool or muted,
/// positive towards tints, warm or vivid; 0 is `color`).
// inv-vectorcraft: crates/color/src/harmony.rs:299 `variation`, :292 `reach`.
pub fn variation(color: Srgba, options: &GuideOptions, step: i32) -> Srgba {
    if step == 0 {
        return color;
    }
    let reach = 0.1 + 0.8 * options.amount.clamp(0.0, 100.0) / 100.0;
    let f = (step.unsigned_abs() as f32 / options.steps.max(1) as f32).min(1.0) * reach;
    let lerp = |to: Srgba, t: f32| Srgba::rgb(color.r + (to.r - color.r) * t, color.g + (to.g - color.g) * t, color.b + (to.b - color.b) * t);
    let v = match options.variation {
        Variation::TintsShades => lerp(if step < 0 { Srgba::BLACK } else { Srgba::WHITE }, f),
        Variation::WarmCool => lerp(if step < 0 { Srgba::rgb(0.2, 0.45, 1.0) } else { Srgba::rgb(1.0, 0.55, 0.1) }, f * 0.6),
        Variation::VividMuted => {
            let Hsb { h, s, b } = color.to_hsb();
            let s2 = if step < 0 { s * (1.0 - f) } else { s + (1.0 - s) * f };
            hsb(h, s2.clamp(0.0, 1.0), b)
        }
    };
    v.with_alpha(color.a)
}

/// The Color Guide for a base colour: the harmony colours and, per colour, its row of variations
/// from −steps to +steps, the colour itself in the centre column.
#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    /// The harmony colours, base first.
    pub colors: Vec<Srgba>,
    /// One row per harmony colour, each with `2 * steps + 1` variations from shades to tints.
    pub grid: Vec<Vec<Srgba>>,
}

impl Guide {
    // inv-vectorcraft: crates/color/src/harmony.rs:328 `Guide::new`.
    /// Builds the guide for `base` under `rule`; `options` is clamped first.
    pub fn new(base: Srgba, rule: Harmony, options: &GuideOptions) -> Guide {
        let options = options.clamped();
        let n = options.steps as i32;
        let colors = rule.apply(base);
        let grid = colors.iter().map(|c| (-n..=n).map(|step| variation(*c, &options, step)).collect()).collect();
        Guide { colors, grid }
    }

    /// The index of the centre column (the harmony colours) in each row.
    pub fn centre(&self) -> usize {
        self.grid.first().map_or(0, |row| row.len() / 2)
    }

    /// The guide with every colour moved to the nearest of `palette` (Limit to Library; ΔE2000
    /// in Lab), the centre column too. An empty palette changes nothing.
    // inv-vectorcraft: crates/color/src/harmony.rs:342 `limited`.
    pub fn limited(mut self, palette: &[Srgba]) -> Guide {
        if palette.is_empty() {
            return self;
        }
        let labs: Vec<[f64; 3]> = palette.iter().map(|c| lab(*c)).collect();
        let nearest = |c: Srgba| {
            let l = lab(c);
            let best =
                (0..palette.len()).min_by(|&a, &b| craft_cms::math::delta_e2000(labs[a], l).total_cmp(&craft_cms::math::delta_e2000(labs[b], l)));
            best.map_or(c, |i| palette[i])
        };
        let centre = self.centre();
        for (row, c) in self.grid.iter_mut().zip(&mut self.colors) {
            for v in row.iter_mut() {
                *v = nearest(*v);
            }
            // The centre column is the harmony colour itself.
            *c = row[centre];
        }
        self
    }
}
