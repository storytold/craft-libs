//! The XML description of a compiled graph (`<graph>.xml`): its identity, inputs and outputs, keyed by the
//! uids the compiled code uses. Only the attributes the decoder needs are read; the reader is a small tag
//! scanner, not a general XML parser (the files are machine-written and flat).

use crate::decode::DecodeError;
use crate::format_names::DESCRIPTION_ELEMENT;

const RECORD: &str = "graph package description";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Description {
    pub asm_uid: Option<u32>,
    pub cooker_build: Option<u32>,
    pub pkg_url: String,
    pub label: String,
    pub inputs: Vec<GraphInputInfo>,
    pub outputs: Vec<GraphOutputInfo>,
}

/// Format data: the `type` of a graph input that takes an image (the package's own numbering, see [`GraphInputInfo::ty`]).
pub const INPUT_TYPE_IMAGE: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct GraphInputInfo {
    pub uid: u32,
    pub identifier: String,
    /// 0 float, 1 float2, 2 float3, 3 float4, 4 int, 5 image, 6 string, 8 int2, 9 int3, 10 int4 (as in the XML).
    pub ty: u32,
    pub default: Option<String>,
    pub label: Option<String>,
    pub widget: Option<String>,
    pub min: Option<String>,
    pub max: Option<String>,
    /// `usertag`, `key=value` pairs separated by `;`.
    pub usertag: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphOutputInfo {
    pub uid: u32,
    pub identifier: String,
    pub format: Option<u32>,
    pub label: Option<String>,
    /// `usertag`, `key=value` pairs separated by `;`.
    pub usertag: Option<String>,
}

impl Description {
    /// The graph the description is about. A description can list several graphs (seen: a material package that also
    /// lists a helper graph without outputs, "Make Radial Snippet", before the material): the first graph with outputs
    /// is taken, else the first.
    pub fn parse(xml: &[u8]) -> Result<Description, DecodeError> {
        let text = std::str::from_utf8(xml).map_err(|e| DecodeError::invalid(RECORD, e.to_string()))?;
        let (mut asm_uid, mut cooker_build) = (None, None);
        let mut graphs = Vec::new();
        let mut rest = text;
        loop {
            let d = Self::parse_graph(rest);
            asm_uid = asm_uid.or(d.asm_uid);
            cooker_build = cooker_build.or(d.cooker_build);
            graphs.push(d);
            // the next graph starts after this one's end tag
            match rest.find("</graph>").and_then(|i| rest.get(i + "</graph>".len()..)) {
                Some(next) if next.contains("<graph ") => rest = next,
                _ => break,
            }
        }
        let pick = graphs.iter().position(|g| !g.outputs.is_empty()).unwrap_or(0);
        let mut d = graphs.into_iter().nth(pick).unwrap_or_default();
        d.asm_uid = asm_uid;
        d.cooker_build = cooker_build;
        Ok(d)
    }

    /// The first graph in `text` (and the root element's attributes, if they come before it).
    fn parse_graph(text: &str) -> Description {
        let mut d = Description::default();
        let mut current_input: Option<GraphInputInfo> = None;
        let mut current_output: Option<GraphOutputInfo> = None;
        for tag in Tags::new(text) {
            match tag.name {
                DESCRIPTION_ELEMENT => {
                    d.asm_uid = tag.attr("asmuid").and_then(|v| v.parse().ok());
                    d.cooker_build = tag.attr("cookerbuild").and_then(|v| v.parse().ok());
                }
                "graph" if d.pkg_url.is_empty() => {
                    d.pkg_url = tag.attr("pkgurl").unwrap_or_default();
                    d.label = tag.attr("label").unwrap_or_default();
                }
                "input" => {
                    d.inputs.extend(current_input.take());
                    let (Some(uid), Some(identifier)) = (tag.attr("uid").and_then(|v| v.parse().ok()), tag.attr("identifier")) else {
                        continue;
                    };
                    current_input = Some(GraphInputInfo {
                        uid,
                        identifier,
                        ty: tag.attr("type").and_then(|v| v.parse().ok()).unwrap_or(0),
                        default: tag.attr("default"),
                        label: None,
                        widget: None,
                        min: None,
                        max: None,
                        usertag: tag.attr("usertag"),
                    });
                }
                "inputgui" => {
                    if let Some(input) = current_input.as_mut() {
                        input.label = tag.attr("label");
                        input.widget = tag.attr("widget");
                    }
                }
                "guislider" => {
                    if let Some(input) = current_input.as_mut() {
                        input.min = tag.attr("min");
                        input.max = tag.attr("max");
                    }
                }
                "output" => {
                    d.outputs.extend(current_output.take());
                    let (Some(uid), Some(identifier)) = (tag.attr("uid").and_then(|v| v.parse().ok()), tag.attr("identifier")) else {
                        continue;
                    };
                    current_output = Some(GraphOutputInfo {
                        uid,
                        identifier,
                        format: tag.attr("format").and_then(|v| v.parse().ok()),
                        label: None,
                        usertag: tag.attr("usertag"),
                    });
                }
                "outputgui" => {
                    if let Some(output) = current_output.as_mut() {
                        output.label = tag.attr("label");
                    }
                }
                "/inputs" => d.inputs.extend(current_input.take()),
                "/outputs" => d.outputs.extend(current_output.take()),
                "/graph" => break,
                _ => {}
            }
        }
        d.inputs.extend(current_input.take());
        d.outputs.extend(current_output.take());
        d
    }

    pub fn input(&self, uid: u32) -> Option<&GraphInputInfo> {
        self.inputs.iter().find(|i| i.uid == uid)
    }

    pub fn output(&self, uid: u32) -> Option<&GraphOutputInfo> {
        self.outputs.iter().find(|o| o.uid == uid)
    }
}

/// One start or end tag: its name (end tags start with `/`) and raw attribute text.
struct Tag<'a> {
    name: &'a str,
    attrs: &'a str,
}

impl Tag<'_> {
    /// The value of attribute `key`, with the five predefined entities resolved.
    fn attr(&self, key: &str) -> Option<String> {
        let mut rest = self.attrs;
        while let Some(eq) = rest.find('=') {
            let (before, after) = rest.split_at(eq);
            let name = before.trim();
            let after = after.get(1..)?.trim_start();
            let quote = after.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let body = after.get(1..)?;
            let end = body.find(quote)?;
            let value = body.get(..end)?;
            if name == key {
                return Some(unescape(value));
            }
            rest = body.get(end + 1..)?;
        }
        None
    }
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

struct Tags<'a> {
    rest: &'a str,
}

impl<'a> Tags<'a> {
    fn new(text: &'a str) -> Self {
        Tags { rest: text }
    }
}

impl<'a> Iterator for Tags<'a> {
    type Item = Tag<'a>;

    fn next(&mut self) -> Option<Tag<'a>> {
        loop {
            let start = self.rest.find('<')?;
            let after = self.rest.get(start + 1..)?;
            let end = after.find('>')?;
            let inner = after.get(..end)?;
            self.rest = after.get(end + 1..)?;
            if inner.starts_with('?') || inner.starts_with('!') {
                continue;
            }
            let inner = inner.strip_suffix('/').unwrap_or(inner);
            let split = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            let (name, attrs) = inner.split_at(split);
            return Some(Tag { name, attrs });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::graph_package::root_xml;

    #[test]
    fn reads_inputs_outputs_and_gui() {
        let xml = root_xml(
            r#"<?xml version="1.0"?>
<!DOCTYPE ROOT SYSTEM "ROOT.dtd">
<ROOT formatversion="2.1" asmuid="7" cookerbuild="9" content="full">
 <graphs count="1"><graph pkgurl="pkg://g" label="G &amp; H">
  <outputs count="1"><output uid="5" identifier="out" format="28" width="256"><outputgui label="Out"/></output></outputs>
  <inputs count="2">
   <input uid="1" identifier="$outputsize" type="8" default="8,8"/>
   <input uid="2" identifier="size" type="0" default="0.5"><inputgui widget="slider" label="Size"><guislider min="0" max="1"/></inputgui></input>
  </inputs>
 </graph></graphs></ROOT>"#,
        );
        let d = Description::parse(xml.as_bytes()).unwrap();
        assert_eq!((d.asm_uid, d.cooker_build), (Some(7), Some(9)));
        assert_eq!(d.label, "G & H");
        assert_eq!(d.outputs.len(), 1);
        assert_eq!(d.output(5).unwrap().format, Some(28));
        assert_eq!(d.inputs.len(), 2);
        let size = d.input(2).unwrap();
        assert_eq!((size.ty, size.default.as_deref(), size.max.as_deref()), (0, Some("0.5"), Some("1")));
        assert_eq!(size.label.as_deref(), Some("Size"));
    }

    #[test]
    fn of_several_graphs_the_first_with_outputs_is_described() {
        let xml = root_xml(
            r#"<?xml version="1.0"?>
<ROOT formatversion="2.1" asmuid="7" cookerbuild="9" content="full">
 <graphs count="2">
  <graph pkgurl="pkg://helper" label="Helper"><outputs count="0"></outputs>
   <inputs count="1"><input uid="1" identifier="$outputsize" type="8" default="8,8"/></inputs></graph>
  <graph pkgurl="pkg://wood" label="Wood">
   <outputs count="2"><output uid="5" identifier="basecolor" format="0"/><output uid="6" identifier="normal" format="0"/></outputs>
   <inputs count="1"><input uid="3" identifier="Hue" type="0" default="0.5"/></inputs></graph>
 </graphs></ROOT>"#,
        );
        let d = Description::parse(xml.as_bytes()).unwrap();
        assert_eq!((d.asm_uid, d.cooker_build), (Some(7), Some(9)));
        assert_eq!((d.pkg_url.as_str(), d.label.as_str()), ("pkg://wood", "Wood"));
        assert_eq!(d.outputs.iter().map(|o| o.identifier.as_str()).collect::<Vec<_>>(), ["basecolor", "normal"]);
        assert_eq!(d.inputs.iter().map(|i| i.identifier.as_str()).collect::<Vec<_>>(), ["Hue"]);
        // a single graph without outputs is still described
        let one = Description::parse(
            root_xml(r#"<ROOT asmuid="1"><graphs><graph pkgurl="pkg://h" label="H"><outputs count="0"></outputs></graph></graphs></ROOT>"#)
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(one.label, "H");
    }
}
