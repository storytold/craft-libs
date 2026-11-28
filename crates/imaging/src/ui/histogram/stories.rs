//! Stories of the histogram: one per state the plot shows.

use super::plot::PLOT;
use super::{ChannelColor, Histogram, HistogramData, HistogramScale, LevelRange, StaleMark};
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

/// The bin count of an 8-bit channel.
const LEVELS: usize = 256;

/// A bell curve of `count` pixels around `centre` with spread `width`, over every level.
fn bell(centre: f32, width: f32, count: f32) -> Vec<u32> {
    (0..LEVELS).map(|i| (count * (-((i as f32 - centre) / width).powi(2) / 2.0).exp()) as u32).collect()
}

/// Two humps, a shadow tone and a highlight tone.
fn humps() -> Vec<u32> {
    bell(90.0, 30.0, 9000.0).iter().zip(bell(190.0, 14.0, 6000.0)).map(|(a, b)| a + b).collect()
}

#[story(Histogram<'static>, covers = [Default], drive = Drive::Never)]
fn tone(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let channels = [
        ("Neutral", ChannelColor::Neutral),
        ("Red", ChannelColor::Red),
        ("Green", ChannelColor::Green),
        ("Blue", ChannelColor::Blue),
        ("Cyan", ChannelColor::Cyan),
        ("Magenta", ChannelColor::Magenta),
        ("Yellow", ChannelColor::Yellow),
    ];
    let channel = *data.choice("channel", ChannelColor::Red, &channels);
    let stale = *data.value("stale", false);
    let marker = *data.value("marker", true);
    let bins = HistogramData::tone(&humps(), channel);
    let mut histogram = Histogram::new("Histogram", &bins).value("Red channel");
    if marker {
        histogram = histogram.marker(128.0);
    }
    if stale {
        histogram = histogram.stale(StaleMark::Mark("".into()));
    }
    histogram.show(ui).into()
}

#[story(Histogram<'static>, covers = [], drive = Drive::Never)]
fn rgb(ui: &mut Ui, story: &mut StoryData) -> StoryOutput {
    let scale = *story.choice("scale", HistogramScale::Typical, &[("Peak", HistogramScale::Peak), ("Typical", HistogramScale::Typical)]);
    let marker = *story.value("marker", true);
    let (mut r, mut g, mut b) = (bell(100.0, 36.0, 9000.0), bell(120.0, 40.0, 8000.0), bell(150.0, 30.0, 7000.0));
    // Clipped pixels at both ends, far above the interior.
    for (bins, edges) in [(&mut r, (60_000, 9000)), (&mut g, (30_000, 2000)), (&mut b, (20_000, 40_000))] {
        bins[0] = edges.0;
        bins[LEVELS - 1] = edges.1;
    }
    let data = HistogramData::rgb([&r, &g, &b]).scale(scale);
    let histogram = Histogram::new("RGB histogram", &data).value("Red, green and blue channels");
    if marker { histogram.marker(100.0) } else { histogram }.show(ui).into()
}

#[story(Histogram<'static>, covers = [], drive = Drive::Never)]
fn inks(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (c, m, y, k) = (bell(70.0, 30.0, 9000.0), bell(110.0, 36.0, 8000.0), bell(150.0, 30.0, 7000.0), bell(40.0, 20.0, 6000.0));
    let channels: [(&[u32], ChannelColor); 4] =
        [(&c, ChannelColor::Cyan), (&m, ChannelColor::Magenta), (&y, ChannelColor::Yellow), (&k, ChannelColor::Neutral)];
    let data = HistogramData::channels(&channels);
    Histogram::new("Ink histogram", &data).value("Cyan, magenta, yellow and black inks").show(ui).into()
}

#[story(Histogram<'static>, covers = [], drive = Drive::Never)]
fn outline(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (data, original) =
        (HistogramData::tone(&humps(), ChannelColor::Red), HistogramData::tone(&bell(120.0, 50.0, 12_000.0), ChannelColor::Neutral));
    Histogram::new("Histogram", &data).outline(&original).value("Red channel with the original outlined").bordered().show(ui).into()
}

#[story(Histogram<'static>, covers = [], drive = Drive::Never)]
fn markers(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let data = HistogramData::tone(&humps(), ChannelColor::Neutral);
    let points = [(12.0, "0".into()), (128.0, "1.00".into()), (243.0, "255".into())];
    Histogram::new("Histogram", &data).markers(&points).value("Levels points").bordered().show(ui).into()
}

#[story(Histogram<'static>, covers = [], drive = Drive::Drag(Grip::new(PLOT)))]
fn selection(ui: &mut Ui, story: &mut StoryData) -> StoryOutput {
    let data = HistogramData::tone(&humps(), ChannelColor::Neutral);
    let range = story.value("range", Some(LevelRange::new(60.0, 120.0)));
    Histogram::new("Histogram", &data).selection(Bind::bound(range)).value("Selected levels").show(ui).into()
}

#[story(Histogram<'static>, covers = [], drive = Drive::Never)]
fn stale_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let data = HistogramData::tone(&humps(), ChannelColor::Neutral);
    Histogram::new("Histogram", &data).stale(StaleMark::Button("Recalculate".into())).show(ui).into()
}

#[story(Histogram<'static>, covers = [Mixed], drive = Drive::Never)]
fn selection_that_differs(ui: &mut Ui, story: &mut StoryData) -> StoryOutput {
    let data = HistogramData::tone(&humps(), ChannelColor::Neutral);
    let range = story.value("mixed", Mixed::<Option<LevelRange>>::Mixed);
    Histogram::new("Histogram", &data).selection(Bind::bound_mixed(range)).show(ui).into()
}

#[story(Histogram<'static>, covers = [Invalid], drive = Drive::Never)]
fn selection_that_is_refused(ui: &mut Ui, story: &mut StoryData) -> StoryOutput {
    let data = HistogramData::tone(&humps(), ChannelColor::Neutral);
    let refuse_any = |range: &Option<LevelRange>| if range.is_some() { Validity::Refuse(Message::new("Not available")) } else { Validity::Ok };
    let range = story.value("refused", Some(LevelRange::new(60.0, 120.0)));
    Histogram::new("Histogram", &data).selection(Bind::bound(range).check(refuse_any)).show(ui).into()
}
