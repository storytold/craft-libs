//! A summary of a PDF's print-relevant structure, and the scene `structure` block it is checked
//! against.
//!
//! Color spaces are described as families: `DeviceCMYK`, `ICCBased(N=4)`,
//! `Separation(Gold, alt DeviceCMYK)`, `DeviceN(Cyan,Black, alt DeviceCMYK)`, `Indexed(…)`,
//! `Pattern`. Transparency group color spaces are reported per group, not in `color_spaces`.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use hayro_syntax::Pdf;
use hayro_syntax::content::UntypedIter;
use hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};
use serde::{Deserialize, Serialize};

use crate::Error;

/// What [`summarize`] found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    /// Every transparency group: pages first, then form XObjects and soft-mask groups.
    pub groups: Vec<Group>,
    pub ext_g_states: Vec<ExtGState>,
    /// Color space families used for painting: resources, images, shadings and the device
    /// spaces set directly by content operators (`rg`, `k`, `g`, `cs /DeviceRGB`).
    pub color_spaces: BTreeSet<String>,
    /// Any painting color space mentions DeviceRGB, including Separation/DeviceN alternates.
    pub device_rgb: bool,
    /// `/S` of each output intent, in order.
    pub output_intents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    /// `page 1`, `form` or `soft mask`.
    pub at: String,
    pub s: String,
    pub cs: Option<String>,
    pub i: bool,
    pub k: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ExtGState {
    /// `/OP` (stroke overprint, and fill overprint when `/op` is absent).
    #[serde(rename = "OP", skip_serializing_if = "Option::is_none")]
    pub op_stroke: Option<bool>,
    /// `/op` (fill overprint).
    #[serde(rename = "op", skip_serializing_if = "Option::is_none")]
    pub op_fill: Option<bool>,
    #[serde(rename = "OPM", skip_serializing_if = "Option::is_none")]
    pub opm: Option<i32>,
    #[serde(rename = "BM", skip_serializing_if = "Option::is_none")]
    pub bm: Option<String>,
    /// `/ca` (fill and text alpha).
    #[serde(rename = "ca", skip_serializing_if = "Option::is_none")]
    pub alpha_fill: Option<f32>,
    /// `/CA` (stroke alpha).
    #[serde(rename = "CA", skip_serializing_if = "Option::is_none")]
    pub alpha_stroke: Option<f32>,
    /// The soft mask's `/S`, when the state sets a mask.
    #[serde(rename = "SMask", skip_serializing_if = "Option::is_none")]
    pub smask: Option<String>,
    /// The soft mask has `/BC`.
    #[serde(rename = "BC", skip_serializing_if = "std::ops::Not::not")]
    pub smask_bc: bool,
    /// The soft mask has a `/TR` other than `/Identity`.
    #[serde(rename = "TR", skip_serializing_if = "std::ops::Not::not")]
    pub smask_tr: bool,
}

/// Summarize `pdf`. Malformed objects are skipped rather than reported.
pub fn summarize(pdf: &[u8]) -> Result<Summary, Error> {
    let pdf = Pdf::new(Arc::new(pdf.to_vec())).map_err(|e| Error::Read(format!("{e:?}")))?;
    let mut walk = Walk::default();
    if let Some(catalog) = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id())
        && let Some(intents) = catalog.get::<Array<'_>>(b"OutputIntents".as_slice())
    {
        for intent in intents.iter::<Dict<'_>>() {
            walk.summary.output_intents.push(intent.get::<Name<'_>>(b"S".as_slice()).map(|n| n.as_str().to_owned()).unwrap_or_default());
        }
    }
    for (index, page) in pdf.pages().iter().enumerate() {
        let raw = page.raw();
        if let Some(group) = raw.get::<Dict<'_>>(b"Group".as_slice()) {
            walk.group(&format!("page {}", index + 1), &group);
        }
        let mut ops = page.operations();
        walk.operators(&mut ops);
        let res = page.resources();
        walk.resources(&Resources {
            ext_g_states: res.ext_g_states.clone(),
            color_spaces: res.color_spaces.clone(),
            x_objects: res.x_objects.clone(),
            patterns: res.patterns.clone(),
            shadings: res.shadings.clone(),
        });
    }
    let mut summary = walk.summary;
    summary.device_rgb = summary.color_spaces.iter().any(|cs| cs.contains("DeviceRGB"));
    Ok(summary)
}

struct Resources<'a> {
    ext_g_states: Dict<'a>,
    color_spaces: Dict<'a>,
    x_objects: Dict<'a>,
    patterns: Dict<'a>,
    shadings: Dict<'a>,
}

impl<'a> Resources<'a> {
    fn of(dict: &Dict<'a>) -> Self {
        let sub = |key: &[u8]| dict.get::<Dict<'a>>(key).unwrap_or_default();
        Self {
            ext_g_states: sub(b"ExtGState"),
            color_spaces: sub(b"ColorSpace"),
            x_objects: sub(b"XObject"),
            patterns: sub(b"Pattern"),
            shadings: sub(b"Shading"),
        }
    }
}

#[derive(Default)]
struct Walk {
    summary: Summary,
    seen: HashSet<ObjectIdentifier>,
}

impl Walk {
    /// True the first time an indirect object is visited.
    fn first_visit(&mut self, id: Option<ObjectIdentifier>) -> bool {
        id.is_none_or(|id| self.seen.insert(id))
    }

    fn group(&mut self, at: &str, group: &Dict<'_>) {
        self.summary.groups.push(Group {
            at: at.to_owned(),
            s: group.get::<Name<'_>>(b"S".as_slice()).map(|n| n.as_str().to_owned()).unwrap_or_default(),
            cs: group.get::<Object<'_>>(b"CS".as_slice()).map(|cs| describe(&cs, 0)),
            i: group.get::<bool>(b"I".as_slice()).unwrap_or(false),
            k: group.get::<bool>(b"K".as_slice()).unwrap_or(false),
        });
    }

    fn operators(&mut self, ops: &mut UntypedIter<'_>) {
        while let Some(op) = ops.next() {
            let family = match &**op.operator {
                b"rg" | b"RG" => Some("DeviceRGB".to_owned()),
                b"k" | b"K" => Some("DeviceCMYK".to_owned()),
                b"g" | b"G" => Some("DeviceGray".to_owned()),
                b"cs" | b"CS" => op
                    .operands()
                    .next()
                    .and_then(|o| o.clone().into_name())
                    .map(|n| n.as_str().to_owned())
                    .filter(|n| matches!(n.as_str(), "DeviceRGB" | "DeviceCMYK" | "DeviceGray")),
                _ => None,
            };
            if let Some(family) = family {
                self.summary.color_spaces.insert(family);
            }
        }
    }

    fn resources(&mut self, res: &Resources<'_>) {
        for key in res.color_spaces.keys() {
            if let Some(cs) = res.color_spaces.get::<Object<'_>>(key.as_ref()) {
                self.summary.color_spaces.insert(describe(&cs, 0));
            }
        }
        for key in res.ext_g_states.keys() {
            if let Some(gs) = res.ext_g_states.get::<Dict<'_>>(key.as_ref())
                && self.first_visit(gs.obj_id())
            {
                self.ext_g_state(&gs);
            }
        }
        for key in res.shadings.keys() {
            if let Some(shading) = res.shadings.get::<Object<'_>>(key.as_ref()) {
                self.shading(&shading);
            }
        }
        for key in res.patterns.keys() {
            let Some(pattern) = res.patterns.get::<Object<'_>>(key.as_ref()) else { continue };
            let (dict, stream) = match pattern {
                Object::Dict(dict) => (dict, None),
                Object::Stream(stream) => (stream.dict().clone(), Some(stream)),
                _ => continue,
            };
            if let Some(shading) = dict.get::<Object<'_>>(b"Shading".as_slice()) {
                self.summary.color_spaces.insert("Pattern".to_owned());
                self.shading(&shading);
            }
            if let Some(stream) = stream
                && self.first_visit(Some(stream.obj_id()))
            {
                self.summary.color_spaces.insert("Pattern".to_owned());
                self.content(&stream);
            }
        }
        for key in res.x_objects.keys() {
            if let Some(xobject) = res.x_objects.get::<Stream<'_>>(key.as_ref())
                && self.first_visit(Some(xobject.obj_id()))
            {
                self.x_object(&xobject, "form");
            }
        }
    }

    fn ext_g_state(&mut self, gs: &Dict<'_>) {
        let mut state = ExtGState {
            op_stroke: gs.get::<bool>(b"OP".as_slice()),
            op_fill: gs.get::<bool>(b"op".as_slice()),
            opm: gs.get::<i32>(b"OPM".as_slice()),
            alpha_fill: gs.get::<f32>(b"ca".as_slice()),
            alpha_stroke: gs.get::<f32>(b"CA".as_slice()),
            bm: gs.get::<Object<'_>>(b"BM".as_slice()).and_then(|bm| match bm {
                Object::Name(name) => Some(name.as_str().to_owned()),
                Object::Array(names) => names.iter::<Name<'_>>().next().map(|n| n.as_str().to_owned()),
                _ => None,
            }),
            ..Default::default()
        };
        if let Some(mask) = gs.get::<Dict<'_>>(b"SMask".as_slice()) {
            state.smask = Some(mask.get::<Name<'_>>(b"S".as_slice()).map(|n| n.as_str().to_owned()).unwrap_or_default());
            state.smask_bc = mask.contains_key(b"BC".as_slice());
            state.smask_tr = mask.get::<Object<'_>>(b"TR".as_slice()).is_some_and(|tr| !matches!(&tr, Object::Name(n) if n.as_str() == "Identity"));
            if let Some(source) = mask.get::<Stream<'_>>(b"G".as_slice())
                && self.first_visit(Some(source.obj_id()))
            {
                self.x_object(&source, "soft mask");
            }
        }
        self.summary.ext_g_states.push(state);
    }

    fn shading(&mut self, shading: &Object<'_>) {
        let dict = match shading {
            Object::Dict(dict) => dict.clone(),
            Object::Stream(stream) => stream.dict().clone(),
            _ => return,
        };
        if let Some(cs) = dict.get::<Object<'_>>(b"ColorSpace".as_slice()) {
            self.summary.color_spaces.insert(describe(&cs, 0));
        }
    }

    fn x_object(&mut self, xobject: &Stream<'_>, at: &str) {
        let dict = xobject.dict();
        match dict.get::<Name<'_>>(b"Subtype".as_slice()).as_ref().map(Name::as_str) {
            Some("Image") => {
                if !dict.get::<bool>(b"ImageMask".as_slice()).unwrap_or(false)
                    && let Some(cs) = dict.get::<Object<'_>>(b"ColorSpace".as_slice())
                {
                    self.summary.color_spaces.insert(describe(&cs, 0));
                }
            }
            Some("Form") => {
                if let Some(group) = dict.get::<Dict<'_>>(b"Group".as_slice()) {
                    self.group(at, &group);
                }
                self.content(xobject);
            }
            _ => {}
        }
    }

    fn content(&mut self, stream: &Stream<'_>) {
        if let Ok(data) = stream.decoded() {
            self.operators(&mut UntypedIter::new(&data));
        }
        if let Some(res) = stream.dict().get::<Dict<'_>>(b"Resources".as_slice()) {
            self.resources(&Resources::of(&res));
        }
    }
}

/// A color space's family, with the names and alternates that matter for separation.
fn describe(cs: &Object<'_>, depth: usize) -> String {
    if depth > 4 {
        return "?".to_owned();
    }
    match cs {
        Object::Name(name) => name.as_str().to_owned(),
        Object::Array(array) => {
            let items: Vec<Object<'_>> = array.iter::<Object<'_>>().collect();
            let Some(Object::Name(kind)) = items.first() else { return "?".to_owned() };
            let alt = |i: usize| items.get(i).map(|o| describe(o, depth + 1)).unwrap_or_else(|| "?".to_owned());
            match kind.as_str() {
                "Separation" => {
                    let name = items.get(1).and_then(|o| o.clone().into_name()).map(|n| n.as_str().to_owned()).unwrap_or_default();
                    format!("Separation({name}, alt {})", alt(2))
                }
                "DeviceN" => {
                    let names: Vec<String> = match items.get(1) {
                        Some(Object::Array(names)) => names.iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect(),
                        _ => Vec::new(),
                    };
                    format!("DeviceN({}, alt {})", names.join(","), alt(2))
                }
                "ICCBased" => {
                    let n = match items.get(1) {
                        Some(Object::Stream(stream)) => stream.dict().get::<i32>(b"N".as_slice()).unwrap_or(0),
                        _ => 0,
                    };
                    format!("ICCBased(N={n})")
                }
                "Indexed" => format!("Indexed({})", alt(1)),
                "Pattern" if items.len() > 1 => format!("Pattern({})", alt(1)),
                other => other.to_owned(),
            }
        }
        _ => "?".to_owned(),
    }
}

/// Live transparency left in a page that should have none, as a flattener's output must: transparency
/// groups, soft masks in graphics states, blend modes other than Normal, and `ca` or `CA` below 1.
/// Soft masks of images are not graphics states and are allowed. Empty when there is none.
pub fn transparency(summary: &Summary) -> Vec<String> {
    let mut out: Vec<String> = summary.groups.iter().filter(|g| g.s == "Transparency").map(|g| format!("transparency group ({})", g.at)).collect();
    for gs in &summary.ext_g_states {
        if let Some(mask) = gs.smask.as_deref().filter(|m| !m.is_empty() && *m != "None") {
            out.push(format!("soft mask /{mask}"));
        }
        if let Some(bm) = gs.bm.as_deref().filter(|b| *b != "Normal" && *b != "Compatible") {
            out.push(format!("blend mode /{bm}"));
        }
        for (key, alpha) in [("ca", gs.alpha_fill), ("CA", gs.alpha_stroke)] {
            if let Some(a) = alpha.filter(|a| *a < 1.0) {
                out.push(format!("/{key} {a}"));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A scene's `structure` block. Every field is optional; only the fields present are checked.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureExpect {
    /// The page group's `/CS`: a family, `"none"` (no page group) or `"no-cs"` (group without
    /// `/CS`). Checked on every page.
    #[serde(default)]
    pub page_group: Option<String>,
    /// The set of `/CS` values of form and soft-mask groups; `"none"` stands for a group
    /// without `/CS`.
    #[serde(default)]
    pub group_cs: Option<BTreeSet<String>>,
    /// Some group has `/K true`.
    #[serde(default)]
    pub knockout: Option<bool>,
    /// Some group (page groups excluded) has `/I true`.
    #[serde(default)]
    pub isolated: Option<bool>,
    /// Some graphics state sets `/OP` or `/op` true.
    #[serde(default)]
    pub overprint: Option<bool>,
    /// Some graphics state sets a soft mask.
    #[serde(default)]
    pub smask: Option<bool>,
    /// The set of blend modes other than Normal.
    #[serde(default)]
    pub blend_modes: Option<BTreeSet<String>>,
    /// Each entry must match some graphics state on every key it gives.
    #[serde(default)]
    pub ext_g_states: Option<Vec<ExtGStateMatch>>,
    /// The exact set of painting color space families.
    #[serde(default)]
    pub color_spaces: Option<BTreeSet<String>>,
    /// Families that must be among the painting color spaces.
    #[serde(default)]
    pub color_spaces_include: Option<BTreeSet<String>>,
    #[serde(default)]
    pub device_rgb: Option<bool>,
    #[serde(default)]
    pub output_intents: Option<Vec<String>>,
}

/// A partial graphics state; keys as in the PDF.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtGStateMatch {
    #[serde(rename = "OP", default)]
    pub op_stroke: Option<bool>,
    #[serde(rename = "op", default)]
    pub op_fill: Option<bool>,
    #[serde(rename = "OPM", default)]
    pub opm: Option<i32>,
    #[serde(rename = "BM", default)]
    pub bm: Option<String>,
    #[serde(rename = "SMask", default)]
    pub smask: Option<String>,
    #[serde(rename = "BC", default)]
    pub smask_bc: Option<bool>,
    #[serde(rename = "TR", default)]
    pub smask_tr: Option<bool>,
}

impl ExtGStateMatch {
    fn matches(&self, gs: &ExtGState) -> bool {
        (self.op_stroke.is_none() || self.op_stroke == gs.op_stroke)
            && (self.op_fill.is_none() || self.op_fill == gs.op_fill)
            && (self.opm.is_none() || self.opm == gs.opm)
            && (self.bm.is_none() || self.bm == gs.bm)
            && (self.smask.is_none() || self.smask == gs.smask)
            && self.smask_bc.is_none_or(|bc| bc == gs.smask_bc)
            && self.smask_tr.is_none_or(|tr| tr == gs.smask_tr)
    }
}

impl StructureExpect {
    /// Differences between the expectation and `summary`; empty when it matches.
    pub fn check(&self, summary: &Summary) -> Vec<String> {
        let mut out = Vec::new();
        let mut differ = |what: &str, want: String, got: String| {
            if want != got {
                out.push(format!("{what}: expected {want}, found {got}"));
            }
        };
        let page_groups: Vec<&Group> = summary.groups.iter().filter(|g| g.at.starts_with("page")).collect();
        let others: Vec<&Group> = summary.groups.iter().filter(|g| !g.at.starts_with("page")).collect();
        if let Some(want) = &self.page_group {
            let got = if page_groups.is_empty() {
                "none".to_owned()
            } else {
                let all: BTreeSet<String> = page_groups.iter().map(|g| g.cs.clone().unwrap_or_else(|| "no-cs".to_owned())).collect();
                all.into_iter().collect::<Vec<_>>().join(" | ")
            };
            differ("page_group", want.clone(), got);
        }
        if let Some(want) = &self.group_cs {
            let got: BTreeSet<String> = others.iter().map(|g| g.cs.clone().unwrap_or_else(|| "none".to_owned())).collect();
            differ("group_cs", format!("{want:?}"), format!("{got:?}"));
        }
        if let Some(want) = self.knockout {
            differ("knockout", want.to_string(), summary.groups.iter().any(|g| g.k).to_string());
        }
        if let Some(want) = self.isolated {
            differ("isolated", want.to_string(), others.iter().any(|g| g.i).to_string());
        }
        let states = &summary.ext_g_states;
        if let Some(want) = self.overprint {
            let got = states.iter().any(|s| s.op_stroke == Some(true) || s.op_fill == Some(true));
            differ("overprint", want.to_string(), got.to_string());
        }
        if let Some(want) = self.smask {
            differ("smask", want.to_string(), states.iter().any(|s| s.smask.is_some()).to_string());
        }
        if let Some(want) = &self.blend_modes {
            let got: BTreeSet<String> = states.iter().filter_map(|s| s.bm.clone()).filter(|bm| bm != "Normal" && bm != "Compatible").collect();
            differ("blend_modes", format!("{want:?}"), format!("{got:?}"));
        }
        if let Some(want) = &self.color_spaces {
            differ("color_spaces", format!("{want:?}"), format!("{:?}", summary.color_spaces));
        }
        if let Some(want) = &self.color_spaces_include {
            let missing: BTreeSet<&String> = want.difference(&summary.color_spaces).collect();
            differ("color_spaces_include missing", "{}".to_owned(), format!("{missing:?}"));
        }
        if let Some(want) = self.device_rgb {
            differ("device_rgb", want.to_string(), summary.device_rgb.to_string());
        }
        if let Some(want) = &self.output_intents {
            differ("output_intents", format!("{want:?}"), format!("{:?}", summary.output_intents));
        }
        for wanted in self.ext_g_states.iter().flatten() {
            if !states.iter().any(|s| wanted.matches(s)) {
                out.push(format!("ext_g_states: nothing matches {wanted:?}"));
            }
        }
        out
    }
}
