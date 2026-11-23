//! `ColorPickerDialog` (F04.03): Photoshop's Color Picker as one value an app opens on its `DialogHost` and one result
//! it reads back.
//!
//! ```ignore
//! dialogs.open(ColorPickerDialog::foreground(foreground));
//! dialogs.run(ui);
//! if let Some(DialogResult::Ok(ColorPickerResult { picked, .. })) = dialogs.take::<ColorPickerDialog>() {
//!     foreground = picked.color;
//! }
//! ```

use craft_behavior::{Mixed, Selection, SelectionChange, keyed};
use craft_cms::convert::CmykConverter;
use craft_host::Host;
use craft_host::target::Caps;
use craft_settings::Remembered;
use craft_theme::{ColorModel, Srgba, builtin_models};
use craft_ui::choice::{SwatchArrangement, SwatchTable};
use craft_ui::contract::{Bind, CanvasGesture, CollectionEvent, Sampler as _, ShowCollection as _, ShowContainer as _};
use craft_ui::dialog::{
    ButtonPlace, Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id, serde_fields, set_serde_field,
};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::form_dialogs::{HostRequests, ResetFooter};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::Initial;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::requests::{ColorRequest, EyedropperRequest, HostAnswer};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::tabs::{Tab, TabKind, TabStrip};
use egui::{Context, Id, Ui};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;

use super::system_panel::{self, Answer, Shown};
use super::{ColorPicker, ColorPickerLayout, ColorWarning, PickedColor, ScreenPickGesture};
use crate::{AreaMode, CmykModel, not_web_safe, web_safe};

type StandardColors = (Vec<(Label, Srgba)>, Vec<(Label, Srgba)>);

craft_ui::words! {
    /// The Color Picker dialog's own words, English by default. The picker body's words are `ColorPickerWords`; OK and
    /// Cancel are the common words.
    #[non_exhaustive]
    pub struct ColorPickerDialogWords {
        /// The title when a component's request gives none.
        title: "colorPickerDialog.title" = "Color Picker";
        foreground_title: "colorPickerDialog.foregroundTitle" = "Color Picker (Foreground Color)";
        background_title: "colorPickerDialog.backgroundTitle" = "Color Picker (Background Color)";
        add_to_swatches: "colorPickerDialog.addToSwatches" = "Add to Swatches";
        out_of_gamut: "colorPickerDialog.outOfGamut" = "Out of gamut for printing: click for the nearest printable color";
        not_web_color: "colorPickerDialog.notWebColor" = "Not a web color: click for the nearest web color";
        standard_tab: "colorPickerDialog.standardTab" = "Standard";
        custom_tab: "colorPickerDialog.customTab" = "Custom";
        standard_colors: "colorPickerDialog.standardColors" = "Standard colors";
        greys: "colorPickerDialog.greys" = "Shades of gray";
    }
}

/// What the picker keeps across opens and runs (`Remembered` under the scope `colorPicker.memory`): the component the
/// field shows and Only Web Colors, as Photoshop. Stored when the dialog closes with OK.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ColorPickerMemory {
    /// The component the area and strip edit.
    pub mode: AreaMode,
    /// Only Web Colors: snap to the 216 web-safe colours.
    pub web_colors: bool,
    /// The tab a tabbed dialog showed last.
    pub tab: ColorPickerTab,
    /// The id of the model the Custom tab's model switch showed last; `None` while no switch shows.
    pub model: Option<String>,
}

/// The remembered scope of [`ColorPickerMemory`].
const MEMORY_SCOPE: &str = "colorPicker.memory";

/// What a [`ColorPickerDialog`] answers with OK: the key it was opened with (which colour it was for) and the colour.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorPickerResult {
    /// The dialog's key: `"foreground"`, `"background"`, or the app's own.
    pub key: String,
    /// The colour, with the model values the user typed while they still give it (CMYK inks, Lab values outside sRGB).
    pub picked: PickedColor,
}

/// How a [`ColorPickerDialog`] lays out its body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColorPickerDialogLayout {
    /// Photoshop's Color Picker: the Full picker.
    #[default]
    Full,
    /// Word's Colors dialog: a Standard tab with the honeycomb and a Custom tab with the compact picker.
    Tabbed,
}

/// The tab a tabbed [`ColorPickerDialog`] shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ColorPickerTab {
    /// The Standard honeycomb and the greys.
    #[default]
    Standard,
    /// The compact picker.
    Custom,
}

/// The dialog's title: given by the app, or one of the two the library words.
#[derive(Clone, Debug, PartialEq)]
enum Title {
    Default,
    Foreground,
    Background,
    Given(Label),
}

/// Photoshop's Color Picker dialog on the `DialogHost`: the Full `ColorPicker` with the compare block, the
/// out-of-gamut and web-colour warnings, Only Web Colors and the eyedroppers, and OK, Cancel and Add to Swatches as the
/// dialog's buttons, which stand in the picker's actions cell over its second field group (`ButtonPlace::Body`). The
/// dialog takes the picker's size: one arrangement in every room.
///
/// - **Opening:** the hex field has the focus with its text selected, so typing replaces it. The field shows the
///   component it showed last time and Only Web Colors is as it was left ([`ColorPickerMemory`]); with Only Web Colors
///   on, the opening colour is snapped to the nearest web colour and the compare swatch's "current" half shows the
///   snapped colour, so OK without an edit changes nothing (storytold/photocraft#4995).
/// - **Warnings:** the out-of-gamut chip shows when the colour leaves the CMYK gamut of the [`cmyk`](Self::cmyk)
///   converter (ΔE over 3); with Only Web Colors off, the web-colour chip shows for a colour that is not one. A click
///   on a chip takes its nearest colour.
/// - **Eyedroppers:** while the dialog is the top dialog it files an `EyedropperRequest` through
///   [`requests`](Self::requests); the host runs its canvas dropper, which samples under the dialog (the dialog
///   registers [`canvas`](Self::canvas) with the sampler) and answers with `HostRequests::answer_eyedropper`. The
///   picker's screen-pick press is kept for the host ([`take_screen_pick`](Self::take_screen_pick)), which drives
///   its `ScreenPick` and answers with [`set_color`](Self::set_color).
/// - **Buttons:** Enter is OK and Esc is Cancel (not while a popup is open or a gesture runs). Alt turns Cancel into
///   Reset: the colour from when the dialog opened, and the dialog stays open. Add to Swatches keeps the dialog open;
///   the app takes the colour with [`take_add_to_swatches`](Self::take_add_to_swatches).
/// - **Preview:** [`current`](Self::current) follows every edit, so an app that previews reads it each pass through
///   `DialogHost::get_by_id` and shows it live; Cancel is the app's cue to put the old colour back.
#[derive(craft_ui::contract::Pattern)]
pub struct ColorPickerDialog {
    id: DialogId,
    /// Which of several pickers of the same key this is ([`instance`](Self::instance)).
    instance: Option<DialogId>,
    key: String,
    title: Title,
    title_args: Vec<(String, String)>,
    /// The colour the dialog opened with; `None` for a request whose items have different colours.
    opening: Option<PickedColor>,
    /// The colour as it stands; `None` until the first pick when the dialog opened without one.
    current: Option<PickedColor>,
    compare: bool,
    remember: bool,
    add_to_swatches: bool,
    cmyk: Option<CmykConverter>,
    eyedropper: bool,
    canvas_samples: bool,
    canvas: Option<Id>,
    screen_busy: bool,
    memory: ColorPickerMemory,
    pending_swatch: Option<PickedColor>,
    pending_screen: Option<ScreenPickGesture>,
    requests: HostRequests,
    sampling: bool,
    sample: Option<Srgba>,
    host: Option<Arc<Host>>,
    started: bool,
    /// The system Colors panel while it stands in for the dialog ([`use_system_color_panel`](super::use_system_color_panel)).
    system: Option<Shown>,
    /// The system panel couldn't show (another dialog has it, or off the main thread): this dialog stays in the window.
    in_window: bool,
    open: bool,
    layout: ColorPickerDialogLayout,
    /// The Standard tab's honeycomb cells and grey row.
    standard: Option<StandardColors>,
    /// The tab the app asked to open on; it wins over the remembered one, once.
    tab: Option<ColorPickerTab>,
}

/// What the tabbed body shows besides the memory and the colour it edits.
struct TabbedBody<'a> {
    title: &'a Label,
    words: &'a ColorPickerDialogWords,
    standard: Option<&'a StandardColors>,
    compare: Option<Mixed<PickedColor>>,
}

/// The selection of the entry whose colour is exactly `current`.
fn chosen(entries: &[(Label, Srgba)], current: Option<Srgba>) -> Selection<usize> {
    current.and_then(|color| entries.iter().position(|(_, entry)| *entry == color)).map_or_else(Selection::default, Selection::single)
}

impl TabbedBody<'_> {
    /// The tab strip over the page of `memory.tab`: the honeycomb and the greys, or the compact picker with the compare
    /// swatch. A pick on either page sets `held`.
    fn show(&self, ui: &mut Ui, memory: &mut ColorPickerMemory, held: &mut Mixed<PickedColor>) {
        let words = self.words;
        Stack::new().show(ui, |ui, _| {
            let tabs = [(ColorPickerTab::Standard, words.standard_tab.clone()), (ColorPickerTab::Custom, words.custom_tab.clone())];
            let source = keyed(&tabs[..], |tab: &(ColorPickerTab, Label)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
            let out = TabStrip::new(self.title.clone()).kind(TabKind::Text).shown(memory.tab).natural_height(true).show_items(
                ui,
                &source,
                &mut |key, _| Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or_else(Label::default, |tab| tab.1.clone())),
            );
            for event in out.response.events {
                if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                    && let Some(picked) = keys.last()
                {
                    memory.tab = *picked;
                }
            }
            match memory.tab {
                ColorPickerTab::Standard => self.standard_page(ui, held),
                ColorPickerTab::Custom => self.custom_page(ui, memory, held),
            }
        });
    }

    /// The honeycomb over the row of greys; the selection follows the colour by exact match.
    fn standard_page(&self, ui: &mut Ui, held: &mut Mixed<PickedColor>) {
        let (cells, greys) = self.standard.map_or((&[][..], &[][..]), |(cells, greys)| (cells.as_slice(), greys.as_slice()));
        let current = held.same().map(|picked| picked.color);
        let mut pick = None;
        Stack::new().show(ui, |ui, _| {
            let honeycomb = SwatchTable::new(self.words.standard_colors.clone(), cells)
                .arrangement(SwatchArrangement::Honeycomb)
                .tips(true)
                .selection(Bind::given(chosen(cells, current)))
                .show_all(ui);
            if let Some(at) = honeycomb.selected {
                pick = cells.get(at).map(|(_, color)| *color);
            }
            let row = SwatchTable::new(self.words.greys.clone(), greys)
                .per_row(greys.len().max(1))
                .tips(true)
                .selection(Bind::given(chosen(greys, current)))
                .show_all(ui);
            if let Some(at) = row.selected {
                pick = greys.get(at).map(|(_, color)| *color);
            }
        });
        if let Some(color) = pick {
            *held = Mixed::Same(PickedColor::rgb(color));
        }
    }

    /// The compact picker over the colour, the compare swatch beside its area; the model switch is the memory's.
    fn custom_page(&self, ui: &mut Ui, memory: &mut ColorPickerMemory, held: &mut Mixed<PickedColor>) {
        let models: Vec<&dyn ColorModel> = builtin_models().into_iter().filter(|model| model.default_on().compact).collect();
        let mut model: Option<&'static str> = memory.model.as_deref().and_then(|id| models.iter().map(|known| known.id()).find(|known| *known == id));
        let mut picker =
            ColorPicker::new(self.title.clone(), Bind::bound_mixed(held)).layout(ColorPickerLayout::Compact).shown_model(Bind::bound(&mut model));
        if let Some(compare) = self.compare {
            picker = picker.compare(compare);
        }
        let _ = picker.show(ui);
        memory.model = model.map(str::to_owned);
    }
}

impl ColorPickerDialog {
    /// A picker for what a component's `ColorRequest` asked: its key, its title (the default title when it has none)
    /// and its colour. The app opens it on its `DialogHost` and answers the component with the result's `key` and
    /// `picked.color`.
    pub fn from_request(request: ColorRequest) -> ColorPickerDialog {
        let ColorRequest { key, title, color, .. } = request;
        let title = title.map_or(Title::Default, Title::Given);
        ColorPickerDialog::with_title(key, title, color)
    }

    /// A picker for the colour `key` names, titled `title`, opening on `color`.
    pub fn new(key: impl Into<String>, title: impl Into<Label>, color: Srgba) -> ColorPickerDialog {
        ColorPickerDialog::with_title(key.into(), Title::Given(title.into()), Some(color))
    }

    /// The foreground colour's picker: key `"foreground"`, "Color Picker (Foreground Color)".
    pub fn foreground(color: Srgba) -> ColorPickerDialog {
        ColorPickerDialog::with_title("foreground".into(), Title::Foreground, Some(color))
    }

    /// The background colour's picker: key `"background"`, "Color Picker (Background Color)".
    pub fn background(color: Srgba) -> ColorPickerDialog {
        ColorPickerDialog::with_title("background".into(), Title::Background, Some(color))
    }

    fn with_title(key: String, title: Title, color: Option<Srgba>) -> ColorPickerDialog {
        let opening = color.map(PickedColor::rgb);
        ColorPickerDialog {
            id: dialog_id(("color-picker", key.as_str())),
            instance: None,
            key,
            title,
            title_args: Vec::new(),
            opening,
            current: opening,
            compare: true,
            remember: true,
            add_to_swatches: true,
            cmyk: None,
            eyedropper: true,
            canvas_samples: true,
            canvas: None,
            screen_busy: false,
            memory: ColorPickerMemory::default(),
            pending_swatch: None,
            pending_screen: None,
            requests: HostRequests::new(),
            sampling: false,
            sample: None,
            host: None,
            started: false,
            system: None,
            in_window: false,
            open: true,
            layout: ColorPickerDialogLayout::default(),
            standard: None,
            tab: None,
        }
    }

    /// Sets the dialog's layout: Full (Photoshop) or Tabbed (Word's Colors dialog).
    #[must_use]
    pub fn layout(mut self, l: ColorPickerDialogLayout) -> ColorPickerDialog {
        self.layout = l;
        self
    }

    /// Sets the Standard tab's honeycomb `cells` and `greys`, each a name and a colour.
    #[must_use]
    pub fn standard(mut self, cells: Vec<(Label, Srgba)>, greys: Vec<(Label, Srgba)>) -> ColorPickerDialog {
        self.standard = Some((cells, greys));
        self
    }

    /// Sets the tab a tabbed dialog shows.
    #[must_use]
    pub fn tab(mut self, t: ColorPickerTab) -> ColorPickerDialog {
        self.tab = Some(t);
        self
    }

    /// Fills `{name}` in a given title with `value`, after the title is translated: the title follows a language
    /// switch while the dialog is open. The value is shown as given, never translated.
    #[must_use]
    pub fn arg(mut self, name: impl Into<String>, value: impl Into<String>) -> ColorPickerDialog {
        self.title_args.push((name.into(), value.into()));
        self
    }

    /// The model values the colour was picked in (a CMYK fill's inks): the fields show them as they were typed, not
    /// what the sRGB colour converts to (storytold/photocraft#4196).
    #[must_use]
    pub fn kept(mut self, values: craft_theme::ModelValues) -> ColorPickerDialog {
        for held in [&mut self.opening, &mut self.current].into_iter().flatten() {
            held.model = Some(values);
        }
        self
    }

    /// Whether the compare block's current half shows the colour the dialog opened with; on by default. Off, it shows the
    /// colour at the picker's first show.
    #[must_use]
    pub fn compare(mut self, on: bool) -> ColorPickerDialog {
        self.compare = on;
        self
    }

    /// Whether the field's component and Only Web Colors are recalled on open and stored on OK
    /// ([`ColorPickerMemory`]); on by default. A picker opened by a feature that wants its own start turns it off.
    #[must_use]
    pub fn web_colors_remembered(mut self, on: bool) -> ColorPickerDialog {
        self.remember = on;
        self
    }

    /// The Add to Swatches button; on by default.
    #[must_use]
    pub fn add_to_swatches(mut self, on: bool) -> ColorPickerDialog {
        self.add_to_swatches = on;
        self
    }

    /// The document's CMYK converter: the C, M, Y, K fields convert through it and the out-of-gamut chip tests against
    /// it. Without it the dialog uses the built-in coated converter, as PhotoCraft does for a document without a CMYK
    /// profile.
    #[must_use]
    pub fn cmyk(mut self, converter: &CmykConverter) -> ColorPickerDialog {
        self.cmyk = Some(converter.clone());
        self
    }

    /// The screen eyedropper button; on by default, and shown only where the platform can capture the screen.
    #[must_use]
    pub fn eyedropper(mut self, on: bool) -> ColorPickerDialog {
        self.eyedropper = on;
        self
    }

    /// Whether the canvas samples into the dialog while it is the top dialog; on by default.
    #[must_use]
    pub fn canvas_samples(mut self, on: bool) -> ColorPickerDialog {
        self.canvas_samples = on;
        self
    }

    /// The canvas whose presses pass through the dialog's modal layer while the dropper is armed
    /// (`Sampler::pass_canvas_gesture`), and whose view stays usable (`keep_view_open`).
    #[must_use]
    pub fn canvas(mut self, canvas: Id) -> ColorPickerDialog {
        self.canvas = Some(canvas);
        self
    }

    /// The dialog for one target among several of the same key: its id is `dialog_id(("color-picker", key,
    /// instance))`, and [`StackDialog::instance`] returns it, so the host replaces a picker only for the same target.
    #[must_use]
    pub fn instance(mut self, instance: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id = dialog_id(("color-picker", self.key.as_str(), instance));
        self.instance = Some(self.id);
        self
    }

    /// Whether a screen pick is running; the picker's screen-pick button is disabled meanwhile. The host sets it each
    /// pass from its `ScreenPick`.
    pub fn screen_busy(&mut self, busy: bool) {
        self.screen_busy = busy;
    }

    /// The key the dialog was opened with.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The colour as it stands this pass.
    pub fn current(&self) -> Option<PickedColor> {
        self.current
    }

    /// The colour the dialog opened with: as given, until the first pass; then, with Only Web Colors on, snapped to the
    /// nearest web colour, which is what the compare swatch's "current" half shows and what Reset returns to.
    pub fn opening(&self) -> Option<PickedColor> {
        self.opening
    }

    /// Sets the colour as an edit would, for automation, scripts and the host's screen pick: the fields show `picked`'s
    /// model values when it has them, and the colour is what OK answers. The compare swatch keeps the opening colour.
    pub fn set_color(&mut self, picked: PickedColor) {
        self.current = Some(picked);
        if let Some(system) = &self.system {
            system.set(picked.color);
        }
    }

    /// The colour Add to Swatches was pressed for, once; `None` otherwise.
    pub fn take_add_to_swatches(&mut self) -> Option<PickedColor> {
        self.pending_swatch.take()
    }

    /// The picker's screen-pick press, once: the host starts or feeds its `ScreenPick` with it.
    pub fn take_screen_pick(&mut self) -> Option<ScreenPickGesture> {
        self.pending_screen.take()
    }

    /// What the dialog asks of the host (the canvas eyedropper) and the host's answers.
    pub fn requests(&mut self) -> &mut HostRequests {
        &mut self.requests
    }

    /// What OK does: stores the remembered field component and Only Web Colors, and answers with the colour.
    fn accept(&mut self, _: &Context) -> Option<ColorPickerResult> {
        let picked = self.current?;
        self.requests.cancel();
        if self.remember
            && let Some(host) = &self.host
        {
            Remembered::<ColorPickerMemory>::new(MEMORY_SCOPE).store(host, &self.memory);
        }
        Some(ColorPickerResult { key: self.key.clone(), picked })
    }

    /// Runs the dialog's start for automation that reaches it before its first pass.
    fn start_headless(&mut self, ctx: &Context) {
        if !self.started {
            let host = Env::of_root(ctx).view().host().cloned();
            self.start(host.as_ref());
        }
    }

    /// The first pass: recalls the remembered field component and Only Web Colors, and with Only Web Colors on snaps
    /// the opening colour, which the compare swatch's current half follows (storytold/photocraft#4995).
    fn start(&mut self, host: Option<&Arc<Host>>) {
        self.started = true;
        self.host = host.cloned();
        if self.remember
            && let Some(host) = &self.host
            && let Some(memory) = Remembered::<ColorPickerMemory>::new(MEMORY_SCOPE).recall(host)
        {
            self.memory = memory;
        }
        if let Some(tab) = self.tab.take() {
            self.memory.tab = tab;
        }
        if self.memory.web_colors
            && self.layout == ColorPickerDialogLayout::Full
            && let Some(opening) = self.opening
        {
            let [r, g, b] = web_safe([opening.color.r, opening.color.g, opening.color.b]);
            self.opening = Some(PickedColor { color: Srgba { r, g, b, a: opening.color.a }, model: None });
            self.current = self.opening;
        }
    }

    /// A pass with the system Colors panel standing in for the dialog: it opens on the first pass, blocks the window
    /// behind it, as the modal dialog does, and answers as OK or Cancel would. `None` when the panel can't show (off the
    /// main thread, as in headless tests), so the dialog shows in the window.
    fn show_system(&mut self, ui: &Ui) -> Option<Option<DialogResult<ColorPickerResult>>> {
        if self.in_window {
            return None;
        }
        if self.system.is_none() {
            let common = Env::of(ui).words::<CommonWords>();
            let color = self.current.or(self.opening).map_or(Srgba::rgb(0.0, 0.0, 0.0), |picked| picked.color);
            self.system = Shown::open(color, common.ok.as_str(), common.cancel.as_str());
            self.in_window = self.system.is_none();
        }
        system_panel::block_behind(ui.ctx());
        let Some(answer) = self.system.as_ref()?.poll() else {
            // The panel is another window: its clicks don't wake this one.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            return Some(None);
        };
        self.system = None;
        Some(match answer {
            Answer::Ok(color) => {
                // The panel shows no alpha: the colour keeps the dialog's.
                let a = self.current.or(self.opening).map_or(1.0, |picked| picked.color.a);
                self.current = Some(PickedColor { color: Srgba { a, ..color }, model: None });
                self.accept(ui.ctx()).map(DialogResult::Ok)
            }
            Answer::Cancel => {
                self.requests.cancel();
                Some(DialogResult::Cancel)
            }
        })
    }

    /// Files or withdraws the canvas eyedropper request as the dialog gains or loses the top.
    fn follow_top(&mut self, top: bool) {
        let sampling = self.canvas_samples && top && self.layout == ColorPickerDialogLayout::Full;
        if sampling && !self.sampling {
            self.requests.request_eyedropper(EyedropperRequest::new(self.key.clone()));
        } else if !sampling && self.sampling {
            self.requests.cancel_eyedropper();
        }
        self.sampling = sampling;
    }
}

impl StackDialog for ColorPickerDialog {
    type Result = ColorPickerResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(&self.key)
    }

    /// Answers OK as the OK button would, without drawing: the dialog's start (recalled memory, the Only Web Colors
    /// snap) has run, the memory is stored, and the colour is the current one.
    fn confirm(&mut self, ctx: &Context) -> Option<ColorPickerResult> {
        self.start_headless(ctx);
        self.accept(ctx)
    }

    fn instance(&self) -> Option<DialogId> {
        self.instance
    }

    /// `color` (`#rrggbb`, null when there is none yet), and the memory's own `mode`, `webColors`, `tab` and `model`.
    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut fields = serde_fields(&self.memory)?;
        let color = self.current.map_or(serde_json::Value::Null, |picked| picked.color.to_hex().into());
        fields.as_object_mut()?.insert("color".to_owned(), color);
        Some(fields)
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        // The dialog's start recalls the memory and snaps the opening colour; a field set before it would be overwritten.
        self.start_headless(ctx);
        if field != "color" {
            return set_serde_field(&mut self.memory, field, value);
        }
        let parsed = value.as_str().and_then(Srgba::from_hex);
        let color = parsed.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
        self.set_color(PickedColor { color, model: None });
        Ok(())
    }

    /// The host closed the dialog from outside: its dropper ends with it.
    fn cancelled(&mut self, _: &Context) {
        self.requests.cancel();
        self.system = None;
    }

    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<ColorPickerResult>> {
        let env = Env::of(ui);
        let words = env.words::<ColorPickerDialogWords>();
        let common = env.words::<CommonWords>();
        if !self.started {
            self.start(env.view().host());
        }
        if system_panel::wanted()
            && let Some(answer) = self.show_system(ui)
        {
            return answer;
        }
        self.follow_top(stack.is_top(self.id));
        let answered = self.requests.take_answers().into_iter().rev().find_map(|answer| match answer {
            HostAnswer::Eyedropper { key, color, .. } if key == self.key => Some(color),
            _ => None,
        });
        self.sample = answered.or(self.sample.take());

        let title = match &self.title {
            Title::Default => words.title.clone(),
            Title::Foreground => words.foreground_title.clone(),
            Title::Background => words.background_title.clone(),
            Title::Given(label) => {
                let resolved = env.view().resolve(label);
                if self.title_args.is_empty() {
                    resolved
                } else {
                    let args: Vec<(&str, &str)> = self.title_args.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
                    Label::from(craft_i18n::fmt(resolved.as_str(), &args))
                }
            }
        };
        let cmyk = CmykModel::new(self.cmyk.clone().unwrap_or_else(|| CmykConverter::coated().clone()));
        let footer = ResetFooter::new(ui, None, true);
        let changed = self.current != self.opening;
        let mut buttons = footer.buttons(FooterButton::new(common.ok.clone(), DialogRole::Accept).enabled(self.current.is_some()), changed).to_vec();
        if self.add_to_swatches && self.layout == ColorPickerDialogLayout::Full {
            buttons.push(FooterButton::new(words.add_to_swatches.clone(), DialogRole::Other).stays_open(true));
        }

        let arm = self.sampling.then_some(self.canvas).flatten();
        let keep_open = self.canvas;
        let screen = self.eyedropper && self.layout == ColorPickerDialogLayout::Full && env.view().shell().caps.contains(Caps::SCREEN_CAPTURE);
        // The Full picker keeps a cell for the buttons; the tabbed body has none, so they stand in the side column, as in
        // Word's Colors dialog.
        let place = if self.layout == ColorPickerDialogLayout::Full { ButtonPlace::Body } else { ButtonPlace::Side };
        let Self { current, opening, compare, memory, screen_busy, sample, pending_screen, open, layout, standard, .. } = self;
        let mut held: Mixed<PickedColor> = current.map_or(Mixed::Mixed, Mixed::Same);
        let (sampled, busy, compare_with) = (sample.take(), *screen_busy, compare.then(|| opening.map_or(Mixed::Mixed, Mixed::Same)));
        let color = current.map(|current| current.color);
        let web_only = memory.web_colors;
        let shown = Dialog::new(title.clone())
            .opened(Bind::bound(open))
            .buttons(buttons)
            .button_place(place)
            .close_on_enter(true)
            .initial_focus(Initial::Keep)
            .roomy(true)
            .show(ui, |ui, cx| {
                if let Some(canvas) = keep_open {
                    cx.keep_view_open(canvas);
                }
                if let Some(canvas) = arm {
                    cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
                }
                if *layout == ColorPickerDialogLayout::Tabbed {
                    let body = TabbedBody { title: &title, words: &words, standard: standard.as_ref(), compare: compare_with };
                    body.show(ui, &mut *memory, &mut held);
                    return;
                }
                let mut models: Vec<&dyn ColorModel> = builtin_models().into_iter().filter(|model| model.default_on().full).collect();
                models.push(&cmyk);
                let mut warnings = Vec::new();
                if let Some(color) = color {
                    let rgb = [color.r, color.g, color.b];
                    if let Some(nearest) = cmyk.converter().nearest_in_gamut(rgb, CmykConverter::PICKER_GAMUT_DE) {
                        warnings
                            .push(ColorWarning::new(words.out_of_gamut.clone(), Srgba { r: nearest[0], g: nearest[1], b: nearest[2], a: color.a }));
                    }
                    if !web_only && let Some(nearest) = not_web_safe(rgb) {
                        warnings.push(ColorWarning::web_colors(
                            words.not_web_color.clone(),
                            Srgba { r: nearest[0], g: nearest[1], b: nearest[2], a: color.a },
                        ));
                    }
                }
                let mut picker = ColorPicker::new(title.clone(), Bind::bound_mixed(&mut held))
                    .layout(ColorPickerLayout::Full)
                    .models(models)
                    .mode(Bind::bound(&mut memory.mode))
                    .web_colors(Bind::bound(&mut memory.web_colors))
                    .warnings(warnings)
                    .eyedropper(screen)
                    .eyedropper_busy(busy);
                if let Some(sampled) = sampled {
                    picker = picker.sampled(sampled);
                }
                if let Some(compare) = compare_with {
                    picker = picker.compare(compare);
                }
                if let Some(gesture) = picker.show(ui).screen_pick {
                    *pending_screen = Some(gesture);
                }
            });
        self.current = held.same().copied();

        let (picked, closed) = (shown.output.picked, shown.output.closed);
        let ok = (picked == Some(0) || closed == Some(CloseReason::Enter)) && self.current.is_some();
        if !ok && picked == Some(1) && footer.resetting() {
            self.current = self.opening;
            return None;
        }
        if picked == Some(2) {
            self.pending_swatch = self.current;
            return None;
        }
        match (ok, closed) {
            (true, _) => self.accept(ui.ctx()).map(DialogResult::Ok),
            (false, Some(_)) => {
                self.requests.cancel();
                Some(DialogResult::Cancel)
            }
            (false, None) => None,
        }
    }
}
