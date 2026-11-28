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
use craft_imaging::ui::layer_style::LayerStyleDialog;
use craft_ui::contract::Role;
use craft_ui::patterns::presets::{PresetFns, PresetRow, PresetSource};
use craft_ui::testing::access::{NodeKey, is_interactive, name};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Key, Modifiers, Vec2};

/// The room the dialog is shown in: tall enough for the longest effect page.
const ROOM: Vec2 = Vec2::new(900.0, 1400.0);

/// A host that applies every change and records what the dialog reported.
struct Host {
    draft: LayerStyleDraft,
    specs: Vec<EffectSpec>,
    channels: Vec<String>,
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
    Host { draft, specs, channels: ["R", "G", "B"].map(String::from).to_vec(), edits: Vec::new(), made: Vec::new() }
}

fn rig_of(host: Host) -> (Fixture, Rc<RefCell<Host>>) {
    let host = Rc::new(RefCell::new(host));
    let shown = host.clone();
    let rows = |_: &(), _: &_| {
        vec![
            PresetRow::group("g".to_owned(), "Basics", 0),
            PresetRow::item("Soft".to_owned(), "Soft", 1),
            PresetRow::item("Hard".to_owned(), "Hard", 1),
        ]
    };
    let presets = PresetSource::new((), PresetFns::new(rows).thumb(|_, cx, rect, _| cx.checker(rect)));
    let mut fixture = Fixture::new(move |ui| {
        let mut guard = shown.borrow_mut();
        let host = &mut *guard;
        let out = LayerStyleDialog::new("Layer Style", &host.draft, &host.channels).specs(&host.specs).style_presets(&presets).show(ui);
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

#[trial]
fn a_fast_effect_sweep_applies_each_crossed_checkbox_and_ends_on_release() {
    let kinds = [EffectKind::DropShadow, EffectKind::Satin, EffectKind::OuterGlow, EffectKind::InnerGlow];
    let effects = kinds.into_iter().enumerate().map(|(i, kind)| instance(i as u32 + 1, kind, true)).collect();
    let draft = LayerStyleDraft::new(LayerStyle { effects, ..LayerStyle::default() });
    let (mut fixture, host) = rig_of(host(draft, kinds.into_iter().map(spec).collect()));
    let first = one(&fixture, Role::CHECK_BOX, "Drop Shadow");
    let last = one(&fixture, Role::CHECK_BOX, "Inner Glow");
    fixture.press(&first);
    fixture.release(&last);
    fixture.idle(2);
    assert!(host.borrow().draft.style.effects.iter().all(|effect| !effect.on));
}

#[trial]
fn cancelling_an_effect_sweep_restores_checks_and_removes_an_effect_it_added() {
    let effects = vec![instance(1, EffectKind::DropShadow, false), instance(2, EffectKind::OuterGlow, false)];
    let draft = LayerStyleDraft::new(LayerStyle { effects, ..LayerStyle::default() });
    let (mut fixture, host) = rig_of(host(draft, [EffectKind::DropShadow, EffectKind::Satin, EffectKind::OuterGlow].into_iter().map(spec).collect()));
    let first = one(&fixture, Role::CHECK_BOX, "Drop Shadow");
    let last = one(&fixture, Role::CHECK_BOX, "Outer Glow");
    fixture.press(&first);
    fixture.frame(vec![pointer::moved(last.center())]);
    {
        let host = host.borrow();
        assert_eq!(host.draft.style.effects.len(), 3);
        assert!(host.draft.style.effects.iter().all(|effect| effect.on));
    }
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.release(&last);
    fixture.idle(2);
    let host = host.borrow();
    assert_eq!(host.draft.style.effects.len(), 2);
    assert!(host.draft.style.effects.iter().all(|effect| !effect.on));
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

/// A Drop Shadow with its own `size`, so an action that rewrote an effect's values shows up in the tests below.
fn shadow(id: u32, size: f64) -> EffectInstance {
    let mut params = StyleParams::default();
    params.set("size", StyleValue::Number(size));
    EffectInstance { id: EffectId(id), kind: EffectKind::DropShadow, on: true, params }
}

/// A rig over Photoshop's ten tables, with `effects` in the style's order and `page` shown.
fn bar_rig(effects: Vec<EffectInstance>, page: LayerStylePage) -> (Fixture, Rc<RefCell<Host>>) {
    let mut draft = LayerStyleDraft::new(LayerStyle { effects, ..LayerStyle::default() });
    draft.page = page;
    rig_of(host(draft, EffectKind::ALL.into_iter().map(spec).collect()))
}

/// The instance ids of `style`, in the style's order.
fn ids(style: &LayerStyle) -> Vec<u32> {
    style.effects.iter().map(|effect| effect.id.0).collect()
}

/// Every instance of `style` by id, with its kind, its check and its parameters: two styles with the same entries hold
/// the same effects whichever order they are in, so this is what a move must leave untouched.
fn keyed(style: &LayerStyle) -> Vec<(u32, &'static str, bool, StyleParams)> {
    let mut keyed: Vec<(u32, &'static str, bool, StyleParams)> =
        style.effects.iter().map(|effect| (effect.id.0, effect.kind.key(), effect.on, effect.params.clone())).collect();
    keyed.sort_by_key(|entry| entry.0);
    keyed
}

/// The bar under the effect list: the fx button, move up, move down and delete. They stand under the list's rows, inside
/// its column, in that order from the leading edge, and all four are reachable for an effect that can move both ways.
#[trial]
fn the_effect_list_keeps_its_fx_move_and_delete_bar_under_it() {
    let effects = vec![shadow(1, 5.0), shadow(2, 15.0), shadow(3, 25.0)];
    let (fixture, _host) = bar_rig(effects, LayerStylePage::Effect(EffectId(2)));
    let (fx, up) = (one(&fixture, Role::BUTTON, "Add a layer style"), one(&fixture, Role::BUTTON, "Move effect up"));
    let (down, delete) = (one(&fixture, Role::BUTTON, "Move effect down"), one(&fixture, Role::BUTTON, "Delete effect"));
    let list = one(&fixture, Role::LIST, "Effects");
    let lowest = fixture.by_role(Role::LIST_ITEM).into_iter().map(|row| row.rect.bottom()).fold(f32::MIN, f32::max);
    for button in [&fx, &up, &down, &delete] {
        assert!(button.rect.top() >= lowest, "{:?} stands under the list's rows", button.rect);
        assert!(
            button.rect.left() >= list.rect.left() - 1.0 && button.rect.right() <= list.rect.right() + 1.0,
            "{:?} stays in the list's column",
            button.rect
        );
    }
    assert!(
        fx.rect.left() < up.rect.left() && up.rect.left() < down.rect.left() && down.rect.left() < delete.rect.left(),
        "fx, up, down and delete from the leading edge: {:?} {:?} {:?} {:?}",
        fx.rect,
        up.rect,
        down.rect,
        delete.rect
    );
    let tree = fixture.tree();
    assert!(
        [up, down, delete].iter().all(|button| is_interactive(tree, button.node)),
        "the middle one of three effects moves both ways and can be deleted"
    );
}

/// The arrows move the selected effect among the effects of its kind and touch nothing else: the same instances, each
/// with its own kind, check and parameters, in the other order.
#[trial]
fn the_bar_moves_an_effect_up_and_down_among_the_effects_of_its_kind() {
    let effects = vec![shadow(1, 5.0), shadow(2, 15.0), shadow(3, 25.0)];
    let (mut fixture, host) = bar_rig(effects, LayerStylePage::Effect(EffectId(2)));
    let before = host.borrow().draft.style.clone();
    fixture.click(&one(&fixture, Role::BUTTON, "Move effect up"));
    fixture.idle(2);
    {
        let host = host.borrow();
        assert_eq!(ids(&host.draft.style), [2, 1, 3], "the second Drop Shadow took the place of the first");
        assert_eq!(keyed(&host.draft.style), keyed(&before), "every effect kept its own kind, check and parameters");
        assert_eq!(host.draft.page, LayerStylePage::Effect(EffectId(2)), "the moved effect keeps its page");
    }
    fixture.click(&one(&fixture, Role::BUTTON, "Move effect down"));
    fixture.idle(2);
    let host = host.borrow();
    assert_eq!(ids(&host.draft.style), [1, 2, 3], "and it moved back down again");
    assert_eq!(host.draft.style, before, "the style is the one the dialog started with");
}

/// The trash takes the selected effect out of the list; the effects that stay keep their values, and the host is asked
/// for one style edit.
#[trial]
fn the_bar_deletes_the_selected_effect_and_keeps_the_others_as_they_are() {
    let satin = EffectInstance { id: EffectId(3), kind: EffectKind::Satin, on: true, params: StyleParams::default() };
    let effects = vec![shadow(1, 5.0), shadow(2, 15.0), satin];
    let (mut fixture, host) = bar_rig(effects, LayerStylePage::Effect(EffectId(1)));
    fixture.click(&one(&fixture, Role::BUTTON, "Delete effect"));
    fixture.idle(2);
    let host = host.borrow();
    assert_eq!(ids(&host.draft.style), [2, 3], "the selected Drop Shadow is gone from the list");
    let left = host.draft.style.effect(EffectId(2)).map(|effect| (effect.kind.key(), effect.on, effect.params.number("size")));
    assert_eq!(left, Some(("dropShadow", true, Some(15.0))), "the Drop Shadow that stays keeps its values");
    assert_eq!(host.edits.len(), 1, "the delete reaches the host as one style edit");
}

/// An effect with no sibling of its kind moves nowhere: the arrows are not pressable, and a click where they stand leaves
/// the style byte for byte as it was and asks the host for no edit.
#[trial]
fn a_single_effect_greys_the_move_buttons_and_moves_nothing() {
    let (mut fixture, host) = bar_rig(vec![shadow(1, 5.0)], LayerStylePage::Effect(EffectId(1)));
    let before = host.borrow().draft.style.clone();
    let (up, down) = (one(&fixture, Role::BUTTON, "Move effect up"), one(&fixture, Role::BUTTON, "Move effect down"));
    {
        let tree = fixture.tree();
        assert!(!is_interactive(tree, up.node) && !is_interactive(tree, down.node), "one effect of its kind has nowhere to move");
        assert!(is_interactive(tree, one(&fixture, Role::BUTTON, "Delete effect").node), "and the delete still works on it");
    }
    fixture.click_at(up.rect.center());
    fixture.click_at(down.rect.center());
    fixture.idle(2);
    let host = host.borrow();
    assert_eq!(host.draft.style, before, "the style is byte for byte what it was");
    assert!(host.edits.is_empty(), "and the host is asked for no edit");
}

/// With no effect at all there is nothing to move or delete: the three arrows are not pressable, a click on them changes
/// nothing, and the fx button still works.
#[trial]
fn a_dialog_with_no_effect_greys_move_and_delete_and_changes_nothing() {
    let (mut fixture, host) = bar_rig(Vec::new(), LayerStylePage::Blending);
    let before = host.borrow().draft.style.clone();
    {
        let tree = fixture.tree();
        for label in ["Move effect up", "Move effect down", "Delete effect"] {
            assert!(!is_interactive(tree, one(&fixture, Role::BUTTON, label).node), "{label} has no effect to act on");
        }
        assert!(is_interactive(tree, one(&fixture, Role::BUTTON, "Add a layer style").node), "fx still takes a pick");
    }
    for label in ["Move effect up", "Move effect down", "Delete effect"] {
        fixture.click_at(one(&fixture, Role::BUTTON, label).rect.center());
    }
    fixture.idle(2);
    let host = host.borrow();
    assert_eq!(host.draft.style, before, "the style with no effects is unchanged");
    assert!(host.edits.is_empty(), "and nothing is reported to the host");
}

/// The fx button's menu lists every effect the host has: the pick of a kind the layer lacks adds it turned on, and the
/// pick of a kind that takes only one turns on the layer's own effect and shows its page.
#[trial]
fn the_fx_menu_adds_an_effect_and_shows_the_one_the_layer_already_has() {
    let satin = EffectInstance { id: EffectId(2), kind: EffectKind::Satin, on: false, params: StyleParams::default() };
    let (mut fixture, host) = bar_rig(vec![shadow(1, 5.0), satin], LayerStylePage::Effect(EffectId(1)));
    fixture.click(&one(&fixture, Role::BUTTON, "Add a layer style"));
    fixture.idle(2);
    assert_eq!(fixture.by_role(Role::MENU_ITEM).len(), EffectKind::ALL.len(), "the menu lists every effect the host has");
    fixture.click(&one(&fixture, Role::MENU_ITEM, "Inner Glow"));
    fixture.idle(2);
    {
        let host = host.borrow();
        assert_eq!(ids(&host.draft.style), [1, 2, 3], "the picked effect is added after the effects of its kind");
        let added = host.draft.style.effect(EffectId(3)).map(|effect| (effect.kind.key(), effect.on));
        assert_eq!(added, Some(("innerGlow", true)), "the added Inner Glow is turned on");
    }
    fixture.click(&one(&fixture, Role::BUTTON, "Add a layer style"));
    fixture.idle(2);
    fixture.click(&one(&fixture, Role::MENU_ITEM, "Satin"));
    fixture.idle(2);
    let host = host.borrow();
    assert_eq!(ids(&host.draft.style), [1, 2, 3], "Satin takes one, so no second Satin is added");
    assert_eq!(host.draft.style.effect(EffectId(2)).map(|effect| effect.on), Some(true), "the layer's own Satin is turned on");
    assert_eq!(host.draft.page, LayerStylePage::Effect(EffectId(2)), "and its page shows");
}

/// The bar works on the effect the page shows, which is what the app's bar worked on: after the fx menu adds a second
/// Drop Shadow the new one is the page shown, and the trash takes that one out, not the row the list still leads.
#[trial]
fn the_bar_acts_on_the_effect_the_page_shows_after_the_fx_menu_adds_one() {
    let (mut fixture, host) = bar_rig(vec![shadow(1, 5.0)], LayerStylePage::Effect(EffectId(1)));
    fixture.click(&one(&fixture, Role::LIST_ITEM, "Drop Shadow"));
    fixture.idle(2);
    fixture.click(&one(&fixture, Role::BUTTON, "Add a layer style"));
    fixture.idle(2);
    fixture.click(&one(&fixture, Role::MENU_ITEM, "Drop Shadow"));
    fixture.idle(2);
    {
        let host = host.borrow();
        assert_eq!(ids(&host.draft.style), [1, 2], "the menu added a second Drop Shadow");
        assert_eq!(host.draft.page, LayerStylePage::Effect(EffectId(2)), "and the new one is the page shown");
    }
    fixture.click(&one(&fixture, Role::BUTTON, "Delete effect"));
    fixture.idle(2);
    let host = host.borrow();
    assert_eq!(ids(&host.draft.style), [1], "the trash took the effect the page shows, not the one the list leads");
    let left = host.draft.style.effect(EffectId(1)).map(|effect| (effect.kind.key(), effect.on, effect.params.number("size")));
    assert_eq!(left, Some(("dropShadow", true, Some(5.0))), "and the effect that stays keeps its values");
}

/// An effect that is turned off moves among its kind like any other, keeps its off check and its values, and the page
/// rule of a delete holds: the other instance of the kind shows, and with none of its kind left the Styles page does.
#[trial]
fn a_disabled_effect_moves_and_the_page_rule_holds_as_the_effects_of_a_kind_go() {
    let mut off = StyleParams::default();
    off.set("size", StyleValue::Number(15.0));
    let effects = vec![shadow(1, 5.0), EffectInstance { id: EffectId(2), kind: EffectKind::DropShadow, on: false, params: off }];
    let (mut fixture, host) = bar_rig(effects, LayerStylePage::Effect(EffectId(2)));
    fixture.click(&one(&fixture, Role::BUTTON, "Move effect up"));
    fixture.idle(2);
    {
        let host = host.borrow();
        assert_eq!(ids(&host.draft.style), [2, 1], "the off effect moved up among its kind");
        let moved = host.draft.style.effect(EffectId(2)).map(|effect| (effect.on, effect.params.number("size")));
        assert_eq!(moved, Some((false, Some(15.0))), "and it is still off with its own size");
        assert_eq!(host.draft.page, LayerStylePage::Effect(EffectId(2)), "with its page still shown");
    }
    fixture.click(&one(&fixture, Role::BUTTON, "Delete effect"));
    fixture.idle(2);
    {
        let host = host.borrow();
        assert_eq!(ids(&host.draft.style), [1], "the off effect is gone");
        assert_eq!(host.draft.page, LayerStylePage::Effect(EffectId(1)), "and the other instance of its kind shows");
    }
    fixture.click(&one(&fixture, Role::BUTTON, "Delete effect"));
    fixture.idle(2);
    let host = host.borrow();
    assert!(host.draft.style.effects.is_empty(), "the last effect of the kind goes too");
    assert_eq!(host.draft.page, LayerStylePage::Styles, "and the Styles page takes over when its kind has none left");
}
