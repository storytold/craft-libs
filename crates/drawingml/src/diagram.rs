//! SmartArt (`dgm:`): a diagram's layout and its outline of nodes.

use serde::{Deserialize, Serialize};

use crate::MediaId;

/// A SmartArt diagram's data: its layout and its outline of nodes (the Text Pane).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Diagram {
    /// The layout: list, blockList, hBulletList, process, chevron, verticalProcess, stepUp, cycle,
    /// blockCycle, radial, hierarchy, pyramid, invertedPyramid, matrix or titledMatrix.
    pub kind: String,
    pub nodes: Vec<DiagramNode>,
    /// SmartArt Style: `flat` (default), `subtle`, `intense` or `outline`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub style: String,
    /// Change Colors: `colorful` (default: each shape its own accent) or `accent1`…`accent6`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub colors: String,
    /// The group's shapes are the file's drawing, not generated from `nodes`: text typed into them
    /// isn't mapped back to nodes (the first outline edit regenerates them).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub imported: bool,
    /// PowerPoint's layout id (`layoutDef@uniqueId`) the diagram came with.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub layout_id: String,
    /// The file's own layout, quick style and colour definitions, written back unchanged while the
    /// layout isn't switched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_xml: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_xml: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors_xml: Option<String>,
}

/// One line of the Text Pane: its text and outline level (0 = top level).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DiagramNode {
    pub text: String,
    pub level: u8,
    /// The picture shown in the node's picture placeholder (picture layouts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<MediaId>,
    /// An assistant (organisation charts): drawn beside its parent, off the row of its siblings.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub assistant: bool,
    /// The job title under the name (Name and Title Organization Chart).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
}

pub mod parts;
