//! Stories of the Layer Style dialog: the dialog on an effect page, the Blending Options page in an RGB and a CMYK
//! document, one story per effect page and the Styles page. The story is the host: it keeps one
//! [`LayerStyleDraft`] per story instance, applies the dialog's edit and view state, and answers the Styles page with
//! fake thumbnails.

use craft_ui::contract::Entry;
use craft_ui::patterns::presets::PresetRow;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{LayerStyleDialog, StylePresets};
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
        mode_multiply: "layerStyleStory.modeMultiply" = "Multiply";
        mode_screen: "layerStyleStory.modeScreen" = "Screen";
        mode_overlay: "layerStyleStory.modeOverlay" = "Overlay";
        mode_soft_light: "layerStyleStory.modeSoftLight" = "Soft Light";
        pattern_group: "layerStyleStory.patternGroup" = "Basics";
        pattern_bubbles: "layerStyleStory.patternBubbles" = "Bubbles";
        pattern_wood: "layerStyleStory.patternWood" = "Wood";
        style_group: "layerStyleStory.styleGroup" = "Basics";
        style_soft: "layerStyleStory.styleSoft" = "Soft";
        style_hard: "layerStyleStory.styleHard" = "Hard";
    }
}

/// The blend modes the story host offers, with a separator between groups.
fn blend_modes(words: &LayerStyleStoryWords) -> Vec<Entry<String>> {
    let entry = |label: &Label| Entry::new(label.as_str().to_owned(), label.clone());
    vec![
        entry(&words.mode_normal),
        entry(&words.mode_dissolve),
        Entry::separator(),
        entry(&words.mode_multiply),
        Entry::separator(),
        entry(&words.mode_screen),
        entry(&words.mode_overlay),
        entry(&words.mode_soft_light),
    ]
}

fn patterns(words: &LayerStyleStoryWords) -> Vec<PresetRow<String>> {
    vec![
        PresetRow::group("group".to_owned(), words.pattern_group.as_str(), 0),
        PresetRow::item("bubbles".to_owned(), words.pattern_bubbles.as_str(), 1),
        PresetRow::item("wood".to_owned(), words.pattern_wood.as_str(), 1),
    ]
}

fn style_rows(words: &LayerStyleStoryWords) -> Vec<PresetRow<String>> {
    vec![
        PresetRow::group("group".to_owned(), words.style_group.as_str(), 0),
        PresetRow::item("Soft".to_owned(), words.style_soft.as_str(), 1),
        PresetRow::item("Hard".to_owned(), words.style_hard.as_str(), 1),
    ]
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

/// The dialog over the story's draft; the host applies the view state and the style edit.
fn host(ui: &mut Ui, data: &mut StoryData, start: impl FnOnce() -> LayerStyleDraft, channels: &[Label]) -> StoryOutput {
    let words = Env::of(ui).words::<LayerStyleStoryWords>();
    let draft = data.value("draft", start());
    let names: Vec<String> = channels.iter().map(|name| name.as_str().to_owned()).collect();
    let (modes, patterns, rows) = (blend_modes(&words), patterns(&words), style_rows(&words));
    let presets = StylePresets::new(&rows, |cx, rect, _| cx.checker(rect));
    let out = LayerStyleDialog::new("Layer Style", draft, &names).blend_modes(&modes).patterns(&patterns).style_presets(presets).show(ui);
    out.apply_view(draft);
    if let Some(value) = out.edit.value() {
        draft.style = value.clone();
    }
    out.into()
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

/// One story per effect kind: the dialog on that effect's page.
macro_rules! effect_stories {
    ($($name:ident => $kind:ident),* $(,)?) => {
        $(
            #[story(LayerStyleDialog<'static>, covers = [], drive = Drive::Never)]
            fn $name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
                let words = Env::of(ui).words::<LayerStyleStoryWords>();
                host(ui, data, || page_of(&[EffectKind::$kind], None), &rgb(&words))
            }
        )*
    };
}

effect_stories! {
    layer_style_bevel_emboss => BevelEmboss,
    layer_style_stroke => Stroke,
    layer_style_inner_shadow => InnerShadow,
    layer_style_inner_glow => InnerGlow,
    layer_style_satin => Satin,
    layer_style_color_overlay => ColorOverlay,
    layer_style_gradient_overlay => GradientOverlay,
    layer_style_pattern_overlay => PatternOverlay,
    layer_style_outer_glow => OuterGlow,
    layer_style_drop_shadow => DropShadow,
}
