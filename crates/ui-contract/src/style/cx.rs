//! `StyleCx`: what `style` and the app colour hooks get. Tokens are readable only through it.

use super::key::{EnvKey, Ink};
use super::look::{DataColor, FocusLook, StyleColor, SurfaceKind, SurfaceLook, TrackLook};
use super::paint::{HandleLook, ResetMarkLook, RuleKind, RuleLook, ScrollBarLook, handle_look, reset_mark_look, rule_look, scroll_bar_look};
use super::surface::{SurfaceStyle, surface_style};
use super::tokens::UiTokens;
use crate::fonts::stack_facts;
use crate::runtime::egui::memory::Service;
use crate::runtime::env::{AppValues, Scale, StyleValue};
use craft_behavior::ControlMetrics;
use craft_host::target::Os;
use craft_i18n::Direction;
use craft_theme::Stroke as StrokeStep;
use craft_theme::{BySize, Contrast, ControlSize, Flags, RadiusStep, Rgba, Size, Stateful, States, TokenGroup, Variant, VariantLook, variant::Tone};
use egui::{Color32, Context, CornerRadius};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// What `fn style` gets. Tokens are read here and nowhere else.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct StyleCx<'a> {
    /// The resolved tokens of the scope.
    pub tokens: &'a UiTokens,
    /// The control size.
    pub size: ControlSize,
    /// The variant, one the component accepts.
    pub variant: Variant,
    /// The layout direction.
    pub direction: Direction,
    /// The interface scale.
    pub scale: Scale,
    /// The operating system.
    pub os: Os,
    /// Whether the window is active, so selection and focus looks dim in an inactive window.
    pub active: bool,
    /// The key of everything above that the style cache depends on.
    pub key: EnvKey,
    /// The text colour of the part this component is shown as the face of.
    host: Option<&'a Stateful<StyleColor>>,
    /// The muted text colour of the host part, when the host has one.
    host_muted: Option<&'a Stateful<StyleColor>>,
    /// The line height per point of text of the interface face, as the scope's frame began.
    line_ratio: f32,
    ctx: &'a Context,
    values: &'a AppValues,
}

/// A variant's look as style parts.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantParts {
    /// The fill per state.
    pub fill: Stateful<StyleColor>,
    /// The text and icon colour per state.
    pub text: Stateful<StyleColor>,
    /// The edge per state.
    pub edge: Option<Stateful<StyleColor>>,
    /// The corner radius.
    pub radius: CornerRadius,
}

/// Token groups built for a token set, and the type behind each group path.
#[derive(Default)]
struct GroupCache {
    built: HashMap<(TypeId, u64), Arc<dyn Any + Send + Sync>>,
    paths: Vec<(&'static str, TypeId)>,
}

/// A hash of every colour and flag mask of `ink`.
fn ink_hash(ink: &Stateful<StyleColor>) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut put = |color: &StyleColor| color.egui().to_array().hash(&mut hasher);
    let mut states = |states: &States<StyleColor>| [states.rest, states.hover, states.pressed, states.disabled].iter().for_each(&mut put);
    states(&ink.off);
    ink.on.iter().for_each(&mut states);
    for (mask, color) in &ink.when {
        mask.0.hash(&mut hasher);
        color.egui().to_array().hash(&mut hasher);
    }
    hasher.finish()
}

fn egui_color(c: Rgba) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl<'a> StyleCx<'a> {
    /// The frame's context, for the gate on effects in a measuring run.
    #[doc(hidden)]
    pub fn frame_ctx(&self) -> &Context {
        self.ctx
    }

    /// The contrast setting of the scope, for a style that adds a cue at high contrast.
    pub fn contrast(&self) -> Contrast {
        self.tokens.contrast()
    }

    /// The density preset of the scope, for a style whose shape differs between presets.
    pub fn density(&self) -> Size {
        self.tokens.density()
    }

    /// A style context for the scope the pipeline or an app colour hook runs in.
    #[expect(clippy::too_many_arguments, reason = "one argument per input the style cache key covers")]
    #[doc(hidden)]
    pub fn new(
        ctx: &'a Context,
        tokens: &'a UiTokens,
        values: &'a AppValues,
        size: ControlSize,
        variant: Variant,
        direction: Direction,
        scale: Scale,
        os: Os,
        active: bool,
        key: EnvKey,
        line_ratio: f32,
    ) -> Self {
        StyleCx { tokens, size, variant, direction, scale, os, active, key, host: None, host_muted: None, line_ratio, ctx, values }
    }

    /// The same context with another variant.
    #[doc(hidden)]
    pub fn with_variant(&self, variant: Variant) -> StyleCx<'a> {
        StyleCx { variant, ..*self }
    }

    /// The same context for a component shown as the face of a part whose text colour is `ink` and whose muted text
    /// colour is `muted`. The key folds both in, so a face's cached styles are per host look.
    pub(crate) fn hosted(&self, ink: &'a Stateful<StyleColor>, muted: Option<&'a Stateful<StyleColor>>) -> StyleCx<'a> {
        let key = muted.map_or(self.key.folded(ink_hash(ink)), |muted| self.key.folded(ink_hash(ink)).folded(ink_hash(muted)));
        StyleCx { host: Some(ink), host_muted: muted, key, ..*self }
    }

    /// An app value set by a scope. Its hash is part of the style cache key.
    pub fn env<T: StyleValue>(&self) -> T {
        debug_assert!(
            T::AFFECTS_STYLE,
            "{} is read by style but does not set AFFECTS_STYLE, so cached styles would go stale",
            std::any::type_name::<T>()
        );
        self.values.get::<T>()
    }

    /// The one constructor of `StyleColor`: a semantic token read or a token-group field becomes a style colour here.
    pub fn color(&self, color: Rgba) -> StyleColor {
        StyleColor(egui_color(color))
    }

    /// The ink for text on a user colour: the higher-contrast of `text.normal` and `text.on_strong` on the fill,
    /// pushed to the contrast the text needs.
    pub fn ink_on(&self, fill: DataColor) -> StyleColor {
        let fill = Rgba::from(fill.0);
        let text = &self.tokens.color.text;
        let start = if craft_theme::contrast_ratio(text.on_strong, fill) > craft_theme::contrast_ratio(text.normal, fill) {
            text.on_strong
        } else {
            text.normal
        };
        self.color(craft_theme::ensure_contrast(start, fill, craft_theme::text_contrast_min(self.contrast())))
    }

    /// The one builder of the focus ring: the line's focus colour at the medium stroke, on the shape's edge.
    pub fn focus_look(&self) -> FocusLook {
        FocusLook::new(egui::Stroke::new(self.tokens.stroke.medium, self.color(self.tokens.color.line.focus).egui()), 0.0)
    }

    /// The colour text and icons of `tone` read in.
    pub fn tone(&self, tone: Tone) -> StyleColor {
        let c = &self.tokens.color;
        self.color(match tone {
            Tone::Accent => c.accent.text,
            Tone::Info => c.status.info.fg,
            Tone::Good => c.status.good.fg,
            Tone::Warn => c.status.warn.fg,
            Tone::Bad => c.status.bad.fg,
            _ => c.text.normal,
        })
    }

    /// The tinted background text and icons of `tone` sit on: the status or accent background, and the sunken surface
    /// for no status.
    pub fn tone_fill(&self, tone: Tone) -> StyleColor {
        let c = &self.tokens.color;
        self.color(match tone {
            Tone::Accent => c.accent.soft,
            Tone::Info => c.status.info.bg,
            Tone::Good => c.status.good.bg,
            Tone::Warn => c.status.warn.bg,
            Tone::Bad => c.status.bad.bg,
            _ => c.surface.sunken,
        })
    }

    /// A text or icon colour that is `base` when the component stands alone and the text colour of the host part
    /// when it is shown as that part's face, so a face reads on whatever fill the host draws under it.
    pub fn host_ink(&self, base: Stateful<StyleColor>) -> Stateful<StyleColor> {
        self.host.cloned().unwrap_or(base)
    }

    /// A muted text colour that is `base` when the component stands alone and the muted text colour of the host part
    /// when it is shown as that part's face and the host has one.
    pub fn host_muted_ink(&self, base: Stateful<StyleColor>) -> Stateful<StyleColor> {
        self.host_muted.cloned().unwrap_or(base)
    }

    /// The fill of an accent control that is on or filled: the strong accent at rest, with its hover and pressed
    /// shades, and the disabled fill when disabled.
    pub fn accent_fill(&self) -> Stateful<StyleColor> {
        let (accent, state) = (&self.tokens.color.accent, &self.tokens.color.state);
        let rest = self.color(accent.strong);
        Stateful::new(rest).off(States::new(
            rest,
            self.color(accent.strong_hover),
            self.color(accent.strong_pressed),
            self.color(state.disabled_fill),
        ))
    }

    /// The look of a value track: a groove of `size.groove` with the part a value fills over it, and its marks.
    pub fn track(&self) -> TrackLook {
        let colors = &self.tokens.color;
        let groove = self.color(colors.line.control_edge);
        let muted = self.color(colors.text.muted);
        let disabled = self.color(colors.state.disabled_fill);
        TrackLook {
            unfilled: Stateful::new(groove).off(States::new(groove, groove, groove, disabled)),
            filled: Stateful::new(muted).off(States::new(muted, muted, muted, self.color(colors.text.disabled))),
            edge: Stateful::new(self.color(colors.line.control_edge)),
            edge_width: self.tokens.stroke.thin,
            thickness: self.tokens.size.groove,
            ramp: self.tokens.size.ramp.groove,
            radius: UiTokens::corner(self.tokens.shape.radius.pill),
            mark_length: self.tokens.space.gap.tight,
            mark_width: self.tokens.stroke.thin,
            mark_color: Stateful::new(self.color(colors.line.mark)),
            snap: self.control().icon / 2.0,
        }
    }

    /// The colour text and icons on a surface read in.
    pub fn ink(&self, ink: Ink) -> StyleColor {
        let c = &self.tokens.color.text;
        self.color(match ink {
            Ink::OnPanel => c.normal,
            Ink::OnDark => c.on_strong,
            Ink::Accent => self.tokens.color.accent.text,
        })
    }

    /// The height, icon side, padding and gap of a control at the ambient size.
    pub fn control(&self) -> ControlMetrics {
        self.control_of(self.size)
    }

    /// `control` at `size`, whatever the ambient size: the part of a control that keeps its own size (a field's
    /// adornment button) reads its height here.
    pub fn control_of(&self, size: ControlSize) -> ControlMetrics {
        control_metrics(self.tokens, size, self.line_ratio)
    }

    /// `control`, with the side padding of a segment of a segmented control: `space.pad.control` at the small size
    /// and `space.pad.segment` at the medium one.
    pub fn segment_control(&self) -> ControlMetrics {
        let pads = &self.tokens.space.pad;
        let mut control = self.control();
        control.pad_x = BySize { sm: pads.control, md: pads.segment }.get(self.size);
        control
    }

    /// The width of `text` set in the face of `role`: the sizes that follow the text (a row header's digit) read it
    /// here instead of borrowing an icon or control size.
    pub fn text_width(&self, role: craft_theme::TypeRole, text: &str) -> f32 {
        super::text_measure::line_width(self.ctx, self.tokens, stack_facts(self.ctx), self.direction, role, text)
    }

    /// What a surface of `kind` draws.
    pub fn surface(&self, kind: SurfaceKind) -> SurfaceStyle {
        surface_style(self.tokens, kind)
    }

    /// The look of a text well: the well surface with an edge that takes the focus colour while the well holds
    /// focus, in any modality, and the quieter divider colour while it is disabled. The edge replaces the surface's
    /// flat stroke; a bevelled well keeps its bevel and draws an edge only while it holds focus. A component that carries no value, so
    /// is never invalid, uses it as it is.
    pub fn text_well(&self) -> SurfaceLook {
        let well = self.surface(SurfaceKind::Well);
        let colors = &self.tokens.color;
        let flat = matches!(well.edge, super::surface::SurfaceEdge::Border(_));
        let (rest, disabled) = if flat { (colors.line.control_edge, colors.line.divider) } else { (Rgba::TRANSPARENT, Rgba::TRANSPARENT) };
        let (rest, disabled) = (self.color(rest), self.color(disabled));
        let edge = Stateful::new(rest).off(States::new(rest, rest, rest, disabled)).when(Flags::FOCUSED, self.color(colors.line.focus));
        let (fill, off) = (StyleColor(well.fill), self.color(self.tokens.color.state.disabled_fill));
        SurfaceLook::new(Stateful::new(fill).off(States::new(fill, fill, fill, off)))
            .kind(SurfaceKind::Well)
            .radius(well.radius)
            .border(edge, StrokeStep::W1.width(1.0))
            .border_replaces_edge()
    }

    /// The look of a text or value field: the text well with the failure colour on its edge while its value is
    /// invalid, which wins over the focus colour.
    pub fn field(&self) -> SurfaceLook {
        let mut look = self.text_well();
        if let Some(edge) = &mut look.border {
            edge.when.insert(0, (Flags::INVALID.into(), self.tone(Tone::Bad)));
        }
        look
    }

    /// The line a rule of `kind` draws.
    pub fn rule(&self, kind: RuleKind) -> RuleLook {
        rule_look(self.tokens, kind)
    }

    /// The look of a canvas handle.
    pub fn handle(&self) -> HandleLook {
        handle_look(self.tokens)
    }

    /// The look of a scroll bar.
    pub fn scroll_bar(&self) -> ScrollBarLook {
        scroll_bar_look(self.tokens)
    }

    /// The look of the reset mark.
    pub fn reset_mark(&self) -> ResetMarkLook {
        reset_mark_look(self.tokens)
    }

    /// The look of the instance's variant as style parts.
    pub fn variant_look(&self) -> VariantParts {
        let look: VariantLook = self.variant.resolve(self.tokens);
        let states = |s: States<Rgba>| {
            let c = |c: Rgba| self.color(c);
            Stateful::new(c(s.rest)).off(States::new(c(s.rest), c(s.hover), c(s.pressed), c(s.disabled)))
        };
        let radius = &self.tokens.shape.radius;
        let px = match look.radius {
            RadiusStep::Button => radius.button,
            RadiusStep::Pill => radius.pill,
            _ => radius.control,
        };
        VariantParts { fill: states(look.fill), text: states(look.text), edge: look.edge.map(states), radius: UiTokens::corner(px) }
    }

    /// The token group `G` for this token set: derived from the tokens with the theme's values under `G::PATH` laid
    /// over it, built on the first read and cached by `(TypeId::of::<G>(), UiTokens::key)`.
    pub fn token_group<G: TokenGroup>(&self) -> G {
        let service = Service::<GroupCache>::of(self.ctx);
        let cache_key = (TypeId::of::<G>(), self.tokens.key());
        let hit = service.with(|cache| cache.built.get(&cache_key).cloned());
        if let Some(hit) = hit.and_then(|entry| entry.downcast::<G>().ok()) {
            return (*hit).clone();
        }
        let clash = service.with(|cache| {
            let clash = cache.paths.iter().any(|(path, ty)| *path == G::PATH && *ty != TypeId::of::<G>());
            if !clash && !cache.paths.iter().any(|(_, ty)| *ty == TypeId::of::<G>()) {
                cache.paths.push((G::PATH, TypeId::of::<G>()));
            }
            clash
        });
        debug_assert!(!clash, "two token group types share the path {}", G::PATH);
        let mut group = G::derive(self.tokens);
        group.overlay(self.tokens.groups());
        // The clone is app code, so it runs before the service lock is taken.
        let shared: Arc<dyn Any + Send + Sync> = Arc::new(group.clone());
        service.with(|cache| cache.built.insert(cache_key, shared));
        group
    }
}

/// The height, icon side, padding and gap of a control of `size`.
///
/// The height is the size token, or more when the face that leads the interface stack sets taller rows than the
/// token's type line: the control's vertical padding (the token height less the label role's line height, which
/// the token already holds) around the leading face's line height at the label size. `line_ratio` is that face's
/// line height per point of text (`0` with no stack installed: the token alone).
pub(crate) fn control_metrics(tokens: &UiTokens, size: ControlSize, line_ratio: f32) -> ControlMetrics {
    let sizes = &tokens.size;
    let label = tokens.typography.label;
    let grown = |token: f32| token + (line_ratio * label.size - label.line_height).max(0.0);
    let pad = tokens.space.pad.control;
    let gap = tokens.space.gap.inline;
    ControlMetrics::at(
        size,
        &BySize { sm: grown(sizes.control.sm), md: grown(sizes.control.md) },
        &BySize { sm: sizes.icon.sm, md: sizes.icon.md },
        &BySize { sm: pad, md: pad },
        &BySize { sm: gap, md: gap },
    )
}
