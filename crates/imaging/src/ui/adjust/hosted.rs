//! [`HostedAdjustDialog`]: an Image › Adjustments dialog on the dialog host, around any adjustment editor.

use craft_behavior::{GestureId, Value};
use craft_ui::contract::{Bind, Sampler as _};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id, serde_fields, set_serde_field};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::memory::SaveKey;
use egui::{Context, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{AdjustContext, LevelsClip, TonePicker};

/// The phase of the editor's edit this pass, mirroring [`craft_behavior::EditEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditPhase {
    /// A gesture began.
    Begin,
    /// A proposed value of the running gesture.
    Update,
    /// The gesture ended with its value.
    Commit,
    /// The gesture was abandoned.
    Cancel,
}

/// What an adjustment editor shares with its dialog for one pass: the context, the canvas, the dialog's own picker,
/// channel and clipping state, and the editor's edit phase.
pub struct AdjustDialogCx<'a> {
    /// Where the editor is shown.
    pub context: AdjustContext,
    /// The canvas the dialog's pickers work on.
    pub canvas: Option<egui::Id>,
    /// The tone picker armed, owned by the dialog for its life.
    pub picker: &'a mut Option<TonePicker>,
    /// The Channel select's shown entry, the one store of Curves and Levels.
    pub channel: &'a mut usize,
    /// The curve point to select.
    pub select_point: Option<usize>,
    /// The Levels clipping display.
    pub clipping: &'a mut Option<LevelsClip>,
    /// The editor's edit phase and its gesture this pass.
    pub edit: Option<(EditPhase, GestureId)>,
}

/// The clipping display as automation names it: `"shadows"`, `"highlights"` or null (`LevelsClip` has no serde).
fn clipping_json(clip: Option<LevelsClip>) -> serde_json::Value {
    match clip {
        Some(LevelsClip::Shadows) => "shadows".into(),
        Some(LevelsClip::Highlights) => "highlights".into(),
        None => serde_json::Value::Null,
    }
}

/// The clipping display `json` names; the outer `None` when it names none of the three.
fn clipping_from_json(json: &serde_json::Value) -> Option<Option<LevelsClip>> {
    match json {
        serde_json::Value::Null => Some(None),
        serde_json::Value::String(name) if name == "shadows" => Some(Some(LevelsClip::Shadows)),
        serde_json::Value::String(name) if name == "highlights" => Some(Some(LevelsClip::Highlights)),
        _ => None,
    }
}

/// Shows the editor over the dialog's value for one pass; `Some` carries what the editor asks of the app this pass.
type Editor<V, E> = dyn for<'b> Fn(&mut Ui, &mut V, &mut AdjustDialogCx<'b>) -> Option<E> + Send;

/// An Image › Adjustments dialog the [`DialogHost`](craft_ui::dialog::DialogHost) owns: an adjustment editor over a
/// value `V` the host keeps, in a form dialog with OK and Cancel beside the editor (Alt turns Cancel into Reset, which
/// restores the value the dialog opened with) and the Preview box under them. The editor runs in its dialog form
/// ([`AdjustContext::dialog`]). What the editor asks of the app (Levels' Auto, Black & White's Auto) is an `E` that
/// waits in [`take_asks`](Self::take_asks). It answers with the value at OK.
///
/// ```ignore
/// host.open(HostedAdjustDialog::new("image.adjustments.brightnessContrast", "Brightness/Contrast", value, |ui, value, cx| {
///     BrightnessContrastEditor::new("Brightness/Contrast", Bind::bound(value)).context(cx.context).show(ui);
///     None::<()>
/// }));
/// // each pass: preview from host.get::<HostedAdjustDialog<BrightnessContrast>>() when its `preview()` is on.
/// ```
pub struct HostedAdjustDialog<V, E = ()> {
    id: DialogId,
    key: String,
    title: Label,
    open: bool,
    value: V,
    preview: bool,
    comparing: bool,
    context: AdjustContext,
    resizable: bool,
    remember: Option<SaveKey>,
    canvas: Option<egui::Id>,
    reset_with: Option<Box<dyn Fn(&V, &V) -> V + Send>>,
    picker: Option<TonePicker>,
    channel: usize,
    clipping: Option<LevelsClip>,
    select_point: Option<usize>,
    editor: Box<Editor<V, E>>,
    asks: Vec<E>,
    #[cfg(feature = "screen-testing")]
    facts: craft_ui::testing::StoryOutput,
}

impl<V: Value + Default + Serialize + DeserializeOwned, E: Send + 'static> HostedAdjustDialog<V, E> {
    /// The dialog of the adjustment command `key`, titled `title`, opening with `value`; `editor` shows the
    /// adjustment's editor over the value in the given context. Preview starts on; the editor's colour sliders show
    /// their colour tracks.
    pub fn new(
        key: impl Into<String>,
        title: impl Into<Label>,
        value: V,
        editor: impl for<'b> Fn(&mut Ui, &mut V, &mut AdjustDialogCx<'b>) -> Option<E> + Send + 'static,
    ) -> Self {
        let key = key.into();
        HostedAdjustDialog {
            id: dialog_id(key.as_str()),
            key,
            title: title.into(),
            open: true,
            value,
            preview: true,
            comparing: false,
            context: AdjustContext::dialog(true),
            resizable: false,
            remember: None,
            canvas: None,
            reset_with: None,
            picker: None,
            channel: 0,
            clipping: None,
            select_point: None,
            editor: Box::new(editor),
            asks: Vec::new(),
            #[cfg(feature = "screen-testing")]
            facts: craft_ui::testing::StoryOutput::default(),
        }
    }

    /// The editor's context: Dynamic Color Sliders, a grey document. Always a dialog context.
    #[must_use]
    pub fn context(mut self, dynamic_color_sliders: bool, gray: bool) -> Self {
        self.context = AdjustContext::dialog(dynamic_color_sliders).gray(gray);
        self
    }

    /// Whether the dialog resizes by its edges (Curves, whose graph grows with it).
    #[must_use]
    pub fn resizable(mut self, on: bool) -> Self {
        self.resizable = on;
        self
    }

    /// Saves the placement and size under `key`, so the dialog reopens where and as large as the user left it.
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.remember = Some(key);
        self
    }

    /// The canvas the dialog's pickers work on.
    #[must_use]
    pub fn canvas(mut self, canvas: egui::Id) -> Self {
        self.canvas = Some(canvas);
        self
    }

    /// Alt turns Cancel into Reset to `target(opening value, current value)`; without it Reset restores the opening value.
    #[must_use]
    pub fn reset_with(mut self, target: impl Fn(&V, &V) -> V + Send + 'static) -> Self {
        self.reset_with = Some(Box::new(target));
        self
    }

    /// The tone picker armed.
    pub fn picker(&self) -> Option<TonePicker> {
        self.picker
    }

    /// Arms or disarms a tone picker.
    pub fn set_picker(&mut self, picker: Option<TonePicker>) {
        self.picker = picker;
    }

    /// The Channel select's shown entry.
    pub fn channel(&self) -> usize {
        self.channel
    }

    /// The Levels clipping display.
    pub fn clipping(&self) -> Option<LevelsClip> {
        self.clipping
    }

    /// Sets the Levels clipping display.
    pub fn set_clipping(&mut self, clip: Option<LevelsClip>) {
        self.clipping = clip;
    }

    /// Selects curve point `index` in the next pass.
    pub fn select_point(&mut self, index: usize) {
        self.select_point = Some(index);
    }

    /// Whether Preview starts on.
    #[must_use]
    pub fn preview(mut self, on: bool) -> Self {
        self.preview = on;
        self
    }

    /// The value the editor shows: the app previews from it.
    pub fn value(&self) -> &V {
        &self.value
    }

    /// The value, for the app's answers to the editor's asks (Auto's levels).
    pub fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    /// Whether the Preview box is ticked.
    pub fn is_previewing(&self) -> bool {
        self.preview
    }

    /// Whether the Preview box is ticked and held: the app shows the unadjusted picture while it is.
    pub fn is_comparing(&self) -> bool {
        self.comparing
    }

    /// What the editor asked of the app since the last call, in order.
    pub fn take_asks(&mut self) -> Vec<E> {
        std::mem::take(&mut self.asks)
    }
}

impl<V: Value + Default + Serialize + DeserializeOwned, E: Send + 'static> StackDialog for HostedAdjustDialog<V, E> {
    type Result = V;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<V>> {
        let HostedAdjustDialog {
            title,
            open,
            value,
            preview,
            comparing,
            context,
            resizable,
            remember,
            editor,
            asks,
            canvas,
            picker,
            channel,
            clipping,
            select_point,
            reset_with,
            ..
        } = self;
        // The request to select a point is this pass's alone: a rerun of the pass in the frame finds none.
        let (canvas, select_point) = (*canvas, select_point.take());
        let (context, resizable, remember) = (*context, *resizable, remember.clone());
        let form = FormDialog::new(title.clone(), Bind::bound(value)).opened(Bind::bound(open)).preview(Some(Bind::bound(&mut *preview))).shaped(
            move |dialog| {
                let dialog = dialog.resizable(resizable);
                match remember {
                    Some(key) => dialog.remember(key),
                    None => dialog,
                }
            },
        );
        let form = match reset_with.as_deref() {
            Some(target) => form.reset_with(target),
            None => form,
        };
        let shown = form.show(ui, |ui, form, value| {
            if let Some(canvas) = canvas {
                form.keep_view_open(canvas);
            }
            let mut cx =
                AdjustDialogCx { context, canvas, picker: &mut *picker, channel: &mut *channel, select_point, clipping: &mut *clipping, edit: None };
            asks.extend(editor(ui, value, &mut cx));
            true
        });
        *comparing = shown.comparing;
        #[cfg(feature = "screen-testing")]
        {
            self.facts = craft_ui::contract::Outcome::story_facts(&shown.response);
        }
        let accepted = shown.accepted.map(DialogResult::Ok);
        accepted.or(shown.closed.map(|_| DialogResult::Cancel))
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }

    fn confirm(&mut self, _: &Context) -> Option<V> {
        Some(self.value.clone())
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.value)?;
        let object = fields.as_object_mut()?;
        object.insert("preview".to_owned(), serde_json::Value::Bool(self.preview));
        object.insert("picker".to_owned(), serde_json::to_value(self.picker).ok()?);
        object.insert("channel".to_owned(), serde_json::Value::from(self.channel));
        object.insert("clipping".to_owned(), clipping_json(self.clipping));
        Some(fields)
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let rejected = |value: &serde_json::Value| FieldRefusal::Rejects(value).message(field);
        match field {
            "preview" => {
                self.preview = value.as_bool().ok_or_else(|| rejected(&value))?;
            }
            "picker" => {
                self.picker = serde_json::from_value::<Option<TonePicker>>(value.clone()).map_err(|_| rejected(&value))?;
            }
            "channel" => {
                let at = value.as_u64().and_then(|at| usize::try_from(at).ok());
                self.channel = at.ok_or_else(|| rejected(&value))?;
            }
            "clipping" => {
                let clip = clipping_from_json(&value).ok_or_else(|| rejected(&value))?;
                self.set_clipping(clip);
            }
            _ => return set_serde_field(&mut self.value, field, value),
        }
        Ok(())
    }

    fn cancelled(&mut self, _: &Context) {
        self.picker = None;
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.facts.clone()
    }
}
