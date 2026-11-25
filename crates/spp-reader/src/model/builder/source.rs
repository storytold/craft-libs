//! `DataSource*` → [`ProjectSource`], `DataProcedural` → [`Procedural`] with tweaks, outputs and inputs.

use super::{MAX_NESTING, ModelBuilder};
use crate::decode::document::Scalar;
use crate::model::channel::{ChannelMask, ProjectChannel};
use crate::model::procedural::{Procedural, ProceduralInput, ProceduralOutput, Tweak, TweakValue};
use crate::model::raw::RawObject;
use crate::model::source::{BitmapSource, ProceduralSource, ProjectSource, ReferenceSource, UniformSource};

impl ModelBuilder {
    pub(super) fn source(&mut self, s: &RawObject, depth: usize) -> ProjectSource {
        let uid = s.uid();
        let channels = ChannelMask(s.channel_mask("channelTypes").unwrap_or(0));
        match s.class.as_str() {
            "DataSourceUniform" => {
                self.seen(s);
                ProjectSource::Uniform(UniformSource {
                    uid,
                    channels,
                    color: s.vec3("color").unwrap_or([0.0; 3]),
                    opacity: s.f32("opacity").unwrap_or(1.0),
                })
            }
            "DataSourceBitmap" => {
                self.seen(s);
                let url = self.bitmap_url(s.object("bitmap"));
                ProjectSource::Bitmap(BitmapSource { uid, channels, url })
            }
            "DataSourceProcedural" if depth < MAX_NESTING => {
                self.seen(s);
                let procedural = s.object("procedural").map(|p| self.procedural(p, depth + 1));
                ProjectSource::Procedural(ProceduralSource {
                    uid,
                    channels,
                    mode: s.u64("mode").unwrap_or(0),
                    stamp_cycle: s.u64("stampCycleCount").unwrap_or(0).min(u64::from(u32::MAX)) as u32,
                    stroke_wires: s.u64("strokeWires").unwrap_or(0).min(u64::from(u32::MAX)) as u32,
                    procedural,
                })
            }
            "DataSourceProcedural" => ProjectSource::Other(self.too_deep(s)),
            "DataSourceReference" => ProjectSource::Reference(self.reference(s, uid, channels)),
            _ => ProjectSource::Other(self.raw(s)),
        }
    }

    fn reference(&mut self, s: &RawObject, uid: Option<u64>, channels: ChannelMask) -> ReferenceSource {
        self.seen(s);
        let levels = s.object("levels").map(|l| self.levels(l));
        let reference = s.object("reference");
        let mut bindings = Vec::new();
        if let Some(r) = reference {
            self.seen(r);
            for binding in r.objects("bindings") {
                self.seen(binding);
                bindings.push((binding.u64("source").unwrap_or(0), binding.u64("output").unwrap_or(0)));
            }
        }
        ReferenceSource {
            uid,
            channels,
            anchor_uid: reference.and_then(|r| r.u64("source")),
            anchor_label: reference.and_then(|r| r.string("sourceLabel")).unwrap_or_default().to_string(),
            levels,
            bindings,
            mode: s.u64("mode").unwrap_or(0),
        }
    }

    /// `DataBitmap.urlToBitmapRes`, empty if there is no bitmap.
    pub(super) fn bitmap_url(&mut self, bitmap: Option<&RawObject>) -> String {
        let Some(b) = bitmap else { return String::new() };
        self.seen(b);
        b.string("urlToBitmapRes").unwrap_or_default().to_string()
    }

    pub(super) fn procedural(&mut self, p: &RawObject, depth: usize) -> Procedural {
        self.seen(p);
        let tweaks = p.objects("tweaks").map(|t| self.tweak(t)).collect();
        let mut outputs = Vec::new();
        for o in p.objects("outputs") {
            self.seen(o);
            outputs.push(ProceduralOutput {
                identifier: o.string("identifier").unwrap_or_default().to_string(),
                channels: ChannelMask(o.channel_mask("channelTypes").unwrap_or(0)),
                alpha: o.string("identifierAlpha").unwrap_or_default().to_string(),
            });
        }
        let inputs = p.objects("sourcesInputs").map(|i| self.input(i, depth)).collect();
        Procedural {
            uid: p.uid(),
            url: p.string("urlToSbsRes").unwrap_or_default().to_string(),
            tweaks,
            outputs,
            inputs,
            alpha_output: p.string("outputIdentifierAlpha").unwrap_or_default().to_string(),
            mono_output: p.u64("monoChannelOutputIndex").unwrap_or(0),
        }
    }

    fn input(&mut self, i: &RawObject, depth: usize) -> ProceduralInput {
        self.seen(i);
        let mut identifiers = Vec::new();
        // Older files hold one inline `input`, newer ones a list `inputs`.
        for input in i.object("input").into_iter().chain(i.objects("inputs")) {
            identifiers.push(self.input_identifier(input));
        }
        let source = i.object("source").map(|s| Box::new(self.source(s, depth + 1)));
        ProceduralInput { identifiers, source }
    }

    /// A `DataProceduralInput`: input identifier and the channel it reads.
    pub(super) fn input_identifier(&mut self, input: &RawObject) -> (String, ProjectChannel) {
        self.seen(input);
        let channel = ProjectChannel::from_bit(self.channel_bit(input, "channelType").unwrap_or(0));
        (input.string("identifier").unwrap_or_default().to_string(), channel)
    }

    pub(super) fn tweak(&mut self, t: &RawObject) -> Tweak {
        let value = match t.class.as_str() {
            "DataTweakInt" => match t.scalar("value") {
                Some(Scalar::Int(v)) => Some(TweakValue::Int(*v)),
                _ => None,
            },
            "DataTweakInt2" => t.ivec2("value").map(TweakValue::Int2),
            "DataTweakFloat" => t.f32("value").map(|v| TweakValue::Floats(vec![v])),
            "DataTweakFloat2" | "DataTweakFloat3" | "DataTweakFloat4" => t.floats("value").map(|v| TweakValue::Floats(v.to_vec())),
            "DataTweakByteArray" => t.string("value").map(|v| TweakValue::Text(v.to_string())),
            _ => None,
        };
        let value = match value {
            Some(v) => {
                self.seen(t);
                v
            }
            None => TweakValue::Other(self.raw(t).class),
        };
        Tweak { identifier: t.string("identifier").unwrap_or_default().to_string(), value }
    }
}
