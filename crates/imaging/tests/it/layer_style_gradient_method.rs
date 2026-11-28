//! The Gradient Overlay page's Method list (storytold/photocraft#14903): the methods in the reference order, and the
//! Custom entry only for a value that names none of them, such as an imported overlay without a method.

use craft_imaging::layer_style::{EffectId, EffectInstance, EffectKind, EffectSpec, LayerStyle, LayerStyleDraft, StyleValue};
use craft_imaging::ui::layer_style::LayerStyleDialog;
use craft_ui::contract::Role;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};

/// A Gradient Overlay page over the spec's defaults, with `method` set when given.
fn rig(method: Option<&str>) -> Fixture {
    let spec = EffectSpec::photoshop(EffectKind::GradientOverlay);
    let mut params = spec.defaults.clone();
    if let Some(method) = method {
        params.set("method", StyleValue::Text(method.to_owned()));
    }
    let effects = vec![EffectInstance { id: EffectId(1), kind: EffectKind::GradientOverlay, on: true, params }];
    let draft = LayerStyleDraft::new(LayerStyle { effects, ..LayerStyle::default() });
    let specs = vec![spec];
    let channels: Vec<String> = ["R", "G", "B"].map(String::from).to_vec();
    let mut fixture = Fixture::new(move |ui| {
        let _ = LayerStyleDialog::new("Layer Style", &draft, &channels).specs(&specs).show(ui);
    });
    fixture.room(Vec2::new(900.0, 1400.0)).settle();
    fixture
}

/// The value the Method field shows and its list's entries top to bottom, read by opening the list.
fn method_list(fixture: &mut Fixture) -> (String, Vec<String>) {
    let field = fixture.by_role(Role::COMBO_BOX).into_iter().find(|t| fixture.tree().name_of(t.node) == Some("Method")).expect("the Method field");
    let shown = fixture.tree().value_of(field.node).unwrap_or_default().to_owned();
    fixture.access(AccessActionKind::Focus, &field).idle(2);
    fixture.key(Key::Space, Modifiers::NONE).idle(3);
    let mut options = fixture.by_role(Role::LIST_BOX_OPTION);
    options.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    let names = options.iter().map(|option| fixture.tree().name_of(option.node).unwrap_or_default().to_owned()).collect();
    (shown, names)
}

#[trial]
fn a_new_overlay_lists_the_methods_in_the_reference_order_without_custom() {
    let mut fixture = rig(None);
    let (shown, names) = method_list(&mut fixture);
    assert_eq!(names, ["Perceptual", "Linear", "Classic", "Smooth"]);
    assert!(names.contains(&shown), "a new overlay shows one of the methods, not {shown:?}");
}

#[trial]
fn a_known_method_lists_no_custom_entry() {
    let mut fixture = rig(Some("classic"));
    let (shown, names) = method_list(&mut fixture);
    assert_eq!(names, ["Perceptual", "Linear", "Classic", "Smooth"]);
    assert_eq!(shown, "Classic");
}

/// An overlay whose value names no method (imported without one) keeps Custom, first, so the gradient stays as it is
/// until a method is picked.
#[trial]
fn an_unknown_method_shows_custom_first() {
    for method in ["custom", "", "unknown"] {
        let mut fixture = rig(Some(method));
        let (shown, names) = method_list(&mut fixture);
        assert_eq!(names, ["Custom", "Perceptual", "Linear", "Classic", "Smooth"], "method {method:?}");
        assert_eq!(shown, "Custom", "method {method:?}");
    }
}
