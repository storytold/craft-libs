//! The words the theme panels use for tokens, sources, sizes and counts.

use craft_theme::{ColorSource, ColorToken, ComponentField, ComponentTokens, Contrast, Mode, PinLayer, SizeLayer, SizeOrigin, SizeValue, StatusKind};

/// "Dark: 3 pairs below minimum" in Standard contrast, "Dark: 3 pairs fail" in High, which must meet every minimum.
pub fn summary(mode: Mode, contrast: Contrast, count: usize) -> String {
    let name = if mode == Mode::Light { "Light" } else { "Dark" };
    let pairs = if count == 1 { "1 pair".to_owned() } else { format!("{count} pairs") };
    match (count, contrast) {
        (0, _) => format!("{name}: every pair meets its minimum"),
        (1, Contrast::High) => format!("{name}: {pairs} fails"),
        (_, Contrast::High) => format!("{name}: {pairs} fail"),
        _ => format!("{name}: {pairs} below minimum"),
    }
}

/// The token's group as in `ColorTokens`: its first path segment.
pub(crate) fn group_of(token: ColorToken) -> &'static str {
    let path = token.path();
    path.split_once('.').map_or(path, |(group, _)| group)
}

/// The heading a size row is listed under.
pub(crate) fn size_group(origin: &SizeOrigin) -> &'static str {
    match origin.path.split_once('.').map_or(origin.path.as_str(), |(group, _)| group) {
        "size" => "sizes",
        "space" => "spacing",
        "radius" => "corner radii",
        _ => "type",
    }
}

/// The heading of the component tokens.
pub(crate) const COMPONENT_GROUP: &str = "component tokens";

/// A size value as shown in a list and the detail.
pub(crate) fn value_words(value: SizeValue) -> String {
    match value {
        SizeValue::Number(n) => n.to_string(),
    }
}

/// The layer that set a size, in words.
pub(crate) fn layer_words(layer: SizeLayer) -> &'static str {
    match layer {
        SizeLayer::SizeSet => "size set",
        SizeLayer::App => "app",
        _ => "other",
    }
}

/// The standard and small size of a component token, as `md / sm`.
pub(crate) fn lengths_words(tokens: &ComponentTokens, field: ComponentField) -> String {
    let by = match field {
        ComponentField::SliderThumbWidth => tokens.slider.thumb_width,
        ComponentField::SliderThumbHeight => tokens.slider.thumb_height,
    };
    format!("{} / {}", by.md, by.sm)
}

/// Where a pin came from.
pub(crate) fn pin_words(layer: PinLayer) -> &'static str {
    match layer {
        PinLayer::Theme => "theme pin",
        PinLayer::App => "app pin",
        PinLayer::ThemeHighContrast => "theme high-contrast pin",
        PinLayer::AppHighContrast => "app high-contrast pin",
        _ => "pin",
    }
}

fn status_name(kind: StatusKind) -> &'static str {
    match kind {
        StatusKind::Good => "good",
        StatusKind::Warn => "warn",
        StatusKind::Bad => "bad",
        StatusKind::Info => "info",
    }
}

/// The ramp a colour takes its step from.
pub(crate) fn ramp_name(source: ColorSource) -> String {
    match source {
        ColorSource::Neutral(_) | ColorSource::Anchored(_) | ColorSource::StateVariant { .. } | ColorSource::NeutralAlpha(..) => "neutral".to_owned(),
        ColorSource::Accent(_) | ColorSource::AccentMix { .. } => "accent".to_owned(),
        ColorSource::Status(kind, _) | ColorSource::StatusMix { kind, .. } => format!("status {}", status_name(kind)),
        _ => String::new(),
    }
}
