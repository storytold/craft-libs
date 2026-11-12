//! Stories of the Brush Settings panel and its rows.

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, Validity};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::primitives::Revision;
use craft_ui::runtime::env::{Env, Message};
use craft_ui::testing::pictures::stamped_stroke;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui};

use super::{
    BrushSection, BrushSectionEvent, BrushSectionList, BrushSettingsPanel, BrushSizeField, BrushStrokeStrip, DynamicControl, DynamicControlRow,
    PreviewStroke, StripImage, TipAngle, TipEllipse,
};

craft_ui::words! {
    /// The captions and status lines of the Brush Settings stories.
    #[non_exhaustive]
    pub(crate) struct BrushSettingsStoryWords {
        /// The section whose pane shows.
        selected: template "brushSettingsStory.selected" = "Selected: {name}";
        /// The caption over Shape Dynamics' control row.
        size_jitter: "brushSettingsStory.sizeJitter" = "Size Jitter";
        /// A section the story has no sample settings for.
        no_body: "brushSettingsStory.noBody" = "This section has no sample settings.";
        /// The caption over the size field with a restore button.
        with_restore: "brushSettingsStory.withRestore" = "With restore";
        /// The caption over the size field without one.
        without_restore: "brushSettingsStory.withoutRestore" = "Without restore";
        /// The app shows its response curve editor.
        curve_open: "brushSettingsStory.curveOpen" = "Response curve open";
    }
}

const ANGLE: TipAngle = TipAngle::new(30.0, 0.6);
const SIZE: f64 = 30.0;
const MAX_SIZE: f64 = 5000.0;
const ORIGINAL: f64 = 64.0;
const FADE: f64 = 25.0;
/// The sections of the Brush Settings list, by key: their index in `NAMES`.
const TIP: u8 = 0;
const DYNAMICS: u8 = 1;
const SCATTER: u8 = 2;
const TEXTURE: u8 = 3;
const NOISE: u8 = 8;
/// Photoshop's sections: Brush Tip Shape, seven panes with a box and a lock, five switch rows.
const NAMES: [&str; 13] = [
    "Brush Tip Shape",
    "Shape Dynamics",
    "Scattering",
    "Texture",
    "Dual Brush",
    "Color Dynamics",
    "Transfer",
    "Brush Pose",
    "Noise",
    "Wet Edges",
    "Build-up",
    "Smoothing",
    "Protect Texture",
];

/// Photoshop's list: Brush Tip Shape without a box, the seven panes with a box and a lock, the five switch rows.
/// With `on`, every pane but Texture is on and of the switches only Noise; without, every box is clear.
fn sections(on: bool) -> Vec<BrushSection<u8>> {
    (0..)
        .zip(NAMES)
        .map(|(key, name)| match key {
            TIP => BrushSection::pane(key, name),
            DYNAMICS..=7 => BrushSection::pane(key, name).enabled(on && key != TEXTURE).locked(false),
            _ => BrushSection::switch(key, name, on && key == NOISE),
        })
        .collect()
}

/// The dynamics controls, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Off,
    Fade,
    PenPressure,
    PenTilt,
}

fn controls() -> [DynamicControl<Control>; 4] {
    [
        DynamicControl::new(Control::Off, "Off"),
        DynamicControl::new(Control::Fade, "Fade").fade(),
        DynamicControl::new(Control::PenPressure, "Pen Pressure").curve(),
        DynamicControl::new(Control::PenTilt, "Pen Tilt").curve(),
    ]
}

/// One control row's settings: its control, its Fade steps and whether the app shows the curve editor.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ControlRow {
    control: Control,
    steps: f64,
    curve_open: bool,
}

impl ControlRow {
    fn new(control: Control) -> Self {
        ControlRow { control, steps: FADE, curve_open: false }
    }

    /// Shows the row and applies what it reports, as the app does: another control closes the curve editor.
    fn show(&mut self, ui: &mut Ui, controls: &[DynamicControl<Control>]) {
        let before = self.control;
        let shown =
            DynamicControlRow::new(controls, Bind::bound(&mut self.control), Bind::bound(&mut self.steps)).curve(self.curve_open, false).show(ui);
        if shown.curve_toggled {
            self.curve_open = !self.curve_open;
        }
        if self.control != before {
            self.curve_open = false;
        }
    }
}

/// The brush settings the panel stories edit: the sections, the shown pane and the two panes' control rows.
#[derive(Clone, Debug, PartialEq)]
struct Brush {
    sections: Vec<BrushSection<u8>>,
    shown: u8,
    jitter: ControlRow,
    scatter: ControlRow,
}

impl Brush {
    /// The list of `sections(on)` with Shape Dynamics shown.
    fn new(on: bool) -> Self {
        Brush { sections: sections(on), shown: DYNAMICS, jitter: ControlRow::new(Control::PenPressure), scatter: ControlRow::new(Control::Fade) }
    }

    fn section(&mut self, key: u8) -> Option<&mut BrushSection<u8>> {
        self.sections.iter_mut().find(|section| section.key == key)
    }

    /// Applies a list event as the app applies it to its brush settings.
    fn apply(&mut self, event: BrushSectionEvent<u8>) {
        match event {
            BrushSectionEvent::Select(key) => self.shown = key,
            BrushSectionEvent::Enable { key, on } | BrushSectionEvent::Switch { key, on } => {
                if let Some(section) = self.section(key) {
                    section.enabled = Some(on);
                }
            }
            BrushSectionEvent::Lock { key, on } => {
                if let Some(section) = self.section(key) {
                    section.locked = Some(on);
                }
            }
            _ => {}
        }
    }

    /// The readout: the section whose pane shows.
    fn readout(&self, ui: &Ui) -> String {
        let words = Env::of(ui).words::<BrushSettingsStoryWords>();
        let name = self.sections.iter().find(|section| section.key == self.shown).map_or("", |section| section.name.as_str());
        craft_i18n::fmt(words.selected.as_str(), &[("name", name)])
    }
}

/// The diameter of the preview stroke's dabs, in points.
const STROKE_DIAMETER: f64 = 18.0;

/// The paint colour the test stroke is stamped in: the brush's colour, as the engine renders it.
const PAINT: Color32 = Color32::from_rgb(0x37, 0x8E, 0xF0);

/// The strip with the test stroke stamped in the paint colour, as the engine renders it.
fn strip<'a>(ui: &Ui) -> BrushStrokeStrip<'a> {
    let color = PAINT;
    let diameter = STROKE_DIAMETER * f64::from(ui.ctx().pixels_per_point());
    BrushStrokeStrip::new(
        "Stroke preview",
        Revision::new(1),
        StripImage::new(move |[width, height]| {
            let stroke = PreviewStroke::new(width as u32, height as u32);
            let samples: Vec<[f64; 3]> = stroke.samples().iter().map(|at| [at.x, at.y, f64::from(at.pressure)]).collect();
            stamped_stroke([width, height], &samples, diameter * stroke.fit(diameter), color)
        }),
    )
}

/// The panel over `brush`, with sample panes for Brush Tip Shape, Shape Dynamics and Scattering; applies what the
/// list reports.
fn show_panel(ui: &mut Ui, brush: &mut Brush, panel: impl FnOnce(BrushSettingsPanel<'_, u8>) -> BrushSettingsPanel<'_, u8>) -> StoryOutput {
    let words = Env::of(ui).words::<BrushSettingsStoryWords>();
    let controls = controls();
    let strip = strip(ui);
    let Brush { sections, shown, jitter, scatter } = brush;
    let panel = panel(BrushSettingsPanel::new("Brush Settings", sections, Bind::bound(shown)).strip(strip));
    let output = panel.show(ui, |ui, _, &key| match key {
        TIP => {
            let _ = BrushSizeField::new("Size", Bind::given(SIZE), MAX_SIZE).show(ui);
        }
        DYNAMICS => {
            Text::new(words.size_jitter.clone()).shade(Shade::Muted).show(ui);
            jitter.show(ui, &controls);
        }
        SCATTER => scatter.show(ui, &controls),
        _ => {
            Text::new(words.no_body.clone()).shade(Shade::Muted).show(ui);
        }
    });
    let events = output.sections.clone();
    let facts = StoryOutput::from(output);
    for event in events {
        brush.apply(event);
    }
    facts.readout(brush.readout(ui))
}

#[story(BrushSettingsPanel<'static, u8>, covers = [Default], drive = Drive::Never)]
fn panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_panel(ui, data.value("brush", Brush::new(true)), |panel| panel.link_row().can_update())
}

#[story(BrushSettingsPanel<'static, u8>, covers = [], drive = Drive::Never)]
fn panel_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_panel(ui, data.value("brush", Brush::new(false)), |panel| panel)
}

#[story(BrushSectionList<'static, u8>, covers = [Default], drive = Drive::Never)]
fn section_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let brush = data.value("brush", Brush::new(true));
    let output = BrushSectionList::new("Brush settings sections", &brush.sections, Bind::bound(&mut brush.shown)).link_row().show(ui);
    let events = output.events.clone();
    let facts = StoryOutput::from(output);
    for event in events {
        brush.apply(event);
    }
    facts.readout(brush.readout(ui))
}

/// How the story words the angle and roundness it shows.
fn angle_note(angle: TipAngle) -> String {
    format!("Angle {:.0}°, roundness {:.0} %", angle.degrees, (angle.roundness * 100.0).round())
}

#[story(TipEllipse<'static>, covers = [Default], drive = Drive::Drag(Grip::new(super::ellipse::ARROW)))]
fn tip_ellipse(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = TipEllipse::new("Tip angle and roundness", Bind::bound(data.value("angle", ANGLE))).show(ui);
    let note = angle_note(*data.value("angle", ANGLE));
    StoryOutput::from(shown).readout(note)
}

#[story(TipEllipse<'static>, covers = [Mixed], drive = Drive::Never)]
fn tip_ellipse_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    TipEllipse::new("Tip angle and roundness", Bind::bound_mixed(data.value("mixed", Mixed::<TipAngle>::Mixed))).show(ui).into()
}

#[story(TipEllipse<'static>, covers = [Invalid], drive = Drive::Never)]
fn tip_ellipse_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_flat = |angle: &TipAngle| if angle.roundness < 0.5 { Validity::Refuse(Message::new("Too flat")) } else { Validity::Ok };
    let shown = TipEllipse::new("Tip angle and roundness", Bind::bound(data.value("flat", ANGLE)).check(refuse_flat)).show(ui);
    let note = angle_note(*data.value("flat", ANGLE));
    StoryOutput::from(shown).readout(note)
}

/// Two control rows, on Fade with its steps and on Pen Pressure with its curve toggle, then the brush size field with
/// and without its restore button, both on one diameter.
#[story(DynamicControlRow<'static, u8>, covers = [Default], drive = Drive::Never)]
fn dynamic_control_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<BrushSettingsStoryWords>();
    let controls = controls();
    let (rows, size) = data.value("rows", ((ControlRow::new(Control::Fade), ControlRow::new(Control::PenPressure)), SIZE));
    let (fade, curve) = rows;
    Stack::new().show(ui, |ui, _| {
        fade.show(ui, &controls);
        curve.show(ui, &controls);
        Text::new(words.with_restore.clone()).shade(Shade::Muted).show(ui);
        let _ = BrushSizeField::new("Size", Bind::bound(&mut *size), MAX_SIZE).restore(ORIGINAL).show(ui);
        Text::new(words.without_restore.clone()).shade(Shade::Muted).show(ui);
        let _ = BrushSizeField::new("Size", Bind::bound(&mut *size), MAX_SIZE).show(ui);
    });
    let output = StoryOutput::default();
    if curve.curve_open { output.readout(words.curve_open.as_str().to_owned()) } else { output }
}

#[story(BrushStrokeStrip<'static>, covers = [Default], drive = Drive::Never)]
fn stroke_strip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    strip(ui).show(ui).into()
}

#[story(BrushSizeField<'static>, covers = [Default], drive = Drive::Never)]
fn size_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    BrushSizeField::new("Size", Bind::bound(data.value("size", SIZE)), MAX_SIZE).restore(ORIGINAL).show(ui).into()
}
