//! Stories of the Layer Style dialog: the dialog on an effect page, the Blending Options page in an RGB and a CMYK
//! document, one story per effect page and the Styles page. Each opens a [`HostedLayerStyle`] on the story's dialog
//! host from a button.

use craft_color_ui::gradient::{ColorStop, GradientStops};
use craft_theme::Srgba;
use craft_ui::contract::Entry;
use craft_ui::dialog::stories::hosted;
use craft_ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut, blend_entries};
use craft_ui::patterns::presets::{PresetFns, PresetRow, PresetSource, ThumbInfo};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui};

use super::{HostedLayerStyle, LayerStyleDialog};
use crate::layer_style::{BlendIf, Blending, EffectId, EffectInstance, EffectKind, EffectSpec, LayerStyle, LayerStyleDraft, LayerStylePage};

craft_ui::words! {
    /// The sample data of the Layer Style stories, English by default.
    #[non_exhaustive]
    pub(crate) struct LayerStyleStoryWords {
        red: "layerStyleStory.red" = "R";
        green: "layerStyleStory.green" = "G";
        blue: "layerStyleStory.blue" = "B";
        cyan: "layerStyleStory.cyan" = "C";
        magenta: "layerStyleStory.magenta" = "M";
        yellow: "layerStyleStory.yellow" = "Y";
        black: "layerStyleStory.black" = "K";
        mode_normal: "layerStyleStory.modeNormal" = "Normal";
        mode_dissolve: "layerStyleStory.modeDissolve" = "Dissolve";
        mode_darken: "layerStyleStory.modeDarken" = "Darken";
        mode_multiply: "layerStyleStory.modeMultiply" = "Multiply";
        mode_lighten: "layerStyleStory.modeLighten" = "Lighten";
        mode_screen: "layerStyleStory.modeScreen" = "Screen";
        mode_overlay: "layerStyleStory.modeOverlay" = "Overlay";
        mode_soft_light: "layerStyleStory.modeSoftLight" = "Soft Light";
        pattern_group: "layerStyleStory.patternGroup" = "Basics";
        pattern_bubbles: "layerStyleStory.patternBubbles" = "Bubbles";
        pattern_wood: "layerStyleStory.patternWood" = "Wood";
        style_group: "layerStyleStory.styleGroup" = "Basics";
        style_soft: "layerStyleStory.styleSoft" = "Soft";
        style_hard: "layerStyleStory.styleHard" = "Hard";
        gradient_sunset: "layerStyleStory.gradientSunset" = "Sunset";
        gradient_sky: "layerStyleStory.gradientSky" = "Sky";
    }
}

/// The blend modes the story host offers; the separators between groups come from the modes' positions.
fn blend_modes(words: &LayerStyleStoryWords) -> Vec<Entry<String>> {
    let modes = [
        (&words.mode_normal, BlendShortcut::Normal),
        (&words.mode_dissolve, BlendShortcut::Dissolve),
        (&words.mode_darken, BlendShortcut::Darken),
        (&words.mode_multiply, BlendShortcut::Multiply),
        (&words.mode_lighten, BlendShortcut::Lighten),
        (&words.mode_screen, BlendShortcut::Screen),
        (&words.mode_overlay, BlendShortcut::Overlay),
        (&words.mode_soft_light, BlendShortcut::SoftLight),
    ];
    // A mode's key is its shown name, as before.
    let keys: Vec<BlendKeyEntry<String>> = modes.iter().map(|(label, shortcut)| BlendKeyEntry::new(label.as_str().to_owned(), *shortcut)).collect();
    blend_entries(&keys, |entry| Label::from(entry.key.clone()))
}

/// Two gradient presets for the Gradient Overlay story's select.
fn gradient_presets(words: &LayerStyleStoryWords) -> Vec<(String, GradientStops)> {
    let stop = |location: f32, hex: &str| ColorStop::new(location, Srgba::from_hex(hex).unwrap_or(Srgba::BLACK));
    vec![
        (words.gradient_sunset.as_str().to_owned(), GradientStops::new(vec![stop(0.0, "#ff7a18"), stop(1.0, "#af002d")])),
        (words.gradient_sky.as_str().to_owned(), GradientStops::new(vec![stop(0.0, "#ffffff"), stop(1.0, "#2a7bde")])),
    ]
}

fn pattern_rows(_: &(), env: &Env) -> Vec<PresetRow<String>> {
    let words = env.words::<LayerStyleStoryWords>();
    vec![
        PresetRow::group("group".to_owned(), words.pattern_group.as_str(), 0),
        PresetRow::item("bubbles".to_owned(), words.pattern_bubbles.as_str(), 1),
        PresetRow::item("wood".to_owned(), words.pattern_wood.as_str(), 1),
    ]
}

fn style_rows(_: &(), env: &Env) -> Vec<PresetRow<String>> {
    let words = env.words::<LayerStyleStoryWords>();
    vec![
        PresetRow::group("group".to_owned(), words.style_group.as_str(), 0),
        PresetRow::item("Soft".to_owned(), words.style_soft.as_str(), 1),
        PresetRow::item("Hard".to_owned(), words.style_hard.as_str(), 1),
    ]
}

/// The Styles page's fake thumbnails.
fn checker(_: &(), cx: &ContentPainterCx<'_>, rect: Rect, _: ThumbInfo<'_, String>) {
    cx.checker(rect);
}

/// A style with one instance of each of `kinds`, turned on, in `kinds` order.
fn style_of(kinds: &[EffectKind]) -> LayerStyle {
    let effects = kinds
        .iter()
        .zip(1..)
        .map(|(kind, id)| EffectInstance { id: EffectId(id), kind: *kind, on: true, params: EffectSpec::photoshop(*kind).defaults })
        .collect();
    LayerStyle { effects, ..LayerStyle::default() }
}

/// The hosted dialog over the story's draft, behind the card's button that opens it; the story answers the Styles page
/// with fake thumbnails.
fn host(ui: &mut Ui, data: &mut StoryData, start: impl FnOnce() -> LayerStyleDraft, channels: &[Label]) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    let names: Vec<String> = channels.iter().map(|name| name.as_str().to_owned()).collect();
    hosted(ui, data, (), || {
        HostedLayerStyle::new("Layer Style", start(), names)
            .blend_modes(blend_modes(&words))
            .patterns(PresetSource::new((), PresetFns::new(pattern_rows)))
            .gradient_presets(gradient_presets(&words))
            .style_presets(PresetSource::new((), PresetFns::new(style_rows).thumb(checker)))
    })
}

fn rgb(words: &LayerStyleStoryWords) -> [Label; 3] {
    [words.red.clone(), words.green.clone(), words.blue.clone()]
}

fn page_of(kinds: &[EffectKind], page: Option<LayerStylePage>) -> LayerStyleDraft {
    let mut draft = LayerStyleDraft::new(style_of(kinds));
    if let Some(page) = page {
        draft.page = page;
    }
    draft
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::DropShadow, EffectKind::Stroke], None), &rgb(&words))
}

/// The Blending Options page of a document with `channels` colour channels: Blend If has a Gray entry before them.
fn blending_draft(channels: usize) -> LayerStyleDraft {
    let ranges = (0..=channels).map(|channel| BlendIf { channel, ..BlendIf::default() }).collect();
    let blending = Blending { channels: vec![true; channels], blend_if: ranges, ..Blending::default() };
    let mut draft = LayerStyleDraft::new(LayerStyle { blending, ..LayerStyle::default() });
    draft.page = LayerStylePage::Blending;
    draft
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_blending_rgb(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || blending_draft(3), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_blending_cmyk(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    let channels = [words.cyan.clone(), words.magenta.clone(), words.yellow.clone(), words.black.clone()];
    host(ui, data, || blending_draft(4), &channels)
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_styles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::DropShadow], Some(LayerStylePage::Styles)), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_bevel_emboss(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::BevelEmboss], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_stroke(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::Stroke], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_inner_shadow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::InnerShadow], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_inner_glow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::InnerGlow], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_satin(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::Satin], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_color_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::ColorOverlay], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_gradient_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::GradientOverlay], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_pattern_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::PatternOverlay], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_outer_glow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::OuterGlow], None), &rgb(&words))
}

#[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
fn layer_style_drop_shadow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    host(ui, data, || page_of(&[EffectKind::DropShadow], None), &rgb(&words))
}
