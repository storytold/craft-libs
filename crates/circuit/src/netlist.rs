//! The parts and the pins on each net, for a board editor or for review.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::connect::Connectivity;
use crate::limits::Error;
use crate::model::Project;
use crate::natural::natural_cmp;

/// The value of [`Netlist::format`].
pub const NETLIST_FORMAT: &str = "craft-circuit-netlist";
/// The version of the netlist this crate writes.
pub const NETLIST_VERSION: u32 = 1;

/// Parts and nets, in designator and net-name order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Netlist {
    pub format: String,
    pub version: u32,
    pub parts: Vec<NetlistPart>,
    /// Nets with at least one pin.
    pub nets: Vec<NetlistNet>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetlistPart {
    pub designator: String,
    pub value: String,
    pub footprint: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetlistNet {
    pub name: String,
    pub nodes: Vec<NetlistNode>,
}

/// One pin on a net. A pin placed on several sheets (a part with several units sharing a
/// pin) is listed once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetlistNode {
    pub designator: String,
    pub pin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pin_name: String,
}

impl Netlist {
    pub fn new(project: &Project, conn: &Connectivity) -> Self {
        let mut parts: Vec<NetlistPart> = project
            .parts
            .iter()
            .map(|p| NetlistPart {
                designator: p.designator.clone(),
                value: p.value.clone(),
                footprint: p.footprint.clone(),
                fields: p.fields.clone(),
            })
            .collect();
        parts.sort_by(|a, b| natural_cmp(&a.designator, &b.designator));
        let nets = conn
            .nets
            .iter()
            .filter(|n| !n.pins.is_empty())
            .map(|net| {
                let mut nodes: Vec<NetlistNode> = net
                    .pins
                    .iter()
                    .filter_map(|&p| project.pin(p))
                    .map(|pin| NetlistNode {
                        designator: project.parts[pin.part].designator.clone(),
                        pin: pin.number.clone(),
                        pin_name: pin.name.clone(),
                    })
                    .collect();
                nodes.sort_by(|a, b| natural_cmp(&a.designator, &b.designator).then_with(|| natural_cmp(&a.pin, &b.pin)));
                nodes.dedup_by(|a, b| a.designator == b.designator && a.pin == b.pin);
                NetlistNet { name: net.name.clone(), nodes }
            })
            .collect();
        Self { format: NETLIST_FORMAT.to_owned(), version: NETLIST_VERSION, parts, nets }
    }

    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self).map_err(|e| Error::Json(e.to_string()))
    }

    /// One row per pin on a net: `net,designator,pin,pin_name`, with a header row.
    pub fn to_csv(&self) -> String {
        let mut out = String::from("net,designator,pin,pin_name\n");
        for net in &self.nets {
            for node in &net.nodes {
                for (i, field) in [&net.name, &node.designator, &node.pin, &node.pin_name].into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    craft_csv::push_field(&mut out, field, ',');
                }
                out.push('\n');
            }
        }
        out
    }
}
