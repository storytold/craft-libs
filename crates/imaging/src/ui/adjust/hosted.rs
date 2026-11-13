//! [`HostedAdjustDialog`]: an Image › Adjustments dialog on the dialog host, around any adjustment editor.

use craft_behavior::Value;
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::memory::SaveKey;
use egui::Ui;

use super::AdjustContext;

/// Shows the editor over the dialog's value for one pass; `Some` carries what the editor asks of the app this pass.
type Editor<V, E> = dyn for<'b> Fn(&mut Ui, Bind<'b, V>, AdjustContext) -> Option<E> + Send;

/// An Image › Adjustments dialog the [`DialogHost`](craft_ui::dialog::DialogHost) owns: an adjustment editor over a
/// value `V` the host keeps, in a form dialog with OK and Cancel beside the editor (Alt turns Cancel into Reset, which
/// restores the value the dialog opened with) and the Preview box under them. The editor runs in its dialog form
/// ([`AdjustContext::dialog`]). What the editor asks of the app (Levels' Auto, Black & White's Auto) is an `E` that
/// waits in [`take_asks`](Self::take_asks). It answers with the value at OK.
///
/// ```ignore
/// host.open(HostedAdjustDialog::new("image.adjustments.brightnessContrast", "Brightness/Contrast", value, |ui, value, cx| {
///     BrightnessContrastEditor::new("Brightness/Contrast", value).context(cx).show(ui);
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
    editor: Box<Editor<V, E>>,
    asks: Vec<E>,
    #[cfg(feature = "screen-testing")]
    facts: craft_ui::testing::StoryOutput,
}

impl<V: Value + Default, E: Send + 'static> HostedAdjustDialog<V, E> {
    /// The dialog of the adjustment command `key`, titled `title`, opening with `value`; `editor` shows the
    /// adjustment's editor over the value in the given context. Preview starts on; the editor's colour sliders show
    /// their colour tracks.
    pub fn new(
        key: impl Into<String>,
        title: impl Into<Label>,
        value: V,
        editor: impl for<'b> Fn(&mut Ui, Bind<'b, V>, AdjustContext) -> Option<E> + Send + 'static,
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

impl<V: Value + Default, E: Send + 'static> StackDialog for HostedAdjustDialog<V, E> {
    type Result = V;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<V>> {
        let HostedAdjustDialog { title, open, value, preview, comparing, context, resizable, remember, editor, asks, .. } = self;
        let (context, resizable, remember) = (*context, *resizable, remember.clone());
        let shown = FormDialog::new(title.clone(), Bind::bound(value))
            .opened(Bind::bound(open))
            .preview(Some(Bind::bound(&mut *preview)))
            .shaped(move |dialog| {
                let dialog = dialog.resizable(resizable);
                match remember {
                    Some(key) => dialog.remember(key),
                    None => dialog,
                }
            })
            .show(ui, |ui, _, value| {
                asks.extend(editor(ui, Bind::bound(value), context));
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

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.facts.clone()
    }
}
