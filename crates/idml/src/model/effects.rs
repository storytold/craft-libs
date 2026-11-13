//! The IDML transparency/effect settings found on page items and object styles.

use craft_xml::{El, Local};

use super::Values;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DropShadowValues {
    pub mode: Option<String>,
    pub x_offset: Option<f64>,
    pub y_offset: Option<f64>,
    pub color: Option<String>,
    pub opacity: Option<f64>,
    pub size: Option<f64>,
    pub spread: Option<f64>,
    pub blend_mode: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatherValues {
    pub mode: Option<String>,
    pub width: Option<f64>,
    pub choke_amount: Option<f64>,
    pub corner_type: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpacityStopValues {
    pub location: Option<f64>,
    pub opacity: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GradientFeatherValues {
    pub applied: bool,
    pub radial: bool,
    pub angle: Option<f64>,
    pub length: Option<f64>,
    pub start: Option<[f64; 2]>,
    pub stops: Vec<OpacityStopValues>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectionalFeatherValues {
    pub applied: bool,
    pub widths: [Option<f64>; 4],
}

/// A typed view of an IDML `TransparencySetting` and its effect elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransparencySetting {
    pub blend_mode: Option<String>,
    pub opacity: Option<f64>,
    pub drop_shadow: Option<DropShadowValues>,
    pub feather: Option<FeatherValues>,
    pub gradient_feather: Option<GradientFeatherValues>,
    pub directional_feather: Option<DirectionalFeatherValues>,
}

impl TransparencySetting {
    pub fn from_el(el: &El) -> Self {
        let child = |name: &str| el.child(Local(name));
        let blending = child("BlendingSetting").map(Values::of);
        let drop_shadow = child("DropShadowSetting").map(|el| {
            let values = Values::of(el);
            DropShadowValues {
                mode: values.get("Mode").map(str::to_owned),
                x_offset: values.num("XOffset"),
                y_offset: values.num("YOffset"),
                color: values.get("EffectColor").map(str::to_owned),
                opacity: values.num("Opacity"),
                size: values.num("Size"),
                spread: values.num("Spread"),
                blend_mode: values.get("BlendMode").map(str::to_owned),
            }
        });
        let feather = child("FeatherSetting").map(|el| {
            let values = Values::of(el);
            FeatherValues {
                mode: values.get("Mode").map(str::to_owned),
                width: values.num("Width"),
                choke_amount: values.num("ChokeAmount"),
                corner_type: values.get("CornerType").map(str::to_owned),
            }
        });
        let gradient_feather = child("GradientFeatherSetting").map(|el| {
            let values = Values::of(el);
            let start = values
                .prop("GradientStart")
                .and_then(|value| value.split_whitespace().map(|part| part.parse::<f64>().ok().filter(|n| n.is_finite())).collect::<Option<Vec<_>>>())
                .and_then(|parts| <[f64; 2]>::try_from(parts).ok());
            GradientFeatherValues {
                applied: values.boolean("Applied") == Some(true),
                radial: values.get("Type") == Some("Radial"),
                angle: values.num("Angle"),
                length: values.num("Length"),
                start,
                stops: el
                    .elements()
                    .filter(|stop| stop.local() == "OpacityGradientStop")
                    .map(|stop| {
                        let values = Values::of(stop);
                        OpacityStopValues { location: values.num("Location"), opacity: values.num("Opacity") }
                    })
                    .collect(),
            }
        });
        let directional_feather = child("DirectionalFeatherSetting").map(|el| {
            let values = Values::of(el);
            DirectionalFeatherValues {
                applied: values.boolean("Applied") == Some(true),
                widths: [values.num("TopWidth"), values.num("LeftWidth"), values.num("BottomWidth"), values.num("RightWidth")],
            }
        });
        Self {
            blend_mode: blending.as_ref().and_then(|values| values.get("BlendMode").map(str::to_owned)),
            opacity: blending.as_ref().and_then(|values| values.num("Opacity")),
            drop_shadow,
            feather,
            gradient_feather,
            directional_feather,
        }
    }
}
