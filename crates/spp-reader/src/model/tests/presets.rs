//! Smart materials and smart masks ([`crate::model::ProjectPreset`]) through the project builder.

use super::doc::*;
use crate::model::*;

#[test]
fn a_smart_material_is_a_folder_of_layers() {
    let red = layer(10, "Red", vec![fill(11, vec![uniform(1, [1.0, 0.0, 0.0])])], vec![], None);
    let root = obj("DataLayersPreset", vec![("layerGroup", folder(1, "Rubber", vec![red])), ("preset_dataversion", V::Int(3))]);
    let (preset, report) = ProjectPreset::from_bytes(&document(53, root)).unwrap();
    let ProjectPreset::SmartMaterial(folder) = preset else { panic!("{preset:?}") };
    assert_eq!(folder.name, "Rubber");
    let LayerKind::Group(children) = &folder.kind else { panic!() };
    assert_eq!((children.len(), children[0].kind.clone()), (1, LayerKind::Fill));
    assert_eq!(report.raw(), 0);
}

#[test]
fn a_smart_mask_is_a_mask_stack_and_other_roots_are_named() {
    let root = obj("DataSmartMask", vec![("actionsStack", stack("DataStackActions", 1, vec![fill(2, vec![uniform(0, [0.5; 3])])]))]);
    let (preset, _) = ProjectPreset::from_bytes(&document(53, root)).unwrap();
    let ProjectPreset::SmartMask(effects) = preset else { panic!("{preset:?}") };
    assert!(matches!(effects.as_slice(), [ProjectEffect::Fill(_)]));
    let (other, _) = ProjectPreset::from_bytes(&document(53, obj("DataSomethingElse", vec![]))).unwrap();
    assert_eq!(other, ProjectPreset::Other("DataSomethingElse".into()));
}
