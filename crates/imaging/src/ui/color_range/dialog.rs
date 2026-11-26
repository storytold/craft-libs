//! The Color Range dialog's body.

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft_behavior::{CursorBadge, EditEvent, EditEvents, IconId, Mixed};
use craft_ui::buttons::Button;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::choice::{Checkbox, Radio, RadioGroup, Segmented};
use craft_ui::contract::{
    Bind, CanvasGesture, EditResponse, Entry, FieldLayout, NoValue, Outcome, Pattern, Props, Sampler as _, Show as _, ShowContainer as _, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, FieldGroup, SliderField};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::patterns::form_dialogs::{PreviewPane, PreviewState, PreviewView};
use craft_ui::pickers::Select;
use craft_ui::primitives::{IconSpec, TextureCache};
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::runtime::pointer;
use craft_ui::style::TypeRole;
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Response, Ui, Vec2};

use super::{ColorRangeSelect, ColorRangeValue, ColorRangeWords, SampleTool};
use crate::selection::PreviewMode;
use crate::ui::selection::SelectionWords;

/// What the Color Range body reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorRangeOutput {
    /// The edit of the value.
    pub edit: EditResponse<ColorRangeValue>,
    /// The body's response.
    pub response: Response,
    /// The sample tool picked this pass.
    pub tool: Option<SampleTool>,
    /// The view picked this pass: `true` for Image, `false` for Selection.
    pub show_image: Option<bool>,
    /// The Selection Preview mode picked this pass.
    pub preview: Option<PreviewMode>,
}

impl Outcome for ColorRangeOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Color Range body: the select, the sliders, the selection preview and the sampling tools.
///
/// The sample tool, the Selection / Image view and the Selection Preview mode are dialog state beside the value: the
/// caller passes them in and takes the changes from the output.
///
/// A press (and each pass the held pointer moves) in the preview samples a colour while Sampled Colors is selected.
/// Each pass applies only the press phase of its own input, so a second pass in the same frame samples nothing.
#[must_use = "show the dialog body with `show`"]
#[derive(Pattern, Props)]
pub struct ColorRangeDialog<'a> {
    label: Label,
    value: Bind<'a, ColorRangeValue>,
    /// The mask the preview shows.
    #[prop]
    mask: Option<&'a dyn RasterSource>,
    /// The picture the preview shows.
    #[prop]
    image: Option<&'a dyn RasterSource>,
    /// The integer proxy factor `k`, halved by integer division.
    #[prop]
    picture_scale: u32,
    /// The document's size in pixels, which a sample is clamped to.
    #[prop]
    document: [u32; 2],
    /// An error to show under the preview.
    #[prop]
    error: Option<Label>,
    /// The canvas the sampling presses reach.
    #[prop]
    canvas: Option<egui::Id>,
    /// The sample tool.
    #[prop]
    tool: SampleTool,
    /// Whether the Image view is chosen rather than the Selection view.
    #[prop]
    show_image: bool,
    /// The Selection Preview mode.
    #[prop]
    preview_mode: PreviewMode,
}

/// The edits one pass collects: each is lifted onto the whole value, the later ones from what the earlier made.
struct Pass {
    current: ColorRangeValue,
    events: EditEvents<ColorRangeValue>,
}

impl Pass {
    fn new(now: ColorRangeValue) -> Self {
        Pass { current: now, events: EditEvents::new() }
    }

    /// A control's `events` of one field as events of the value, `set` writing the field into the value.
    fn lift<T: Clone + 'static>(&mut self, events: EditEvents<T>, set: impl Fn(&mut ColorRangeValue, T)) {
        let base = self.current.clone();
        let lifted = events.map(move |x| {
            let mut next = base.clone();
            set(&mut next, x);
            next
        });
        if let Some(value) = lifted.value() {
            self.current = value.clone();
        }
        for event in lifted {
            self.events.push(event);
        }
    }

    /// A one-step change the body makes itself (a sample, Clear Samples): `after` as one `Set` named `label`.
    fn set(&mut self, label: &Label, after: ColorRangeValue) {
        if after == self.current {
            return;
        }
        let event = EditEvent::set(Mixed::Same(self.current.clone()), after.clone(), Arc::from(label.as_str()), None);
        self.current = after;
        self.events.push(event);
    }

    /// A whole-number slider row over `range`, enabled or not.
    fn slider(&mut self, ui: &mut Ui, label: &Label, value: f64, scale: Scale, enabled: bool, set: fn(&mut ColorRangeValue, f64)) {
        let mut field = SliderField::new(label.clone(), Bind::given(value), scale.range).step(1.0).enabled(enabled);
        if scale.percent {
            field = field.unit_kind(UnitKind::Percent);
        }
        let events = field.show(ui).edit.events;
        self.lift(events, move |value, x| set(value, x.round()));
    }
}

/// How a slider row reads: its range and whether the value shows as a percentage.
struct Scale {
    range: RangeInclusive<f64>,
    percent: bool,
}

impl Scale {
    fn percent(range: RangeInclusive<f64>) -> Self {
        Scale { range, percent: true }
    }

    fn plain(range: RangeInclusive<f64>) -> Self {
        Scale { range, percent: false }
    }
}

/// What a pass of the body gathered for the output.
struct Collected {
    events: EditEvents<ColorRangeValue>,
    tool: Option<SampleTool>,
    show_image: Option<bool>,
    preview: Option<PreviewMode>,
}

/// The texture of `source` at its own size, kept by the texture cache under the source's revision. `kind` keeps the
/// mask and the image apart.
fn texture(ctx: &egui::Context, source: &dyn RasterSource, kind: &'static str) -> SizedTexture {
    let size = source.size();
    let (width, height) = (size.width as usize, size.height as usize);
    let id = TextureCache::texture(ctx, (kind, source.revision(), size.width, size.height), || {
        let mut image = ColorImage::filled([width, height], Color32::TRANSPARENT);
        source.read(PixelRect::of_size(size), 1, &mut image);
        image
    });
    SizedTexture::new(id, Vec2::new(width as f32, height as f32))
}

/// The Select's entries, in Photoshop's order.
fn select_entries(words: &ColorRangeWords) -> Vec<Entry<ColorRangeSelect>> {
    [
        (ColorRangeSelect::Sampled, &words.sampled),
        (ColorRangeSelect::Reds, &words.reds),
        (ColorRangeSelect::Yellows, &words.yellows),
        (ColorRangeSelect::Greens, &words.greens),
        (ColorRangeSelect::Cyans, &words.cyans),
        (ColorRangeSelect::Blues, &words.blues),
        (ColorRangeSelect::Magentas, &words.magentas),
        (ColorRangeSelect::Highlights, &words.highlights),
        (ColorRangeSelect::Midtones, &words.midtones),
        (ColorRangeSelect::Shadows, &words.shadows),
        (ColorRangeSelect::SkinTones, &words.skin_tones),
        (ColorRangeSelect::OutOfGamut, &words.out_of_gamut),
    ]
    .into_iter()
    .map(|(select, name)| Entry::new(select, name.clone()))
    .collect()
}

impl<'a> ColorRangeDialog<'a> {
    /// The body named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, ColorRangeValue>) -> Self {
        ColorRangeDialog {
            label: label.into(),
            value,
            mask: None,
            image: None,
            picture_scale: 1,
            document: [0, 0],
            error: None,
            canvas: None,
            tool: SampleTool::default(),
            show_image: false,
            preview_mode: PreviewMode::default(),
        }
    }

    /// Draws the body.
    pub fn show(self, ui: &mut Ui) -> ColorRangeOutput {
        let ColorRangeDialog { label, mut value, mask, image, picture_scale, document, error, canvas, tool, show_image, preview_mode } = self;
        let env = Env::of(ui);
        let words = env.words::<ColorRangeWords>();
        let preview_words = env.words::<SelectionWords>();
        // Holding ⌘ (Ctrl off macOS; `Mods` folds Ctrl into `cmd` there) shows the other view while held: view only,
        // `show_image` does not change.
        let swapped = pointer::mods(ui.ctx()).cmd;
        let image_shown = show_image != swapped;
        let source = if image_shown { image } else { mask };
        let state = match (&error, source) {
            (Some(error), _) => PreviewState::Failed(error.clone()),
            (None, Some(source)) => PreviewState::Ready(texture(ui.ctx(), source, if image_shown { "image" } else { "mask" })),
            (None, None) => PreviewState::Pending(None),
        };
        let k = picture_scale.max(1);
        let mut collected = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = match value.reborrow().read(cx) {
                Some(Mixed::Same(now)) => now,
                _ => ColorRangeValue::default(),
            };
            let c = now.controls();
            if let Some(canvas) = canvas {
                cx.keep_view_open(canvas);
                if c.sampling {
                    cx.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
                }
            }
            let mut pass = Pass::new(now.clone());
            let (mut picked_tool, mut picked_view, mut picked_preview) = (None, None, None);

            Field::new(words.select.clone()).show(ui, |ui, _| {
                let entries = select_entries(&words);
                let events = Select::new(words.select.clone(), Bind::given(now.select), &entries).show(ui).events;
                pass.lift(events, |value, select| value.select = select);
            });
            let events = Checkbox::new(Bind::given(now.localized)).label(words.localized.clone()).enabled(c.sampling).show(ui).events;
            pass.lift(events, |value, on| value.localized = on);
            if c.tonal {
                pass.slider(ui, &words.fuzziness, now.tone_fuzziness, Scale::percent(0.0..=100.0), c.fuzziness, |v, x| v.tone_fuzziness = x);
            } else {
                pass.slider(ui, &words.fuzziness, now.fuzziness, Scale::plain(0.0..=200.0), c.fuzziness, |v, x| v.fuzziness = x);
            }
            if c.range {
                pass.slider(ui, &words.range, now.range, Scale::percent(0.0..=100.0), true, |v, x| v.range = x);
            }
            if c.tonal {
                match now.select {
                    ColorRangeSelect::Shadows => {
                        pass.slider(ui, &words.shadows_up_to, now.shadows_level, Scale::plain(0.0..=255.0), true, |v, x| v.shadows_level = x);
                    }
                    ColorRangeSelect::Highlights => {
                        pass.slider(ui, &words.highlights_from, now.highlights_level, Scale::plain(0.0..=255.0), true, |v, x| v.highlights_level = x);
                    }
                    _ => {
                        pass.slider(ui, &words.midtones_from, now.midtones_low, Scale::plain(0.0..=255.0), true, |v, x| v.midtones_low = x);
                        pass.slider(ui, &words.midtones_to, now.midtones_high, Scale::plain(0.0..=255.0), true, |v, x| v.midtones_high = x);
                    }
                }
            }

            Row::new().show(ui, |ui, _| {
                let pane = PreviewPane::state(words.preview.clone(), state.clone(), &PreviewView::fixed()).aspect(1.0).click_centres(false).show(ui);
                if c.sampling
                    && let Some(p) = pane.pressed
                {
                    // The picture is a proxy reduced `k` times: the sample is the centre of the proxy pixel in document pixels.
                    let at = [
                        f64::from(p.x.floor().max(0.0)) * f64::from(k) + f64::from(k / 2),
                        f64::from(p.y.floor().max(0.0)) * f64::from(k) + f64::from(k / 2),
                    ];
                    let mods = pointer::mods(ui.ctx());
                    let mut next = pass.current.clone();
                    next.samples.pick(at, document, tool, mods);
                    pass.set(&words.eyedropper, next);
                }
                Stack::new().fill_width().show(ui, |ui, _| {
                    Stack::new().enabled(c.sampling).show(ui, |ui, _| {
                        let tools = [
                            Entry::new(SampleTool::Sample, words.eyedropper.clone()).icon(IconSpec::svg(IconId::new("eyedropper"))),
                            Entry::new(SampleTool::Add, words.add_to_sample.clone()).icon(IconSpec::mark(CursorBadge::Add)),
                            Entry::new(SampleTool::Subtract, words.subtract_from_sample.clone()).icon(IconSpec::mark(CursorBadge::Subtract)),
                        ];
                        picked_tool = Segmented::new(words.sample_tools.clone(), Bind::given(tool), &tools).show(ui).value().copied();
                        let (added, removed) = (now.samples.add.len(), now.samples.subtract.len());
                        let count = |n: usize| {
                            let n_text = n.to_string();
                            if n == 1 { words.one_sample.fill(&[("n", n_text.as_str())]) } else { words.many_samples.fill(&[("n", n_text.as_str())]) }
                        };
                        let text = match (added, removed) {
                            (0, 0) => words.sample_foreground.as_str().to_owned(),
                            (added, 0) => count(added),
                            (added, removed) => {
                                let (added_text, removed_text) = (count(added), removed.to_string());
                                words.with_subtracted.fill(&[("added", added_text.as_str()), ("removed", removed_text.as_str())])
                            }
                        };
                        Text::new(text).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                        if added + removed > 0 && clicked(&Button::new(words.clear_samples.clone()).show(ui)) {
                            let mut next = pass.current.clone();
                            next.samples.clear();
                            pass.set(&words.clear_samples, next);
                        }
                    });
                    let events = Checkbox::new(Bind::given(now.invert)).label(words.invert.clone()).show(ui).events;
                    pass.lift(events, |value, on| value.invert = on);
                    Field::new(words.selection.clone()).layout(FieldLayout::Above).id_salt("view").show(ui, |ui, _| {
                        let group = RadioGroup::new(words.selection.clone(), Bind::given(show_image)).show(ui, |ui, _| {
                            Row::new().show(ui, |ui, _| {
                                Radio::new(false).label(words.selection.clone()).show(ui);
                                Radio::new(true).label(words.image.clone()).show(ui);
                            });
                        });
                        picked_view = group.output.value().copied();
                    });
                    Field::new(words.selection_preview.clone()).layout(FieldLayout::Above).show(ui, |ui, _| {
                        let modes = [
                            Entry::new(PreviewMode::None, words.none.clone()),
                            Entry::new(PreviewMode::Grayscale, preview_words.grayscale.clone()),
                            Entry::new(PreviewMode::BlackMatte, preview_words.black_matte.clone()),
                            Entry::new(PreviewMode::WhiteMatte, preview_words.white_matte.clone()),
                            Entry::new(PreviewMode::QuickMask, preview_words.quick_mask.clone()),
                        ];
                        picked_preview = Select::new(words.selection_preview.clone(), Bind::given(preview_mode), &modes).show(ui).value().copied();
                    });
                });
            });
            for event in &pass.events {
                let _ = value.reborrow().write(cx, event);
            }
            collected = Some(Collected {
                events: pass.events,
                tool: picked_tool.filter(|t| *t != tool),
                show_image: picked_view.filter(|v| *v != show_image),
                preview: picked_preview.filter(|m| *m != preview_mode),
            });
        });
        let response = shown.output;
        match collected {
            Some(Collected { events, tool, show_image, preview }) => {
                ColorRangeOutput { edit: EditResponse::lifted(response.clone(), events, None), response, tool, show_image, preview }
            }
            // The body did not run (a parent's measuring pass reuses what it measured): nothing was edited.
            None => ColorRangeOutput {
                edit: EditResponse::lifted(response.clone(), EditEvents::new(), None),
                response,
                tool: None,
                show_image: None,
                preview: None,
            },
        }
    }
}
