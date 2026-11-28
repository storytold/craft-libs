//! In-memory migration of version 1 documents to the revised v2 model, in one hop
//! (D-11/D-13/D-14): v1 layers come in, geometry-once shapes, geometry-only layers and
//! first-class operations come out. Grouping is by the full effective pass key —
//! `Shape.tool` first, then `ShapeProduction.operation`, then the `style` fallback — so
//! two distinct tool passes that cook to the same `OperationKind` (two Score tips,
//! Emboss vs PowerEngrave) stay in separate operations with their settings intact.
//! Colour, visibility, lock and order survive: each v1 layer becomes one geometry layer
//! holding all of its shapes, plus one operation per distinct pass in first-appearance
//! order.

use crate::production::{Operation, OperationKind, OperationSettings};
use crate::wire_v1::{V1Layer, V1Operation, V1Shape, V1ShapeProduction, V1SourceGroup};
use crate::{CutStyle, DocError, Layer, Shape, ShapeProduction, ToolOp};

/// Migration seed for per-operation defaults (D-15). v1 has no per-layer settings, so the
/// global v1 `GcodeSettings` are mapped into this by the app; `load_json` uses the neutral
/// `Default` and never imports the app's settings type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OperationDefaults {
    pub power: f64,
    pub speed_mm_per_min: f64,
    pub passes: u32,
    pub focus_mm: f64,
    pub kerf_mm: f64,
}

impl Default for OperationDefaults {
    fn default() -> Self {
        Self { power: 0.0, speed_mm_per_min: 0.0, passes: 1, focus_mm: 0.0, kerf_mm: 0.0 }
    }
}

impl OperationDefaults {
    fn settings(&self) -> OperationSettings {
        OperationSettings {
            power: self.power,
            speed_mm_per_min: self.speed_mm_per_min,
            passes: self.passes,
            focus_mm: self.focus_mm,
            kerf_mm: self.kerf_mm,
            ..OperationSettings::default()
        }
    }
}

/// The grouping key: the effective pass of a v1 shape, keeping the full `ToolOp` variant
/// so distinct tool settings never merge (D-14).
#[derive(Debug, Clone, PartialEq)]
enum PassKey {
    Tool(ToolOp),
    Plain(V1Operation),
}

impl PassKey {
    fn of(shape: &V1Shape) -> Self {
        if let Some(tool) = &shape.tool {
            return PassKey::Tool(tool.clone());
        }
        let op = shape.production.operation.unwrap_or(match shape.style {
            CutStyle::Cut => V1Operation::Cut,
            CutStyle::Engrave => V1Operation::Engrave,
        });
        PassKey::Plain(op)
    }

    /// Maps the key losslessly into a cooked v2 pass: kind plus its settings snapshot.
    fn into_pass(self, seed: &OperationDefaults) -> (OperationKind, OperationSettings) {
        match self {
            PassKey::Tool(ToolOp::Score { tip, passes }) => {
                let mut s = seed.settings();
                s.tip = Some(tip);
                s.passes = passes;
                (OperationKind::Score, s)
            }
            PassKey::Tool(ToolOp::Emboss { tip, passes, deboss }) => {
                let mut s = seed.settings();
                s.tip = Some(tip);
                s.passes = passes;
                s.deboss = deboss;
                (OperationKind::Engrave, s)
            }
            PassKey::Tool(ToolOp::PowerEngrave) | PassKey::Tool(ToolOp::HeatPen) => (OperationKind::Engrave, seed.settings()),
            PassKey::Tool(ToolOp::Punch { diameter_mm }) => {
                let mut s = seed.settings();
                s.punch_diameter_mm = Some(diameter_mm);
                (OperationKind::Punch, s)
            }
            PassKey::Plain(V1Operation::Cut) => (OperationKind::Cut, seed.settings()),
            PassKey::Plain(V1Operation::KissCut) => (OperationKind::KissCut, seed.settings()),
            PassKey::Plain(V1Operation::DieCut) => (OperationKind::DieCut, seed.settings()),
            PassKey::Plain(V1Operation::Draw) => (OperationKind::Draw, seed.settings()),
            PassKey::Plain(V1Operation::Score) => (OperationKind::Score, seed.settings()),
            PassKey::Plain(V1Operation::Engrave) => (OperationKind::Engrave, seed.settings()),
            PassKey::Plain(V1Operation::Perforate) => (OperationKind::Perforate, seed.settings()),
            PassKey::Plain(V1Operation::PrintThenCut) => (OperationKind::PrintThenCut, seed.settings()),
            PassKey::Plain(V1Operation::Guide) => (OperationKind::Guide, OperationSettings::default()),
        }
    }
}

fn v1_production_to_production(v1: &V1ShapeProduction) -> ShapeProduction {
    ShapeProduction {
        line_color: v1.line_color.clone(),
        fill_color: v1.fill_color.clone(),
        material: v1.material.clone(),
        attach: v1.attach,
        mat: v1.mat,
        send: v1.send,
        hidden_contours: v1.hidden_contours.clone(),
        group: v1.group.as_ref().map(|g| Box::new(v1_group_to_group(g))),
    }
}

fn v1_group_to_group(v1: &V1SourceGroup) -> crate::production::SourceGroup {
    crate::production::SourceGroup { kind: v1.kind, members: v1.members.iter().map(v1_shape_to_shape).collect(), path: v1.path.clone() }
}

fn v1_shape_to_shape(v1: &V1Shape) -> Shape {
    Shape {
        id: v1.id,
        path: v1.path.clone(),
        hidden: v1.hidden,
        locked: v1.locked,
        text: v1.text.clone(),
        production: v1_production_to_production(&v1.production),
    }
}

/// The result of migrating a v1 document: shapes, geometry-only layers, operations.
pub(crate) struct Migrated {
    pub shapes: Vec<Shape>,
    pub layers: Vec<Layer>,
    pub operations: Vec<Operation>,
    pub next_shape_id: u64,
}

/// The accumulating result of a v1 migration: geometry-once shapes, geometry-only
/// layers, and first-class operations in pass order.
struct Migration {
    shapes: Vec<Shape>,
    layers: Vec<Layer>,
    operations: Vec<Operation>,
    max_shape_id: u64,
    next_op_id: u64,
}

impl Migration {
    fn new() -> Self {
        Self { shapes: Vec::new(), layers: Vec::new(), operations: Vec::new(), max_shape_id: 0, next_op_id: 1 }
    }

    fn push_shape(&mut self, shape: &V1Shape) {
        self.max_shape_id = self.max_shape_id.max(shape.id);
        if !self.shapes.iter().any(|s| s.id == shape.id) {
            self.shapes.push(v1_shape_to_shape(shape));
        }
    }

    fn push_operation(&mut self, layer: &V1Layer, kind: OperationKind, settings: OperationSettings, members: Vec<u64>) {
        self.operations.push(Operation {
            id: self.next_op_id,
            name: layer.name.clone(),
            kind,
            settings,
            color: layer.color.clone(),
            visible: layer.visible,
            locked: layer.locked,
            members,
            icon: None,
        });
        self.next_op_id = self.next_op_id.saturating_add(1);
    }
}

/// Migrates v1 layers into the revised model in one hop. Geometry is stored once in
/// `shapes`; each v1 layer becomes one geometry-only layer holding all of its shapes,
/// and each distinct effective pass of a v1 layer becomes one ordered operation whose
/// `members` preserve intra-layer shape order (D-19/D-20). A `template` v1 layer becomes
/// a single `OperationKind::Guide` operation (D-17). Colour, visibility, lock and order
/// are preserved on the operations.
pub(crate) fn migrate_v1_layers(v1_layers: Vec<V1Layer>, stored_next_id: u64, seed: &OperationDefaults) -> Result<Migrated, DocError> {
    let mut m = Migration::new();

    for v1_layer in v1_layers {
        let all_members: Vec<u64> = v1_layer.shapes.iter().map(|s| s.id).collect();
        for shape in &v1_layer.shapes {
            m.push_shape(shape);
        }
        m.layers.push(Layer { name: v1_layer.name.clone(), visible: v1_layer.visible, locked: v1_layer.locked, members: all_members.clone() });

        if v1_layer.template {
            m.push_operation(&v1_layer, OperationKind::Guide, OperationSettings::default(), all_members);
            continue;
        }

        // Group by effective pass in first-appearance order; Vec + linear lookup because
        // ToolOp is PartialEq but not Hash.
        let mut groups: Vec<(PassKey, Vec<u64>)> = Vec::new();
        for shape in &v1_layer.shapes {
            let key = PassKey::of(shape);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, members)) => members.push(shape.id),
                None => groups.push((key, vec![shape.id])),
            }
        }
        for (key, members) in groups {
            let (kind, settings) = key.into_pass(seed);
            m.push_operation(&v1_layer, kind, settings, members);
        }
    }

    let next_shape_id = stored_next_id.max(m.max_shape_id.saturating_add(1));
    Ok(Migrated { shapes: m.shapes, layers: m.layers, operations: m.operations, next_shape_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tip;
    use kurbo::BezPath;

    fn square(id: u64) -> V1Shape {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 0.0));
        path.line_to((10.0, 10.0));
        path.close_path();
        V1Shape { id, path, style: CutStyle::Engrave, hidden: false, locked: false, text: None, tool: None, production: V1ShapeProduction::default() }
    }

    fn layer(shapes: Vec<V1Shape>) -> V1Layer {
        V1Layer { name: "l".to_string(), kind: CutStyle::Engrave, visible: true, shapes, locked: false, template: false, color: None }
    }

    #[test]
    fn two_score_tips_stay_in_separate_score_operations() {
        let mut fine = square(1);
        fine.tool = Some(ToolOp::Score { tip: Tip::Fine, passes: 1 });
        let mut wide = square(2);
        wide.tool = Some(ToolOp::Score { tip: Tip::Wide, passes: 3 });
        let m = migrate_v1_layers(vec![layer(vec![fine, wide])], 3, &OperationDefaults::default()).unwrap();
        let (shapes, layers, ops, next_id) = (m.shapes, m.layers, m.operations, m.next_shape_id);
        assert_eq!(shapes.len(), 2);
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].members, vec![1, 2]);
        assert_eq!(ops.len(), 2);
        assert_eq!(next_id, 3);
        let [a, b] = [&ops[0], &ops[1]];
        assert_eq!(a.members, vec![1]);
        assert_eq!(b.members, vec![2]);
        assert_eq!(a.kind, OperationKind::Score);
        assert_eq!(b.kind, OperationKind::Score);
        assert_eq!(a.settings.tip, Some(Tip::Fine));
        assert_eq!(a.settings.passes, 1);
        assert_eq!(b.settings.tip, Some(Tip::Wide));
        assert_eq!(b.settings.passes, 3);
        assert_eq!((a.id, b.id), (1, 2));
    }

    #[test]
    fn emboss_and_power_engrave_both_map_to_engrave_but_stay_separate() {
        let mut emboss = square(1);
        emboss.tool = Some(ToolOp::Emboss { tip: Tip::Fine, passes: 2, deboss: true });
        let mut power = square(2);
        power.tool = Some(ToolOp::PowerEngrave);
        let m = migrate_v1_layers(vec![layer(vec![emboss, power])], 3, &OperationDefaults::default()).unwrap();
        let ops = m.operations;
        assert_eq!(ops.len(), 2);
        let (a, b) = (&ops[0], &ops[1]);
        assert_eq!(a.kind, OperationKind::Engrave);
        assert_eq!(b.kind, OperationKind::Engrave);
        assert!(a.settings.deboss && a.settings.passes == 2 && a.settings.tip == Some(Tip::Fine));
        assert!(!b.settings.deboss && b.settings.tip.is_none());
    }

    #[test]
    fn template_layer_becomes_one_guide_operation_and_style_fallback_applies() {
        let mut plain = square(1);
        plain.style = CutStyle::Cut;
        let template = V1Layer {
            name: "t".to_string(),
            kind: CutStyle::Cut,
            visible: true,
            shapes: vec![square(2)],
            locked: false,
            template: true,
            color: Some("#00ff00".to_string()),
        };
        let m = migrate_v1_layers(vec![layer(vec![plain]), template], 5, &OperationDefaults::default()).unwrap();
        let (layers, ops, next_id) = (m.layers, m.operations, m.next_shape_id);
        assert_eq!(next_id, 5);
        assert_eq!(layers.len(), 2);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].kind, OperationKind::Cut);
        assert_eq!(ops[1].kind, OperationKind::Guide);
        assert_eq!(ops[1].members, vec![2]);
        assert!(!ops[1].is_output());
        // Colour survives the one-hop migration.
        assert_eq!(ops[1].color.as_deref(), Some("#00ff00"));
    }
}
