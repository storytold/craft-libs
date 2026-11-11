//! `Watermark`: the watermark rows of an export dialog: the style (text or graphic), the text or the image, the
//! orientation, the position, the rotation, the size, the opacity, the insets and the shadow.

use super::host_requests::HostRequests;
use crate::buttons::Button;
use crate::choice::{Checkbox, Segmented};
use crate::runtime::requests::{IMAGE_EXTENSIONS, PathKind, PathRequest};

use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};

use crate::form::{Field, FieldGroup, SliderField, SliderFieldLayout};

use crate::layout_components::Row;
use crate::pickers::{Anchor9, ReferencePoint, Select};

use crate::runtime::env::{Env, Label, UnitKind, Units};

use crate::runtime::memory::Remember as _;
use crate::text::TextField;
use craft_behavior::{EditEvent, Mixed};

use egui::{Response, Ui};

use serde::{Deserialize, Serialize};

use std::ops::RangeInclusive;
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// The key of the image request the Choose button files.
pub const WATERMARK_IMAGE_KEY: &str = "watermark";

/// The rotations a watermark takes, in degrees.
pub const ROTATIONS: [u16; 4] = [0, 90, 180, 270];

/// The range of the size, in percent of the image's short edge.
const SIZE_RANGE: RangeInclusive<f64> = 1.0..=15.0;
/// The range of the opacity, in percent.
const OPACITY_RANGE: RangeInclusive<f64> = 5.0..=100.0;
/// The range of an inset, in percent of the image's short edge.
const INSET_RANGE: RangeInclusive<f64> = 0.0..=40.0;
/// The step of the size and inset sliders.
const STEP: f64 = 0.5;

craft_ui::words! {
    /// The words of the watermark rows.
    pub struct WatermarkWords {
        style: "watermark.style" = "Style";
        text_style: "watermark.textStyle" = "Text";
        graphic_style: "watermark.graphicStyle" = "Graphic";
        text: "watermark.text" = "Watermark text";
        image: "watermark.image" = "Image";
        choose: "watermark.choose" = "Choose…";
        orientation: "watermark.orientation" = "Orientation";
        horizontal: "watermark.horizontal" = "Horizontal";
        vertical: "watermark.vertical" = "Vertical";
        position: "watermark.position" = "Position";
        rotation: "watermark.rotation" = "Rotation";
        size: "watermark.size" = "Size";
        opacity: "watermark.opacity" = "Opacity";
        inset_x: "watermark.insetX" = "Inset X";
        inset_y: "watermark.insetY" = "Inset Y";
        shadow: "watermark.shadow" = "Shadow";
    }
}

/// Text or an image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum WatermarkStyle {
    /// Watermark text.
    Text(String),
    /// The image file's path.
    Graphic(String),
}

impl WatermarkStyle {
    /// Whether the watermark is an image.
    pub fn is_graphic(&self) -> bool {
        matches!(self, WatermarkStyle::Graphic(_))
    }
}

/// A watermark. Sizes are percent of the image's short edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WatermarkSettings {
    /// Text or image.
    pub style: WatermarkStyle,
    /// Lays text out vertically instead of horizontally.
    pub vertical: bool,
    /// One of the nine anchors.
    pub position: Anchor9,
    /// 0, 90, 180 or 270 degrees.
    pub rotation: u16,
    /// 1 to 15 percent.
    pub size: f32,
    /// 5 to 100 percent.
    pub opacity: f32,
    /// 0 to 40 percent each, across and down.
    pub inset: [f32; 2],
    /// Draws a drop shadow behind the watermark.
    pub shadow: bool,
}

impl Default for WatermarkSettings {
    /// Size 3.5 %, opacity 70 %, inset 2.5 % on both axes, bottom right, shadow on.
    fn default() -> Self {
        Self {
            style: WatermarkStyle::Text(String::new()),
            vertical: false,
            position: Anchor9::BottomRight,
            rotation: 0,
            size: 3.5,
            opacity: 70.0,
            inset: [2.5, 2.5],
            shadow: true,
        }
    }
}

impl WatermarkSettings {
    /// Whether the inset across (left and right) has anything to offset: along an axis the position centres on, it
    /// has not.
    pub fn insets_across(&self) -> bool {
        !matches!(self.position, Anchor9::Top | Anchor9::Centre | Anchor9::Bottom)
    }

    /// Whether the inset down (top and bottom) has anything to offset.
    pub fn insets_down(&self) -> bool {
        !matches!(self.position, Anchor9::Left | Anchor9::Centre | Anchor9::Right)
    }
}

/// The text and the image path apart, so switching the style back restores what was typed.
#[derive(Clone, Debug, Default, PartialEq)]
struct Stash {
    text: String,
    image: String,
}

impl Stash {
    /// Remembers the content of the active style.
    fn keep(&mut self, style: &WatermarkStyle) {
        match style {
            WatermarkStyle::Text(text) => self.text.clone_from(text),
            WatermarkStyle::Graphic(path) => self.image.clone_from(path),
        }
    }

    /// The style `graphic` picks, with the content last kept for it.
    fn style(&self, graphic: bool) -> WatermarkStyle {
        if graphic { WatermarkStyle::Graphic(self.image.clone()) } else { WatermarkStyle::Text(self.text.clone()) }
    }
}

/// What the watermark rows report.
#[derive(Debug)]
#[non_exhaustive]
pub struct WatermarkOutput {
    /// The rows' group.
    pub response: Response,
    /// The watermark after the pass when a row changed it. A binding that writes through has it already; a given one
    /// applies it.
    pub value: Option<WatermarkSettings>,
    /// The Choose button was pressed: the host opens its file dialog for the path request keyed
    /// [`WATERMARK_IMAGE_KEY`] and writes the answer as [`WatermarkStyle::Graphic`].
    pub requests: HostRequests,
}

impl Outcome for WatermarkOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The watermark rows of an export dialog: Style (Text, Graphic), the text field or the image path with Choose…,
/// Orientation, Position as the 3 by 3 reference point, Rotation, Size, Opacity, Inset X and Y, and Shadow.
///
/// - The text and the image path are kept apart: switching the style restores each.
/// - Size, opacity and the insets are percent of the image's short edge. An inset along an axis the position centres
///   on is disabled.
/// - The preview stays the export dialog's.
#[must_use = "show the rows with `show`"]
#[derive(crate::contract::Pattern)]
pub struct Watermark<'a> {
    label: Label,
    value: Bind<'a, WatermarkSettings>,
}

impl<'a> Watermark<'a> {
    /// Watermark rows named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, WatermarkSettings>) -> Self {
        Watermark { label: label.into(), value }
    }

    /// Shows the rows.
    pub fn show(self, ui: &mut Ui) -> WatermarkOutput {
        let Watermark { label, mut value } = self;
        let env = Env::of(ui);
        let words = env.words::<WatermarkWords>();
        let units = env.units();
        let name: Arc<str> = label.as_str().into();
        let mut next = None;
        let mut requests = HostRequests::new();
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let before = match value.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => WatermarkSettings::default(),
            };
            let mut now = before.clone();
            let mut stash = cx.memory::<Stash>().clone();
            stash.keep(&now.style);
            rows(ui, &words, &units, &mut now, &mut stash, &mut requests);
            *cx.memory::<Stash>() = stash;
            if now != before {
                let _ = value.reborrow().write(cx, &EditEvent::set(Mixed::Same(before), now.clone(), name.clone(), None));
                next = Some(now);
            }
        });
        WatermarkOutput { response: shown.output, value: next, requests }
    }
}

/// The rows over `now`; the Choose button files its request in `requests`.
fn rows(ui: &mut Ui, words: &WatermarkWords, units: &Units, now: &mut WatermarkSettings, stash: &mut Stash, requests: &mut HostRequests) {
    let styles = [Entry::new(false, words.text_style.clone()), Entry::new(true, words.graphic_style.clone())];
    Field::new(words.style.clone()).id_salt("style").show(ui, |ui, _| {
        if let Some(graphic) = Segmented::new(words.style.clone(), Bind::given(now.style.is_graphic()), &styles).show(ui).value().cloned() {
            now.style = stash.style(graphic);
        }
    });
    match now.style.clone() {
        WatermarkStyle::Graphic(path) => {
            Field::new(words.image.clone()).id_salt("image").show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    if let Some(path) = TextField::new(words.image.clone(), Bind::given(path.clone())).id_salt("path").show(ui).value().cloned() {
                        now.style = WatermarkStyle::Graphic(path);
                    }
                    if clicked(&Button::new(words.choose.clone()).id_salt("choose").show(ui)) {
                        requests.request_path(PathRequest::new(WATERMARK_IMAGE_KEY, PathKind::open_file(&IMAGE_EXTENSIONS)));
                    }
                });
            });
        }
        WatermarkStyle::Text(text) => {
            Field::new(words.text.clone()).id_salt("text").show(ui, |ui, _| {
                if let Some(text) = TextField::new(words.text.clone(), Bind::given(text.clone())).lines(2).id_salt("text").show(ui).value().cloned() {
                    now.style = WatermarkStyle::Text(text);
                }
            });
            let orientations = [Entry::new(false, words.horizontal.clone()), Entry::new(true, words.vertical.clone())];
            Field::new(words.orientation.clone()).id_salt("orientation").show(ui, |ui, _| {
                if let Some(vertical) = Select::new(words.orientation.clone(), Bind::given(now.vertical), &orientations).show(ui).value().cloned() {
                    now.vertical = vertical;
                }
            });
        }
    }
    Field::new(words.position.clone()).id_salt("position").show(ui, |ui, _| {
        if let Some(position) = ReferencePoint::new(words.position.clone(), Bind::given(now.position)).show(ui).value().cloned() {
            now.position = position;
        }
    });
    let rotations: Vec<Entry<u16>> = ROTATIONS.iter().map(|degrees| Entry::new(*degrees, Label::from(units.degrees(f64::from(*degrees))))).collect();
    Field::new(words.rotation.clone()).id_salt("rotation").show(ui, |ui, _| {
        if let Some(rotation) = Select::new(words.rotation.clone(), Bind::given(now.rotation), &rotations).show(ui).value().cloned() {
            now.rotation = rotation;
        }
    });
    if let Some(size) = percent_row(ui, &words.size, "size", f64::from(now.size), SIZE_RANGE, true) {
        now.size = size;
    }
    if let Some(opacity) = percent_row(ui, &words.opacity, "opacity", f64::from(now.opacity), OPACITY_RANGE, true) {
        now.opacity = opacity;
    }
    let (across, down) = (now.insets_across(), now.insets_down());
    if let Some(inset) = percent_row(ui, &words.inset_x, "inset-x", f64::from(now.inset[0]), INSET_RANGE, across) {
        now.inset[0] = inset;
    }
    if let Some(inset) = percent_row(ui, &words.inset_y, "inset-y", f64::from(now.inset[1]), INSET_RANGE, down) {
        now.inset[1] = inset;
    }
    if !now.style.is_graphic()
        && let Some(on) = Checkbox::new(Bind::given(now.shadow)).label(words.shadow.clone()).id_salt("shadow").show(ui).value().cloned()
    {
        now.shadow = on;
    }
}

/// A percent row: a slider with its number field on `range`; the value it reports.
fn percent_row(ui: &mut Ui, label: &Label, key: &'static str, value: f64, range: RangeInclusive<f64>, enabled: bool) -> Option<f32> {
    let mut picked = None;
    Field::new(label.clone()).id_salt(key).enabled(enabled).show(ui, |ui, _| {
        let slider = SliderField::new(label.clone(), Bind::given(value), range.clone())
            .step(STEP)
            .unit_kind(UnitKind::Percent)
            .id_salt("slider")
            .layout(SliderFieldLayout::Inline);
        picked = slider.show(ui).value().cloned().map(|v| v as f32);
    });
    picked
}
