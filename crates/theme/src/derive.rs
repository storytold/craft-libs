use crate::{CoreRoles, Rgba, Variant, contrast};

/// Interaction and secondary colours computed from the core roles. Apps may overwrite any field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Derived {
    /// A control under the pointer.
    pub hover: Rgba,
    /// A control held down.
    pub pressed: Rgba,
    pub selected_hover: Rgba,
    pub accent_hover: Rgba,
    pub accent_pressed: Rgba,
    /// Links and accent-coloured labels: readable as text.
    pub accent_text: Rgba,
    /// A tinted fill such as a highlighted card or badge background.
    pub accent_soft: Rgba,
    /// Tertiary labels, still at text contrast.
    pub text_faint: Rgba,
    /// Edges of fields and buttons: visible at 3:1.
    pub control_edge: Rgba,
}

impl Derived {
    /// The field names, as [`contrast::Paint::Derived`] uses them.
    pub const KEYS: [&'static str; 9] =
        ["hover", "pressed", "selected_hover", "accent_hover", "accent_pressed", "accent_text", "accent_soft", "text_faint", "control_edge"];

    pub fn get(&self, name: &str) -> Option<Rgba> {
        Some(match name {
            "hover" => self.hover,
            "pressed" => self.pressed,
            "selected_hover" => self.selected_hover,
            "accent_hover" => self.accent_hover,
            "accent_pressed" => self.accent_pressed,
            "accent_text" => self.accent_text,
            "accent_soft" => self.accent_soft,
            "text_faint" => self.text_faint,
            "control_edge" => self.control_edge,
            _ => return None,
        })
    }
}

const STEP: f32 = 0.02;

/// Derived states for one variant, mixed in encoded sRGB (as PdfCraft's pressed states are).
///
/// Each state keeps every painted pair it takes part in (`contrast::painted_pairs`) at its
/// requirement where the core roles allow it: a state that would lose contrast by moving toward
/// `text` moves away from it instead, and searches stop before a failing step.
pub fn derive(roles: &CoreRoles, v: Variant) -> Derived {
    let (hover, pressed) = match v {
        Variant::Light => (0.06, 0.12),
        Variant::Dark => (0.08, 0.14),
    };
    Derived {
        hover: guarded_mix("hover", roles.surface, roles.text, hover, roles),
        pressed: guarded_mix("pressed", roles.surface, roles.text, pressed, roles),
        selected_hover: guarded_mix("selected_hover", roles.selection, roles.text, 0.08, roles),
        accent_hover: accent_state(roles, 5),
        accent_pressed: accent_state(roles, 11),
        accent_text: first_passing("accent_text", roles.accent, roles.text, roles),
        accent_soft: roles.surface.lerp_gamma(roles.accent, 0.14),
        text_faint: furthest_passing("text_faint", roles.text_muted, roles.surface, roles),
        control_edge: first_passing("control_edge", roles.line, roles.text, roles),
    }
}

/// `base` moved `t` toward `text`, or the same amount away when that keeps the painted pairs
/// and toward doesn't. With neither passing, the one with more contrast.
fn guarded_mix(name: &str, base: Rgba, text: Rgba, t: f32, roles: &CoreRoles) -> Rgba {
    let toward = base.lerp_gamma(text, t);
    let toward_margin = contrast::margin(name, toward, roles);
    if toward_margin >= 1.0 {
        return toward;
    }
    let away = base.step_away(text, t);
    if contrast::margin(name, away, roles) > toward_margin { away } else { toward }
}

/// The accent moved `steps` × 2% away from `on_accent`'s luminance, stopping before a step that
/// would take `on_accent` below 4.5:1 (or lower its contrast, if the accent already misses that).
fn accent_state(roles: &CoreRoles, steps: u8) -> Rgba {
    let target = if roles.on_accent.luminance() > roles.accent.luminance() { Rgba::BLACK } else { Rgba::WHITE };
    let mut current = roles.accent;
    let mut current_ratio = contrast::ratio(roles.on_accent, current);
    for k in 1..=steps {
        let next = roles.accent.lerp_gamma(target, STEP * f32::from(k));
        let r = contrast::ratio(roles.on_accent, next);
        if r < contrast::Need::Text.min_ratio() && r < current_ratio {
            break;
        }
        (current, current_ratio) = (next, r);
    }
    current
}

/// The first 2% step from `from` toward `to` whose painted pairs all pass; with none passing,
/// the step with the most contrast.
fn first_passing(name: &str, from: Rgba, to: Rgba, roles: &CoreRoles) -> Rgba {
    let mut best = (from, f32::NEG_INFINITY);
    for k in 0..=50u8 {
        let c = from.lerp_gamma(to, STEP * f32::from(k));
        let m = contrast::margin(name, c, roles);
        if m >= 1.0 {
            return c;
        }
        if m > best.1 {
            best = (c, m);
        }
    }
    best.0
}

/// `from` moved toward `to` in 2% steps while its painted pairs all still pass.
fn furthest_passing(name: &str, from: Rgba, to: Rgba, roles: &CoreRoles) -> Rgba {
    let mut current = from;
    for k in 1..=50u8 {
        let c = from.lerp_gamma(to, STEP * f32::from(k));
        if contrast::margin(name, c, roles) < 1.0 {
            break;
        }
        current = c;
    }
    current
}
