//! [`HostedLayerStyle`]: the Layer Style dialog on the dialog host, framed with its buttons and Preview box.

use std::collections::HashMap;

use craft_theme::Srgba;
use craft_ui::contract::{Bind, Entry};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::patterns::presets::{PresetRow, ThumbInfo};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::requests::ColorRequest;
use egui::{Rect, Ui};

use super::contour::SharedSampler;
use super::{ContourAction, ContourRequest, LayerStyleDialog, LayerStyleOutput, StylePresets};
use crate::layer_style::{Contour, EffectId, EffectKind, EffectSpec, LayerStyle, LayerStyleDraft, StyleParams};

/// What the open Layer Style dialog asks of the app in a pass; [`HostedLayerStyle::take_asks`] hands them over in the
/// order they came.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LayerStyleAsk {
    /// A colour well was pressed: the app opens its picker and answers with [`HostedLayerStyle::set_color`].
    PickColor(ColorRequest),
    /// A style preset was picked: the app replaces the draft's style with it ([`HostedLayerStyle::draft_mut`]).
    ApplyStyle(String),
    /// New Style… was pressed: the app asks for the name and saves the draft's style.
    NewStyle,
    /// Make Default on an effect page: the app stores the kind's parameters as its defaults.
    MakeDefault(EffectKind, StyleParams),
    /// Reset to Default on an effect page: the app writes its stored defaults into the instance.
    ResetToDefault(EffectId),
    /// A contour library request (load, replace, save, delete).
    Contour(ContourAction),
    /// A contour thumbnail was clicked: the app opens a [`ContourEditorDialog`](super::ContourEditorDialog) as a child
    /// of this dialog and answers with [`HostedLayerStyle::set_contour`].
    EditContour(ContourRequest),
}

impl LayerStyleOutput {
    /// The asks of this pass, in the order [`LayerStyleAsk`] lists them.
    fn asks(self) -> impl Iterator<Item = LayerStyleAsk> {
        let LayerStyleOutput { pick_color, apply_style, new_style, make_default, reset_to_default, contour_action, edit_contour, .. } = self;
        [
            pick_color.map(LayerStyleAsk::PickColor),
            apply_style.map(LayerStyleAsk::ApplyStyle),
            new_style.then_some(LayerStyleAsk::NewStyle),
            make_default.map(|(kind, params)| LayerStyleAsk::MakeDefault(kind, params)),
            reset_to_default.map(LayerStyleAsk::ResetToDefault),
            contour_action.map(LayerStyleAsk::Contour),
            edit_contour.map(LayerStyleAsk::EditContour),
        ]
        .into_iter()
        .flatten()
    }
}

/// Paints a style preset's thumbnail.
type Thumb = dyn Fn(&ContentPainterCx, Rect, ThumbInfo<'_, String>) + Send;

/// The Layer Style dialog the [`DialogHost`](craft_ui::dialog::DialogHost) owns: the [`LayerStyleDialog`] body over a
/// draft the host keeps, in a form dialog with OK and Cancel beside the body (Alt turns Cancel into Reset) and the
/// Preview box under them. Style edits and view changes go into the draft; what the page asks of the app waits in
/// [`take_asks`](Self::take_asks). It answers with the draft's style at OK.
///
/// ```ignore
/// host.open(HostedLayerStyle::new("Layer Style", LayerStyleDraft::new(style), channels).blend_modes(modes));
/// // each pass: preview from host.get::<HostedLayerStyle>().map(HostedLayerStyle::draft);
/// // answer host.get_mut::<HostedLayerStyle>().take_asks();
/// if let Some(DialogResult::Ok(style)) = host.take::<HostedLayerStyle>() { commit(style) }
/// ```
pub struct HostedLayerStyle {
    id: DialogId,
    label: Label,
    open: bool,
    draft: LayerStyleDraft,
    channel_names: Vec<String>,
    specs: Option<Vec<EffectSpec>>,
    blend_keys: Option<HashMap<String, String>>,
    blend_modes: Vec<Entry<String>>,
    patterns: Vec<PresetRow<String>>,
    user_contours: Vec<Contour>,
    sampler: Option<SharedSampler>,
    styles: Option<(Vec<PresetRow<String>>, Box<Thumb>)>,
    asks: Vec<LayerStyleAsk>,
    comparing: bool,
}

/// The key of the hosted Layer Style dialog.
const KEY: &str = "layer.style";

impl HostedLayerStyle {
    /// The dialog titled `label` over `draft`; `channel_names` are the document's colour channels, as
    /// [`LayerStyleDialog::new`] takes them.
    pub fn new(label: impl Into<Label>, draft: LayerStyleDraft, channel_names: Vec<String>) -> Self {
        HostedLayerStyle {
            id: dialog_id(KEY),
            label: label.into(),
            open: true,
            draft,
            channel_names,
            specs: None,
            blend_keys: None,
            blend_modes: Vec::new(),
            patterns: Vec::new(),
            user_contours: Vec::new(),
            sampler: None,
            styles: None,
            asks: Vec::new(),
            comparing: false,
        }
    }

    /// [`LayerStyleDialog::specs`].
    #[must_use]
    pub fn specs(mut self, specs: Vec<EffectSpec>) -> Self {
        self.specs = Some(specs);
        self
    }

    /// [`LayerStyleDialog::blend_keys`].
    #[must_use]
    pub fn blend_keys(mut self, map: HashMap<String, String>) -> Self {
        self.blend_keys = Some(map);
        self
    }

    /// [`LayerStyleDialog::blend_modes`].
    #[must_use]
    pub fn blend_modes(mut self, modes: Vec<Entry<String>>) -> Self {
        self.blend_modes = modes;
        self
    }

    /// [`LayerStyleDialog::patterns`].
    #[must_use]
    pub fn patterns(mut self, patterns: Vec<PresetRow<String>>) -> Self {
        self.patterns = patterns;
        self
    }

    /// [`LayerStyleDialog::user_contours`].
    #[must_use]
    pub fn user_contours(mut self, contours: Vec<Contour>) -> Self {
        self.user_contours = contours;
        self
    }

    /// [`LayerStyleDialog::sampler`].
    #[must_use]
    pub fn sampler(mut self, sample: impl Fn(&[[f32; 2]]) -> Vec<f32> + Send + Sync + 'static) -> Self {
        self.sampler = Some(std::sync::Arc::new(sample));
        self
    }

    /// The Styles page's presets `rows`, each thumbnail painted by `thumb` ([`StylePresets::new`]).
    #[must_use]
    pub fn style_presets(
        mut self,
        rows: Vec<PresetRow<String>>,
        thumb: impl Fn(&ContentPainterCx, Rect, ThumbInfo<'_, String>) + Send + 'static,
    ) -> Self {
        self.styles = Some((rows, Box::new(thumb)));
        self
    }

    /// The draft the dialog shows: the app previews from it.
    pub fn draft(&self) -> &LayerStyleDraft {
        &self.draft
    }

    /// Whether the Preview box is ticked and held: the app shows the layer without the style while it is.
    pub fn is_comparing(&self) -> bool {
        self.comparing
    }

    /// The draft, for the app's answers to [`LayerStyleAsk`]s (an applied style preset, stored defaults).
    pub fn draft_mut(&mut self) -> &mut LayerStyleDraft {
        &mut self.draft
    }

    /// The user's contour library, after the app changed it for a [`LayerStyleAsk::Contour`].
    pub fn set_user_contours(&mut self, contours: Vec<Contour>) {
        self.user_contours = contours;
    }

    /// What the dialog asked of the app since the last call, in order.
    pub fn take_asks(&mut self) -> Vec<LayerStyleAsk> {
        std::mem::take(&mut self.asks)
    }

    /// Answers a [`LayerStyleAsk::PickColor`]: [`LayerStyleDialog::set_color`] on the draft.
    pub fn set_color(&mut self, key: &str, color: Srgba) -> bool {
        LayerStyleDialog::set_color(&mut self.draft, key, color)
    }

    /// Answers a [`LayerStyleAsk::EditContour`]: [`LayerStyleDialog::set_contour`] on the draft.
    pub fn set_contour(&mut self, key: &str, contour: Contour) -> bool {
        LayerStyleDialog::set_contour(&mut self.draft, key, contour)
    }
}

impl StackDialog for HostedLayerStyle {
    type Result = LayerStyle;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<LayerStyle>> {
        let HostedLayerStyle {
            label,
            open,
            draft,
            channel_names,
            specs,
            blend_keys,
            blend_modes,
            patterns,
            user_contours,
            sampler,
            styles,
            asks,
            comparing,
            ..
        } = self;
        // The draft holds Preview: the box edits a copy, written back only when the box changed it, so a Reset that
        // restores the draft keeps the restored Preview.
        let previewing = draft.preview;
        let mut preview = previewing;
        let shown = FormDialog::new(label.clone(), Bind::bound(&mut *draft)).opened(Bind::bound(open)).preview(Some(Bind::bound(&mut preview))).show(
            ui,
            |ui, _, draft| {
                let mut body = LayerStyleDialog::new(label.clone(), draft, channel_names.as_slice())
                    .blend_modes(blend_modes.as_slice())
                    .patterns(patterns.as_slice())
                    .user_contours(user_contours.as_slice())
                    .shared_sampler(sampler.clone());
                if let Some(specs) = specs.as_deref() {
                    body = body.specs(specs);
                }
                if let Some(map) = blend_keys.as_ref() {
                    body = body.blend_keys(map);
                }
                if let Some((rows, thumb)) = styles.as_ref() {
                    body = body.style_presets(StylePresets::new(rows, |cx, rect, info| thumb(cx, rect, info)));
                }
                let out = body.show(ui);
                out.apply_view(draft);
                if let Some(style) = out.edit.value() {
                    draft.style = style.clone();
                }
                asks.extend(out.asks());
                true
            },
        );
        if preview != previewing {
            draft.preview = preview;
        }
        *comparing = shown.comparing;
        let accepted = shown.accepted.map(|draft| DialogResult::Ok(draft.style));
        accepted.or(shown.closed.map(|_| DialogResult::Cancel))
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }
}
