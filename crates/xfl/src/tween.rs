//! Borrowed views of native XFL frame-tween and custom-ease records.
//!
//! This parses the native attributes and point lists. Interpreting easing curves and applying
//! them to an application's timeline model remain consumer responsibilities.

use crate::shape::{XflElement, boolean, number};

/// Native motion/shape tween settings on a `DOMFrame`.
#[derive(Clone, Copy, Debug)]
pub struct FrameTween<'a, E: XflElement> {
    frame: &'a E,
}

impl<'a, E: XflElement> FrameTween<'a, E> {
    pub fn new(frame: &'a E) -> Self {
        Self { frame }
    }

    /// The classic slider value, when present and finite.
    pub fn acceleration(&self) -> Option<f32> {
        number(self.frame, "acceleration")
    }

    pub fn has_custom_ease(&self) -> Option<bool> {
        boolean(self.frame, "hasCustomEase")
    }

    /// Animate's default is a single curve unless this is explicitly false.
    pub fn use_single_ease_curve(&self) -> Option<bool> {
        boolean(self.frame, "useSingleEaseCurve")
    }

    pub fn orient_to_path(&self) -> Option<bool> {
        boolean(self.frame, "motionTweenOrientToPath")
    }

    /// Raw native rotation setting; interpretation stays in the consuming application.
    pub fn rotate(&self) -> Option<&'a str> {
        self.frame.attr("motionTweenRotate")
    }

    /// Custom ease curves in source order; unknown tween entries are skipped.
    pub fn custom_eases(&self) -> impl Iterator<Item = CustomEase<'a, E>> + 'a {
        self.frame.child("tweens").into_iter().flat_map(|wrapper| wrapper.elements_named("CustomEase")).map(|element| CustomEase { element })
    }
}

/// A native custom-ease curve.
#[derive(Clone, Copy, Debug)]
pub struct CustomEase<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> CustomEase<'a, E> {
    /// The property-group name, defaulting to `all` in XFL.
    pub fn target(&self) -> &'a str {
        self.element.attr("target").map(str::trim).unwrap_or("all")
    }

    pub fn points(&self) -> impl Iterator<Item = EasePoint<'a, E>> + 'a {
        self.element.elements_named("Point").map(|element| EasePoint { element })
    }
}

/// One native custom-ease point.
#[derive(Clone, Copy, Debug)]
pub struct EasePoint<'a, E: XflElement> {
    element: &'a E,
}

impl<E: XflElement> EasePoint<'_, E> {
    pub fn x(&self) -> Option<f32> {
        number(self.element, "x")
    }

    pub fn y(&self) -> Option<f32> {
        number(self.element, "y")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &[u8]) -> craft_xml::El {
        craft_xml::parse(source).expect("well-formed test XML")
    }

    #[test]
    fn reads_frame_tween_settings_and_custom_eases_in_order() {
        let frame = parse(
            br#"<x:DOMFrame xmlns:x="urn:xfl" acceleration="-45" hasCustomEase="true" useSingleEaseCurve="false" motionTweenOrientToPath="true" motionTweenRotate="clockwise"><x:tweens><x:Other/><x:CustomEase target="position"><x:Point x="0" y="0"/><x:Point x="0.5" y="0.25"/></x:CustomEase><x:CustomEase><x:Point x="1" y="1"/></x:CustomEase></x:tweens></x:DOMFrame>"#,
        );
        let tween = FrameTween::new(&frame);
        assert_eq!(tween.acceleration(), Some(-45.0));
        assert_eq!(tween.has_custom_ease(), Some(true));
        assert_eq!(tween.use_single_ease_curve(), Some(false));
        assert_eq!(tween.orient_to_path(), Some(true));
        assert_eq!(tween.rotate(), Some("clockwise"));
        let curves: Vec<_> = tween.custom_eases().collect();
        assert_eq!(curves.len(), 2);
        assert_eq!(curves[0].target(), "position");
        assert_eq!(curves[0].points().map(|point| (point.x(), point.y())).collect::<Vec<_>>(), [(Some(0.0), Some(0.0)), (Some(0.5), Some(0.25))]);
        assert_eq!(curves[1].target(), "all");
        assert_eq!(curves[1].points().next().map(|point| (point.x(), point.y())), Some((Some(1.0), Some(1.0))));
    }

    #[test]
    fn absent_and_malformed_values_remain_optional() {
        let frame = parse(br#"<DOMFrame acceleration="NaN" hasCustomEase="bad" useSingleEaseCurve="false"/>"#);
        let tween = FrameTween::new(&frame);
        assert_eq!(tween.acceleration(), None);
        assert_eq!(tween.has_custom_ease(), Some(false));
        assert_eq!(tween.use_single_ease_curve(), Some(false));
        assert!(tween.custom_eases().next().is_none());
    }
}
