//! The Blending Options page's channel names (storytold/photocraft#14858): the Blend If entries take the host's own
//! names when it gives them, the Channels checks keep the channel names, and an empty channel list draws no check box.

use craft_imaging::layer_style::{BlendIf, BlendIfRange, LayerStyle, LayerStyleDraft, LayerStylePage};
use craft_imaging::ui::layer_style::LayerStyleDialog;
use craft_ui::contract::Role;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// The Blending Options page with Blend If entries for `channels` (the host's channel indices), the Channels checks
/// named `checks`, and the Blend If names `blend_if` when given.
fn rig(checks: &[&str], blend_if: Option<&[&str]>, channels: std::ops::RangeInclusive<usize>) -> Fixture {
    let mut style = LayerStyle::default();
    style.blending.channels = vec![true; checks.len()];
    style.blending.blend_if = channels.map(|channel| BlendIf { channel, this_layer: BlendIfRange::FULL, underlying: BlendIfRange::FULL }).collect();
    let mut draft = LayerStyleDraft::new(style);
    draft.page = LayerStylePage::Blending;
    let checks = strings(checks);
    let blend_if = blend_if.map(strings);
    let mut fixture = Fixture::new(move |ui| {
        let mut dialog = LayerStyleDialog::new("Layer Style", &draft, &checks);
        if let Some(names) = blend_if.as_deref() {
            dialog = dialog.blend_if_names(names);
        }
        let _ = dialog.show(ui);
    });
    fixture.room(Vec2::new(900.0, 1400.0)).settle();
    fixture
}

/// The names of the dialog's check boxes.
fn check_boxes(fixture: &Fixture) -> Vec<String> {
    fixture.by_role(Role::CHECK_BOX).iter().map(|target| fixture.tree().name_of(target.node).unwrap_or_default().to_owned()).collect()
}

/// The entry the Blend If select shows and its entries top to bottom, read by opening the list.
fn blend_if_list(fixture: &mut Fixture) -> (String, Vec<String>) {
    let select =
        fixture.by_role(Role::COMBO_BOX).into_iter().find(|t| fixture.tree().name_of(t.node) == Some("Blend If:")).expect("the Blend If select");
    let shown = fixture.tree().value_of(select.node).unwrap_or_default().to_owned();
    fixture.access(AccessActionKind::Focus, &select).idle(2);
    fixture.key(Key::Space, Modifiers::NONE).idle(3);
    let mut options = fixture.by_role(Role::LIST_BOX_OPTION);
    options.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    let names = options.iter().map(|option| fixture.tree().name_of(option.node).unwrap_or_default().to_owned()).collect();
    (shown, names)
}

#[trial]
fn the_hosts_blend_if_names_are_used_as_given() {
    let mut fixture = rig(&["R", "G", "B"], Some(&["Gray", "Red", "Green", "Blue"]), 0..=3);
    assert_eq!(blend_if_list(&mut fixture).1, ["Gray", "Red", "Green", "Blue"]);

    // A Lab host lists its channel entries only, with no Gray put in front, and the page opens on the first.
    let mut fixture = rig(&["L", "A", "B"], Some(&["Lightness", "a", "b"]), 1..=3);
    let (shown, names) = blend_if_list(&mut fixture);
    assert_eq!(names, ["Lightness", "a", "b"]);
    assert_eq!(shown, "Lightness");
}

#[trial]
fn without_blend_if_names_the_entries_take_the_channel_names_after_gray() {
    let mut fixture = rig(&["R", "G", "B"], None, 0..=3);
    let (shown, names) = blend_if_list(&mut fixture);
    assert_eq!(names, ["Gray", "R", "G", "B"]);
    assert_eq!(shown, "Gray");

    let mut fixture = rig(&["Gray"], None, 1..=1);
    assert_eq!(blend_if_list(&mut fixture).1, ["Gray"], "a one-channel document's entry is its channel's name");
}

/// The Channels checks keep the channel names when the Blend If names differ, and an empty channel list (a Grayscale
/// host) draws no Channels check while the Blend If entry stays.
#[trial]
fn an_empty_channel_list_draws_no_channel_check_box() {
    let rgb = check_boxes(&rig(&["R", "G", "B"], Some(&["Gray", "Red", "Green", "Blue"]), 0..=3));
    assert!(["R", "G", "B"].iter().all(|name| rgb.iter().any(|shown| shown == name)), "the Channels checks: {rgb:?}");

    let mut fixture = rig(&[], Some(&["Gray"]), 1..=1);
    let mut gray = check_boxes(&fixture);
    let mut others: Vec<String> = rgb.into_iter().filter(|name| !["R", "G", "B"].contains(&name.as_str())).collect();
    gray.sort();
    others.sort();
    assert_eq!(gray, others, "the same check boxes but the Channels ones");
    assert_eq!(blend_if_list(&mut fixture).1, ["Gray"]);
}
