//! [`PatternFillEditor`].

use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::Arc;

use super::{PatternColors, PatternFillOptions, PatternSwatch, PatternWords};
use craft_behavior::{EditEvent, EditEvents, Mixed, NumberCodec, Selection};
use craft_color_ui::ui::{ColorPopover, PickedColor};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, FieldLayout, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, SideLabel, Sizing, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::patterns::presets::{
    CellShape, PresetBrowser, PresetFns, PresetPicker, PresetRow, PresetRowMenu, PresetSource, PresetState, PresetView,
};
use craft_ui::primitives::{ContentPainterCx, FacePaint};
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::text::NumberField;
use craft_ui::value_drag::AngleDial;
use egui::{Rect, Response, Ui};

/// The angle of the dial runs over half a turn each way.
pub(super) const ANGLE_RANGE: std::ops::RangeInclusive<f64> = -180.0..=180.0;

/// What [`PatternFillEditor`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PatternFillOutput {
    /// The editor's response.
    pub response: Response,
    /// The edits of the options this pass, in order: a pick, a check box, Snap to Origin and a typed value are `Set`;
    /// a scrub or a dial drag is a gesture. The binding holds the new value when it writes through.
    pub events: EditEvents<PatternFillOptions>,
}

impl Outcome for PatternFillOutput {
    type Value = PatternFillOptions;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// One pattern fill editor body.
///
/// Bitmap mode ([`PatternFillOptions::colors`] `None`, Photoshop's Pattern Fill):
/// - The pattern picker: a thumbnail field opening a grid of every swatch, names in tooltips; "No patterns" when the
///   list is empty. "Snap to Origin" beside it sets the phase to (0, 0).
/// - "Angle": an angle dial, -180 to 180. "Scale": a percent number field, [`PatternFillOptions::SCALE`]. "Link
///   with Layer" check box.
///
/// Preset mode (`colors` set, PowerPoint's Pattern fill): a grid of the swatches, then Foreground and Background
/// colour chips.
///
/// A tile shows its swatch's [`picture`](PatternSwatch::picture); without one, the swatch's name. Keyboard: Tab
/// through the fields; the grid takes arrows. Right to left the rows mirror. The editor is the body a host passes to
/// a paint chip's pattern editor.
#[must_use = "show the editor with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct PatternFillEditor<'a> {
    label: Label,
    options: Bind<'a, PatternFillOptions>,
    swatches: &'a [PatternSwatch],
}

impl<'a> PatternFillEditor<'a> {
    /// The editor named `label` over `options`, offering `swatches`.
    pub fn new(label: impl Into<Label>, options: Bind<'a, PatternFillOptions>, swatches: &'a [PatternSwatch]) -> Self {
        PatternFillEditor { label: label.into(), options, swatches }
    }

    /// Draws the editor and reports the edit.
    pub fn show(self, ui: &mut Ui) -> PatternFillOutput {
        let PatternFillEditor { label, mut options, swatches } = self;
        let env = Env::of(ui);
        let words = env.words::<PatternWords>();
        let units = env.units();
        let name: Arc<str> = label.as_str().into();
        let rows = PresetSource::new(swatches, PresetFns::new(tile_rows));
        let mut events = EditEvents::new();
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            events = EditEvents::new();
            let Some(Mixed::Same(current)) = options.reborrow().read(cx) else { return };
            let mut state = cx.keep::<PresetState<String>>().clone();
            let mut next: EditEvents<PatternFillOptions> = EditEvents::new();
            let mut take = |edits: EditEvents<PatternFillOptions>| next.extend(edits);
            if let Some(colors) = current.colors {
                let browser = Field::new(words.pattern.clone()).id_salt("grid").show(ui, |ui, _| {
                    PresetBrowser::new(words.pattern.clone(), Bind::bound(&mut state), &rows)
                        .selection(Bind::given(Selection::single(current.pattern.clone())))
                        .cell_shape(CellShape::Square)
                        .view_lock(PresetView::Grid)
                        .row_menu(PresetRowMenu::Off)
                        .show(ui)
                        .events
                });
                for picked in browser.inner.into_iter().flatten().filter_map(selected_key) {
                    if picked != current.pattern {
                        take(set(&current, &name, |o| o.pattern = picked).into());
                    }
                }
                take(colour_row(ui, &words.foreground, "foreground", colors.foreground, &current, |c, color| c.foreground = color));
                take(colour_row(ui, &words.background, "background", colors.background, &current, |c, color| c.background = color));
            } else {
                if swatches.is_empty() {
                    Text::new(words.no_patterns.clone()).shade(Shade::Muted).show(ui);
                } else {
                    let (picked, snapped) = Field::new(words.pattern.clone())
                        .id_salt("pattern")
                        .show(ui, |ui, _| {
                            Row::new()
                                .show(ui, |ui, _| {
                                    let picker = PresetPicker::pattern(
                                        words.pattern.clone(),
                                        Bind::given(Some(current.pattern.clone())),
                                        &rows,
                                        Bind::bound(&mut state),
                                    )
                                    .show(ui);
                                    let snap = Button::new(words.snap_to_origin.clone()).id_salt("snap").show(ui);
                                    (picker.picked, clicked(&snap))
                                })
                                .inner
                        })
                        .inner
                        .flatten()
                        .unwrap_or_default();
                    if let Some(pattern) = picked.filter(|pattern| *pattern != current.pattern) {
                        take(set(&current, &name, |o| o.pattern = pattern).into());
                    }
                    if snapped {
                        take(set(&current, &name, |o| o.phase = [0.0, 0.0]).into());
                    }
                }
                let angle = Field::new(words.angle.clone())
                    .id_salt("angle")
                    .side(SideLabel::new().scrub(true))
                    .show(ui, |ui, _| AngleDial::new(Bind::given(current.angle)).range(ANGLE_RANGE).show(ui).events)
                    .inner
                    .unwrap_or_default();
                take(angle.map(|v| edited(&current, |o| o.angle = v.clamp(*ANGLE_RANGE.start(), *ANGLE_RANGE.end()))));
                let scale = Field::new(words.scale.clone())
                    .id_salt("scale")
                    .side(SideLabel::new().scrub(true))
                    .show(ui, |ui, _| {
                        NumberField::new(words.scale.clone(), Bind::given(Some(current.scale)))
                            .range(PatternFillOptions::SCALE)
                            .unit_kind(UnitKind::Percent)
                            .codec(NumberCodec::new(units.reader(Some(UnitKind::Percent)), SCALE_DECIMALS))
                            .scrub_by_label()
                            .show(ui)
                            .edit
                            .events
                    })
                    .inner
                    .unwrap_or_default();
                take(scale.map(|v| edited(&current, |o| o.scale = v.unwrap_or(current.scale))));
                let link = Field::new(words.link.clone())
                    .id_salt("link")
                    .layout(FieldLayout::After)
                    .sizing(Sizing::Content)
                    .show(ui, |ui, _| Checkbox::new(Bind::given(current.link)).show(ui).events)
                    .inner
                    .unwrap_or_default();
                take(link.map(|v| edited(&current, |o| o.link = v)));
            }
            for edit in &next {
                let _ = options.reborrow().write(cx, edit);
            }
            events = next;
            *cx.keep::<PresetState<String>>() = state;
        });
        PatternFillOutput { response: shown.output, events }
    }
}

/// The most decimals the Scale field shows.
const SCALE_DECIMALS: usize = 2;

/// The preset row of each swatch: its name, with the tile painter when the host gave one.
fn tile_rows(swatches: &&[PatternSwatch], _: &Env) -> Vec<PresetRow<String>> {
    let tile_row = |swatch: &PatternSwatch| {
        let row = PresetRow::item(swatch.id.clone(), swatch.name.clone(), 0);
        match swatch.picture {
            Some(picture) => {
                let mut key = DefaultHasher::new();
                picture.hash(&mut key);
                row.preview_face(CellShape::Square.aspect(), FacePaint::new(key.finish(), move |cx, rect| paint_picture(cx, rect, picture)))
            }
            None => row,
        }
    };
    swatches.iter().map(tile_row).collect()
}

/// Paints the whole of a swatch's `picture` into `rect`.
fn paint_picture(cx: &ContentPainterCx<'_>, rect: Rect, picture: egui::TextureId) {
    cx.picture(picture, rect, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), 0.0);
}

/// The key a browser event picked: a click or Enter on a tile.
fn selected_key(event: craft_ui::patterns::presets::PresetEvent<String>) -> Option<String> {
    match event {
        craft_ui::patterns::presets::PresetEvent::Select { key, .. } | craft_ui::patterns::presets::PresetEvent::Activate { key, .. } => Some(key),
        _ => None,
    }
}

/// One colour chip of the preset mode: its edit as an edit of the options.
fn colour_row(
    ui: &mut Ui,
    caption: &Label,
    key: &'static str,
    color: craft_theme::Srgba,
    current: &PatternFillOptions,
    change: impl Fn(&mut PatternColors, craft_theme::Srgba),
) -> EditEvents<PatternFillOptions> {
    let edits = Field::new(caption.clone())
        .id_salt(key)
        .show(ui, |ui, _| ColorPopover::new(caption.clone(), Bind::given(PickedColor::rgb(color))).show(ui).events)
        .inner
        .unwrap_or_default();
    edits.map(|picked| edited(current, |o| set_colors(o, |c| change(c, picked.color))))
}

/// A copy of `options` with `change` applied.
fn edited(options: &PatternFillOptions, change: impl FnOnce(&mut PatternFillOptions)) -> PatternFillOptions {
    let mut copy = options.clone();
    change(&mut copy);
    copy
}

/// A one-step edit of `current` made by `change`.
fn set(current: &PatternFillOptions, name: &Arc<str>, change: impl FnOnce(&mut PatternFillOptions)) -> EditEvent<PatternFillOptions> {
    EditEvent::set(Mixed::Same(current.clone()), edited(current, change), Arc::clone(name), None)
}

fn set_colors(options: &mut PatternFillOptions, change: impl FnOnce(&mut PatternColors)) {
    if let Some(colors) = options.colors.as_mut() {
        change(colors);
    }
}
