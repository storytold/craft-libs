//! Stories of the scope panel and its parts.
//!
//! Every story uses fakes: histograms generated from a seed (RGB and CMYK modes), a vectorscope counted from generated
//! pixels and a generated picture for the clipping overlay. Nothing runs in the background: every count is made in the
//! frame that shows it.

use super::zones::ZONE;
use super::{
    COLORS_CHANNEL, ChannelOverlay, ClipWarning, ClippingFrame, ClippingMode, ClippingOverlay, ClippingWarnings, HistogramSet, ScopeChannel,
    ScopeFrame, ScopePanel, ScopePanelState, ScopeSettings, ScopeSource, ScopeView, ScopeZoneEdit, ToneZone, ToneZones, VectorscopeFrame,
    VectorscopePlot, ZoneChange,
};
use crate::scopes::{ChannelMask, ClippingMask, HUES, SATURATIONS, Vectorscope, hue_saturation};
use crate::ui::histogram::ChannelColor;
use crate::ui::mock_image::{MockImage, StoryView, story_canvas};
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::primitives::Revision;
use craft_ui::runtime::env::{Label, Message};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, vec2};

/// A generator of repeatable noise in 0..1 (a linear congruential step).
fn noise(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*state >> 8) as f32 / 16_777_216.0
}

/// A hill of counts around `centre` with a smaller one 60 levels up, and noise.
fn hill(seed: u32, centre: f32, width: f32) -> Vec<u32> {
    let mut state = seed;
    (0..256)
        .map(|level| {
            let x = level as f32;
            let main = (-((x - centre) / width).powi(2) / 2.0).exp();
            let side = 0.5 * (-((x - centre - 60.0) / (width * 1.6)).powi(2) / 2.0).exp();
            ((main + side) * 3000.0 * (0.85 + 0.3 * noise(&mut state))) as u32 + 8
        })
        .collect()
}

/// The colour mode of a story.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Rgb,
    Cmyk,
}

/// One generated channel, owned.
struct Curve {
    key: &'static str,
    label: &'static str,
    ink: ChannelColor,
    bins: Vec<u32>,
    color_channel: bool,
}

/// The mode's channels in list order, the composite first and Luminosity last. `shift` moves every hill, which is how
/// the adjusted histogram differs from the original.
fn curves(mode: Mode, shift: f32) -> Vec<Curve> {
    let colours: Vec<(&'static str, &'static str, ChannelColor, f32)> = match mode {
        Mode::Rgb => {
            vec![("red", "Red", ChannelColor::Red, 150.0), ("green", "Green", ChannelColor::Green, 110.0), ("blue", "Blue", ChannelColor::Blue, 70.0)]
        }
        Mode::Cmyk => vec![
            ("cyan", "Cyan", ChannelColor::Cyan, 70.0),
            ("magenta", "Magenta", ChannelColor::Magenta, 100.0),
            ("yellow", "Yellow", ChannelColor::Yellow, 130.0),
            ("black", "Black", ChannelColor::Neutral, 40.0),
        ],
    };
    let mut channels: Vec<Curve> = colours
        .iter()
        .zip(1u32..)
        .map(|((key, label, ink, centre), seed)| Curve {
            key,
            label,
            ink: *ink,
            bins: hill(seed, (centre + shift).clamp(0.0, 255.0), 34.0),
            color_channel: true,
        })
        .collect();
    let count = channels.len() as u32;
    let composite: Vec<u32> = (0..256).map(|level| channels.iter().map(|c| c.bins[level]).sum::<u32>() / count).collect();
    let (key, label) = if mode == Mode::Rgb { ("rgb", "RGB") } else { ("cmyk", "CMYK") };
    channels.insert(0, Curve { key, label, ink: ChannelColor::Neutral, bins: composite, color_channel: false });
    channels.push(Curve {
        key: "luminosity",
        label: "Luminosity",
        ink: ChannelColor::Neutral,
        bins: hill(9, (105.0 + shift).clamp(0.0, 255.0), 46.0),
        color_channel: false,
    });
    channels
}

fn scope_channels(curves: &[Curve]) -> Vec<ScopeChannel<'_>> {
    curves.iter().map(|c| ScopeChannel { key: c.key, label: c.label, ink: c.ink, bins: &c.bins, color_channel: c.color_channel }).collect()
}

fn overlay_of(mode: Mode) -> ChannelOverlay {
    match mode {
        Mode::Rgb => ChannelOverlay::Light,
        Mode::Cmyk => ChannelOverlay::Ink,
    }
}

/// What a histogram story adds to the plain panel.
#[derive(Clone, Copy, Default)]
struct Look {
    colors: bool,
    original: bool,
    stale: bool,
    sources: bool,
}

fn view(view: ScopeView) -> ScopeSettings {
    ScopeSettings { view, ..ScopeSettings::default() }
}

/// A histogram panel over generated counts; its state lives in the story's data under `name`.
fn histogram(ui: &mut Ui, data: &mut StoryData, name: &'static str, mode: Mode, settings: ScopeSettings, look: Look) -> StoryOutput {
    let state = data.value(name, ScopePanelState { settings, ..ScopePanelState::default() });
    let adjusted = curves(mode, if look.original { 25.0 } else { 0.0 });
    let before = curves(mode, 0.0);
    let channels = scope_channels(&adjusted);
    let originals = scope_channels(&before);
    let mut set = HistogramSet::new(&channels).stale(look.stale);
    if look.original {
        set = set.original(Some(&originals));
    }
    if look.colors {
        set = set.colors(overlay_of(mode));
    }
    let sources = [ScopeSource { key: "image", label: "Entire Image" }, ScopeSource { key: "layer", label: "Selected Layer" }];
    let sources: &[ScopeSource<'_>] = if look.sources { &sources } else { &[] };
    let frame = ScopeFrame::histogram(set);
    ScopePanel::new(state).frame(&frame).sources(sources).show(ui).into()
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    histogram(ui, data, "compact", Mode::Rgb, view(ScopeView::Compact), Look::default())
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn expanded_with_statistics(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    histogram(ui, data, "expanded", Mode::Rgb, view(ScopeView::Expanded), Look { sources: true, ..Look::default() })
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn all_channels(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    histogram(ui, data, "all-channels", Mode::Cmyk, view(ScopeView::AllChannels), Look::default())
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn colors_light(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let colors = ScopeSettings { channel: COLORS_CHANNEL.to_owned(), ..view(ScopeView::Expanded) };
    histogram(ui, data, "colors-light", Mode::Rgb, colors, Look { colors: true, ..Look::default() })
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn colors_ink(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let colors = ScopeSettings { channel: COLORS_CHANNEL.to_owned(), ..view(ScopeView::Expanded) };
    histogram(ui, data, "colors-ink", Mode::Cmyk, colors, Look { colors: true, ..Look::default() })
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn stale_with_refresh(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = ScopeSettings { show_channels_in_color: true, ..view(ScopeView::Expanded) };
    histogram(ui, data, "stale", Mode::Rgb, settings, Look { original: true, stale: true, ..Look::default() })
}

fn zone_text(key: &str, value: f32) -> String {
    if key == "exposure" { format!("{value:+.2}") } else { format!("{value:+.0}") }
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn tone_zones(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = *data.value("zone-values", [0.0f32; 5]);
    let state = data.value("zones", ScopePanelState { settings: view(ScopeView::Expanded), ..ScopePanelState::default() });
    let curves = curves(Mode::Rgb, 0.0);
    let channels = scope_channels(&curves);
    let frame = ScopeFrame::histogram(HistogramSet::new(&channels));
    let zones = ToneZone::CAMERA_RAW;
    let texts: Vec<Label> = zones.iter().zip(values).map(|(zone, value)| Label::from(zone_text(zone.key, value))).collect();
    ScopePanel::new(state).frame(&frame).zones(&zones, &texts).show(ui).into()
}

/// Generated pixels in a skin cluster, a sky and some foliage, with a few saturated ones.
fn scope_pixels() -> Vec<[f32; 4]> {
    let mut state = 5;
    let mut around = |centre: [f32; 3], spread: f32| {
        let mut next = |c: f32| (c + (noise(&mut state) - 0.5) * spread).clamp(0.0, 1.0);
        [next(centre[0]), next(centre[1]), next(centre[2]), 1.0]
    };
    let mut pixels = Vec::new();
    for _ in 0..1500 {
        pixels.push(around([0.88, 0.66, 0.54], 0.12));
    }
    for _ in 0..800 {
        pixels.push(around([0.35, 0.55, 0.9], 0.2));
    }
    for _ in 0..600 {
        pixels.push(around([0.3, 0.65, 0.25], 0.25));
    }
    for _ in 0..100 {
        pixels.push(around([0.95, 0.1, 0.1], 0.1));
    }
    pixels
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn vectorscope_with_skin_tone(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = ScopeSettings { vectorscope: true, skin_tone: true, ..view(ScopeView::Compact) };
    let state = data.value("vectorscope", ScopePanelState { settings, ..ScopePanelState::default() });
    let curves = curves(Mode::Rgb, 0.0);
    let channels = scope_channels(&curves);
    let counts = Vectorscope::from_rgba(&scope_pixels());
    let (hue, saturation) = hue_saturation([0.88, 0.66, 0.54]);
    let scope = VectorscopeFrame { counts: &counts.counts, hues: HUES, saturations: SATURATIONS, hover: Some([hue, saturation]) };
    let frame = ScopeFrame::histogram(HistogramSet::new(&channels)).vectorscope(scope);
    ScopePanel::new(state).frame(&frame).show(ui).into()
}

#[story(VectorscopePlot<'static>, covers = [Default], drive = Drive::Never)]
fn vectorscope_alone(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let counts = Vectorscope::from_rgba(&scope_pixels());
    let scope = VectorscopeFrame { counts: &counts.counts, hues: HUES, saturations: SATURATIONS, hover: None };
    VectorscopePlot::new("Vectorscope", &scope).red_right(true).show(ui).into()
}

const IMAGE_SIZE: [usize; 2] = [96, 64];

/// A generated picture in straight RGBA whose contrast is stretched until both ends clip: shadows in blue, highlights
/// in red.
fn clipped_pixels(image: &MockImage) -> Vec<[f32; 4]> {
    let [width, height] = IMAGE_SIZE;
    (0..width * height)
        .map(|i| {
            let [r, g, b, a] = image.pixel((i % width) as f64 + 0.5, (i / width) as f64 + 0.5);
            let stretch = |c: u8| ((f32::from(c) / 255.0 - 0.5) * 2.4 + 0.5).clamp(0.0, 1.0);
            [stretch(r), stretch(g), stretch(b), f32::from(a) / 255.0]
        })
        .collect()
}

/// The counts of one channel of `pixels`.
fn counted(pixels: &[[f32; 4]], channel: usize) -> Vec<u32> {
    let mut bins = vec![0u32; 256];
    for pixel in pixels {
        bins[(pixel[channel] * 255.0).round() as usize] += 1;
    }
    bins
}

#[story(ScopePanel<'static>, covers = [Default], drive = Drive::Never)]
fn clipping_warnings_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = ScopeSettings { shadow_warning: true, highlight_warning: true, ..view(ScopeView::Compact) };
    let state = data.value("clipping", ScopePanelState { settings, ..ScopePanelState::default() });
    let pixels = clipped_pixels(&MockImage::new([IMAGE_SIZE[0] as u32, IMAGE_SIZE[1] as u32]));
    let Some(mask) = ClippingMask::from_rgba(IMAGE_SIZE, &pixels) else { return StoryOutput::default() };
    let curves: Vec<Curve> = [
        ("rgb", "RGB", ChannelColor::Neutral),
        ("red", "Red", ChannelColor::Red),
        ("green", "Green", ChannelColor::Green),
        ("blue", "Blue", ChannelColor::Blue),
    ]
    .into_iter()
    .enumerate()
    .map(|(channel, (key, label, ink))| {
        let bins = if channel == 0 {
            let parts: Vec<Vec<u32>> = (0..3).map(|c| counted(&pixels, c)).collect();
            (0..256).map(|level| parts.iter().map(|p| p[level]).sum::<u32>() / 3).collect()
        } else {
            counted(&pixels, channel - 1)
        };
        Curve { key, label, ink, bins, color_channel: channel > 0 }
    })
    .collect();
    let channels = scope_channels(&curves);
    let any = mask.any();
    let frame = ScopeFrame::histogram(HistogramSet::new(&channels)).clipping(ClippingFrame { shadows: any.shadows, highlights: any.highlights });
    ScopePanel::new(state).frame(&frame).show(ui).into()
}

#[story(ClipWarning<'static>, covers = [Default], drive = Drive::Activate)]
fn clip_warning(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let on = data.value("on", true);
    ClipWarning::new("Highlight Clipping Warning (O)", Bind::bound(on), ChannelMask::R).show(ui).into()
}

#[story(ClipWarning<'static>, covers = [Mixed], drive = Drive::Never)]
fn clip_warning_that_differs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let on = data.value("mixed", Mixed::<bool>::Mixed);
    ClipWarning::new("Highlight Clipping Warning (O)", Bind::bound_mixed(on), ChannelMask::R).show(ui).into()
}

#[story(ClipWarning<'static>, covers = [Invalid], drive = Drive::Never)]
fn clip_warning_that_is_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let on = data.value("refused", false);
    let refuse = |_: &bool| Validity::Refuse(Message::new("Not available"));
    ClipWarning::new("Highlight Clipping Warning (O)", Bind::bound(on).check(refuse), ChannelMask::R).show(ui).into()
}

/// The edit the zone stories start from.
fn first_edit() -> ScopeZoneEdit {
    ScopeZoneEdit { zone: 0, key: ToneZone::CAMERA_RAW[0].key, change: ZoneChange::Step(0) }
}

/// The zones of a story over a picture, the value text of each, and the story's canvas.
fn zones_story(ui: &mut Ui, mut value: Bind<'_, ScopeZoneEdit>) -> StoryOutput {
    let zones = ToneZone::CAMERA_RAW;
    let texts: Vec<Label> = zones.iter().map(|zone| Label::from(zone_text(zone.key, 0.0))).collect();
    let image = MockImage::default();
    story_canvas(ui, Some(&image), StoryView::Fit, |ui, rect| {
        ToneZones::new("Tone zones", &zones, &texts).value(value.reborrowed()).show_at(ui, rect).into()
    })
}

#[story(ToneZones<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ZONE.at(1)).toward(vec2(1.0, 0.0))))]
fn zones_over_a_plot(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let edit = data.value("edit", first_edit());
    zones_story(ui, Bind::bound(edit))
}

#[story(ToneZones<'static>, covers = [Mixed], drive = Drive::Never)]
fn zones_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let edit = data.value("mixed", Mixed::<ScopeZoneEdit>::Mixed);
    zones_story(ui, Bind::bound_mixed(edit))
}

#[story(ToneZones<'static>, covers = [Invalid], drive = Drive::Never)]
fn zones_that_are_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let edit = data.value("refused", first_edit());
    let refuse = |_: &ScopeZoneEdit| Validity::Refuse(Message::new("Not available"));
    zones_story(ui, Bind::bound(edit).check(refuse))
}

#[story(ClippingOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn clipping_overlay(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let image = MockImage::new([IMAGE_SIZE[0] as u32, IMAGE_SIZE[1] as u32]);
    let pixels = clipped_pixels(&image);
    let Some(mask) = ClippingMask::from_rgba(IMAGE_SIZE, &pixels) else { return StoryOutput::default() };
    let mode = ClippingMode::Warnings(ClippingWarnings { shadows: true, highlights: true });
    story_canvas(ui, Some(&image), StoryView::Fit, |ui, rect| {
        ClippingOverlay::new("Clipping", &mask, Revision::new(1), mode).show_at(ui, rect).into()
    })
}
