//! Fields the reader does not handle are kept on the nearest layer, effect or set with their values; fields it does
//! not know at all are reported ([`crate::model::fields`]).

use super::doc::*;
use crate::model::*;

fn build(bytes: &[u8]) -> (ProjectDocument, ModelReport) {
    ProjectDocument::from_bytes(bytes).expect("model")
}

/// `layer`, with extra fields on the layer object.
fn layer_with(uid: u64, content: Vec<V>, extra: Vec<(&'static str, V)>) -> V {
    let V::Obj(class, mut fields) = layer(uid, "L", content, vec![], None) else { panic!("the builder makes an object") };
    fields.extend(extra);
    V::Obj(class, fields)
}

/// `fill`, with extra fields on the effect object.
fn fill_with(uid: u64, sources: Vec<V>, extra: Vec<(&'static str, V)>) -> V {
    let V::Obj(class, mut fields) = fill(uid, sources) else { panic!("the builder makes an object") };
    fields.extend(extra);
    V::Obj(class, fields)
}

fn flagged(list: &[UnhandledSetting]) -> Vec<String> {
    list.iter().filter(|s| s.may_change_pixels()).map(|s| s.reason()).collect()
}

#[test]
fn settings_are_kept_on_the_object_they_belong_to() {
    let source = obj("DataSourceUniform", vec![("channelTypes", V::Uid(1)), ("color", V::Vec3([0.5; 3])), ("opacity", V::Float(1.0))]);
    let effect = fill_with(30, vec![source], vec![("selection", V::Ref(9)), ("tags", V::UInt(4))]);
    let l = layer_with(10, vec![effect], vec![("gammaCompensation", V::Bool(true)), ("colorTag", V::Enum(3))]);
    let (doc, report) = build(&single_set(vec![l]));
    let layer = &doc.texture_sets[0].layers[0];
    assert_eq!(flagged(&layer.unhandled), ["setting DataLayerColor.gammaCompensation = true not handled"]);
    let header = layer.content[0].header().expect("fill");
    assert_eq!(flagged(&header.unhandled), ["setting DataActionFill.selection = ref:set not handled"]);
    assert!(report.unknown_fields.is_empty(), "{:?}", report.unknown_fields);
}

#[test]
fn settings_at_their_default_are_kept_but_not_flagged() {
    let l = layer_with(10, vec![], vec![("gammaCompensation", V::Bool(false))]);
    let (doc, _) = build(&single_set(vec![l]));
    let kept = &doc.texture_sets[0].layers[0].unhandled;
    assert_eq!(kept.len(), 1);
    assert!(kept[0].is_default() && !kept[0].may_change_pixels());
    assert_eq!(kept[0].value, "false");
}

#[test]
fn a_field_the_reader_does_not_know_is_reported_and_flagged() {
    let l = layer_with(10, vec![], vec![("fromAFutureVersion", V::Float(2.0))]);
    let (doc, report) = build(&single_set(vec![l]));
    let kept = &doc.texture_sets[0].layers[0].unhandled;
    assert_eq!(kept[0].kind, SettingKind::Unknown);
    assert!(kept[0].may_change_pixels());
    assert_eq!(report.unknown_fields.get("DataLayerColor.fromAFutureVersion"), Some(&1));
}

#[test]
fn objects_below_an_effect_report_to_the_effect_and_nested_layers_keep_their_own() {
    let source =
        obj("DataSourceUniform", vec![("channelTypes", V::Uid(1)), ("color", V::Vec3([0.5; 3])), ("opacity", V::Float(1.0)), ("newKnob", V::Bool(true))]);
    let inner = layer_with(110, vec![fill(120, vec![source])], vec![]);
    let outer = folder(100, "F", vec![inner]);
    let (doc, report) = build(&single_set(vec![outer]));
    let folder = &doc.texture_sets[0].layers[0];
    assert!(flagged(&folder.unhandled).is_empty());
    let inner = &folder.children()[0];
    assert!(flagged(&inner.unhandled).is_empty());
    let header = inner.content[0].header().expect("fill");
    assert_eq!(flagged(&header.unhandled), ["setting DataSourceUniform.newKnob = true not handled (unknown field)"]);
    assert_eq!(report.unknown_fields.len(), 1);
}

#[test]
fn document_settings_are_on_the_document_and_on_every_set() {
    let bytes = single_set(vec![]);
    // the test documents have no document-level settings; build one by hand from the raw tree
    let mut tree = crate::model::raw::RawTree::parse(&bytes).expect("tree");
    let root = tree.root.as_mut().expect("root");
    root.fields.push(("tangentSpaceMode".into(), crate::model::raw::RawValue::Scalar(crate::decode::document::Scalar::Enum(1))));
    let (doc, _) = ProjectDocument::from_tree(&tree);
    let expected = ["setting DataDocument.tangentSpaceMode = e1 not handled"];
    assert_eq!(flagged(&doc.unhandled), expected);
    assert_eq!(flagged(&doc.texture_sets[0].unhandled), expected);
}
