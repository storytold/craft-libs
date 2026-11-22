//! [`PatternFillDialog`].

use super::editor::ANGLE_RANGE;
use super::{PatternColors, PatternFillEditor, PatternFillOptions, PatternSwatch, PatternWords};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, CanvasGesture, Sampler as _, ShowContainer as _};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::Env;
use egui::Ui;
use serde_json::{Value, json};

/// Photoshop's Pattern Fill dialog (Layer, New Fill Layer, Pattern...; a Pattern Fill layer's thumbnail; Layer Content
/// Options): [`PatternFillEditor`] in bitmap mode with OK and Cancel. No Preview check box: the app previews live
/// from [`current`](Self::current) while [`changed`](Self::changed) says the values moved this pass (PhotoCraft's
/// proxy preview).
///
/// OK answers the options; OK with the values it opened with answers them too. When the dialog edits an existing fill
/// the app makes no step for an unchanged edit ([`unchanged`](Self::unchanged)); a new fill layer is made whatever
/// the values (PhotoCraft). Cancel answers nothing. Enter is OK, Esc Cancel.
pub struct PatternFillDialog {
    id: DialogId,
    initial: PatternFillOptions,
    current: PatternFillOptions,
    swatches: Vec<PatternSwatch>,
    changed: bool,
    canvas: Option<egui::Id>,
    /// Whether the dialog is open: bound to the form's open state, so the host's dialog closes with it.
    open: bool,
}

impl PatternFillDialog {
    /// The dialog editing `options`, offering `swatches`.
    pub fn new(options: PatternFillOptions, swatches: Vec<PatternSwatch>) -> Self {
        PatternFillDialog {
            id: dialog_id("patternFill"),
            initial: options.clone(),
            current: options,
            swatches,
            changed: false,
            canvas: None,
            open: true,
        }
    }

    /// The canvas whose primary drag moves the pattern while Link with Layer is on: the body passes
    /// that gesture to it each pass.
    #[must_use]
    pub fn canvas(mut self, canvas: egui::Id) -> Self {
        self.canvas = Some(canvas);
        self
    }

    /// Sets the pattern's phase, as the app's canvas drag does (M-12); [`changed`](Self::changed) reports it so the
    /// app refreshes its preview.
    pub fn set_phase(&mut self, phase: [f64; 2]) {
        self.current.phase = phase;
        self.changed = true;
    }

    /// The values the fields show now, for the app's live preview.
    pub fn current(&self) -> &PatternFillOptions {
        &self.current
    }

    /// Whether the values changed in the last pass the dialog drew (the app refreshes its preview).
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Whether the current values equal the ones it opened with. Only the edit of an existing fill skips its history
    /// step on this; creating a new fill layer ignores it.
    pub fn unchanged(&self) -> bool {
        self.current == self.initial
    }
}

impl StackDialog for PatternFillDialog {
    type Result = PatternFillOptions;

    /// `pattern`, `scale`, `angle`, `link`, `phase` (`[x, y]`) and `colors` (`{foreground, background}` in the
    /// `Srgba` form, `null` in bitmap mode).
    fn json_fields(&self) -> Option<Value> {
        let options = &self.current;
        let colors = options.colors.map(|colors| json!({ "foreground": colors.foreground, "background": colors.background }));
        Some(json!({
            "pattern": options.pattern,
            "scale": options.scale,
            "angle": options.angle,
            "link": options.link,
            "phase": options.phase,
            "colors": colors,
        }))
    }

    /// Sets a field as the editor's row does: `pattern` takes one of the swatches, `scale` and `angle` are held to
    /// their ranges. The bitmap rows (`scale`, `angle`, `link`, `phase`) refuse as inactive in preset mode, and
    /// `colors` in bitmap mode.
    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let preset = self.current.colors.is_some();
        let bitmap_row = matches!(field, "scale" | "angle" | "link" | "phase");
        if (bitmap_row && preset) || (field == "colors" && !preset) {
            return Err(FieldRefusal::Inactive.message(field));
        }
        let options = &mut self.current;
        match field {
            "pattern" => {
                let pattern = value.as_str().filter(|id| self.swatches.iter().any(|swatch| swatch.id == *id)).ok_or_else(rejects)?;
                options.pattern = pattern.to_owned();
            }
            "scale" => {
                options.scale = value.as_f64().ok_or_else(rejects)?.clamp(*PatternFillOptions::SCALE.start(), *PatternFillOptions::SCALE.end())
            }
            "angle" => options.angle = value.as_f64().ok_or_else(rejects)?.clamp(*ANGLE_RANGE.start(), *ANGLE_RANGE.end()),
            "link" => options.link = value.as_bool().ok_or_else(rejects)?,
            "phase" => options.phase = serde_json::from_value::<[f64; 2]>(value.clone()).map_err(|_| rejects())?,
            "colors" => {
                let color = |key: &str| value.get(key).and_then(|color| serde_json::from_value::<Srgba>(color.clone()).ok());
                let (Some(foreground), Some(background)) = (color("foreground"), color("background")) else { return Err(rejects()) };
                options.colors = Some(PatternColors::new(foreground, background));
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        self.changed = true;
        Ok(())
    }

    /// Answers the current values, as OK does.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Self::Result> {
        Some(self.current.clone())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PatternFillOptions>> {
        let words = Env::of(ui).words::<PatternWords>();
        let PatternFillDialog { current, swatches, changed, canvas, open, .. } = self;
        let before = current.clone();
        let form = FormDialog::new(words.title.clone(), Bind::bound(&mut *current)).opened(Bind::bound(open));
        let shown = form.show(ui, |ui, _, value| {
            // While Link with Layer is on, a drag on the document moves the pattern: the modal layer lets that press
            // through to the canvas, and the app's drag sets the phase.
            let link = value.link && value.colors.is_none();
            Stack::new().show(ui, |ui, cx| {
                if let (Some(canvas), true) = (*canvas, link) {
                    cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
                }
                PatternFillEditor::new(words.pattern.clone(), Bind::bound(&mut *value), swatches).show(ui);
            });
            true
        });
        *changed = *current != before;
        match (shown.accepted, shown.closed) {
            (Some(options), _) => Some(DialogResult::Ok(options)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some("patternFill")
    }
}
