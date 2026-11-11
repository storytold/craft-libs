//! [`LayerStyleDialog`]: the Layer Style dialog's body.

use std::borrow::Cow;
use std::collections::HashMap;

use craft_theme::{Srgba, TypeRole};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, CommandSet as _, Entry, NoValue, Outcome, Pattern, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Divider, Text};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::patterns::presets::PresetRow;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::ColorRequest;
use egui::{Response, Ui};

use super::blending::{BlendingInput, blending_page};
use super::contour::SharedSampler;
use super::contour::{ContourAction, ContourRequest, ContourWords};
use super::labels::LayerStyleWords;
use super::list::{LayerStyleCommand, ListAction, effect_list};
use super::page::{PageInput, StyleOp, effect_page, push_edit, set_edit};
use super::styles::{StylePresets, styles_page};
use craft_behavior::EditEvents;

use crate::layer_style::{Contour, EffectId, EffectKind, EffectSpec, LayerStyle, LayerStyleDraft, LayerStylePage, StyleParams, StyleValue};

/// What [`LayerStyleDialog::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edit and answers the requests"]
pub struct LayerStyleOutput {
    /// The dialog's response.
    pub response: Response,
    /// The style after this pass's change: a slider drag is a gesture (`Begin`, `Update`…, `Commit`, Esc `Cancel`); a
    /// check, select or an instance added, removed or turned on or off is a `Set`. Only style changes are edits, so a
    /// page switch never reaches the host's history. The host keeps the value and previews it while the draft's
    /// `preview` is on; everything stays inside the dialog's outer gesture, and OK makes the one "Layer Style" history
    /// step.
    pub edit: EditEvents<LayerStyle>,
    /// The page the user chose this pass (a list row, an added or removed instance); the host keeps it in
    /// [`LayerStyleDraft::page`], and the dialog draws it from then on.
    pub page: Option<LayerStylePage>,
    /// The Preview check box changed to this; the host keeps it in [`LayerStyleDraft::preview`].
    pub preview: Option<bool>,
    /// The Blend If entry chosen this pass, an index into `blending.blend_if`; the host keeps it in
    /// [`LayerStyleDraft::blend_if_channel`].
    pub blend_if_channel: Option<usize>,
    /// A colour swatch was clicked: the host opens its colour picker and answers with [`LayerStyleDialog::set_color`].
    pub pick_color: Option<ColorRequest>,
    /// A style preset was picked on the Styles page (its key): the host looks it up and calls
    /// [`LayerStyleDraft::apply_preset`].
    pub apply_style: Option<String>,
    /// New Style… was pressed: the host asks for a name and saves the enabled instances and the blend mode and
    /// opacities as a preset.
    pub new_style: bool,
    /// Make Default on an effect page: the kind and the page's shown values, to keep as the user's default for new
    /// instances of the kind.
    pub make_default: Option<(EffectKind, StyleParams)>,
    /// Reset to Default on an effect page: the host writes the user's default (or the spec's defaults before the first
    /// Make Default) into the instance's parameters.
    pub reset_to_default: Option<EffectId>,
    /// A request for the host's contour library from a contour field.
    pub contour_action: Option<ContourAction>,
    /// A contour field's thumbnail was clicked: the host opens a [`ContourEditorDialog`](super::ContourEditorDialog)
    /// over the request as a child of the Layer Style dialog, previews from its current contour and answers with
    /// [`LayerStyleDialog::set_contour`].
    pub edit_contour: Option<ContourRequest>,
    /// A parameter value the page shows from a default the draft lacks (a Pattern Overlay with no pattern shows the
    /// first one): the instance, the parameter's key and the value. It is not an edit:
    /// [`apply_view`](LayerStyleOutput::apply_view) writes it into the draft without an undo step.
    pub fill_default: Option<(EffectId, String, StyleValue)>,
}

impl Outcome for LayerStyleOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl LayerStyleOutput {
    /// Writes the view state this pass reported (page, Preview, Blend If channel) and the
    /// [`fill_default`](LayerStyleOutput::fill_default) into `draft`. The style edit is the host's, as its history rules
    /// apply it.
    pub fn apply_view(&self, draft: &mut LayerStyleDraft) {
        if let Some(page) = self.page {
            draft.page = page;
        }
        if let Some(preview) = self.preview {
            draft.preview = preview;
        }
        if let Some(channel) = self.blend_if_channel {
            draft.blend_if_channel = channel;
        }
        if let Some((id, key, value)) = &self.fill_default {
            draft.set_param(*id, key, value.clone());
        }
    }
}

/// The Layer Style dialog's body: the effect list on the leading side, the shown page beside it with a vertical divider
/// between. The host owns the frame (a dialog with OK, Cancel and Alt+Cancel Reset), the draft, the engine and the
/// preview.
///
/// - The list ([`LayerStyleCommand`] holds its `+` and `−` keys): Styles, Blending Options, the effects, with muted
///   names for effects that are off and dimmed rows for absent ones.
/// - A page change is reported in [`LayerStyleOutput::page`] and drawn once the host has applied it. The page drawn in
///   the pass of a click is the one the click left, so its fields see their focus loss and commit what was typed
///   before the page changes.
/// - The page title is the effect's name with the Preview check box at its end (it flips
///   [`LayerStyleDraft::preview`]; the Styles page has none, and its title carries New Style…).
/// - The Styles page ([`StylePresets`]), the Blending Options page, or an effect page drawn from the kind's
///   [`EffectSpec`].
/// - Use Global Light: on an effect that uses it, the Angle edits the shared light angle of every such effect
///   ([`LayerStyleDraft::set_param`]).
///
/// ```ignore
/// let out = LayerStyleDialog::new("Layer Style", &draft, &channels).show(ui);
/// out.apply_view(&mut draft);
/// if let Some(value) = out.edit.value() {
///     draft.style = value.clone(); // and re-render the preview when `draft.render_hash()` changed
/// }
/// ```
#[must_use = "show the dialog body with `show`"]
#[derive(Pattern)]
pub struct LayerStyleDialog<'a> {
    label: Label,
    draft: &'a LayerStyleDraft,
    channel_names: &'a [String],
    specs: Option<&'a [EffectSpec]>,
    blend_keys: Option<&'a HashMap<String, String>>,
    blend_modes: &'a [Entry<String>],
    patterns: &'a [PresetRow<String>],
    user_contours: &'a [Contour],
    sampler: Option<SharedSampler>,
    style_presets: Option<StylePresets<'a>>,
}

impl<'a> LayerStyleDialog<'a> {
    /// The body named `label` over `draft`; `channel_names` are the document's colour channels ("R", "G", "B"; "C", "M",
    /// "Y", "K"; "L", "a", "b"; "Gray") for the Channels checks and Blend If.
    pub fn new(label: impl Into<Label>, draft: &'a LayerStyleDraft, channel_names: &'a [String]) -> Self {
        LayerStyleDialog {
            label: label.into(),
            draft,
            channel_names,
            specs: None,
            blend_keys: None,
            blend_modes: &[],
            patterns: &[],
            user_contours: &[],
            sampler: None,
            style_presets: None,
        }
    }

    /// The effect tables in list order; Photoshop's ten ([`EffectSpec::photoshop`] for each of [`EffectKind::ALL`]) by
    /// default. A kind without a spec is not listed.
    pub fn specs(mut self, specs: &'a [EffectSpec]) -> Self {
        self.specs = Some(specs);
        self
    }

    /// Renames the blend defaults of every spec, the built-in ones included, from Photoshop's English names to the
    /// host's blend keys (`"Multiply"` to the host's key). Effects added in the dialog start from the renamed defaults,
    /// so their Blend selects show the host's entry; instances already in the draft are the host's to key.
    pub fn blend_keys(mut self, map: &'a HashMap<String, String>) -> Self {
        self.blend_keys = Some(map);
        self
    }

    /// The blend modes the Blend Mode selects offer (the host's list, with `Entry::separator()` between groups).
    pub fn blend_modes(mut self, modes: &'a [Entry<String>]) -> Self {
        self.blend_modes = modes;
        self
    }

    /// The patterns Pattern Overlay offers, keyed by the pattern's key.
    pub fn patterns(mut self, patterns: &'a [PresetRow<String>]) -> Self {
        self.patterns = patterns;
        self
    }

    /// The host's contour library, after the built-in presets in every contour picker.
    pub fn user_contours(mut self, contours: &'a [Contour]) -> Self {
        self.user_contours = contours;
        self
    }

    /// The host's interpolation of a contour's points: the output of every input level through the points, so the
    /// contour fields draw the curve the engine renders; a monotone cubic by default.
    pub fn sampler(mut self, sample: impl Fn(&[[f32; 2]]) -> Vec<f32> + Send + Sync + 'static) -> Self {
        self.sampler = Some(std::sync::Arc::new(sample));
        self
    }

    /// The Styles page's presets; without them the page shows only New Style….
    pub fn style_presets(mut self, presets: StylePresets<'a>) -> Self {
        self.style_presets = Some(presets);
        self
    }

    /// Draws the list and the page and reports this pass's change. List actions and field edits become the style edit;
    /// the page, Preview and Blend If channel are reported beside it.
    pub fn show(self, ui: &mut Ui) -> LayerStyleOutput {
        let LayerStyleDialog { label, draft, channel_names, specs, blend_keys, blend_modes, patterns, user_contours, sampler, style_presets } = self;
        let env = Env::of(ui);
        let words = env.words::<LayerStyleWords>();
        let contour_words = env.words::<ContourWords>();
        let units = env.units();
        let base: Cow<'_, [EffectSpec]> = match specs {
            Some(specs) => Cow::Borrowed(specs),
            None => Cow::Owned(EffectKind::ALL.into_iter().map(EffectSpec::photoshop).collect()),
        };
        let keyed: Vec<EffectSpec>;
        let specs: &[EffectSpec] = match blend_keys {
            Some(map) => {
                keyed = base.iter().cloned().map(|spec| spec.with_blend_keys(map)).collect();
                &keyed
            }
            None => &base,
        };
        let mut work = Work { next: draft.clone(), edit: EditEvents::new(), extras: Extras::default() };
        let title = page_title(draft.page, draft, &words);
        let name = label.as_str().to_owned();
        let shown = Row::new().label(label.clone()).show(ui, |ui, _| {
            let list = Stack::new().command_scope(LayerStyleCommand::SCOPE).show(ui, |ui, cx| effect_list(ui, cx, draft, specs, &words, &units));
            if let Some(action) = list.inner.flatten() {
                work.list_action(action, specs, draft, &name);
            }
            Divider::vertical().show(ui);
            Stack::new().label(title.clone()).show(ui, |ui, _| {
                let page = PageEnv {
                    draft,
                    specs,
                    blend_modes,
                    patterns,
                    user_contours,
                    sampler: sampler.as_ref(),
                    channel_names,
                    words: &words,
                    contour_words: &contour_words,
                    units: &units,
                };
                work.page(ui, &page, style_presets.as_ref());
            });
        });
        let Work { next, edit, extras } = work;
        LayerStyleOutput {
            response: shown.output,
            edit,
            page: (next.page != draft.page).then_some(next.page),
            preview: (next.preview != draft.preview).then_some(next.preview),
            blend_if_channel: (next.blend_if_channel != draft.blend_if_channel).then_some(next.blend_if_channel),
            pick_color: extras.pick_color,
            apply_style: extras.apply_style,
            new_style: extras.new_style,
            make_default: extras.make_default,
            reset_to_default: extras.reset_to_default,
            contour_action: extras.contour_action,
            edit_contour: extras.edit_contour,
            fill_default: extras.fill_default,
        }
    }

    /// Answers a [`ColorRequest`] the dialog made: sets the colour parameter its key names (`"fx2/color"`; the instance
    /// id, a slash, the parameter key) to `color`. Returns false, changing nothing, for a key that names no instance of
    /// `draft`.
    pub fn set_color(draft: &mut LayerStyleDraft, key: &str, color: Srgba) -> bool {
        Self::set_param_by_key(draft, key, StyleValue::Color(color))
    }

    /// Answers a [`ContourRequest`] the dialog made: sets the contour parameter its key names to `contour` (the editor's
    /// live values and its OK; Cancel writes the editor's initial contour). Returns false, changing nothing, for a key
    /// that names no instance of `draft`.
    pub fn set_contour(draft: &mut LayerStyleDraft, key: &str, contour: Contour) -> bool {
        Self::set_param_by_key(draft, key, StyleValue::Contour(contour))
    }

    fn set_param_by_key(draft: &mut LayerStyleDraft, key: &str, value: StyleValue) -> bool {
        let Some((id, field)) = key.split_once('/') else { return false };
        let Some(id) = EffectId::from_key(id).filter(|id| draft.style.effect(*id).is_some()) else { return false };
        if field.is_empty() {
            return false;
        }
        draft.set_param(id, field, value);
        true
    }
}

/// What the page needs besides the draft's view state.
struct PageEnv<'a> {
    draft: &'a LayerStyleDraft,
    specs: &'a [EffectSpec],
    blend_modes: &'a [Entry<String>],
    patterns: &'a [PresetRow<String>],
    user_contours: &'a [Contour],
    sampler: Option<&'a SharedSampler>,
    channel_names: &'a [String],
    words: &'a LayerStyleWords,
    contour_words: &'a ContourWords,
    units: &'a craft_ui::runtime::env::Units,
}

/// The requests of a pass besides the edit.
#[derive(Default)]
struct Extras {
    pick_color: Option<ColorRequest>,
    apply_style: Option<String>,
    new_style: bool,
    make_default: Option<(EffectKind, StyleParams)>,
    reset_to_default: Option<EffectId>,
    contour_action: Option<ContourAction>,
    edit_contour: Option<ContourRequest>,
    fill_default: Option<(EffectId, String, StyleValue)>,
}

/// The pass's working copy of the draft, for the view state the pass changes, and what the pass reported.
struct Work {
    next: LayerStyleDraft,
    edit: EditEvents<LayerStyle>,
    extras: Extras,
}

impl Work {
    /// Applies a list action to the copy (its view state: the page an added effect opens on) and records its change to
    /// the style as the pass's edit.
    fn list_action(&mut self, action: ListAction, specs: &[EffectSpec], draft: &LayerStyleDraft, label: &str) {
        let op = match action {
            ListAction::Select(page) => {
                self.next.page = page;
                return;
            }
            ListAction::Toggle(id) => {
                self.next.toggle(id);
                StyleOp::Toggle(id)
            }
            ListAction::Add(kind) => {
                let defaults = specs.iter().find(|spec| spec.kind == kind).map(|spec| spec.defaults.clone()).unwrap_or_default();
                self.next.add_instance(kind, &defaults);
                StyleOp::Add { kind, defaults }
            }
            ListAction::Remove(id) => {
                self.next.remove_instance(id);
                StyleOp::Remove(id)
            }
        };
        push_edit(&mut self.edit, set_edit(&draft.style, &op, label));
    }

    /// Draws the page of the pass; its controls add their edits to the pass's.
    fn page(&mut self, ui: &mut Ui, env: &PageEnv<'_>, presets: Option<&StylePresets<'_>>) {
        let (words, draft) = (env.words, env.draft);
        let instance = match draft.page {
            LayerStylePage::Effect(id) => draft.style.effect(id).cloned(),
            _ => None,
        };
        match (draft.page, instance) {
            (LayerStylePage::Blending, _) => {
                self.title_with_preview(ui, words, words.blending_options.clone());
                let input = BlendingInput {
                    style: &draft.style,
                    blend_if_channel: self.next.blend_if_channel,
                    channel_names: env.channel_names,
                    blend_modes: env.blend_modes,
                };
                let out = blending_page(ui, &input);
                push_edit(&mut self.edit, out.edit);
                if let Some(channel) = out.blend_if_channel {
                    self.next.blend_if_channel = channel;
                }
            }
            (LayerStylePage::Effect(id), Some(instance)) => {
                self.title_with_preview(ui, words, words.effect(instance.kind).clone());
                let Some(spec) = env.specs.iter().find(|spec| spec.kind == instance.kind) else { return };
                let input = PageInput {
                    spec,
                    instance: &instance,
                    style: &draft.style,
                    blend_modes: env.blend_modes,
                    patterns: env.patterns,
                    user_contours: env.user_contours,
                    sampler: env.sampler,
                    words,
                    contour_words: env.contour_words,
                    units: env.units,
                };
                let out = effect_page(ui, &input);
                push_edit(&mut self.edit, out.edit);
                self.extras.pick_color = out.pick_color;
                self.extras.contour_action = out.contour_action;
                self.extras.edit_contour = out.edit_contour;
                self.extras.make_default = out.make_default.map(|params| (instance.kind, params));
                self.extras.reset_to_default = out.reset_to_default.then_some(id);
                self.extras.fill_default = out.fill_default.map(|(key, value)| (id, key, value));
            }
            (LayerStylePage::Effect(_), None) => {}
            (LayerStylePage::Styles, _) => {
                let out = styles_page(ui, presets, words);
                self.extras.apply_style = out.apply_style;
                self.extras.new_style = out.new_style;
            }
        }
    }

    /// The page title with the Preview check box at its end; a click flips the copy's `preview`.
    fn title_with_preview(&mut self, ui: &mut Ui, words: &LayerStyleWords, title: Label) {
        let (shown, flipped) = (self.next.preview, &mut self.next.preview);
        title_row(ui, title, |ui| {
            let check = Checkbox::new(Bind::given(shown)).label(words.preview.clone()).id_salt("preview").show(ui);
            if let Some(on) = check.value() {
                *flipped = *on;
            }
        });
    }
}

/// The title of `page` of `draft`, for the page's AccessKit group.
fn page_title(page: LayerStylePage, draft: &LayerStyleDraft, words: &LayerStyleWords) -> Label {
    match page {
        LayerStylePage::Blending => words.blending_options.clone(),
        LayerStylePage::Effect(id) => match draft.style.effect(id) {
            Some(instance) => words.effect(instance.kind).clone(),
            None => words.styles.clone(),
        },
        LayerStylePage::Styles => words.styles.clone(),
    }
}

/// A page title in the title role with `end` at the row's end.
pub(super) fn title_row(ui: &mut Ui, title: Label, mut end: impl FnMut(&mut Ui)) {
    Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
        Text::new(title.clone()).role(TypeRole::Title).show(ui);
        end(ui);
    });
}
