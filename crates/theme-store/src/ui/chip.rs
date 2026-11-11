//! `ThemeChip`: a miniature of a theme, a member of a row's radio group.

use super::page::AppearanceWords;
use craft_behavior::{Flow, IconId, Metrics, PartKey, Sense};
use craft_theme::{Appearance, Contrast, Mode, Size, Stateful, States, TextSize, ThemeDef, ThemeId};
use craft_ui::buttons::IconButton;
use craft_ui::choice::RadioKind;
use craft_ui::contract::{
    Access, Common, Component, Cx, GroupHandle, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Props as _, Remember as _, Role, Styled as _,
    WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, DataColor, Flags, FocusLook, StyleColor, StyleCx, SurfaceKind};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2};

/// The favourite star on the chip's corner.
const STAR: PartKey = PartKey::new("favorite");

/// Why the star has no keyboard path of its own.
const STAR_REASON: &str = "The chip's context menu has Add to Favorites";

/// What a chip reported.
#[derive(Debug)]
#[non_exhaustive]
pub struct ThemeChipOutput {
    /// The chip's response; a context menu hangs from it.
    pub response: Response,
    /// The star badge was clicked.
    pub favorite_clicked: bool,
}

impl Outcome for ThemeChipOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The look of a theme chip.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ThemeChipStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The chip's size.
    size: Vec2,
    /// The outline: the card border, the muted ink under the pointer, the accent when chosen.
    outline: Stateful<StyleColor>,
    /// The corner radius of the outline and of the miniature.
    radius: CornerRadius,
    /// The room between the outline and the miniature.
    inset: f32,
    /// The height of the chrome strip along the miniature's top.
    strip: f32,
    /// The diameter of the accent dot.
    dot: f32,
    /// The side of the favourite star's box.
    badge: f32,
}

/// The miniature's colours, kept while the theme and the mode stay the same.
#[derive(Clone, Default)]
struct Sampled {
    theme: Option<ThemeDef>,
    dark: bool,
    /// Panel, chrome and accent.
    colors: [Color32; 3],
}

impl Sampled {
    /// `theme` resolved in the mode.
    fn of(theme: &ThemeDef, dark: bool) -> Self {
        let appearance = Appearance {
            mode: if dark { Mode::Dark } else { Mode::Light },
            contrast: Contrast::Standard,
            size: Size::Compact,
            text_size: TextSize::Small,
            reduced_motion: false,
        };
        let color = craft_theme::resolve(theme, appearance, None).tokens.color;
        let egui = |rgba: craft_theme::Rgba| {
            let [r, g, b, a] = rgba.0;
            Color32::from_rgba_unmultiplied(r, g, b, a)
        };
        Sampled { theme: Some(theme.clone()), dark, colors: [egui(color.surface.panel), egui(color.surface.chrome), egui(color.accent.solid)] }
    }
}

/// A miniature of a theme, after PhotoCraft's theme swatch.
///
/// The outline holds a fill of the theme's panel colour with a strip of its chrome colour along the top and an accent
/// dot near the bottom end corner, all resolved for the row's mode. The outline is the accent when chosen, the muted ink
/// under the pointer, else the card border. A star badge on the top end corner shows filled for a favourite, and as an
/// outline while the pointer is over the chip; a click on the badge reports `favorite_clicked` and does not pick the
/// theme. The chip is a member of the [`RadioGroup`](craft_ui::choice::RadioGroup) it is shown in: the group picks it
/// on a click or Space, moves between chips with the arrow keys and takes Tab to the chosen one.
#[must_use = "show the chip with `show`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ThemeChip<'a> {
    #[common]
    common: Common,
    theme: &'a ThemeDef,
    dark: bool,
    /// Whether the chip shows the favourite star.
    #[prop]
    favorite: bool,
    checked: bool,
    sampled: [Color32; 3],
    star: Option<Rect>,
    toggled: bool,
}

impl<'a> ThemeChip<'a> {
    /// The chip of `theme` in its dark variant when `dark`, else its light one.
    pub fn new(theme: &'a ThemeDef, dark: bool) -> Self {
        ThemeChip {
            common: Common::new(),
            theme,
            dark,
            favorite: false,
            checked: false,
            sampled: [Color32::TRANSPARENT; 3],
            star: None,
            toggled: false,
        }
        .label(theme.name.clone())
    }
}

impl Component for ThemeChip<'_> {
    const ROLE: Role = Role::RADIO_BUTTON;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = ThemeChipStyle;
    type Output = ThemeChipOutput;

    fn style(cx: &StyleCx<'_>, _: ThemeChipStyleKey) -> ThemeChipStyle {
        let tokens = cx.tokens;
        let (colors, size, space) = (&tokens.color, &tokens.size, &tokens.space);
        let rest = cx.color(colors.line.border);
        let muted = cx.color(colors.text.muted);
        let accent = cx.color(colors.accent.solid);
        let disabled = cx.color(colors.text.disabled);
        ThemeChipStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            size: Vec2::new(size.thumb.md, size.thumb.sm),
            outline: Stateful::new(rest).off(States::new(rest, muted, muted, disabled)).on(States::new(accent, accent, accent, disabled)),
            radius: cx.surface(SurfaceKind::Raised).radius,
            inset: space.gap.tight,
            strip: space.gap.inline,
            dot: size.handle.sm,
            badge: size.handle.md,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.style.size
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.star = None;
        if !(self.favorite || cx.hovered()) {
            return;
        }
        let (side, inset) = (cx.style.badge, cx.style.inset);
        let (slot, _) = Flow::new(rect, cx.flow.is_rtl()).take_end(side);
        let badge = Rect::from_min_size(Pos2::new(slot.min.x, rect.top() + inset), Vec2::splat(side));
        let _ = cx.part(STAR, badge).sense(Sense::CLICK).pointer_only(STAR_REASON);
        self.star = Some(badge);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let group = cx.env.get::<GroupHandle<RadioKind<Option<ThemeId>>>>();
        let member = Some(self.theme.id.clone());
        group.register(member.clone(), cx.id, cx.resolved.enabled);
        cx.join_roving();
        self.checked = group.is(&member);
        self.toggled = cx.part_activated(STAR).is_some();
        let kept = cx.memory::<Sampled>();
        if kept.theme.as_ref() != Some(self.theme) || kept.dark != self.dark {
            *kept = Sampled::of(self.theme, self.dark);
        }
        self.sampled = kept.colors;
    }

    fn flags(&self) -> Flags {
        let mut flags = Flags::empty();
        flags.set(Flags::CHECKED, self.checked);
        flags
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<AppearanceWords>();
        if self.favorite {
            access.value(words.favorite.as_str());
        }
        if self.star.is_some() {
            let action = if self.favorite { &words.remove_favorite } else { &words.add_favorite };
            access.node(STAR, Role::BUTTON).name = Name::Text(format!("{}: {}", action.as_str(), cx.resolved.label));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (style, rect) = (cx.style, cx.rect);
        let [panel, chrome, accent] = self.sampled.map(DataColor::new);
        let inner = rect.shrink(style.inset);
        let draw = ui.draw();
        draw.data_fill(&PaintShape::Rect(inner, style.radius), panel);
        draw.data_fill(&PaintShape::Rect(Rect::from_min_size(inner.min, Vec2::new(inner.width(), style.strip)), CornerRadius::ZERO), chrome);
        let rtl = cx.flow.is_rtl();
        let toward_end = style.strip;
        let x = if rtl { inner.left() + toward_end } else { inner.right() - toward_end };
        draw.data_fill(&PaintShape::Circle(Pos2::new(x, inner.bottom() - toward_end), style.dot / 2.0), accent);
        let width = if self.checked { StrokeWidth::Medium } else { StrokeWidth::Thin };
        draw.stroke(&PaintShape::Rect(rect, style.radius), width, cx.resolve(&style.outline));
        if self.star.is_some() {
            let icon = IconId::new(if self.favorite { "star_filled" } else { "star" });
            ui.face(STAR, IconButton::new(icon).label(cx.resolved.label.to_owned()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ThemeChipOutput {
        ThemeChipOutput { response: cx.response.clone(), favorite_clicked: self.toggled }
    }
}
