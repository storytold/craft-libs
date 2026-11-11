//! [`PictureFillEditor`].

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed, Value};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, FieldGroup, SliderField};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::patterns::form_dialogs::HostRequests;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::runtime::requests::{IMAGE_EXTENSIONS, PathKind, PathRequest};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use super::{PictureFill, PictureFillWords, PictureMode, TileAlignment, TileMirror};

/// The key of the picture request Insert… files: the host opens its file dialog for it, reads and stores the picture and
/// sets [`PictureFill::source`].
pub const PICTURE_FILL_KEY: &str = "picture";

/// Offsets have no limit.
const OFFSET: RangeInclusive<f64> = f64::NEG_INFINITY..=f64::INFINITY;
/// The range of the transparency, in percent.
const TRANSPARENCY: RangeInclusive<f64> = 0.0..=100.0;
/// The decimals a tile offset or scale keeps (DeckCraft's fields show one decimal).
const TILE_DECIMALS: usize = 1;
/// The decimals a stretch offset keeps.
const STRETCH_DECIMALS: usize = 0;

/// What the picture fill editor reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the editor's event and answers its requests"]
pub struct PictureFillOutput {
    /// The editor's whole area.
    pub response: Response,
    /// The changes of the fill when a row changed it, in order: a check, a menu pick or a typed value is a `Set`; a
    /// scrub or the Transparency slider is a gesture. A binding that writes through has them already.
    pub events: EditEvents<PictureFill>,
    /// Insert… was pressed: the host opens its file dialog for the path request keyed [`PICTURE_FILL_KEY`].
    pub requests: HostRequests,
}

impl PictureFillOutput {
    /// Whether anything changed this pass or the host has a request to answer: a paint chip's Picture tab redraws its
    /// preview.
    pub fn changed(&self) -> bool {
        !self.events.is_empty()
    }
}

impl Outcome for PictureFillOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The values of each mode, so the one that is off keeps its last values while the other is on.
#[derive(Clone, Debug, PartialEq)]
struct Modes {
    stretch: [f64; 4],
    tile: TileValues,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TileValues {
    offset: [f64; 2],
    scale: [f64; 2],
    alignment: TileAlignment,
    mirror: TileMirror,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            stretch: [0.0; 4],
            tile: TileValues { offset: [0.0; 2], scale: [100.0; 2], alignment: TileAlignment::default(), mirror: TileMirror::default() },
        }
    }
}

/// PowerPoint's Picture or texture fill pane, in its order:
///
/// 1. "Picture source" with the source's name and "Insert…": the press files a path request for images in
///    [`PictureFillOutput::requests`]; the host opens, reads and stores the picture and sets [`PictureFill::source`].
/// 2. "Tile picture as texture" check box (switches [`PictureMode`], keeping each mode's last values).
/// 3. Tile: Offset X and Offset Y (points), Scale X and Scale Y (percent), Alignment (9 values), Mirror type (None,
///    Horizontal, Vertical, Both). Stretch: Offset left, top, right, bottom (percent).
/// 4. "Rotate with shape" check box.
/// 5. "Transparency" slider and field, 0–100 %.
///
/// ```ignore
/// let out = PictureFillEditor::new("Picture", Bind::bound(&mut fill)).show(ui);
/// ```
#[must_use = "show the editor with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct PictureFillEditor<'a> {
    label: Label,
    value: Bind<'a, PictureFill>,
}

impl<'a> PictureFillEditor<'a> {
    /// The editor named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, PictureFill>) -> Self {
        PictureFillEditor { label: label.into(), value }
    }

    /// Shows the editor.
    pub fn show(self, ui: &mut Ui) -> PictureFillOutput {
        let PictureFillEditor { label, mut value } = self;
        let env = Env::of(ui);
        let words = env.words::<PictureFillWords>();
        let name: Arc<str> = label.as_str().into();
        let mut found = EditEvents::new();
        let mut requests = HostRequests::new();
        let shown = Stack::new().label(label).show(ui, |ui, cx| {
            found = EditEvents::new();
            requests = HostRequests::new();
            let fill = match value.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => PictureFill::default(),
            };
            let mut modes = cx.memory::<Modes>().clone();
            match fill.mode {
                PictureMode::Stretch { offsets } => modes.stretch = offsets,
                PictureMode::Tile { offset, scale, alignment, mirror } => modes.tile = TileValues { offset, scale, alignment, mirror },
            }
            *cx.memory::<Modes>() = modes.clone();
            let mut pass = Pass { fill: &fill, name: name.clone(), events: EditEvents::new() };
            rows(ui, &words, &mut pass, &modes, &mut requests);
            for event in &pass.events {
                let _ = value.reborrow().write(cx, event);
            }
            found = pass.events;
        });
        PictureFillOutput { response: shown.output, events: found, requests }
    }
}

/// One pass: the fill the rows edit and the events they raised.
struct Pass<'a> {
    fill: &'a PictureFill,
    name: Arc<str>,
    events: EditEvents<PictureFill>,
}

impl Pass<'_> {
    /// A copy of the fill with `change` applied.
    fn edited(&self, change: impl FnOnce(&mut PictureFill)) -> PictureFill {
        let mut copy = self.fill.clone();
        change(&mut copy);
        copy
    }

    /// Keeps the row's events, turned into fills.
    fn take<T>(&mut self, events: EditEvents<T>, fill: impl Fn(T) -> PictureFill) {
        self.events.extend(events.map(fill));
    }

    /// Keeps a one-step change unless a row already reported.
    fn set(&mut self, next: PictureFill) {
        if self.events.is_empty() {
            self.events.push(EditEvent::set(Mixed::Same(self.fill.clone()), next, self.name.clone(), None));
        }
    }
}

fn rows(ui: &mut Ui, words: &PictureFillWords, pass: &mut Pass<'_>, modes: &Modes, requests: &mut HostRequests) {
    let fill = pass.fill;
    FieldGroup::new(words.source.clone()).show(ui, |ui, _| {
        Field::new(words.source.clone()).id_salt("source").show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let shown = fill.source.as_ref().map_or_else(|| words.no_source.clone(), |source| Label::from(source.clone()));
                Text::new(shown).shade(Shade::Muted).show(ui);
                if clicked(&Button::new(words.insert.clone()).id_salt("insert").show(ui)) {
                    requests.request_path(PathRequest::new(PICTURE_FILL_KEY, PathKind::open_file(&IMAGE_EXTENSIONS)));
                }
            });
        });
        let tile = matches!(fill.mode, PictureMode::Tile { .. });
        if let Some(on) = Checkbox::new(Bind::given(tile)).label(words.tile.clone()).id_salt("tile").show(ui).value().cloned() {
            let next = pass.edited(|f| {
                f.mode = if on {
                    let t = modes.tile;
                    PictureMode::Tile { offset: t.offset, scale: t.scale, alignment: t.alignment, mirror: t.mirror }
                } else {
                    PictureMode::Stretch { offsets: modes.stretch }
                };
            });
            pass.set(next);
        }
        match fill.mode {
            PictureMode::Tile { offset, scale, alignment, mirror } => {
                let at = [
                    (&words.offset_x, "offset-x", offset[0], UnitKind::Points, OFFSET),
                    (&words.offset_y, "offset-y", offset[1], UnitKind::Points, OFFSET),
                    (&words.scale_x, "scale-x", scale[0], UnitKind::Percent, PictureMode::SCALE),
                    (&words.scale_y, "scale-y", scale[1], UnitKind::Percent, PictureMode::SCALE),
                ];
                for (index, (label, key, value, kind, range)) in at.into_iter().enumerate() {
                    let event = number(ui, label, key, value, kind, range, TILE_DECIMALS);
                    let base = fill.clone();
                    pass.take(event, move |v| {
                        let mut next = base.clone();
                        if let PictureMode::Tile { offset, scale, .. } = &mut next.mode {
                            if index < 2 {
                                offset[index] = v;
                            } else {
                                scale[index - 2] = v;
                            }
                        }
                        next
                    });
                }
                let alignments = [
                    Entry::new(TileAlignment::TopLeft, words.top_left.clone()),
                    Entry::new(TileAlignment::Top, words.top.clone()),
                    Entry::new(TileAlignment::TopRight, words.top_right.clone()),
                    Entry::new(TileAlignment::Left, words.left.clone()),
                    Entry::new(TileAlignment::Center, words.center.clone()),
                    Entry::new(TileAlignment::Right, words.right.clone()),
                    Entry::new(TileAlignment::BottomLeft, words.bottom_left.clone()),
                    Entry::new(TileAlignment::Bottom, words.bottom.clone()),
                    Entry::new(TileAlignment::BottomRight, words.bottom_right.clone()),
                ];
                if let Some(picked) = pick(ui, &words.alignment, "alignment", alignment, &alignments) {
                    let next = pass.edited(|f| {
                        if let PictureMode::Tile { alignment, .. } = &mut f.mode {
                            *alignment = picked;
                        }
                    });
                    pass.set(next);
                }
                let mirrors = [
                    Entry::new(TileMirror::None, words.mirror_none.clone()),
                    Entry::new(TileMirror::Horizontal, words.horizontal.clone()),
                    Entry::new(TileMirror::Vertical, words.vertical.clone()),
                    Entry::new(TileMirror::Both, words.both.clone()),
                ];
                if let Some(picked) = pick(ui, &words.mirror, "mirror", mirror, &mirrors) {
                    let next = pass.edited(|f| {
                        if let PictureMode::Tile { mirror, .. } = &mut f.mode {
                            *mirror = picked;
                        }
                    });
                    pass.set(next);
                }
            }
            PictureMode::Stretch { offsets } => {
                // The order of the model and of DeckCraft's pane: left, top, right, bottom.
                let labels = [&words.offset_left, &words.offset_top, &words.offset_right, &words.offset_bottom];
                for (at, label) in labels.into_iter().enumerate() {
                    let event = number(ui, label, ["left", "top", "right", "bottom"][at], offsets[at], UnitKind::Percent, OFFSET, STRETCH_DECIMALS);
                    let base = fill.clone();
                    pass.take(event, move |v| {
                        let mut next = base.clone();
                        if let PictureMode::Stretch { offsets } = &mut next.mode {
                            offsets[at] = v;
                        }
                        next
                    });
                }
            }
        }
        if let Some(on) = Checkbox::new(Bind::given(fill.rotate_with_shape)).label(words.rotate.clone()).id_salt("rotate").show(ui).value().cloned() {
            let next = pass.edited(|f| f.rotate_with_shape = on);
            pass.set(next);
        }
        let slider = SliderField::new(words.transparency.clone(), Bind::given(fill.transparency * 100.0), TRANSPARENCY)
            .unit_kind(UnitKind::Percent)
            .id_salt("transparency");
        let event = slider.show(ui).events;
        let base = fill.clone();
        pass.take(event, move |v| PictureFill { transparency: (v / 100.0).clamp(0.0, 1.0), ..base.clone() });
    });
}

/// A number row; the event of an edit.
fn number(ui: &mut Ui, label: &Label, key: &'static str, value: f64, kind: UnitKind, range: RangeInclusive<f64>, decimals: usize) -> EditEvents<f64> {
    let mut event = EditEvents::new();
    Field::new(label.clone()).id_salt(key).show(ui, |ui, _| {
        let field = NumberField::new(label.clone(), Bind::<Option<f64>>::given(Some(value)))
            .id_salt("field")
            .unit_kind(kind)
            .range(range.clone())
            .commit_decimals(decimals);
        event = field.show(ui).edit.events.map(|v| v.unwrap_or(value));
    });
    event
}

/// A select row; the value picked.
fn pick<T: Value + 'static>(ui: &mut Ui, label: &Label, key: &'static str, current: T, entries: &[Entry<T>]) -> Option<T> {
    let mut picked = None;
    Field::new(label.clone()).id_salt(key).show(ui, |ui, _| {
        picked = Select::new(label.clone(), Bind::given(current.clone()), entries).show(ui).value().cloned();
    });
    picked
}
