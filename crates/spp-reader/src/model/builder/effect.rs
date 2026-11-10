//! `DataStackActions` → [`ProjectEffect`]s, one builder per effect class.

use super::{MAX_NESTING, ModelBuilder};
use crate::model::channel::ChannelMask;
use crate::model::effect::{ColorSelectionEffect, EffectGroup, EffectHeader, FilterEffect, Levels, LevelsEffect, PolygonFillEffect, ProjectEffect};
use crate::model::fill::{FillEffect, UvTransform};
use crate::model::paint::{PaintEffect, PaintTool};
use crate::model::raw::RawObject;

impl ModelBuilder {
    /// The items of a `DataStackActions`, bottom → top as stored.
    pub(super) fn effect_stack(&mut self, stack: &RawObject, depth: usize) -> Vec<ProjectEffect> {
        self.seen(stack);
        stack.objects("items").map(|item| self.effect(item, depth)).collect()
    }

    fn effect(&mut self, e: &RawObject, depth: usize) -> ProjectEffect {
        self.begin(e);
        let mut built = self.effect_fields(e, depth);
        let found = self.end(e);
        match built.header_mut() {
            Some(header) => header.unhandled = found,
            // an effect class the model doesn't know: what it holds goes to the layer
            None => {
                if let Some(outer) = self.settings.open.last_mut() {
                    outer.extend(found);
                }
            }
        }
        built
    }

    fn effect_fields(&mut self, e: &RawObject, depth: usize) -> ProjectEffect {
        if let Some(tool) = PaintTool::from_class(&e.class) {
            return ProjectEffect::Paint(self.paint(e, tool, depth));
        }
        match e.class.as_str() {
            "DataActionFill" => ProjectEffect::Fill(self.fill(e, depth)),
            "DataActionGroup" if depth < MAX_NESTING => {
                let header = self.header(e);
                let effects = match e.object("subStack") {
                    Some(sub) => self.effect_stack(sub, depth + 1),
                    None => Vec::new(),
                };
                ProjectEffect::Group(EffectGroup { header, effects })
            }
            "DataActionGroup" => ProjectEffect::Other(self.too_deep(e)),
            "DataActionFilterProcedural" => {
                let header = self.header(e);
                let procedural = e.object("procedural").map(|p| self.procedural(p, depth));
                let inputs = e.objects("inputs").map(|i| self.input_identifier(i)).collect();
                let input_alphas = e
                    .objects("inputs")
                    .filter_map(|i| {
                        let alpha = i.string("identifierAlpha").filter(|a| !a.is_empty())?;
                        Some((i.string("identifier").unwrap_or_default().to_string(), alpha.to_string()))
                    })
                    .collect();
                ProjectEffect::Filter(FilterEffect { header, procedural, inputs, input_alphas })
            }
            "DataActionFilterLevels" => {
                let header = self.header(e);
                let levels = e.object("levels").map(|l| self.levels(l));
                ProjectEffect::Levels(LevelsEffect { header, levels })
            }
            "DataActionFilterColorSelection" => ProjectEffect::ColorSelection(self.color_selection(e)),
            "DataActionAnchor" => ProjectEffect::Anchor(self.header(e)),
            "DataActionDecal" => {
                let header = self.header(e);
                let all: Vec<&RawObject> = e.objects("hits").collect();
                let color = self.decal_color(e);
                let hits: Vec<_> = all.iter().filter_map(|h| self.decal_hit(h, color)).collect();
                let unread = all.len() - hits.len();
                ProjectEffect::PolygonFill(PolygonFillEffect { header, hits, unread })
            }
            _ => ProjectEffect::Other(self.raw(e)),
        }
    }

    /// Marks the effect as recognised and reads the fields every effect class has.
    fn header(&mut self, e: &RawObject) -> EffectHeader {
        self.seen(e);
        EffectHeader {
            uid: e.uid(),
            enabled: e.bool("enabled").unwrap_or(true),
            label: e.string("label").unwrap_or_default().to_string(),
            channels: ChannelMask(e.channel_mask("channelTypes").unwrap_or(0)),
            blending: self.blending(e),
            unhandled: Vec::new(),
        }
    }

    fn fill(&mut self, e: &RawObject, depth: usize) -> FillEffect {
        let header = self.header(e);
        let sources = e.objects("sources").map(|s| self.source(s, depth)).collect();
        FillEffect {
            header,
            sources,
            projection: e.u64("projection").unwrap_or(0),
            filtering: e.u64("filtering").unwrap_or(0),
            uv_transform: e.object("uvTransformation0").or_else(|| e.object("uvTransformation")).map(|t| self.uv_transform(t)),
            placement: self.fill_placement(e),
        }
    }

    fn uv_transform(&mut self, t: &RawObject) -> UvTransform {
        self.seen(t);
        let vec2 = |name: &str, default: f32| match t.floats(name) {
            Some([x, y, ..]) => [*x, *y],
            _ => [default; 2],
        };
        UvTransform {
            scale: match t.field("scaleFactors") {
                Some(_) => vec2("scaleFactors", 1.0),
                None => vec2("scale", 1.0),
            },
            translation: vec2("translation", 0.0),
            rotation: t.f32("rotation").unwrap_or(0.0),
        }
    }

    fn paint(&mut self, e: &RawObject, tool: PaintTool, depth: usize) -> PaintEffect {
        let header = self.header(e);
        let strokes = e.objects("strokes").map(|s| self.stroke(s)).collect();
        let paths = e.objects("strokes3D").map(|s| self.path(s)).collect();
        PaintEffect {
            header,
            tool,
            opacity: e.f32("opacity").unwrap_or(1.0),
            strokes,
            paths,
            brush_uid: e.object("brush").and_then(|b| b.uid()),
            brush: e.object("brush").map(|b| self.brush(b)),
            stamp_blending: e.u64("stampBlendingMode").unwrap_or(0),
            colors: e.objects("sourceColor").map(|s| self.source(s, depth)).collect(),
            alpha: e.one_or_many("sourceTransparent").map(|s| self.source(s, depth)).collect(),
            stencil: e.object("sourceStencil").map(|s| self.source(s, depth)),
            stencil_matrix: e.floats("projMatrix").and_then(|m| <[f32; 9]>::try_from(m).ok()),
        }
    }

    pub(super) fn levels(&mut self, l: &RawObject) -> Levels {
        self.seen(l);
        let v = |name: &str, default: f32| l.vec3(name).unwrap_or([default; 3]);
        Levels {
            input_minimum: v("inputMinimum", 0.0),
            input_maximum: v("inputMaximum", 1.0),
            gamma: v("gamma", 1.0),
            output_minimum: v("outputMinimum", 0.0),
            output_maximum: v("outputMaximum", 1.0),
            clamp: l.u64("clamp").unwrap_or(0),
        }
    }

    fn color_selection(&mut self, e: &RawObject) -> ColorSelectionEffect {
        let header = self.header(e);
        let mut colors = Vec::new();
        for c in e.objects("selectedColors") {
            self.seen(c);
            colors.extend(c.vec3("value"));
        }
        let bitmap_url = self.bitmap_url(e.object("sourceBitmap"));
        ColorSelectionEffect { header, colors, tolerance: e.f32("tolerance").unwrap_or(0.0), hardness: e.f32("hardness").unwrap_or(1.0), bitmap_url }
    }
}
