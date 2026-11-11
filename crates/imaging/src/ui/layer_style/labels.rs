//! The words of the Layer Style dialog: pages, effects, fields, choices and the Blending Options page.

use crate::layer_style::EffectKind;
use craft_ui::runtime::env::Label;

craft_ui::words! {
    /// Every word of the Layer Style dialog, English by default: pages, effect names, field
    /// labels, choice labels and the Blending Options page. The built-in tables
    /// ([`EffectSpec::photoshop`](crate::layer_style::EffectSpec::photoshop)) are shown through
    /// it ([`field`](Self::field), [`choice`](Self::choice), [`effect`](Self::effect)); an
    /// app's own specs show their own labels.
    #[non_exhaustive]
    pub struct LayerStyleWords {
        /// The effect list's accessible name.
        effects: "layerStyle.effects" = "Effects";
        styles: "layerStyle.styles" = "Styles";
        blending_options: "layerStyle.blendingOptions" = "Blending Options";
        bevel_emboss: "layerStyle.effectBevelEmboss" = "Bevel & Emboss";
        stroke: "layerStyle.effectStroke" = "Stroke";
        inner_shadow: "layerStyle.effectInnerShadow" = "Inner Shadow";
        inner_glow: "layerStyle.effectInnerGlow" = "Inner Glow";
        satin: "layerStyle.effectSatin" = "Satin";
        color_overlay: "layerStyle.effectColorOverlay" = "Color Overlay";
        gradient_overlay: "layerStyle.effectGradientOverlay" = "Gradient Overlay";
        pattern_overlay: "layerStyle.effectPatternOverlay" = "Pattern Overlay";
        outer_glow: "layerStyle.effectOuterGlow" = "Outer Glow";
        drop_shadow: "layerStyle.effectDropShadow" = "Drop Shadow";
        /// The `+` button's accessible name; `{effect}` is the effect's name.
        add_instance: template "layerStyle.addInstance" = "Add another {effect}";
        /// The `−` button's accessible name.
        remove_instance: template "layerStyle.removeInstance" = "Remove this {effect}";
        /// The name of the instance `{number}` of an effect that has several: "Drop Shadow 2".
        instance_name: template "layerStyle.instanceName" = "{effect} {number}";
        preview: "layerStyle.preview" = "Preview";
        new_style: "layerStyle.newStyle" = "New Style…";
        make_default: "layerStyle.makeDefault" = "Make Default";
        reset_to_default: "layerStyle.resetToDefault" = "Reset to Default";
        /// The colour picker's title for an effect colour.
        color_picker_title: "layerStyle.colorPickerTitle" = "Color Picker (Layer Style Color)";
        /// One line over a page whose host values have the wrong type; `{field}` names the first.
        invalid_value: template "layerStyle.invalidValue" = "{field} has a value this page can't show. It is kept as it is.";
        blend_mode: "layerStyle.fieldBlend" = "Blend Mode";
        color: "layerStyle.fieldColor" = "Color";
        opacity: "layerStyle.fieldOpacity" = "Opacity";
        fill_opacity: "layerStyle.fieldFillOpacity" = "Fill Opacity";
        angle: "layerStyle.fieldAngle" = "Angle";
        use_global_light: "layerStyle.fieldUseGlobalLight" = "Use Global Light";
        distance: "layerStyle.fieldDistance" = "Distance";
        spread: "layerStyle.fieldSpread" = "Spread";
        choke: "layerStyle.fieldChoke" = "Choke";
        size: "layerStyle.fieldSize" = "Size";
        contour: "layerStyle.fieldContour" = "Contour";
        noise: "layerStyle.fieldNoise" = "Noise";
        knocks_out: "layerStyle.fieldKnocksOut" = "Layer Knocks Out Drop Shadow";
        technique: "layerStyle.fieldTechnique" = "Technique";
        source: "layerStyle.fieldSource" = "Source";
        range: "layerStyle.fieldRange" = "Range";
        position: "layerStyle.fieldPosition" = "Position";
        overprint: "layerStyle.fieldOverprint" = "Overprint";
        from: "layerStyle.fieldFrom" = "From";
        to: "layerStyle.fieldTo" = "To";
        reverse: "layerStyle.fieldReverse" = "Reverse";
        style: "layerStyle.fieldStyle" = "Style";
        scale: "layerStyle.fieldScale" = "Scale";
        method: "layerStyle.fieldMethod" = "Method";
        pattern: "layerStyle.fieldPattern" = "Pattern";
        link: "layerStyle.fieldLink" = "Link with Layer";
        depth: "layerStyle.fieldDepth" = "Depth";
        direction: "layerStyle.fieldDirection" = "Direction";
        soften: "layerStyle.fieldSoften" = "Soften";
        altitude: "layerStyle.fieldAltitude" = "Altitude";
        gloss_contour: "layerStyle.fieldGlossContour" = "Gloss Contour";
        highlight_mode: "layerStyle.fieldHighlightBlend" = "Highlight Mode";
        highlight_color: "layerStyle.fieldHighlightColor" = "Highlight Color";
        highlight_opacity: "layerStyle.fieldHighlightOpacity" = "Highlight Opacity";
        shadow_mode: "layerStyle.fieldShadowBlend" = "Shadow Mode";
        shadow_color: "layerStyle.fieldShadowColor" = "Shadow Color";
        shadow_opacity: "layerStyle.fieldShadowOpacity" = "Shadow Opacity";
        invert: "layerStyle.fieldInvert" = "Invert";
        softer: "layerStyle.choiceSofter" = "Softer";
        precise: "layerStyle.choicePrecise" = "Precise";
        edge: "layerStyle.choiceEdge" = "Edge";
        center: "layerStyle.choiceCenter" = "Center";
        outside: "layerStyle.choiceOutside" = "Outside";
        inside: "layerStyle.choiceInside" = "Inside";
        linear: "layerStyle.choiceLinear" = "Linear";
        radial: "layerStyle.choiceRadial" = "Radial";
        angle_style: "layerStyle.choiceAngle" = "Angle";
        reflected: "layerStyle.choiceReflected" = "Reflected";
        diamond: "layerStyle.choiceDiamond" = "Diamond";
        inner_bevel: "layerStyle.choiceInner" = "Inner Bevel";
        outer_bevel: "layerStyle.choiceOuter" = "Outer Bevel";
        emboss: "layerStyle.choiceEmboss" = "Emboss";
        pillow_emboss: "layerStyle.choicePillow" = "Pillow Emboss";
        smooth: "layerStyle.choiceSmooth" = "Smooth";
        chisel_hard: "layerStyle.choiceChiselHard" = "Chisel Hard";
        chisel_soft: "layerStyle.choiceChiselSoft" = "Chisel Soft";
        up: "layerStyle.choiceUp" = "Up";
        down: "layerStyle.choiceDown" = "Down";
        perceptual: "layerStyle.choicePerceptual" = "Perceptual";
        classic: "layerStyle.choiceClassic" = "Classic";
        custom: "layerStyle.choiceCustom" = "Custom";
        general_blending: "layerStyle.generalBlending" = "General Blending";
        advanced_blending: "layerStyle.advancedBlending" = "Advanced Blending";
        channels: "layerStyle.channels" = "Channels:";
        knockout: "layerStyle.knockout" = "Knockout";
        knockout_none: "layerStyle.knockoutNone" = "None";
        knockout_shallow: "layerStyle.knockoutShallow" = "Shallow";
        knockout_deep: "layerStyle.knockoutDeep" = "Deep";
        blend_interior_effects: "layerStyle.blendInteriorEffectsAsGroup" = "Blend Interior Effects as Group";
        blend_clipped_layers: "layerStyle.blendClippedLayersAsGroup" = "Blend Clipped Layers as Group";
        transparency_shapes_layer: "layerStyle.transparencyShapesLayer" = "Transparency Shapes Layer";
        layer_mask_hides_effects: "layerStyle.layerMaskHidesEffects" = "Layer Mask Hides Effects";
        vector_mask_hides_effects: "layerStyle.vectorMaskHidesEffects" = "Vector Mask Hides Effects";
        blend_if: "layerStyle.blendIf" = "Blend If:";
        /// The Blend If entry of a colour document's composite.
        gray: "layerStyle.gray" = "Gray";
        this_layer: "layerStyle.thisLayer" = "This Layer";
        underlying_layer: "layerStyle.underlyingLayer" = "Underlying Layer";
    }
}

impl LayerStyleWords {
    /// The name of `kind` ("Drop Shadow").
    pub fn effect(&self, kind: EffectKind) -> &Label {
        match kind {
            EffectKind::BevelEmboss => &self.bevel_emboss,
            EffectKind::Stroke => &self.stroke,
            EffectKind::InnerShadow => &self.inner_shadow,
            EffectKind::InnerGlow => &self.inner_glow,
            EffectKind::Satin => &self.satin,
            EffectKind::ColorOverlay => &self.color_overlay,
            EffectKind::GradientOverlay => &self.gradient_overlay,
            EffectKind::PatternOverlay => &self.pattern_overlay,
            EffectKind::OuterGlow => &self.outer_glow,
            EffectKind::DropShadow => &self.drop_shadow,
        }
    }

    /// The label of a built-in field key (`"distance"` → "Distance"); `None` for a key the
    /// built-in tables don't have, so the page shows the spec's own label. Bevel & Emboss's
    /// `contour` check is "Contour", its `contourName` row has no label, `contourRange` is
    /// "Range".
    pub fn field(&self, key: &str) -> Option<&Label> {
        let label = match key {
            "blend" => &self.blend_mode,
            "color" => &self.color,
            "opacity" => &self.opacity,
            "fillOpacity" => &self.fill_opacity,
            "angle" => &self.angle,
            "useGlobalLight" => &self.use_global_light,
            "distance" => &self.distance,
            "spread" => &self.spread,
            "choke" => &self.choke,
            "size" => &self.size,
            "contour" => &self.contour,
            "noise" => &self.noise,
            "knocksOut" => &self.knocks_out,
            "technique" => &self.technique,
            "source" => &self.source,
            "range" | "contourRange" => &self.range,
            "position" => &self.position,
            "overprint" => &self.overprint,
            "from" => &self.from,
            "to" => &self.to,
            "reverse" => &self.reverse,
            "style" => &self.style,
            "scale" => &self.scale,
            "method" => &self.method,
            "pattern" => &self.pattern,
            "link" => &self.link,
            "depth" => &self.depth,
            "direction" => &self.direction,
            "soften" => &self.soften,
            "altitude" => &self.altitude,
            "glossContour" => &self.gloss_contour,
            "highlightBlend" => &self.highlight_mode,
            "highlightColor" => &self.highlight_color,
            "highlightOpacity" => &self.highlight_opacity,
            "shadowBlend" => &self.shadow_mode,
            "shadowColor" => &self.shadow_color,
            "shadowOpacity" => &self.shadow_opacity,
            "invert" => &self.invert,
            _ => return None,
        };
        Some(label)
    }

    /// The label of a built-in choice value (`"chiselHard"` → "Chisel Hard"; Gradient Overlay's
    /// `"angle"` style → "Angle"); `None` for another value.
    pub fn choice(&self, value: &str) -> Option<&Label> {
        let label = match value {
            "softer" => &self.softer,
            "precise" => &self.precise,
            "edge" => &self.edge,
            "center" => &self.center,
            "outside" => &self.outside,
            "inside" => &self.inside,
            "linear" => &self.linear,
            "radial" => &self.radial,
            "angle" => &self.angle_style,
            "reflected" => &self.reflected,
            "diamond" => &self.diamond,
            "inner" => &self.inner_bevel,
            "outer" => &self.outer_bevel,
            "emboss" => &self.emboss,
            "pillow" => &self.pillow_emboss,
            "smooth" => &self.smooth,
            "chiselHard" => &self.chisel_hard,
            "chiselSoft" => &self.chisel_soft,
            "up" => &self.up,
            "down" => &self.down,
            "perceptual" => &self.perceptual,
            "classic" => &self.classic,
            "custom" => &self.custom,
            _ => return None,
        };
        Some(label)
    }
}
