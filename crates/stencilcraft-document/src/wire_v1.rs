//! Read-only wire mirrors of the version 1 document body (D-16).
//!
//! These types deserialize only — deliberately no `Serialize` — so legacy fields
//! (`Layer.kind`, `Layer.template`, `Shape.style`, `Shape.tool`,
//! `ShapeProduction.operation`) can be read for migration but can never be written
//! into a v2 file. Every field the v1 fixtures may omit carries `#[serde(default)]`.

use kurbo::BezPath;
use serde::Deserialize;

use crate::production::{GroupKind, SendAssign};
use crate::{CutStyle, TextSpec, ToolOp};

/// The nine legacy v1 operation tags (the v2 `Operation` adds `Punch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum V1Operation {
    Cut,
    KissCut,
    DieCut,
    Draw,
    Score,
    Engrave,
    Perforate,
    PrintThenCut,
    Guide,
}

/// A v1 shape's retained Combine/Flatten sources.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct V1SourceGroup {
    pub kind: GroupKind,
    pub members: Vec<V1Shape>,
    pub path: BezPath,
}

/// A v1 shape's production metadata, mirroring every v1 field so nothing is truncated.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct V1ShapeProduction {
    #[serde(default)]
    pub operation: Option<V1Operation>,
    #[serde(default)]
    pub line_color: Option<String>,
    #[serde(default)]
    pub fill_color: Option<String>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub attach: Option<u64>,
    #[serde(default)]
    pub mat: Option<u32>,
    #[serde(default)]
    pub send: Option<SendAssign>,
    #[serde(default)]
    pub hidden_contours: Vec<usize>,
    #[serde(default)]
    pub group: Option<Box<V1SourceGroup>>,
}

/// One v1 shape: geometry plus its legacy style/tool/production fields.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct V1Shape {
    pub id: u64,
    pub path: BezPath,
    pub style: CutStyle,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub text: Option<Box<TextSpec>>,
    #[serde(default)]
    pub tool: Option<ToolOp>,
    #[serde(default)]
    pub production: V1ShapeProduction,
}

/// One v1 layer: named, styled, with its shapes embedded inline.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct V1Layer {
    pub name: String,
    pub kind: CutStyle,
    pub visible: bool,
    pub shapes: Vec<V1Shape>,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub template: bool,
    #[serde(default)]
    pub color: Option<String>,
}
