//! The IDML transparency/effect settings found on page items and object styles.

use craft_xml::{El, Local, Node};

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

impl DropShadowValues {
    pub fn to_el(&self) -> El {
        let mut element = El::new("DropShadowSetting");
        if let Some(value) = &self.mode {
            element.set_attr("Mode", value);
        }
        for (key, value) in
            [("XOffset", self.x_offset), ("YOffset", self.y_offset), ("Size", self.size), ("Spread", self.spread), ("Opacity", self.opacity)]
        {
            if let Some(value) = value {
                element.set_attr(key, crate::value::num(value));
            }
        }
        if let Some(value) = &self.blend_mode {
            element.set_attr("BlendMode", value);
        }
        if let Some(value) = &self.color {
            element.set_attr("EffectColor", value);
        }
        element
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatherValues {
    pub mode: Option<String>,
    pub width: Option<f64>,
    pub choke_amount: Option<f64>,
    pub corner_type: Option<String>,
}

impl FeatherValues {
    pub fn to_el(&self) -> El {
        let mut element = El::new("FeatherSetting");
        if let Some(value) = &self.mode {
            element.set_attr("Mode", value);
        }
        for (key, value) in [("Width", self.width), ("ChokeAmount", self.choke_amount)] {
            if let Some(value) = value {
                element.set_attr(key, crate::value::num(value));
            }
        }
        if let Some(value) = &self.corner_type {
            element.set_attr("CornerType", value);
        }
        element
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpacityStopValues {
    pub location: Option<f64>,
    pub opacity: Option<f64>,
}

impl OpacityStopValues {
    pub fn to_el(&self, self_id: &str, midpoint: bool) -> El {
        let mut element = El::new("OpacityGradientStop").with_attr("Self", self_id);
        if let Some(value) = self.opacity {
            element.set_attr("Opacity", crate::value::num(value));
        }
        if let Some(value) = self.location {
            element.set_attr("Location", crate::value::num(value));
        }
        if midpoint {
            element.set_attr("Midpoint", "50");
        }
        element
    }
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

impl GradientFeatherValues {
    pub fn to_el(&self, id_prefix: &str) -> El {
        let mut element =
            El::new("GradientFeatherSetting").with_attr("Applied", "true").with_attr("Type", if self.radial { "Radial" } else { "Linear" });
        if let Some(value) = self.angle {
            element.set_attr("Angle", crate::value::num(value));
        }
        if let Some(value) = self.length {
            element.set_attr("Length", crate::value::num(value));
        }
        if let Some([x, y]) = self.start {
            element.set_attr("GradientStart", format!("{} {}", crate::value::num(x), crate::value::num(y)));
        }
        for (index, stop) in self.stops.iter().enumerate() {
            let stop_id = format!("{id_prefix}TransparencySetting1GradientFeatherSetting1OpacityGradientStop{index}");
            element.push(stop.to_el(&stop_id, index > 0));
        }
        element
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectionalFeatherValues {
    pub applied: bool,
    pub widths: [Option<f64>; 4],
}

impl DirectionalFeatherValues {
    pub fn to_el(&self) -> El {
        let mut element = El::new("DirectionalFeatherSetting").with_attr("Applied", "true");
        for (key, value) in
            [("LeftWidth", self.widths[1]), ("RightWidth", self.widths[3]), ("TopWidth", self.widths[0]), ("BottomWidth", self.widths[2])]
        {
            if let Some(value) = value {
                element.set_attr(key, crate::value::num(value));
            }
        }
        element
    }
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
    pub fn to_el(&self, id_prefix: &str) -> El {
        let mut element = El::new("TransparencySetting");
        if self.blend_mode.is_some() || self.opacity.is_some() {
            let mut blending = El::new("BlendingSetting");
            if let Some(value) = &self.blend_mode {
                blending.set_attr("BlendMode", value);
            }
            if let Some(value) = self.opacity {
                blending.set_attr("Opacity", crate::value::num(value));
            }
            element.push(blending);
        }
        if let Some(effect) = &self.drop_shadow {
            element.push(effect.to_el());
        }
        if let Some(effect) = &self.feather {
            element.push(effect.to_el());
        }
        if let Some(effect) = &self.directional_feather {
            element.push(effect.to_el());
        }
        if let Some(effect) = &self.gradient_feather {
            element.push(effect.to_el(id_prefix));
        }
        element
    }

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

/// Resolved object-style chain used to inherit item-level IDML settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectStyleStack {
    chain: Vec<El>,
}

impl ObjectStyleStack {
    /// Resolves a style reference and its `BasedOn` chain (nearest first, cycle-safe, max 16).
    pub fn from_styles(styles: &[El], reference: Option<&str>, include_none: bool) -> Self {
        let find = |reference: &str| {
            styles.iter().find(|style| Values::of(style).self_id() == Some(reference)).or_else(|| {
                let normalized = if reference.starts_with("ObjectStyle/") {
                    reference.to_owned()
                } else {
                    format!("ObjectStyle/{}", crate::value::escape_id(reference.trim()))
                };
                styles.iter().find(|style| Values::of(style).self_id() == Some(normalized.as_str()))
            })
        };
        let mut chain = Vec::new();
        let mut next = reference.map(str::to_owned);
        while let Some(id) = next {
            let Some(style) = find(&id) else { break };
            let self_id = Values::of(style).self_id().map(str::to_owned);
            if chain.len() >= 16 || self_id.as_deref().is_some_and(|id| chain.iter().any(|entry: &El| Values::of(entry).self_id() == Some(id))) {
                break;
            }
            chain.push(style.clone());
            next = Values::of(style).prop("BasedOn").map(|base| {
                let base = base.trim();
                if base.starts_with("ObjectStyle/") { base.to_owned() } else { format!("ObjectStyle/{}", crate::value::escape_id(base)) }
            });
        }
        if include_none
            && !chain.iter().any(|style| Values::of(style).self_id() == Some("ObjectStyle/$ID/[None]"))
            && let Some(none) = find("ObjectStyle/$ID/[None]")
        {
            chain.push(none.clone());
        }
        Self { chain }
    }

    /// The resolved style elements, nearest first.
    pub fn styles(&self) -> &[El] {
        &self.chain
    }

    /// Merges an item's child over the corresponding elements in the resolved style chain.
    pub fn merged_child(&self, item: &El, name: &str) -> Option<El> {
        let category = match name {
            "TransparencySetting" => Some("ObjectStyleObjectEffectsCategorySettings"),
            "StrokeTransparencySetting" => Some("ObjectStyleStrokeEffectsCategorySettings"),
            "FillTransparencySetting" => Some("ObjectStyleFillEffectsCategorySettings"),
            "ContentTransparencySetting" => Some("ObjectStyleContentEffectsCategorySettings"),
            _ => None,
        };
        let enabled = |effect: &str| {
            let (Some(category), Some(flag)) = (category, effect_flag(effect)) else { return true };
            self.chain.iter().find_map(|style| style.child(Local(category)).and_then(|c| Values::of(c).get(flag).map(str::to_owned))).as_deref()
                != Some("false")
        };
        let mut result = None;
        for style in self.chain.iter().rev() {
            let Some(child) = style.child(Local(name)) else { continue };
            let mut child = child.clone();
            child.children.retain(|node| !matches!(node, Node::El(element) if !enabled(element.local())));
            match &mut result {
                Some(current) => overlay(current, &child, 0),
                None => result = Some(child),
            }
        }
        if let Some(child) = item.child(Local(name)) {
            match &mut result {
                Some(current) => overlay(current, child, 0),
                None => result = Some(child.clone()),
            }
        }
        result
    }

    /// Returns the drop-shadow and feather records enabled by the style's effect category.
    pub fn enabled_shadow_feather(&self) -> Option<El> {
        let category = self.chain.iter().find_map(|style| style.child(Local("ObjectStyleObjectEffectsCategorySettings")));
        let setting = |name: &str| self.chain.iter().find_map(|style| style.child(Local("TransparencySetting"))?.child(Local(name)).cloned());
        let mut result = El::new("TransparencySetting");
        let mut any = false;
        for (name, flag) in [("DropShadowSetting", "EnableDropShadow"), ("FeatherSetting", "EnableFeather")] {
            let element = setting(name);
            let on = match category {
                Some(category) => Values::of(category).get(flag) == Some("true"),
                None => element.as_ref().and_then(|element| Values::of(element).get("Mode").map(str::to_owned)).is_some_and(|mode| mode != "None"),
            };
            any |= on;
            if let (true, Some(element)) = (on, element) {
                result.push(element);
            }
        }
        any.then_some(result)
    }
}

fn effect_flag(effect: &str) -> Option<&'static str> {
    Some(match effect {
        "BlendingSetting" => "EnableTransparency",
        "DropShadowSetting" => "EnableDropShadow",
        "FeatherSetting" => "EnableFeather",
        "InnerShadowSetting" => "EnableInnerShadow",
        "OuterGlowSetting" => "EnableOuterGlow",
        "InnerGlowSetting" => "EnableInnerGlow",
        "BevelAndEmbossSetting" => "EnableBevelEmboss",
        "SatinSetting" => "EnableSatin",
        "DirectionalFeatherSetting" => "EnableDirectionalFeather",
        "GradientFeatherSetting" => "EnableGradientFeather",
        _ => return None,
    })
}

fn overlay(base: &mut El, top: &El, depth: usize) {
    if depth > 32 {
        *base = top.clone();
        return;
    }
    for (key, value) in &top.attrs {
        base.set_attr(key, value);
        if let Some(Node::El(properties)) = base.children.iter_mut().find(|node| matches!(node, Node::El(el) if el.local() == "Properties")) {
            properties.children.retain(|node| !matches!(node, Node::El(el) if el.local() == key));
        }
    }
    if let Some(properties) = top.child(Local("Properties")) {
        let names: Vec<&str> = properties.elements().map(El::local).collect();
        base.attrs.retain(|(key, _)| !names.contains(&key.as_str()));
    }
    let has_text = top.children.iter().any(|node| matches!(node, Node::Text(text) | Node::CData(text) if !text.trim().is_empty()));
    if has_text {
        base.children.retain(|node| !matches!(node, Node::Text(_) | Node::CData(_)));
    }
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for node in &top.children {
        match node {
            Node::El(child) => {
                let index = seen.entry(child.local()).or_insert(0);
                let slot = base
                    .children
                    .iter_mut()
                    .filter_map(|node| match node {
                        Node::El(el) if el.local() == child.local() => Some(el),
                        _ => None,
                    })
                    .nth(*index);
                *index += 1;
                match slot {
                    Some(slot) => overlay(slot, child, depth + 1),
                    None => base.children.push(node.clone()),
                }
            }
            Node::Text(_) | Node::CData(_) if has_text => base.children.push(node.clone()),
            Node::Text(_) | Node::CData(_) | Node::Comment(_) | Node::Pi { .. } => {}
        }
    }
}
