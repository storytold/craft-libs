//! [`HostedLayerStyle`]: the Layer Style dialog on the dialog host, framed with its buttons and Preview box.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use craft_behavior::{Metrics, Sense};
use craft_color_ui::gradient::GradientStops;
use craft_theme::Srgba;
use craft_ui::choice::SwatchMetrics;
use craft_ui::contract::{
    Bind, CanvasGesture, Common, Component, Cx, DisabledLook, Entry, MeasureCx, PaintUi, Props as _, Role, Sampler as _, Show as _,
};
use craft_ui::dialog::{ButtonPlace, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::patterns::presets::PresetSource;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Context, Rect, Response, Ui, Vec2};

use super::contour::ContourSampler;
use super::labels::LayerStyleWords;
use super::{ContourAction, ContourRequest, LayerStyleDialog, LayerStyleOutput};
use crate::layer_style::{
    Contour, EffectId, EffectKind, EffectSpec, FieldKind, LayerStyle, LayerStyleDraft, LayerStylePage, StyleParams, StyleValue,
};

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
        let LayerStyleOutput { pick_color, apply_style, make_default, reset_to_default, contour_action, edit_contour, .. } = self;
        [
            pick_color.map(LayerStyleAsk::PickColor),
            apply_style.map(LayerStyleAsk::ApplyStyle),
            make_default.map(|(kind, params)| LayerStyleAsk::MakeDefault(kind, params)),
            reset_to_default.map(LayerStyleAsk::ResetToDefault),
            contour_action.map(LayerStyleAsk::Contour),
            edit_contour.map(LayerStyleAsk::EditContour),
        ]
        .into_iter()
        .flatten()
    }
}

/// Paints the preview swatch beside the footer buttons from the draft.
type PreviewPaint = dyn Fn(&ContentPainterCx, Rect, &LayerStyleDraft) + Send + Sync;

/// The style of the preview swatch: a square as tall as a colour swatch.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PreviewSwatchStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side of the square.
    side: f32,
}

/// The preview swatch: the host's painting of the draft's style, in a square. It shows a picture and takes no input.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter, sample = PreviewSwatch::sample)]
struct PreviewSwatch<'a> {
    #[common]
    common: Common,
    paint: &'a PreviewPaint,
    draft: &'a LayerStyleDraft,
}

impl<'a> PreviewSwatch<'a> {
    /// A swatch named `label` that `paint` fills from `draft`.
    fn new(label: impl Into<Label>, paint: &'a PreviewPaint, draft: &'a LayerStyleDraft) -> Self {
        PreviewSwatch { common: Common::new(), paint, draft }.label(label)
    }
}

impl PreviewSwatch<'static> {
    /// A swatch of the default style, for the suite.
    fn sample() -> Self {
        static DRAFT: LazyLock<LayerStyleDraft> = LazyLock::new(LayerStyleDraft::default);
        static PAINT: LazyLock<Box<PreviewPaint>> = LazyLock::new(|| Box::new(|cx, rect, _| cx.checker(rect)));
        PreviewSwatch::new("Preview", &**PAINT, &DRAFT)
    }
}

impl Component for PreviewSwatch<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = PreviewSwatchStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: PreviewSwatchStyleKey) -> PreviewSwatchStyle {
        PreviewSwatchStyle { metrics: Metrics::new(), side: SwatchMetrics::of(cx).side }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        (self.paint)(&ui.content_painter(cx), cx.rect, self.draft);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

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
    patterns: Option<PresetSource<'static, String>>,
    user_contours: Vec<Contour>,
    sampler: Option<ContourSampler>,
    styles: Option<PresetSource<'static, String>>,
    gradient_presets: Vec<(String, GradientStops)>,
    canvas: Option<egui::Id>,
    preview_thumb: Option<Arc<PreviewPaint>>,
    asks: Vec<LayerStyleAsk>,
    comparing: bool,
    #[cfg(feature = "screen-testing")]
    facts: craft_ui::testing::StoryOutput,
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
            patterns: None,
            user_contours: Vec::new(),
            sampler: None,
            styles: None,
            gradient_presets: Vec::new(),
            canvas: None,
            preview_thumb: None,
            asks: Vec::new(),
            comparing: false,
            #[cfg(feature = "screen-testing")]
            facts: craft_ui::testing::StoryOutput::default(),
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
    pub fn patterns(mut self, patterns: PresetSource<'static, String>) -> Self {
        self.patterns = Some(patterns);
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
    pub fn sampler(mut self, sample: ContourSampler) -> Self {
        self.sampler = Some(sample);
        self
    }

    /// [`LayerStyleDialog::style_presets`].
    #[must_use]
    pub fn style_presets(mut self, presets: PresetSource<'static, String>) -> Self {
        self.styles = Some(presets);
        self
    }

    /// The canvas the dialog's offset drag works on: while the dialog is modal its pointer presses keep their canvas.
    #[must_use]
    pub fn canvas(mut self, canvas: egui::Id) -> Self {
        self.canvas = Some(canvas);
        self
    }

    /// [`LayerStyleDialog::gradient_presets`].
    #[must_use]
    pub fn gradient_presets(mut self, presets: Vec<(String, GradientStops)>) -> Self {
        self.gradient_presets = presets;
        self
    }

    /// The painter of the preview swatch: `paint` draws the draft's style into the swatch rect. Without one the footer
    /// has no swatch.
    #[must_use]
    pub fn preview_thumb(mut self, paint: impl Fn(&ContentPainterCx, Rect, &LayerStyleDraft) + Send + Sync + 'static) -> Self {
        self.preview_thumb = Some(Arc::new(paint));
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

    /// The values new effects of `kind` start from, after the app stored them for a [`LayerStyleAsk::MakeDefault`]: the
    /// open dialog uses them at once, as later openings do. Without [`specs`](Self::specs) from the app the dialog's
    /// own (Photoshop's) specs take them.
    pub fn set_defaults(&mut self, kind: EffectKind, defaults: StyleParams) {
        let specs = self.specs.get_or_insert_with(|| EffectKind::ALL.into_iter().map(EffectSpec::photoshop).collect());
        match specs.iter_mut().find(|spec| spec.kind == kind) {
            Some(spec) => spec.defaults = defaults,
            None => specs.push(EffectSpec { defaults, ..EffectSpec::photoshop(kind) }),
        }
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
            gradient_presets,
            canvas,
            preview_thumb,
            asks,
            comparing,
            #[cfg(feature = "screen-testing")]
            facts,
            ..
        } = self;
        let new_style = Env::of(ui).words::<LayerStyleWords>().new_style.clone();
        // The footer paints beside the form's `&mut` draft, so it reads a copy taken at the start of the pass.
        let snapshot = preview_thumb.as_ref().map(|_| draft.clone());
        let swatch_label = label.clone();
        // The draft holds Preview: the box edits a copy, written back only when the box changed it, so a Reset that
        // restores the draft keeps the restored Preview.
        let previewing = draft.preview;
        let mut preview = previewing;
        let shown = FormDialog::new(label.clone(), Bind::bound(&mut *draft))
            .opened(Bind::bound(open))
            .preview(Some(Bind::bound(&mut preview)))
            .button_place(ButtonPlace::Side)
            .extra_buttons([FooterButton::new(new_style, DialogRole::Other)])
            .footer_extra(|ui| {
                if let (Some(paint), Some(draft)) = (preview_thumb.as_deref(), snapshot.as_ref()) {
                    PreviewSwatch::new(swatch_label.clone(), paint, draft).show(ui);
                }
            })
            .show(ui, |ui, cx, draft| {
                if let Some(canvas) = *canvas {
                    cx.keep_view_open(canvas);
                    if draft.drags_on_canvas() {
                        cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
                    }
                }
                let mut body = LayerStyleDialog::new(label.clone(), draft, channel_names.as_slice())
                    .blend_modes(blend_modes.as_slice())
                    .user_contours(user_contours.as_slice())
                    .gradient_presets(gradient_presets.as_slice());
                if let Some(patterns) = patterns.as_ref() {
                    body = body.patterns(patterns);
                }
                if let Some(sampler) = *sampler {
                    body = body.sampler(sampler);
                }
                if let Some(specs) = specs.as_deref() {
                    body = body.specs(specs);
                }
                if let Some(map) = blend_keys.as_ref() {
                    body = body.blend_keys(map);
                }
                if let Some(presets) = styles.as_ref() {
                    body = body.style_presets(presets);
                }
                let out = body.show(ui);
                #[cfg(feature = "screen-testing")]
                {
                    *facts = craft_ui::contract::Outcome::story_facts(&out);
                }
                out.apply_view(draft);
                if let Some(style) = out.edit.value() {
                    draft.style = style.clone();
                }
                asks.extend(out.asks());
                true
            });
        // `picked` is this pass's own press: a second pass in the same frame sees none and queues nothing.
        if shown.picked == Some(2) {
            asks.push(LayerStyleAsk::NewStyle);
        }
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

    fn confirm(&mut self, _: &Context) -> Option<LayerStyle> {
        Some(self.draft.style.clone())
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        let page = match self.draft.page {
            LayerStylePage::Styles => "styles".to_owned(),
            LayerStylePage::Blending => "blendingOptions".to_owned(),
            LayerStylePage::Effect(id) => id.key(),
        };
        let effects: Vec<_> =
            self.draft.style.effects.iter().map(|e| serde_json::json!({ "id": e.id.key(), "kind": e.kind.key(), "on": e.on })).collect();
        Some(serde_json::json!({ "page": page, "preview": self.draft.preview, "effects": effects }))
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let rejected = || Err(FieldRefusal::Rejects(&value).message(field));
        match field {
            "page" => {
                let Some(name) = value.as_str() else { return rejected() };
                match name {
                    "styles" => self.draft.page = LayerStylePage::Styles,
                    "blendingOptions" => self.draft.page = LayerStylePage::Blending,
                    _ => {
                        if let Some(id) = EffectId::from_key(name).filter(|id| self.draft.style.effect(*id).is_some()) {
                            self.draft.page = LayerStylePage::Effect(id);
                        } else if let Some(kind) = EffectKind::from_key(name) {
                            self.draft.open_on(kind);
                        } else {
                            return rejected();
                        }
                    }
                }
                Ok(())
            }
            "preview" => {
                let Some(on) = value.as_bool() else { return rejected() };
                self.draft.preview = on;
                Ok(())
            }
            "editContour" => {
                let id = value.get("effect").and_then(serde_json::Value::as_str).and_then(EffectId::from_key);
                let key = value.get("key").and_then(serde_json::Value::as_str);
                let (Some(id), Some(key)) = (id, key) else { return rejected() };
                let Some(instance) = self.draft.style.effect(id) else { return rejected() };
                let owned;
                let spec = match self.specs.as_ref().and_then(|specs| specs.iter().find(|spec| spec.kind == instance.kind)) {
                    Some(spec) => spec,
                    None => {
                        owned = EffectSpec::photoshop(instance.kind);
                        &owned
                    }
                };
                if !spec.field(key).is_some_and(|f| matches!(f.kind, FieldKind::Contour { .. })) {
                    return rejected();
                }
                let shown = instance.params.over(&spec.defaults);
                let contour = match shown.get(key) {
                    Some(StyleValue::Contour(contour)) => contour.clone(),
                    _ => Contour::linear(),
                };
                self.asks.push(LayerStyleAsk::EditContour(ContourRequest::new(format!("{}/{key}", id.key()), contour)));
                Ok(())
            }
            "effects" => Err(FieldRefusal::ReadOnly.message(field)),
            _ => Err(FieldRefusal::Unknown.message(field)),
        }
    }

    fn cancelled(&mut self, _: &Context) {
        self.draft.end_drag();
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.facts.clone()
    }
}
