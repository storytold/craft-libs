//! SBS `<constantValue*>` XML scalar/vector value representation.

use crate::xml::{parse_floats, parse_ints};
use roxmltree::Node as XmlNode;

#[derive(Clone, Debug, PartialEq)]
pub enum SbsValue {
    Floats(Vec<f32>),
    Ints(Vec<i32>),
    Bool(bool),
    Text(String),
}

impl SbsValue {
    pub fn float(&self, i: usize) -> f32 {
        match self {
            SbsValue::Floats(v) => v.get(i).or(v.first()).copied().unwrap_or(0.0),
            SbsValue::Ints(v) => v.get(i).or(v.first()).copied().unwrap_or(0) as f32,
            SbsValue::Bool(b) => f32::from(u8::from(*b)),
            SbsValue::Text(_) => 0.0,
        }
    }

    pub fn int(&self, i: usize) -> i32 {
        match self {
            SbsValue::Ints(v) => v.get(i).or(v.first()).copied().unwrap_or(0),
            SbsValue::Floats(v) => v.get(i).or(v.first()).copied().unwrap_or(0.0).round() as i32,
            SbsValue::Bool(b) => i32::from(*b),
            SbsValue::Text(_) => 0,
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            SbsValue::Bool(b) => *b,
            SbsValue::Text(s) => s == "1" || s.eq_ignore_ascii_case("true"),
            _ => self.float(0) != 0.0,
        }
    }

    pub fn component_count(&self) -> usize {
        match self {
            SbsValue::Floats(v) => v.len(),
            SbsValue::Ints(v) => v.len(),
            _ => 1,
        }
    }
}

pub fn sbs_constant(holder: XmlNode) -> Option<SbsValue> {
    sbs_constant_element(holder.children().find(|child| child.is_element())?)
}

pub fn sbs_constant_element(element: XmlNode) -> Option<SbsValue> {
    let text = element.attribute("v").unwrap_or("");
    let tag = element.tag_name().name();

    if tag == "constantValueBool" {
        Some(SbsValue::Bool(parse_ints(text).first().is_some_and(|b| *b != 0) || text.trim().eq_ignore_ascii_case("true")))
    } else if tag == "constantValueString" {
        Some(SbsValue::Text(text.to_string()))
    } else if tag.starts_with("constantValueFloat") {
        Some(SbsValue::Floats(parse_floats(text)))
    } else if tag.starts_with("constantValueInt") {
        Some(SbsValue::Ints(parse_ints(text)))
    } else {
        None
    }
}
