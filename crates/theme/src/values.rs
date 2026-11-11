//! Every tunable number of the semantic layer in one place: the ramp step each colour token
//! takes, fixed colours, contrast minimums, Size presets, type roles, text-size factors,
//! elevation and motion choices. The gallery tunes these; nothing else should hold such values.

use super::color::Rgba;
use super::ramp::Mode;
use super::scale::{Duration, Easing, Elevation};
use super::tokens::{ColorToken, StatusKind};

use Bg::{HaloOver, Token};
use ColorToken as T;

/// Where a colour token's default value comes from: a ramp step, a mix of two steps, a fixed
/// colour, or other tokens. [`explain`](crate::explain) reports it per token.
///
/// The background greys (surfaces, state fills, lines) are [`Anchored`](Self::Anchored) on
/// `surface.panel`, so they keep their curve distance from a panel the theme pins, in the
/// pinned panel's hue and chroma. A state variant keeps its distance from its own base instead
/// when the theme pins that base ([`StateVariant`](Self::StateVariant)). Bevel edges are the
/// panel lit and shaded ([`Lighting`](Self::Lighting)). Text greys stay on the curve and the
/// contrast pass keeps them readable, moving derived text first and pulling an anchored grey
/// back toward its anchor for the pairs text can't pass by moving.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColorSource {
    /// A step (1 to 12) of the neutral ramp.
    Neutral(usize),
    /// A neutral step (1 to 12) at its curve distance from `surface.panel`: the step's lightness
    /// moved by as much as the panel's value lies from the panel's own step, clamped to 0..=1.
    /// Hue and chroma are the step's when the panel is not pinned, and the pinned panel's when it
    /// is: the pin defines the grey family. Equal to the step when the panel is not pinned. The
    /// distance is a target: when a contrast pair on the token fails and its foreground can't
    /// move (pinned) or can't pass by moving, the contrast pass pulls the token's lightness back
    /// toward the panel's, no further than needed and never past it.
    Anchored(usize),
    /// A state variant of `base` (`state.pressed` of `state.hover`, `state.selected_hover` of
    /// `state.selected`): neutral step `step` at its curve distance from `base`'s step when the
    /// theme pins `base`, in the pinned base's hue and chroma, so a pinned base carries its
    /// variants; the contrast pass pulls it back toward the base. When `base` is not pinned, as
    /// [`Anchored`](Self::Anchored)`(step)`.
    StateVariant {
        /// The state the variant derives from.
        base: ColorToken,
        /// The variant's neutral step.
        step: usize,
    },
    /// `surface.panel` lit (a positive amount) or shaded (a negative one), as a bevel's edges:
    /// its OKLCH lightness moved by the amount, clamped to 0..=1, hue and chroma kept.
    Lighting(f64),
    /// A neutral step with an alpha (0..=1).
    NeutralAlpha(usize, f64),
    /// Another token's colour with its alpha replaced (0..=255).
    Faded(ColorToken, u8),
    /// A step of the accent ramp.
    Accent(usize),
    /// The accent step `from` moved `t` (0..=1) of the way toward step `to`, in OKLab.
    AccentMix {
        /// The step mixed from.
        from: usize,
        /// The step mixed toward.
        to: usize,
        /// How far toward `to`, 0..=1.
        t: f64,
    },
    /// A step of a status ramp.
    Status(StatusKind, usize),
    /// The status step `from` moved `t` (0..=1) of the way toward step `to`, in OKLab.
    StatusMix {
        /// The status whose ramp is mixed.
        kind: StatusKind,
        /// The step mixed from.
        from: usize,
        /// The step mixed toward.
        to: usize,
        /// How far toward `to`, 0..=1.
        t: f64,
    },
    /// A fixed colour.
    Fixed(Rgba),
    /// An OKLCH colour (lightness, chroma, hue), gamut-mapped.
    Oklch(f64, f64, f64),
    /// The final value of another token.
    Same(ColorToken),
    /// A solid fill taken from another token: that token's value when the contrast pass reaches
    /// this token's text rule, which then moves the fill's lightness away from its text as far
    /// as the text needs.
    SolidFrom(ColorToken),
    /// A solid fill taken from `base` with its OKLCH lightness moved away from `away_from` (darker
    /// under light text, lighter under dark text) by how far `to` lies from `from` in the
    /// unpinned ramp defaults: the pointer or pressed variant of a solid under text. It always
    /// differs from `base`, and the text rule moves it further only if `base` itself misses.
    ShiftedFrom {
        /// The solid the fill starts from.
        base: ColorToken,
        /// The token whose default the offset is measured from.
        from: ColorToken,
        /// The token whose default the offset is measured to.
        to: ColorToken,
        /// The text the fill moves away from.
        away_from: ColorToken,
    },
    /// White or black, whichever has the higher lowest contrast on all of these tokens.
    OnSolid(&'static [ColorToken]),
}

/// Converts an alpha in 0..=1 to a channel value.
pub(super) fn alpha(a: f64) -> u8 {
    (a.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// OKLCH hues of the categorical data series.
pub const DATA_HUES: [f64; 8] = [25.0, 70.0, 115.0, 160.0, 205.0, 250.0, 295.0, 340.0];
/// OKLCH lightness of the data series, dark and light mode.
pub const DATA_L: (f64, f64) = (0.72, 0.56);
/// OKLCH chroma of the data series.
pub const DATA_C: f64 = 0.15;

/// The status solids that `text.on_status` must read on.
const STATUS_SOLIDS: &[ColorToken] = &[
    T::StatusGoodSolid,
    T::StatusGoodSolidHover,
    T::StatusGoodSolidPressed,
    T::StatusWarnSolid,
    T::StatusWarnSolidHover,
    T::StatusWarnSolidPressed,
    T::StatusBadSolid,
    T::StatusBadSolidHover,
    T::StatusBadSolidPressed,
    T::StatusInfoSolid,
    T::StatusInfoSolidHover,
    T::StatusInfoSolidPressed,
];

/// The ramp step of a solid fill under the pointer (`accent.hover`, `status.<s>.solid_hover`).
const SOLID_HOVER: usize = 10;
/// A pressed solid fill: step `.0` moved `.2` of the way toward step `.1` (`accent.pressed`,
/// `status.<s>.solid_pressed`).
const SOLID_PRESSED: (usize, usize, f64) = (10, 11, 0.5);

/// How far a bevel's lit edge (`edge.light`) lies above `surface.panel` in OKLCH lightness, in
/// either mode. Windows 2000's #d4d0c8 face (L 0.86) has a white lit edge.
pub const BEVEL_LIT: f64 = 0.15;
/// How far a bevel's shaded edge (`edge.dark`) lies below `surface.panel` in OKLCH lightness, in
/// either mode: Windows 2000's #808080 shade of its #d4d0c8 face.
pub const BEVEL_SHADED: f64 = 0.26;

/// The alpha (0..=255) of the pressed close button against `caption.close_hover`: Windows 11 draws
/// the pressed close red at 90 % over the bar.
const CAPTION_CLOSE_PRESSED_ALPHA: u8 = 0xe6;
/// The alpha (0..=255) of the resting disc of a round caption button against `text.normal`
/// (Adwaita: 10 %).
const CAPTION_DISC_ALPHA: u8 = 26;

/// The neutral step of `surface.panel` in `mode`: the step every [`ColorSource::Anchored`] token
/// keeps its distance from.
pub(super) fn panel_step(mode: Mode) -> usize {
    match mode {
        Mode::Dark => 3,
        Mode::Light => 2,
    }
}

/// The default source of `token` in `mode`.
pub(super) fn source(token: ColorToken, mode: Mode) -> ColorSource {
    use ColorSource::{
        Accent, AccentMix, Anchored, Faded, Fixed, Lighting, Neutral, Oklch, OnSolid, Same, ShiftedFrom, SolidFrom, StateVariant, Status, StatusMix,
    };
    use StatusKind::{Bad, Good, Info, Warn};
    let dark = mode == Mode::Dark;
    let pick = |d: usize, l: usize| if dark { d } else { l };
    let pickf = |d: f64, l: f64| if dark { d } else { l };
    let series = |i: usize| Oklch(pickf(DATA_L.0, DATA_L.1), DATA_C, DATA_HUES[i]);
    let (pressed_from, pressed_to, pressed_t) = SOLID_PRESSED;
    let pressed = |kind| StatusMix { kind, from: pressed_from, to: pressed_to, t: pressed_t };
    match token {
        T::SurfaceBackground => Anchored(pick(2, 3)),
        T::SurfaceCanvas => Same(T::SurfaceBackground),
        T::SurfaceCanvasPattern => Same(T::SurfaceCanvas),
        T::SurfacePanel => Neutral(panel_step(mode)),
        T::SurfaceChrome => Same(T::SurfacePanel),
        T::SurfaceRaised => Anchored(pick(4, 1)),
        T::SurfaceSunken => Anchored(1),
        // Near white in both modes (dark: about 236/255), as macOS, GNOME and PhotoCraft knobs. A
        // knob face is not a background grey, so it stays on the curve.
        T::SurfaceKnob => Neutral(pick(12, 1)),
        T::SurfaceScrim => Fixed(Rgba::BLACK.with_alpha(alpha(pickf(0.5, 0.35)))),
        T::SurfaceShadow => Fixed(Rgba::BLACK.with_alpha(pick(SHADOW_ALPHA.1, SHADOW_ALPHA.0) as u8)),
        T::TextNormal => Neutral(12),
        T::TextMuted => Neutral(11),
        T::TextFaint => Neutral(10),
        T::TextDisabled => Neutral(9),
        T::TextOnAccent => OnSolid(&[T::AccentSolid]),
        T::TextOnStrong => Fixed(Rgba::WHITE),
        T::TextOnStatus => OnSolid(STATUS_SOLIDS),
        // `text.normal`'s step, so neutral selections look like the rest of the text.
        T::TextOnSelected => Neutral(12),
        T::LineDivider => Anchored(6),
        T::LineBorder => Anchored(7),
        T::LineControlEdge => Anchored(8),
        // A light knob needs an outline on a light panel (PhotoCraft's `text_dim`); on a dark one
        // its face is the edge. The Pro themes pin PhotoCraft's dark ring.
        T::LineKnobEdge => {
            if dark {
                Fixed(Rgba::TRANSPARENT)
            } else {
                Same(T::TextMuted)
            }
        }
        // The contrast pass moves it to the non-text minimum on the control surfaces.
        T::LineMark => Neutral(8),
        T::LineFocus => Accent(9),
        T::AccentSolid => Accent(9),
        T::AccentHover => Accent(SOLID_HOVER),
        T::AccentPressed => AccentMix { from: pressed_from, to: pressed_to, t: pressed_t },
        T::AccentStrong => SolidFrom(T::AccentSolid),
        T::AccentStrongHover => ShiftedFrom { base: T::AccentStrong, from: T::AccentSolid, to: T::AccentHover, away_from: T::TextOnStrong },
        T::AccentStrongPressed => ShiftedFrom { base: T::AccentStrong, from: T::AccentSolid, to: T::AccentPressed, away_from: T::TextOnStrong },
        T::AccentSoft => Accent(pick(4, 3)),
        T::AccentBorder => Same(T::AccentSolid),
        T::AccentText => Accent(11),
        T::AccentOnSoft => Same(T::AccentText),
        T::StatusGoodFg => Status(Good, 11),
        T::StatusGoodBg => Status(Good, 3),
        T::StatusGoodBorder => Status(Good, 7),
        T::StatusGoodSolid => Status(Good, 9),
        T::StatusGoodSolidHover => Status(Good, SOLID_HOVER),
        T::StatusGoodSolidPressed => pressed(Good),
        T::StatusWarnFg => Status(Warn, 11),
        T::StatusWarnBg => Status(Warn, 3),
        T::StatusWarnBorder => Status(Warn, 7),
        T::StatusWarnSolid => Status(Warn, 9),
        T::StatusWarnSolidHover => Status(Warn, SOLID_HOVER),
        T::StatusWarnSolidPressed => pressed(Warn),
        T::StatusBadFg => Status(Bad, 11),
        T::StatusBadBg => Status(Bad, 3),
        T::StatusBadBorder => Status(Bad, 7),
        T::StatusBadSolid => Status(Bad, 9),
        T::StatusBadSolidHover => Status(Bad, SOLID_HOVER),
        T::StatusBadSolidPressed => pressed(Bad),
        T::StatusInfoFg => Status(Info, 11),
        T::StatusInfoBg => Status(Info, 3),
        T::StatusInfoBorder => Status(Info, 7),
        T::StatusInfoSolid => Status(Info, 9),
        T::StatusInfoSolidHover => Status(Info, SOLID_HOVER),
        T::StatusInfoSolidPressed => pressed(Info),
        T::StateHover => Anchored(pick(4, 3)),
        T::StatePressed => StateVariant { base: T::StateHover, step: pick(5, 4) },
        // A selection is a state of its own, not a variant of hover.
        T::StateSelected => Anchored(pick(6, 5)),
        T::StateSelectedHover => StateVariant { base: T::StateSelected, step: pick(7, 6) },
        T::StateDisabledFill => Same(T::SurfacePanel),
        T::StateTextSelection => Faded(T::AccentSolid, 0x46),
        T::StateMatch => Faded(T::DataYellow, 0x5a),
        T::StateMatchCurrent => Faded(T::StatusWarnSolid, 0x6e),
        T::OverlayStroke => Accent(9),
        T::OverlayLine => Same(T::AccentSolid),
        T::OverlayHalo => Fixed(Rgba::BLACK.with_alpha(alpha(0.6))),
        T::OverlayHandleFill => Fixed(Rgba::WHITE),
        T::OverlayHandleStroke => Same(T::AccentSolid),
        T::OverlayMarquee => Fixed(Rgba::WHITE),
        // Guides sit on the document, not the panel, so they keep one colour in both modes.
        T::OverlayGuide => Fixed(Rgba::rgb(0x4a, 0xff, 0xff)),
        // Drawn over the document, not the panel, so one colour in both modes (PhotoCraft's default).
        T::OverlayGrid => Fixed(Rgba::rgb(0x8c, 0x8c, 0x8c)),
        T::OverlayCheckerLight => Fixed(Rgba::rgb(0xfa, 0xfa, 0xfa)),
        T::OverlayCheckerDark => Fixed(Rgba::rgb(0xd6, 0xd6, 0xd6)),
        T::OverlayTrail => Faded(T::AccentSolid, 0x6e),
        T::DataSeries0 => series(0),
        T::DataSeries1 => series(1),
        T::DataSeries2 => series(2),
        T::DataSeries3 => series(3),
        T::DataSeries4 => series(4),
        T::DataSeries5 => series(5),
        T::DataSeries6 => series(6),
        T::DataSeries7 => series(7),
        // PhotoCraft's histogram channel colours (`histogram_panel.rs:191`), the same in every theme and mode.
        T::DataRed => Fixed(Rgba::rgb(235, 30, 30)),
        T::DataGreen => Fixed(Rgba::rgb(30, 235, 30)),
        T::DataBlue => Fixed(Rgba::rgb(40, 40, 240)),
        T::DataLuma => Neutral(12),
        T::DataCyan => Fixed(Rgba::rgb(0, 200, 235)),
        T::DataMagenta => Fixed(Rgba::rgb(230, 0, 160)),
        T::DataYellow => Fixed(Rgba::rgb(240, 220, 0)),
        T::DataScope => Fixed(Rgba::rgb(0x28, 0x28, 0x28)),
        T::DataBackdrop => Faded(T::TextMuted, 0x80),
        T::DataSelection => Faded(T::AccentSolid, 0x59),
        T::CaptionCloseHover => Fixed(Rgba::rgb(0xc4, 0x2b, 0x1c)),
        T::CaptionClosePressed => Faded(T::CaptionCloseHover, CAPTION_CLOSE_PRESSED_ALPHA),
        T::CaptionCloseText => Fixed(Rgba::WHITE),
        T::CaptionDisc => Faded(T::TextNormal, CAPTION_DISC_ALPHA),
        T::CaptionInactive => Same(T::TextFaint),
        T::CaptionPressed => Same(T::TextMuted),
        T::EdgeLight => Lighting(BEVEL_LIT),
        T::EdgeDark => Lighting(-BEVEL_SHADED),
    }
}

/// A background in a contrast pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Bg {
    /// A colour token.
    Token(ColorToken),
    /// `overlay.halo` composited over a fixed backdrop.
    HaloOver(Rgba),
}

/// A contrast requirement: each fg must reach the minimum on each bg.
#[derive(Clone, Copy, Debug)]
pub(super) struct PairRule {
    /// Foreground tokens.
    pub fgs: &'static [ColorToken],
    /// Backgrounds they appear on.
    pub bgs: &'static [Bg],
    /// Minimum ratio with standard contrast.
    pub standard: f64,
    /// Minimum ratio with high contrast.
    pub high: f64,
    /// The fg is white or black on solid fills; when neither reaches the minimum, the fills move.
    pub on_solid: bool,
}

const fn rule(fgs: &'static [ColorToken], bgs: &'static [Bg], standard: f64, high: f64) -> PairRule {
    PairRule { fgs, bgs, standard, high, on_solid: false }
}

/// WCAG AA text minimum. The large-text minimum (3:1) applies to 18.66 px semibold or 24 px
/// regular text; no type role is that large, so no rule uses it.
pub const TEXT_MIN: (f64, f64) = (4.5, 7.0);
/// WCAG non-text minimum (control boundaries, focus, indicators, overlays).
pub const NON_TEXT_MIN: (f64, f64) = (3.0, 4.5);
/// The cue of a selected or on state (a fill or an edge on what it sits on): required in high
/// contrast only. Standard contrast keeps the theme's own fills and has no minimum for it.
pub const STATE_CUE_MIN: (f64, f64) = (0.0, NON_TEXT_MIN.1);
/// The edge that identifies a control (a field's or button's border, a slider track, the active
/// row of a popup) against the surface it sits on: WCAG 1.4.11,
/// required in high contrast only. Standard contrast keeps the theme's own edges.
pub const CONTROL_EDGE_MIN: (f64, f64) = (0.0, 3.0);

const TEXT_BGS: &[Bg] = &[
    Token(T::SurfaceBackground),
    Token(T::SurfacePanel),
    Token(T::SurfaceChrome),
    Token(T::SurfaceRaised),
    Token(T::SurfaceSunken),
    Token(T::StateHover),
    Token(T::StatePressed),
];
const SELECTED_BGS: &[Bg] = &[Token(T::StateSelected), Token(T::StateSelectedHover)];
const CONTROL_BGS: &[Bg] = &[Token(T::SurfacePanel), Token(T::SurfaceRaised), Token(T::SurfaceSunken)];
/// The focus line also marks focus on the title bars and tab strips, which sit on the background.
const FOCUS_BGS: &[Bg] =
    &[Token(T::SurfaceBackground), Token(T::SurfacePanel), Token(T::SurfaceChrome), Token(T::SurfaceRaised), Token(T::SurfaceSunken)];
/// Where a mark or a badge ring is drawn: control surfaces and the hover and pressed fills. The
/// selected fills are left out: no single colour reaches the minimum on both a light panel and
/// Classic's dark selection fill.
const MARK_BGS: &[Bg] = &[Token(T::SurfacePanel), Token(T::SurfaceRaised), Token(T::SurfaceSunken), Token(T::StateHover), Token(T::StatePressed)];
const HALO_BGS: &[Bg] = &[HaloOver(Rgba::BLACK), HaloOver(Rgba::WHITE)];

/// Every contrast pair, in the order the contrast pass fixes them: fills that serve as
/// backgrounds later (state fills, `accent.solid`) come before the text drawn on them.
pub(super) const RULES: &[PairRule] = &[
    rule(&[T::LineFocus], FOCUS_BGS, NON_TEXT_MIN.0, NON_TEXT_MIN.1),
    rule(&[T::AccentSolid], CONTROL_BGS, NON_TEXT_MIN.0, NON_TEXT_MIN.1),
    // The outline of a selected or on control is a state cue: Standard keeps PhotoCraft's translucent
    // one. Unpinned, it is the final `accent.solid`.
    rule(&[T::AccentBorder], CONTROL_BGS, STATE_CUE_MIN.0, STATE_CUE_MIN.1),
    rule(&[T::LineMark], MARK_BGS, NON_TEXT_MIN.0, NON_TEXT_MIN.1),
    rule(&[T::TextNormal, T::TextMuted, T::TextFaint], TEXT_BGS, TEXT_MIN.0, TEXT_MIN.1),
    // A canvas scroll bar's thumb is a boundary on its track: Standard keeps the theme's own greys.
    rule(&[T::TextFaint, T::TextMuted], &[Token(T::SurfaceChrome)], CONTROL_EDGE_MIN.0, CONTROL_EDGE_MIN.1),
    rule(&[T::TextOnSelected], SELECTED_BGS, TEXT_MIN.0, TEXT_MIN.1),
    rule(&[T::AccentText], CONTROL_BGS, TEXT_MIN.0, TEXT_MIN.1),
    // After `accent.text`, which it starts from.
    rule(&[T::AccentOnSoft], &[Token(T::AccentSoft)], TEXT_MIN.0, TEXT_MIN.1),
    // High contrast only: where the text reaches no further than white, the soft fill moves.
    PairRule { on_solid: true, ..rule(&[T::AccentOnSoft], &[Token(T::AccentSoft)], 0.0, TEXT_MIN.1) },
    rule(&[T::StatusGoodFg], &[Token(T::SurfacePanel), Token(T::SurfaceRaised), Token(T::StatusGoodBg)], TEXT_MIN.0, TEXT_MIN.1),
    rule(&[T::StatusWarnFg], &[Token(T::SurfacePanel), Token(T::SurfaceRaised), Token(T::StatusWarnBg)], TEXT_MIN.0, TEXT_MIN.1),
    rule(&[T::StatusBadFg], &[Token(T::SurfacePanel), Token(T::SurfaceRaised), Token(T::StatusBadBg)], TEXT_MIN.0, TEXT_MIN.1),
    rule(&[T::StatusInfoFg], &[Token(T::SurfacePanel), Token(T::SurfaceRaised), Token(T::StatusInfoBg)], TEXT_MIN.0, TEXT_MIN.1),
    PairRule {
        on_solid: true,
        ..rule(&[T::TextOnAccent], &[Token(T::AccentSolid), Token(T::AccentHover), Token(T::AccentPressed)], TEXT_MIN.0, TEXT_MIN.1)
    },
    // After every rule that moves `accent.solid`, which `accent.strong` starts from.
    PairRule { on_solid: true, ..rule(&[T::TextOnStrong], &[Token(T::AccentStrong)], TEXT_MIN.0, TEXT_MIN.1) },
    // After the rule on `accent.strong`, which the variants start from.
    PairRule { on_solid: true, ..rule(&[T::TextOnStrong], &[Token(T::AccentStrongHover), Token(T::AccentStrongPressed)], TEXT_MIN.0, TEXT_MIN.1) },
    PairRule {
        on_solid: true,
        ..rule(
            &[T::TextOnStatus],
            &[
                Token(T::StatusGoodSolid),
                Token(T::StatusGoodSolidHover),
                Token(T::StatusGoodSolidPressed),
                Token(T::StatusWarnSolid),
                Token(T::StatusWarnSolidHover),
                Token(T::StatusWarnSolidPressed),
                Token(T::StatusBadSolid),
                Token(T::StatusBadSolidHover),
                Token(T::StatusBadSolidPressed),
                Token(T::StatusInfoSolid),
                Token(T::StatusInfoSolidHover),
                Token(T::StatusInfoSolidPressed),
            ],
            TEXT_MIN.0,
            TEXT_MIN.1,
        )
    },
    rule(&[T::OverlayStroke], HALO_BGS, NON_TEXT_MIN.0, NON_TEXT_MIN.1),
    // A selected anchor is a handle filled with its outline colour: the outline on the fill is the
    // only mark of the selection. Standard keeps the accent PhotoCraft outlines its handles in.
    rule(&[T::OverlayHandleStroke], &[Token(T::OverlayHandleFill)], STATE_CUE_MIN.0, STATE_CUE_MIN.1),
    // A curve editor's curve and points in a channel colour, on its plot (`field.fill`, `surface.sunken`).
    // PhotoCraft's channel colours are the same in every theme, so Standard keeps them as they are.
    rule(
        &[T::DataRed, T::DataGreen, T::DataBlue, T::DataCyan, T::DataMagenta, T::DataYellow],
        &[Token(T::SurfaceSunken)],
        STATE_CUE_MIN.0,
        STATE_CUE_MIN.1,
    ),
    // Guide and smart-guide colours have no pair: they are the user's choice, as in the apps'
    // preferences, and a bare line on an image can't promise a ratio. `overlay.line` neither, for
    // the same reason.
];

/// Scaled type sizes round to multiples of this, in points.
pub const TYPE_ROUNDING: f32 = 0.5;

/// The shadow of a floating window (`elevation.floating`): PhotoCraft's `window_shadow`
/// (`theme.rs:1036`), 10 points down with a 32 point blur, in `surface.shadow`'s alpha.
pub const WINDOW_SHADOW: Elevation = Elevation { offset: [0.0, 10.0], blur: 32.0, spread: 0.0, alpha: 0.0 };
/// The shadow of a popup, menu or dialog (`elevation.popup`): PhotoCraft's `popup_shadow` (`theme.rs:1037`), 6
/// points down with a 20 point blur. Its alpha is `surface.shadow`'s, like every cast shadow's.
pub const POPUP_SHADOW: Elevation = Elevation { offset: [0.0, 6.0], blur: 20.0, spread: 0.0, alpha: 0.0 };
/// The alpha of `surface.shadow` in light and dark mode (0..=255): PhotoCraft's Pro
/// `Tokens::shadow` (`theme.rs`), 50 in light and 150 in dark; themes pin their own.
pub const SHADOW_ALPHA: (usize, usize) = (50, 150);
/// The shadow of a knob on a track with a flat edge (`elevation.knob`): PhotoCraft's hard circle,
/// the knob's size grown by half a point, one point down, black at 90/255 in both modes
/// (`slider.rs:206-207`). egui rounds a shadow's spread to whole points.
pub const KNOB_SHADOW: Elevation = Elevation { offset: [0.0, 1.0], blur: 0.0, spread: 0.5, alpha: 90.0 / 255.0 };
/// Durations for hover feedback, opening and moving.
pub const MOTION_DURATIONS: (Duration, Duration, Duration) = (Duration::Fast, Duration::Normal, Duration::Normal);
/// Easing for every semantic motion.
pub const MOTION_EASING: Easing = Easing::Standard;

/// The alpha a menu row's label colour starts at before its text is derived against it
/// (PhotoCraft draws 0.3; 0.24 is the most that keeps `text.normal` at 4.5:1 on all its hues in
/// every built-in theme).
pub const MENU_TINT_ALPHA: f32 = 0.24;

/// Least contrast ratio (standard, high) between a list row's hover fill and the fills it sits
/// beside: the rest surface and the selected fill. The built-in hover steps are 1.1 to 1.4 away
/// from the panel; 1.12 is the least that still reads on a screen at normal brightness. A hover
/// step that is closer is moved away along OKLCH lightness.
pub const LIST_HOVER_MIN: (f64, f64) = (1.12, 1.12);

/// How far under the icon size a slider's thumb is, in points, at the small and standard sizes
/// (Compact 12, PhotoCraft's).
pub const SLIDER_THUMB_INSET: (f32, f32) = (2.0, 4.0);

// Step 5 controls (S5C-F8).

/// Duration of a text caret's glide, in milliseconds (WordCraft's `caret_glide.rs`, measured from Word).
pub const MOTION_CARET_MS: u32 = 100;

/// Easing of a text caret's glide: Word's `cubic-bezier(0, 0, 0, 1)` (WordCraft's `caret_glide.rs`).
pub const MOTION_CARET_EASING: Easing = Easing::Enter;

/// Duration of a selection outline's glide, in milliseconds (GridCraft's `selection_glide.rs`).
pub const MOTION_OUTLINE_MS: u32 = 120;

/// Easing of a selection outline's glide. GridCraft's `ease_out` is `1 - (1 - t)^3` (0.875 at the
/// half way); `Standard` reaches 0.878 there and is the closest of the three curves.
pub const MOTION_OUTLINE_EASING: Easing = Easing::Standard;
