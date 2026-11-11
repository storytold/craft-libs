//! `Rating`: stars the user clicks, a row of `max` parts over a value from 0 to `max`. A click or Space on the current
//! star clears the rating. The stars are one Tab stop: arrows, Home and End move focus between them, and the digits set
//! the rating while the row has focus.

use super::words::ChoiceWords;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Numeric, PaintUi, PartsCx,
    Pointer as _, Role, SetPosition, Styled as _, WordSetCx as _,
};
use crate::primitives::IconSpec;
use crate::runtime::focus::{ActivateKeys, KeyUse};
use crate::style::{ComponentStyle, Flags, FocusLook, IconLook, Ink, StyleCx, SurfaceKind, SurfaceLook};
use craft_behavior::nav::{NavShape, StepResult, ValueAxes, ValueSteps, step_value};
use craft_behavior::{Flow, IconId, Metrics, PartKey, Roving, Sense};
use craft_keymap::{Chord, Key};
use craft_theme::{Rgba, Stateful, States, variant::Tone};
use egui::{Rect, Vec2};

const STAR: PartKey = PartKey::new("star");

/// The digit keys, 0 to 5; a rating with fewer stars takes the digits up to its own maximum.
const DIGITS: [Chord; 6] = [
    Chord::plain(Key::Num0),
    Chord::plain(Key::Num1),
    Chord::plain(Key::Num2),
    Chord::plain(Key::Num3),
    Chord::plain(Key::Num4),
    Chord::plain(Key::Num5),
];

/// The style of a rating.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RatingStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The failure tint behind the stars of an invalid rating: the status pair `bad.bg` and `bad.fg` read on each
    /// other where `bad.fg` alone need not read on the surface the rating sits on.
    #[style(surface)]
    surface: Option<SurfaceLook>,
    #[style(focus)]
    focus: FocusLook,
    /// A star the rating covers, in the host's ink when the rating is a face.
    #[style(icon)]
    lit: IconLook,
    /// A star past the rating.
    #[style(icon)]
    unlit: IconLook,
    /// The square one star takes.
    cell: f32,
}

/// A rating over a `u8` from 0 (unrated) to `max`. A `Mixed` binding shows no star lit.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Rating<'a> {
    #[common]
    common: Common,
    value: Bind<'a, u8>,
    max: u8,
    /// The compact form, for a rating shown over a picture (a tile's badge bar): only the lit stars, at the small
    /// glyph size, one star and the tight gap apart. The cells keep their places, so a click on cell n still means n.
    #[style_key]
    #[prop]
    compact: bool,
}

impl<'a> Rating<'a> {
    /// A rating of `value` with `max` stars.
    pub fn new(value: Bind<'a, u8>, max: u8) -> Self {
        Rating { common: Common::new(), value, max, compact: false }
    }

    fn current(&self, shown: Option<&u8>) -> u8 {
        shown.copied().unwrap_or(0).min(self.max)
    }

    fn star(at: u8) -> craft_behavior::PartRef {
        STAR.at(u64::from(at))
    }

    /// The rect of each star: a cell as wide as the star's square and as tall as `rect`. `parts` declares these
    /// and `paint` draws in them, so a click lands where the star is painted.
    fn cells(rect: Rect, cell: f32, rtl: bool, max: u8) -> impl Iterator<Item = Rect> {
        let mut flow = Flow::new(rect, rtl);
        (0..max).map(move |_| {
            let (taken, rest) = flow.take_start(cell);
            flow = rest;
            taken
        })
    }
}

impl Component for Rating<'_> {
    const ROLE: Role = Role::RADIO_GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Space).with_moves();
    type Style = RatingStyle;
    type Output = EditResponse<u8>;

    fn style(cx: &StyleCx<'_>, key: RatingStyleKey) -> RatingStyle {
        let control = cx.control();
        let (side, cell, metrics) = if key.compact {
            let side = cx.tokens.size.icon.glyph_sm;
            (side, side + cx.tokens.space.gap.tight, Metrics::new())
        } else {
            (control.icon, control.height, Metrics::new().min_size(Vec2::splat(control.height)))
        };
        let text = &cx.tokens.color.text;
        let bad = cx.tone(Tone::Bad);
        let ink = |rest| Stateful::new(rest).off(States::new(rest, rest, rest, cx.color(text.disabled))).when(Flags::INVALID, bad);
        let tint = Stateful::new(cx.color(Rgba::TRANSPARENT)).when(Flags::INVALID, cx.color(cx.tokens.color.status.bad.bg));
        RatingStyle {
            metrics,
            surface: Some(SurfaceLook::new(tint).radius(cx.surface(SurfaceKind::Well).radius)),
            focus: cx.focus_look(),
            lit: IconLook::new(side, cx.host_ink(ink(cx.ink(Ink::OnPanel)))),
            unlit: IconLook::new(side, ink(cx.color(text.muted))),
            cell,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let high = if self.compact { cx.style.lit.side } else { cx.style.cell };
        Vec2::new(cx.style.cell * f32::from(self.max), high)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let current = self.current(cx.value());
        for (at, cell) in (0..self.max).zip(Self::cells(rect, cx.style.cell, cx.flow.is_rtl(), self.max)) {
            let part = cx.part(Self::star(at), cell).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if at + 1 == current {
                part.flags(Flags::CHECKED).selected();
            }
        }
        cx.roving(Roving::new(NavShape::Flat));
    }

    fn bind(&mut self) -> Option<BindMut<'_, u8>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let current = self.current(cx.value());
        for at in 0..self.max {
            if cx.part_activated(Self::star(at)).is_some() {
                cx.set(if at + 1 == current { 0 } else { at + 1 });
            }
        }
        // Arrows move focus between the stars; AccessKit Increment and Decrement arrive as moves of the whole row.
        let steps = ValueSteps::new(f64::from(current), 0.0, f64::from(self.max), 1.0, 1.0);
        while let Some((mv, mods)) = cx.intents().take_move() {
            if let StepResult::Set(next) = step_value(&steps, ValueAxes::Horizontal, mv, mods) {
                let next = next as u8;
                if next != current {
                    cx.set(next);
                }
            }
        }
        for (digit, chord) in (0..=self.max).zip(DIGITS) {
            if cx.take_claimed(chord) && digit != current {
                cx.set(digit);
            }
        }
        for chord in DIGITS.into_iter().take(usize::from(self.max) + 1) {
            cx.claim(chord);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ChoiceWords>();
        let stars = |n: u8| words.stars.text(u64::from(n), &cx.format().integer(u64::from(n)));
        let current = self.current(cx.value());
        let text = if current == 0 { words.unrated.as_str().to_owned() } else { stars(current) };
        access.numeric(Numeric::new(f64::from(current), 0.0..=f64::from(self.max)).step(1.0).text(text));
        for at in 0..self.max {
            let node = access.node(Self::star(at), Role::RADIO_BUTTON);
            node.name = Name::Text(stars(at + 1));
            node.position = Some(SetPosition::new(usize::from(at)).of(usize::from(self.max)));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let current = self.current(cx.value());
        let hovered = (0..self.max).find(|at| cx.part(Self::star(*at)).hovered).map(|at| at + 1);
        let shown = hovered.unwrap_or(current);
        let draw = ui.draw();
        for (at, cell) in (0..self.max).zip(Self::cells(cx.rect, cx.style.cell, cx.flow.is_rtl(), self.max)) {
            if self.compact && at >= shown {
                break;
            }
            let (look, name) = if at < shown { (&cx.style.lit, "star_filled") } else { (&cx.style.unlit, "star") };
            let icon = Rect::from_center_size(cell.center(), Vec2::splat(look.side));
            draw.icon(icon, &IconSpec::svg(IconId::new(name)).color(cx.resolve(&look.color)));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<u8> {
        cx.edit_response()
    }
}

#[cfg(feature = "testing")]
mod stories;
