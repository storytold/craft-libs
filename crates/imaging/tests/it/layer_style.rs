//! `LayerStyleDialog`: the effect list, the Styles page, the effect pages and the Blending Options page, driven through
//! the dialog the way a host shows it.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_imaging::layer_style::{
    EffectField, EffectId, EffectInstance, EffectKind, EffectSpec, FieldKind, FieldUnit, LayerStyle, LayerStyleDraft, LayerStylePage, StyleParams,
    StyleValue,
};
use craft_imaging::ui::layer_style::{LayerStyleDialog, StylePresets};
use craft_ui::contract::Role;
use craft_ui::patterns::presets::PresetRow;
use craft_ui::testing::access::{NodeKey, name};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers, Vec2};

/// The room the dialog is shown in: tall enough for the longest effect page.
const ROOM: Vec2 = Vec2::new(900.0, 1400.0);

/// A host that applies every change and records what the dialog reported.
struct Host {
    draft: LayerStyleDraft,
    specs: Vec<EffectSpec>,
    channels: Vec<String>,
    patterns: Vec<PresetRow<String>>,
    edits: Vec<EditEvent<LayerStyle>>,
    made: Vec<StyleParams>,
}

fn spec(kind: EffectKind) -> EffectSpec {
    EffectSpec { kind, label: Cow::Borrowed(""), fields: Vec::new(), defaults: StyleParams::default() }
}

fn instance(id: u32, kind: EffectKind, on: bool) -> EffectInstance {
    EffectInstance { id: EffectId(id), kind, on, params: StyleParams::default() }
}

fn host(draft: LayerStyleDraft, specs: Vec<EffectSpec>) -> Host {
    Host { draft, specs, channels: ["R", "G", "B"].map(String::from).to_vec(), patterns: Vec::new(), edits: Vec::new(), made: Vec::new() }
}

fn rig_of(host: Host) -> (Fixture, Rc<RefCell<Host>>) {
    let host = Rc::new(RefCell::new(host));
    let shown = host.clone();
    let rows = vec![
        PresetRow::group("g".to_owned(), "Basics", 0),
        PresetRow::item("Soft".to_owned(), "Soft", 1),
        PresetRow::item("Hard".to_owned(), "Hard", 1),
    ];
    let mut fixture = Fixture::new(move |ui| {
        let mut guard = shown.borrow_mut();
        let host = &mut *guard;
        let presets = StylePresets::new(&rows, |cx, rect, _| cx.checker(rect));
        let out = LayerStyleDialog::new("Layer Style", &host.draft, &host.channels)
            .specs(&host.specs)
            .patterns(&host.patterns)
            .style_presets(presets)
            .show(ui);
        out.apply_view(&mut host.draft);
        for event in out.edit {
            if let Some(value) = event.value() {
                host.draft.style = value.clone();
            }
            host.edits.push(event);
        }
        host.made.extend(out.make_default.map(|(_, params)| params));
    });
    fixture.room(ROOM).settle();
    (fixture, host)
}

/// The nodes of `role` named `label`, top to bottom.
fn named(fixture: &Fixture, role: Role, label: &str) -> Vec<Target> {
    let mut found: Vec<Target> = fixture.by_role(role).into_iter().filter(|target| name(fixture.tree(), target.node) == Some(label)).collect();
    found.sort_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
    found
}

fn one(fixture: &Fixture, role: Role, label: &str) -> Target {
    named(fixture, role, label).into_iter().next().unwrap_or_else(|| panic!("a {role:?} named {label}"))
}

/// Gives `target` the focus through its access node's focus action.
fn focus(fixture: &mut Fixture, target: &Target) {
    fixture.access(AccessActionKind::Focus, target);
    fixture.idle(1);
    assert!(fixture.focused_stop().is_some_and(|(id, _)| NodeKey::of(id) == target.node), "{target:?} took the focus");
}

fn size_spec(kind: EffectKind) -> EffectSpec {
    let field = EffectField { key: "size".into(), label: "Size".into(), kind: FieldKind::Slider { min: 0.0, max: 250.0, unit: FieldUnit::Pixels } };
    let mut defaults = StyleParams::default();
    defaults.set("size", StyleValue::Number(5.0));
    EffectSpec { kind, label: Cow::Borrowed(""), fields: vec![field], defaults }
}

/// Typing in Size and then clicking another effect's row commits the typing: the page the click leaves is drawn in the
/// click's pass, so its field sees the focus loss.
#[trial]
fn a_list_click_commits_the_typing_of_the_page_it_leaves() {
    let mut style = LayerStyle::default();
    style.effects.push(instance(1, EffectKind::DropShadow, true));
    style.effects.push(instance(2, EffectKind::Satin, true));
    let mut draft = LayerStyleDraft::new(style);
    draft.page = LayerStylePage::Effect(EffectId(1));
    let (mut fixture, host) = rig_of(host(draft, vec![size_spec(EffectKind::DropShadow), spec(EffectKind::Satin)]));
    let field = one(&fixture, Role::SPIN_BUTTON, "Size");
    fixture.click(&field);
    fixture.type_text("42");
    let row = one(&fixture, Role::LIST_ITEM, "Satin");
    fixture.click(&row);
    fixture.idle(3);
    let typed = host.borrow().draft.style.effect(EffectId(1)).and_then(|e| e.params.number("size"));
    assert_eq!(typed, Some(42.0));
    assert_eq!(host.borrow().draft.page, LayerStylePage::Effect(EffectId(2)), "the chosen page shows once the host applied it");
}

/// A rig showing one effect of the Photoshop tables with its defaults, or `params` over them.
fn page_rig(kind: EffectKind, params: Option<StyleParams>) -> (Fixture, Rc<RefCell<Host>>) {
    let spec = EffectSpec::photoshop(kind);
    let params = params.unwrap_or_else(|| spec.defaults.clone());
    let effects = vec![EffectInstance { id: EffectId(1), kind, on: true, params }];
    let draft = LayerStyleDraft::new(LayerStyle { effects, ..LayerStyle::default() });
    rig_of(host(draft, vec![spec]))
}

/// A page shows the spec's defaults for keys the instance lacks without writing them; editing one field writes only
/// that key.
#[trial]
fn page_shows_defaults_under_the_values_and_writes_only_edits() {
    let (mut fixture, host) = page_rig(EffectKind::DropShadow, Some(StyleParams::default()));
    assert!(host.borrow().edits.is_empty());
    let make = fixture.by_label("Make Default").expect("Make Default");
    fixture.click(&make);
    fixture.idle(2);
    assert_eq!(host.borrow().made.len(), 1);
    let defaults = EffectSpec::photoshop(EffectKind::DropShadow).defaults;
    assert_eq!(host.borrow().made[0].number("opacity"), defaults.number("opacity"));

    let distance = one(&fixture, Role::SLIDER, "Distance");
    focus(&mut fixture, &distance);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    let host = host.borrow();
    let [edit] = host.edits.as_slice() else { panic!("one edit: {}", host.edits.len()) };
    let written = edit.value().and_then(|style| style.effect(EffectId(1))).map(|e| e.params.clone()).unwrap_or_default();
    assert!(written.number("distance") > defaults.number("distance"), "the arrow key steps the distance up from its default");
    assert_eq!(written.number("opacity"), None, "only the edited key is written");
}
