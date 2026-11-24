//! In-memory migration of version 1 documents to the v2 operation-first model (D-13/D-14).
//!
//! Pure function, no I/O: v1 layers come in, geometry-once shapes and ordered operation
//! layers come out. Grouping is by the full effective pass key — `Shape.tool` first, then
//! `ShapeProduction.operation`, then the `style` fallback — so two distinct tool passes
//! that cook to the same `Operation` (two Score tips, Emboss vs PowerEngrave) stay in
//! separate layers with their settings intact.

use crate::production::{Operation, OperationSettings};
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

    /// Maps the key losslessly into a cooked v2 `Operation` with its settings snapshot.
    fn into_operation(self, seed: &OperationDefaults) -> Operation {
        match self {
            PassKey::Tool(ToolOp::Score { tip, passes }) => {
                let mut s = seed.settings();
                s.tip = Some(tip);
                s.passes = passes;
                Operation::Score(s)
            }
            PassKey::Tool(ToolOp::Emboss { tip, passes, deboss }) => {
                let mut s = seed.settings();
                s.tip = Some(tip);
                s.passes = passes;
                s.deboss = deboss;
                Operation::Engrave(s)
            }
            PassKey::Tool(ToolOp::PowerEngrave) | PassKey::Tool(ToolOp::HeatPen) => Operation::Engrave(seed.settings()),
            PassKey::Tool(ToolOp::Punch { diameter_mm }) => {
                let mut s = seed.settings();
                s.punch_diameter_mm = Some(diameter_mm);
                Operation::Punch(s)
            }
            PassKey::Plain(V1Operation::Cut) => Operation::Cut(seed.settings()),
            PassKey::Plain(V1Operation::KissCut) => Operation::KissCut(seed.settings()),
            PassKey::Plain(V1Operation::DieCut) => Operation::DieCut(seed.settings()),
            PassKey::Plain(V1Operation::Draw) => Operation::Draw(seed.settings()),
            PassKey::Plain(V1Operation::Score) => Operation::Score(seed.settings()),
            PassKey::Plain(V1Operation::Engrave) => Operation::Engrave(seed.settings()),
            PassKey::Plain(V1Operation::Perforate) => Operation::Perforate(seed.settings()),
            PassKey::Plain(V1Operation::PrintThenCut) => Operation::PrintThenCut(seed.settings()),
            PassKey::Plain(V1Operation::Guide) => Operation::Guide,
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

/// Migrates v1 layers into `(shapes, layers, next_shape_id)`. Geometry is stored once in
/// `shapes`; each distinct effective pass of a v1 layer becomes one ordered v2 layer whose
/// `members` preserve intra-layer shape order (D-19/D-20). A `template` v1 layer becomes a
/// single `Operation::Guide` layer (D-17).
pub(crate) fn migrate_v1_layers(
    v1_layers: Vec<V1Layer>,
    stored_next_id: u64,
    seed: &OperationDefaults,
) -> Result<(Vec<Shape>, Vec<Layer>, u64), DocError> {
    let mut shapes: Vec<Shape> = Vec::new();
    let mut layers: Vec<Layer> = Vec::new();
    let mut max_id = 0u64;

    for v1_layer in v1_layers {
        let mut push_shape = |shape: &V1Shape, shapes: &mut Vec<Shape>| {
            max_id = max_id.max(shape.id);
            if !shapes.iter().any(|s| s.id == shape.id) {
                shapes.push(v1_shape_to_shape(shape));
            }
        };

        if v1_layer.template {
            let members: Vec<u64> = v1_layer.shapes.iter().map(|s| s.id).collect();
            for shape in &v1_layer.shapes {
                push_shape(shape, &mut shapes);
            }
            layers.push(Layer {
                name: v1_layer.name,
                operation: Some(Operation::Guide),
                preset_id: None,
                preset_linked: false,
                visible: v1_layer.visible,
                locked: v1_layer.locked,
                color: v1_layer.color,
                members,
            });
            continue;
        }

        // Group by effective pass in first-appearance order; Vec + linear lookup because
        // ToolOp is PartialEq but not Hash.
        let mut groups: Vec<(PassKey, Vec<u64>)> = Vec::new();
        for shape in &v1_layer.shapes {
            push_shape(shape, &mut shapes);
            let key = PassKey::of(shape);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, members)) => members.push(shape.id),
                None => groups.push((key, vec![shape.id])),
            }
        }
        for (key, members) in groups {
            layers.push(Layer {
                name: v1_layer.name.clone(),
                operation: Some(key.into_operation(seed)),
                preset_id: None,
                preset_linked: false,
                visible: v1_layer.visible,
                locked: v1_layer.locked,
                color: v1_layer.color.clone(),
                members,
            });
        }
    }

    let next_shape_id = stored_next_id.max(max_id.saturating_add(1));
    Ok((shapes, layers, next_shape_id))
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
    fn two_score_tips_stay_in_separate_score_layers() {
        let mut fine = square(1);
        fine.tool = Some(ToolOp::Score { tip: Tip::Fine, passes: 1 });
        let mut wide = square(2);
        wide.tool = Some(ToolOp::Score { tip: Tip::Wide, passes: 3 });
        let (shapes, layers, next_id) = migrate_v1_layers(vec![layer(vec![fine, wide])], 3, &OperationDefaults::default()).unwrap();
        assert_eq!(shapes.len(), 2);
        assert_eq!(layers.len(), 2);
        assert_eq!(next_id, 3);
        let [a, b] = [&layers[0], &layers[1]];
        assert_eq!(a.members, vec![1]);
        assert_eq!(b.members, vec![2]);
        let (Operation::Score(sa), Operation::Score(sb)) = (&a.operation.clone().unwrap(), &b.operation.clone().unwrap()) else {
            panic!("both layers must be Score passes");
        };
        assert_eq!(sa.tip, Some(Tip::Fine));
        assert_eq!(sa.passes, 1);
        assert_eq!(sb.tip, Some(Tip::Wide));
        assert_eq!(sb.passes, 3);
    }

    #[test]
    fn emboss_and_power_engrave_both_map_to_engrave_but_stay_separate() {
        let mut emboss = square(1);
        emboss.tool = Some(ToolOp::Emboss { tip: Tip::Fine, passes: 2, deboss: true });
        let mut power = square(2);
        power.tool = Some(ToolOp::PowerEngrave);
        let (_, layers, _) = migrate_v1_layers(vec![layer(vec![emboss, power])], 3, &OperationDefaults::default()).unwrap();
        assert_eq!(layers.len(), 2);
        let (Operation::Engrave(a), Operation::Engrave(b)) = (&layers[0].operation.clone().unwrap(), &layers[1].operation.clone().unwrap()) else {
            panic!("both layers must be Engrave passes");
        };
        assert!(a.deboss && a.passes == 2 && a.tip == Some(Tip::Fine));
        assert!(!b.deboss && b.tip.is_none());
    }

    #[test]
    fn template_layer_becomes_one_guide_layer_and_style_fallback_applies() {
        let mut plain = square(1);
        plain.style = CutStyle::Cut;
        let template = V1Layer {
            name: "t".to_string(),
            kind: CutStyle::Cut,
            visible: true,
            shapes: vec![square(2)],
            locked: false,
            template: true,
            color: None,
        };
        let (_, layers, next_id) = migrate_v1_layers(vec![layer(vec![plain]), template], 5, &OperationDefaults::default()).unwrap();
        assert_eq!(next_id, 5);
        assert!(matches!(layers[0].operation, Some(Operation::Cut(_))));
        assert!(matches!(layers[1].operation, Some(Operation::Guide)));
        assert_eq!(layers[1].members, vec![2]);
        assert!(!layers[1].is_output());
    }
}
