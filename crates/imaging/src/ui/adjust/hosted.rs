//! [`HostedAdjustDialog`]: an Image › Adjustments dialog on the dialog host, around any adjustment editor.

use craft_behavior::{GestureId, Value};
use craft_ui::contract::{Bind, Sampler as _};
use craft_ui::dialog::{
    DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id, serde_fields, set_serde_field,
};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::memory::SaveKey;
use egui::{Context, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{AdjustContext, LevelsClip, TonePicker};

/// The target a reset aims for, from the current and the opening value.
type ResetWith<V> = Box<dyn Fn(&V, &V) -> V + Send>;

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
#[non_exhaustive]
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

impl<'a> AdjustDialogCx<'a> {
    /// The context for an editor shown in `context` over the dialog's own `picker`, `channel` and `clipping` state, with
    /// no canvas, no curve point to select and no edit phase; the fields set the rest.
    pub fn new(context: AdjustContext, picker: &'a mut Option<TonePicker>, channel: &'a mut usize, clipping: &'a mut Option<LevelsClip>) -> Self {
        Self { context, canvas: None, picker, channel, select_point: None, clipping, edit: None }
    }
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

type JsonForm<V> = dyn Fn(&V, bool) -> serde_json::Map<String, serde_json::Value> + Send;

/// An Image › Adjustments dialog the [`DialogHost`](craft_ui::dialog::DialogHost) owns: an adjustment editor over a
/// value `V` the host keeps, in a form dialog with the Preview box at the start of the footer and OK and Cancel at its
/// end (Alt turns Cancel into Reset, which restores the value the dialog opened with). The editor runs in its dialog form
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
    /// The value Reset returns to and the footer compares with: the opening value, then the last
    /// [`seed`](HostedAdjustDialog::seed).
    opening: V,
    /// Whether the value was edited since the dialog opened: by the editor, Reset, a field set or
    /// [`value_mut`](HostedAdjustDialog::value_mut). A [`seed`](HostedAdjustDialog::seed) is not an edit.
    edited: bool,
    preview: bool,
    comparing: bool,
    context: AdjustContext,
    resizable: bool,
    remember: Option<SaveKey>,
    canvas: Option<egui::Id>,
    reset_with: Option<ResetWith<V>>,
    picker: Option<TonePicker>,
    channel: usize,
    clipping: Option<LevelsClip>,
    select_point: Option<usize>,
    editor: Box<Editor<V, E>>,
    /// The app's form of the value's automation fields; without one the value's serde fields list.
    json_form: Option<Box<JsonForm<V>>>,
    asks: Vec<E>,
    /// Whether OK files its value for the app instead of closing ([`held_ok`](HostedAdjustDialog::held_ok)).
    hold: bool,
    /// The OK the app holds back until it answers `answer_ok`, and its refusal, shown until the value changes.
    held: HeldAnswer<V>,
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
            opening: value.clone(),
            value,
            edited: false,
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
            json_form: None,
            asks: Vec::new(),
            hold: false,
            held: HeldAnswer::new(),
            #[cfg(feature = "screen-testing")]
            facts: craft_ui::testing::StoryOutput::default(),
        }
    }

    /// Makes OK file its value for the app instead of closing: the app runs the adjustment and answers with
    /// [`answer_ok`](HostedAdjustDialog::answer_ok); a refusal keeps the dialog open and shows why. OK stays disabled
    /// until the answer.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// The value a held OK filed, once.
    pub fn take_ok(&mut self) -> Option<V> {
        self.held.take()
    }

    /// The app's answer to the held OK: `Ok` closes the dialog with the held value in the next pass; `Err` enables OK
    /// again and shows the reason under the editor until the value changes. Without a held OK it does nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
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

    /// Lists the value's automation fields as `form(value, edited)` gives them (the command's parameters, when they are
    /// not the value's serde form), beside the dialog's own `preview`, `picker`, `channel` and `clipping`. `edited` is
    /// [`edited`](HostedAdjustDialog::edited). Sets of the value's fields still go to its serde form.
    #[must_use]
    pub fn json_form(mut self, form: impl Fn(&V, bool) -> serde_json::Map<String, serde_json::Value> + Send + 'static) -> Self {
        self.json_form = Some(Box::new(form));
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

    /// The value, for the app's edits of it (Auto's levels, a picked colour, an automation set): the dialog counts
    /// as [`edited`](HostedAdjustDialog::edited) from then on.
    pub fn value_mut(&mut self) -> &mut V {
        self.edited = true;
        &mut self.value
    }

    /// Replaces the value with the one the dialog opens at (the app seeds it after opening, or again once its target
    /// resolves or changes), without counting as an edit. Reset returns to the last seed.
    pub fn seed(&mut self, value: V) {
        self.opening = value.clone();
        self.value = value;
    }

    /// Whether the value was edited since the dialog opened: by the editor, Reset, a field set or
    /// [`value_mut`](HostedAdjustDialog::value_mut). Preview, the picker, the channel and the clipping display are not
    /// the value; seeds are not edits.
    pub fn edited(&self) -> bool {
        self.edited
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
            opening,
            edited,
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
            hold,
            held,
            ..
        } = self;
        // The request to select a point is this pass's alone: a rerun of the pass in the frame finds none.
        let (canvas, select_point) = (*canvas, select_point.take());
        let (context, resizable, remember) = (*context, *resizable, remember.clone());
        // What the editor, Reset and the form's undo change in this pass is the dialog's edit.
        let before = value.clone();
        let form = FormDialog::new(title.clone(), Bind::bound(value))
            .opened(Bind::bound(open))
            .preview(Some(Bind::bound(&mut *preview)))
            .opening(opening.clone())
            .shaped(move |dialog| {
                let dialog = dialog.resizable(resizable);
                match remember {
                    Some(key) => dialog.remember(key),
                    None => dialog,
                }
            });
        let form = match reset_with.as_deref() {
            Some(target) => form.reset_with(|current, opened, _| target(current, opened)),
            None => form,
        };
        // A held OK files the value for the app; the form shows the app's refusal and closes with the accepted value.
        let form = if *hold { form.held(&mut *held) } else { form };
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
        *edited |= *value != before;
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

    /// OK as the button: a held dialog files the value and answers none, and answers the held value once the app
    /// accepted it.
    fn confirm(&mut self, _: &Context) -> Option<V> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() {
            return None;
        }
        if self.hold {
            self.held.file(self.value.clone());
            return None;
        }
        Some(self.value.clone())
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut object = match &self.json_form {
            Some(form) => form(&self.value, self.edited),
            None => serde_fields(&self.value)?.as_object()?.clone(),
        };
        object.insert("preview".to_owned(), serde_json::Value::Bool(self.preview));
        object.insert("picker".to_owned(), serde_json::to_value(self.picker).ok()?);
        object.insert("channel".to_owned(), serde_json::Value::from(self.channel));
        object.insert("clipping".to_owned(), clipping_json(self.clipping));
        Some(serde_json::Value::Object(object))
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
            _ => {
                set_serde_field(&mut self.value, field, value)?;
                self.edited = true;
            }
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
