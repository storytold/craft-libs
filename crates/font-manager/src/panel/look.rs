//! The colours and type the panel draws with, so it reads as part of the app that shows it.

use egui::{Color32, Context, CornerRadius, FontFamily, Stroke, Visuals};

/// Colours for one appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Page and card surface.
    pub bg: Color32,
    /// Fields, wells, hovered rows.
    pub sunken: Color32,
    /// Body text.
    pub ink: Color32,
    /// Headings and family names.
    pub ink_strong: Color32,
    /// Secondary text.
    pub muted: Color32,
    /// Labels and tertiary text.
    pub faint: Color32,
    /// Hairlines.
    pub line: Color32,
    /// Outlined buttons and stronger rules.
    pub line_strong: Color32,
    /// Selection and progress.
    pub accent: Color32,
    /// The accent for text and focus rings.
    pub accent_ink: Color32,
    /// Primary buttons and the chosen option.
    pub invert_bg: Color32,
    pub invert_fg: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
}

impl Palette {
    /// A palette from egui's own visuals, for a host that sets none.
    pub fn from_visuals(v: &Visuals) -> Palette {
        let ink = v.text_color();
        Palette {
            bg: v.panel_fill,
            sunken: v.extreme_bg_color,
            ink,
            ink_strong: v.strong_text_color(),
            muted: ink.gamma_multiply(0.75),
            faint: ink.gamma_multiply(0.5),
            line: v.widgets.noninteractive.bg_stroke.color,
            line_strong: v.widgets.inactive.fg_stroke.color.gamma_multiply(0.6),
            accent: v.selection.bg_fill,
            accent_ink: v.hyperlink_color,
            invert_bg: v.strong_text_color(),
            invert_fg: v.panel_fill,
            good: if v.dark_mode { Color32::from_rgb(0x3f, 0xd6, 0xb8) } else { Color32::from_rgb(0x0a, 0x75, 0x63) },
            warn: v.warn_fg_color,
            bad: v.error_fg_color,
        }
    }

    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.line)
    }
}

/// How the panel looks: the host's colours for both appearances and the font families it
/// registered. [`Look::default`] follows egui's visuals and default families.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// `None`: from the context's visuals.
    pub dark: Option<Palette>,
    pub light: Option<Palette>,
    /// Page titles.
    pub display: FontFamily,
    /// Family names on cards.
    pub strong: FontFamily,
    /// Labels, counts and badges.
    pub label: FontFamily,
    /// Button text.
    pub button: FontFamily,
    /// Labels and buttons set upper case and spaced out.
    pub caps: bool,
    pub corner: CornerRadius,
    /// Buttons, fields and option cells.
    pub control_height: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            dark: None,
            light: None,
            display: FontFamily::Proportional,
            strong: FontFamily::Proportional,
            label: FontFamily::Proportional,
            button: FontFamily::Proportional,
            caps: false,
            corner: CornerRadius::same(3),
            control_height: 30.0,
        }
    }
}

impl Look {
    /// The palette for the context's current appearance.
    pub fn palette(&self, ctx: &Context) -> Palette {
        let dark = ctx.theme() == egui::Theme::Dark;
        match if dark { self.dark } else { self.light } {
            Some(p) => p,
            None => Palette::from_visuals(&ctx.global_style().visuals),
        }
    }
}
