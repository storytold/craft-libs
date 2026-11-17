//! Borrowed views of native XFL filter records.
//!
//! This module parses the wire-level filter names, fields, and gradient-ramp entries. Consumers
//! remain responsible for mapping those values into their own rendering/filter models.

use crate::shape::{self, Color, XflElement, boolean, integer, number};

/// Native filter variants understood by XFL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Blur,
    DropShadow,
    Glow,
    AdjustColor,
    Bevel,
    GradientGlow,
    GradientBevel,
}

/// Bevel/glow placement recorded by the native `type` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterSide {
    Inner,
    Outer,
    Full,
}

/// The native `<filters>` list on an element, preserving source order and unknown records.
#[derive(Clone, Copy, Debug)]
pub struct FilterList<'a, E: XflElement> {
    wrapper: Option<&'a E>,
}

impl<'a, E: XflElement> FilterList<'a, E> {
    pub fn new(element: &'a E) -> Self {
        Self { wrapper: element.child("filters") }
    }

    pub fn iter(&self) -> impl Iterator<Item = Filter<'a, E>> + 'a {
        let wrapper = self.wrapper;
        wrapper.into_iter().flat_map(XflElement::elements).map(|element| Filter { element })
    }
}

/// One native filter record. Unknown records are retained so the consumer can report/skip them.
#[derive(Clone, Copy, Debug)]
pub struct Filter<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> Filter<'a, E> {
    pub fn kind(&self) -> Option<FilterKind> {
        match self.element.local_name() {
            "BlurFilter" => Some(FilterKind::Blur),
            "DropShadowFilter" => Some(FilterKind::DropShadow),
            "GlowFilter" => Some(FilterKind::Glow),
            "AdjustColorFilter" => Some(FilterKind::AdjustColor),
            "BevelFilter" => Some(FilterKind::Bevel),
            "GradientGlowFilter" => Some(FilterKind::GradientGlow),
            "GradientBevelFilter" => Some(FilterKind::GradientBevel),
            _ => None,
        }
    }

    /// XFL treats filters as enabled unless explicitly set to false.
    pub fn enabled(&self) -> bool {
        boolean(self.element, "enabled") != Some(false)
    }

    /// Quality is a native integer in the range 1 through 3, defaulting to 1.
    pub fn quality(&self) -> u8 {
        integer(self.element, "quality").unwrap_or(1).clamp(1, 3) as u8
    }

    pub fn blur_x(&self) -> Option<f32> {
        number(self.element, "blurX")
    }

    pub fn blur_y(&self) -> Option<f32> {
        number(self.element, "blurY")
    }

    pub fn strength(&self) -> Option<f32> {
        number(self.element, "strength")
    }

    pub fn angle(&self) -> Option<f32> {
        number(self.element, "angle")
    }

    pub fn distance(&self) -> Option<f32> {
        number(self.element, "distance")
    }

    pub fn brightness(&self) -> Option<f32> {
        number(self.element, "brightness")
    }

    pub fn contrast(&self) -> Option<f32> {
        number(self.element, "contrast")
    }

    pub fn saturation(&self) -> Option<f32> {
        number(self.element, "saturation")
    }

    pub fn hue(&self) -> Option<f32> {
        number(self.element, "hue")
    }

    pub fn inner(&self) -> Option<bool> {
        boolean(self.element, "inner")
    }

    pub fn knockout(&self) -> Option<bool> {
        boolean(self.element, "knockout")
    }

    pub fn hide_object(&self) -> Option<bool> {
        boolean(self.element, "hideObject")
    }

    pub fn side(&self) -> Option<FilterSide> {
        match self.element.attr("type").map(str::trim) {
            Some("inner") => Some(FilterSide::Inner),
            Some("outer") => Some(FilterSide::Outer),
            Some("full") => Some(FilterSide::Full),
            _ => None,
        }
    }

    pub fn color(&self) -> Color {
        shape::color(self.element, "color", "alpha")
    }

    pub fn shadow_color(&self) -> Color {
        shape::color(self.element, "shadowColor", "shadowAlpha")
    }

    /// Explicit highlight color, absent when the native attribute is missing so consumers can
    /// apply the format-specific white highlight default.
    pub fn highlight_color(&self) -> Option<Color> {
        self.element.attr("highlightColor").map(|_| shape::color(self.element, "highlightColor", "highlightAlpha"))
    }

    pub fn highlight_alpha(&self) -> Option<f32> {
        number(self.element, "highlightAlpha")
    }

    /// Native gradient entries, either direct children or one wrapper level below the filter.
    pub fn ramp_entries(&self) -> impl Iterator<Item = RampEntry<'a, E>> + 'a {
        self.element
            .elements()
            .flat_map(|child| std::iter::once(child).chain(child.elements()))
            .filter(|entry| entry.local_name() == "GradientEntry")
            .map(|element| RampEntry { element })
    }
}

/// One XFL gradient-ramp entry.
#[derive(Clone, Copy, Debug)]
pub struct RampEntry<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> RampEntry<'a, E> {
    pub fn ratio(&self) -> Option<f32> {
        number(self.element, "ratio")
    }

    pub fn color(&self) -> Color {
        shape::color(self.element, "color", "alpha")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &[u8]) -> craft_xml::El {
        craft_xml::parse(source).expect("well-formed test XML")
    }

    #[test]
    fn recognizes_filter_kinds_and_preserves_unknown_entries_and_order() {
        let root = parse(br##"<DOMSymbolInstance><filters><BlurFilter/><x:GlowFilter xmlns:x="urn:xfl" enabled="false"/><FutureFilter/><GradientBevelFilter/></filters></DOMSymbolInstance>"##);
        let filters: Vec<_> = FilterList::new(&root).iter().collect();
        assert_eq!(filters.len(), 4);
        assert_eq!(filters[0].kind(), Some(FilterKind::Blur));
        assert_eq!(filters[1].kind(), Some(FilterKind::Glow));
        assert!(!filters[1].enabled());
        assert_eq!(filters[2].kind(), None);
        assert_eq!(filters[3].kind(), Some(FilterKind::GradientBevel));
    }

    #[test]
    fn reads_native_filter_values_colors_and_gradient_entries_in_order() {
        let root = parse(
            br##"<DOMSymbolInstance><filters><DropShadowFilter blurX="7.5" blurY="8" strength="1.25" quality="9" angle="45" distance="4" color="#123456" alpha="0.5" inner="true" knockout="false" hideObject="true"/><BevelFilter type="full" shadowColor="#010203" shadowAlpha="0.25" highlightColor="#aabbcc" highlightAlpha="0.75"/><GradientGlowFilter><GradientEntry ratio="255" color="#ff0000"/><Ramp><GradientEntry ratio="64" color="#00ff00"/></Ramp></GradientGlowFilter></filters></DOMSymbolInstance>"##,
        );
        let filters: Vec<_> = FilterList::new(&root).iter().collect();
        let shadow = &filters[0];
        assert_eq!(shadow.quality(), 3);
        assert_eq!(
            (shadow.blur_x(), shadow.blur_y(), shadow.strength(), shadow.angle(), shadow.distance()),
            (Some(7.5), Some(8.0), Some(1.25), Some(45.0), Some(4.0))
        );
        assert_eq!(shadow.color(), Color { rgb: 0x123456, alpha: 0.5 });
        assert_eq!(shadow.inner(), Some(true));
        assert_eq!(shadow.knockout(), Some(false));
        assert_eq!(shadow.hide_object(), Some(true));

        let bevel = &filters[1];
        assert_eq!(bevel.side(), Some(FilterSide::Full));
        assert_eq!(bevel.shadow_color(), Color { rgb: 0x010203, alpha: 0.25 });
        assert_eq!(bevel.highlight_color(), Some(Color { rgb: 0xaabbcc, alpha: 0.75 }));

        let ramp: Vec<_> = filters[2].ramp_entries().collect();
        assert_eq!(ramp.iter().map(RampEntry::ratio).collect::<Vec<_>>(), [Some(255.0), Some(64.0)]);
        assert_eq!(ramp[0].color(), Color { rgb: 0xff0000, alpha: 1.0 });
    }

    #[test]
    fn absent_lists_are_empty_and_values_use_shared_wire_validation() {
        let root = parse(b"<DOMSymbolInstance/>");
        assert_eq!(FilterList::new(&root).iter().count(), 0);

        let root = parse(br##"<BlurFilter quality="bad" blurX="NaN" enabled="unexpected"/>"##);
        let filter = FilterList::new(&root).iter().next();
        assert_eq!(filter.map(|filter| filter.kind()), None);
        let parent =
            parse(br##"<DOMSymbolInstance><filters><BlurFilter quality="bad" blurX="NaN" enabled="unexpected"/></filters></DOMSymbolInstance>"##);
        let filter = FilterList::new(&parent).iter().next().expect("filter entry");
        assert_eq!(filter.kind(), Some(FilterKind::Blur));
        assert_eq!(filter.quality(), 1);
        assert_eq!(filter.blur_x(), None);
        assert!(!filter.enabled());
    }
}
