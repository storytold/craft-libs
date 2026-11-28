//! Copy / paste, presets at an amount, section reset and settings from outside the app: the settings the stored
//! JSON leaves out at their defaults, and the sliders of list entries, behave like every other setting.
#![allow(clippy::unwrap_used)]

use craft_develop_model::{DevelopSettings, Mask, PointColor, Preset, Section, SegMask, SettingsGroup, Upright, WbMode, extract_groups};
use craft_geom::Point;
use serde_json::json;

fn preset(settings: serde_json::Value) -> Preset {
    Preset { id: "t".into(), name: "t".into(), group: "g".into(), settings, favorite: false, builtin: false }
}

/// The camera's local tone and an HDR edit are left out of the stored JSON when off. Copying Profile and Light from
/// a photo without them turns them off on the photo pasted onto.
#[test]
fn copying_a_group_carries_what_the_stored_json_leaves_out() {
    let copied = extract_groups(&DevelopSettings::default(), &[SettingsGroup::Profile, SettingsGroup::Light]);
    let mut target = DevelopSettings::default();
    target.profile.camera_local_tone = true;
    target.hdr.enabled = true;
    target.hdr.sdr_brightness = 40.0;
    let pasted = target.merged(&copied).unwrap();
    assert!(!pasted.profile.camera_local_tone);
    assert_eq!(pasted.hdr, DevelopSettings::default().hdr);
    // the stored JSON itself is unchanged: settings saved before these existed keep their hash
    assert!(DevelopSettings::default().to_json().get("hdr").is_none());
}

/// Lens Blur's own left-out settings are stated in a copy too.
#[test]
fn copying_lens_blur_clears_the_target_s_boost_and_focus_mask() {
    let copied = extract_groups(&DevelopSettings::default(), &[SettingsGroup::LensBlur]);
    let mut target = DevelopSettings::default();
    target.lens_blur.boost = 30.0;
    target.lens_blur.focus_mask = Some(2);
    let pasted = target.merged(&copied).unwrap();
    assert_eq!((pasted.lens_blur.boost, pasted.lens_blur.focus_mask), (0.0, None));
}

/// A preset's HDR value at half amount is halfway from the original's, though the original's JSON has no `hdr`.
#[test]
fn a_preset_at_half_amount_starts_from_a_default_the_json_leaves_out() {
    let d = DevelopSettings::default();
    let half = preset(json!({"hdr": {"sdr_brightness": 40.0}})).apply_on(&d, &d, 0.5);
    assert_eq!(half.hdr.sdr_brightness, 20.0);
}

/// Settings from a sidecar are kept in range, the sliders of point colours and red eyes too.
#[test]
fn untrusted_settings_keep_list_sliders_in_range() {
    let s = DevelopSettings::from_untrusted_json(&json!({"point_colors": [{"hue_shift": 250.0}]})).unwrap();
    assert_eq!(s.point_colors[0].hue_shift, 100.0);
}

/// As shot, the white balance numbers are the photo's own. A chosen white balance is an edit.
#[test]
fn a_chosen_white_balance_is_an_edit() {
    let mut s = DevelopSettings::default();
    s.wb.temp = 5200.0;
    assert!(s.is_unedited(), "as shot: the numbers are the photo's");
    s.wb.mode = WbMode::Daylight;
    assert!(!s.is_unedited());
}

#[test]
fn resetting_point_color_resets_each_sample_s_sliders() {
    let mut s = DevelopSettings::default();
    s.point_colors.push(PointColor { hue_shift: 40.0, ..Default::default() });
    s.reset_section(Section::PointColor);
    assert_eq!(s.point_colors[0].hue_shift, 0.0);
}

#[test]
fn resetting_geometry_and_optics_resets_what_is_not_a_slider() {
    let mut s = DevelopSettings::default();
    s.geometry.upright = Upright::Guided;
    s.geometry.guides.push((Point::new(0.1, 0.1), Point::new(0.1, 0.9)));
    s.optics.remove_ca = true;
    s.optics.lens_profile = true;
    s.reset_section(Section::Geometry);
    s.reset_section(Section::Optics);
    assert!(s.geometry.is_identity() && s.geometry.guides.is_empty());
    assert!(!s.optics.remove_ca && !s.optics.lens_profile);
}

/// At no amount a preset adds no mask, so applying it again at an amount still adds its masks.
#[test]
fn a_preset_at_no_amount_adds_no_mask() {
    let masks = serde_json::to_value([Mask { id: 1, name: "Sky".into(), ..Default::default() }]).unwrap();
    let p = preset(json!({"masks": masks}));
    let none = p.apply(&DevelopSettings::default(), 0.0);
    assert!(none.masks.is_empty());
    let full = p.apply(&none, 1.0);
    assert_eq!(full.masks.len(), 1);
    assert_eq!(full.masks[0].adjust.amount, Mask::default().adjust.amount);
}

/// A preset's Lens Blur focus mask follows its mask to the id the photo gives it.
#[test]
fn a_preset_s_focus_mask_follows_its_mask_s_new_id() {
    let masks = serde_json::to_value([Mask { id: 2, name: "Subject".into(), ..Default::default() }]).unwrap();
    let p = preset(json!({"masks": masks, "lens_blur": {"amount": 50.0, "focus_mask": 2}}));
    let mut mine = DevelopSettings::default();
    for id in 1..=3 {
        mine.masks.push(Mask { id, name: format!("Mine {id}"), ..Default::default() });
    }
    let out = p.apply(&mine, 1.0);
    let subject = out.masks.iter().find(|m| m.name == "Subject").unwrap();
    assert_eq!(subject.id, 4);
    assert_eq!(out.lens_blur.focus_mask, Some(4), "not the photo's own mask 2");
    // applied again, the mask is already there and the focus mask still names it
    assert_eq!(p.apply(&out, 1.0).lens_blur.focus_mask, Some(4));
}

/// A side whose square is past `usize` takes the logits there are.
#[test]
fn a_mask_grid_of_an_absurd_side_does_not_overflow() {
    let m = SegMask::from_logits(usize::MAX, &[0.0, 1.0]);
    assert!(m.logits().is_none(), "and reads as damaged");
}
