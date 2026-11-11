//! The Blending Options page: General Blending, Advanced Blending and Blend If.

use std::cell::RefCell;

use craft_behavior::EditEvents;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup, SliderField};
use craft_ui::layout_components::Stack;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::UnitKind;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::{HeaderStrength, Section};
use egui::Ui;

use super::BlendIfBar;
use super::labels::LayerStyleWords;
use super::page::{StyleOp, style_edit};
use crate::layer_style::{BlendIfRange, Blending, Knockout, LayerStyle};

/// What the Blending Options page reads.
pub(super) struct BlendingInput<'a> {
    /// The style the page edits.
    pub style: &'a LayerStyle,
    /// The Blend If entry shown.
    pub blend_if_channel: usize,
    /// The document's colour channels.
    pub channel_names: &'a [String],
    /// The blend modes the Blend Mode select offers; a separator entry divides the groups.
    pub blend_modes: &'a [Entry<String>],
}

/// What the Blending Options page reports in one pass.
#[derive(Default)]
pub(super) struct BlendingOutput {
    /// The edits of the style the controls made, in order.
    pub edit: EditEvents<LayerStyle>,
    /// The Blend If select picked another entry (view state).
    pub blend_if_channel: Option<usize>,
}

/// What a Blending Options control did to the [`Blending`]: the operation a held event replays.
#[derive(Clone, Debug)]
pub(super) enum BlendingOp {
    Blend(String),
    Opacity(f32),
    FillOpacity(f32),
    /// Channel `index` of `of` channels blends or not.
    Channel {
        index: usize,
        of: usize,
        on: bool,
    },
    Knockout(Knockout),
    /// Advanced check `index` (Photoshop's order, [`ADVANCED_GETTERS`]).
    Check {
        index: usize,
        on: bool,
    },
    /// The range of This Layer or Underlying Layer of Blend If entry `entry`.
    BlendIf {
        entry: usize,
        underlying: bool,
        range: BlendIfRange,
    },
}

impl BlendingOp {
    /// Applies the operation to `blending`.
    pub(super) fn apply(&self, blending: &mut Blending) {
        match self {
            BlendingOp::Blend(blend) => blending.blend.clone_from(blend),
            BlendingOp::Opacity(opacity) => blending.opacity = *opacity,
            BlendingOp::FillOpacity(opacity) => blending.fill_opacity = *opacity,
            BlendingOp::Channel { index, of, on } => {
                blending.channels.resize((*of).max(*index + 1), true);
                blending.channels[*index] = *on;
            }
            BlendingOp::Knockout(knockout) => blending.knockout = *knockout,
            BlendingOp::Check { index, on } => (advanced_set(*index))(blending, *on),
            BlendingOp::BlendIf { entry, underlying, range } => {
                if let Some(entry) = blending.blend_if.get_mut(*entry) {
                    if *underlying {
                        entry.underlying = *range;
                    } else {
                        entry.this_layer = *range;
                    }
                }
            }
        }
    }
}

/// Draws Photoshop's Blending Options page:
///
/// - General Blending: Blend Mode (a `Select` over `input.blend_modes`), Opacity 0..=100 %.
/// - Advanced Blending: Fill Opacity 0..=100 %; "Channels:" with one check per name in `input.channel_names`
///   (checked: the channel blends); Knockout (None, Shallow, Deep); the five checks in Photoshop's order (Blend
///   Interior Effects as Group, Blend Clipped Layers as Group, Transparency Shapes Layer, Layer Mask Hides Effects,
///   Vector Mask Hides Effects).
/// - Blend If (hidden with no entries): the "Blend If:" select ([`blend_if_entry_label`] names the entries), then
///   for This Layer and Underlying Layer a [`BlendIfBar`] with its readout.
///
/// Every control shows the style's value and reports its edit; the page maps the edits of the pass to edits of
/// the style ([`style_edit`]) in `BlendingOutput::edit`.
pub(super) fn blending_page(ui: &mut Ui, input: &BlendingInput<'_>) -> BlendingOutput {
    let env = Env::of(ui);
    let words = env.words::<LayerStyleWords>();
    let style = input.style;
    let b = &style.blending;
    let out = RefCell::new(BlendingOutput::default());
    let push = |events: EditEvents<LayerStyle>| out.borrow_mut().edit.extend(events);

    Stack::new().show(ui, |ui, _| {
        Section::fixed(words.general_blending.clone()).strength(HeaderStrength::Caption).show(ui, |ui, _| {
            FieldGroup::new(words.general_blending.clone()).show(ui, |ui, _| {
                Field::new(words.blend_mode.clone()).show(ui, |ui, _| {
                    let shown = with_current(input.blend_modes, &b.blend);
                    let edit = Select::new(words.blend_mode.clone(), Bind::given(b.blend.clone()), &shown).show(ui);
                    push(style_edit(&edit, style, |v: &String| StyleOp::Blending(BlendingOp::Blend(v.clone()))));
                });
                Field::new(words.opacity.clone()).show(ui, |ui, _| {
                    let edit = percent(ui, &words.opacity, b.opacity);
                    push(style_edit(&edit, style, |v: &f64| StyleOp::Blending(BlendingOp::Opacity(*v as f32))));
                });
            });
        });

        Section::fixed(words.advanced_blending.clone()).strength(HeaderStrength::Caption).show(ui, |ui, _| {
            FieldGroup::new(words.advanced_blending.clone()).show(ui, |ui, _| {
                Field::new(words.fill_opacity.clone()).show(ui, |ui, _| {
                    let edit = percent(ui, &words.fill_opacity, b.fill_opacity);
                    push(style_edit(&edit, style, |v: &f64| StyleOp::Blending(BlendingOp::FillOpacity(*v as f32))));
                });
                let of = input.channel_names.len();
                let mut channels = b.channels.clone();
                channels.resize(of, true);
                Field::new(words.channels.clone()).show(ui, |ui, _| {
                    for (index, name) in input.channel_names.iter().enumerate() {
                        let edit = Checkbox::new(Bind::given(channels[index])).label(name.clone()).show(ui);
                        push(style_edit(&edit, style, |v: &bool| StyleOp::Blending(BlendingOp::Channel { index, of, on: *v })));
                    }
                });
                let knockouts = [
                    Entry::new(Knockout::None, words.knockout_none.clone()),
                    Entry::new(Knockout::Shallow, words.knockout_shallow.clone()),
                    Entry::new(Knockout::Deep, words.knockout_deep.clone()),
                ];
                Field::new(words.knockout.clone()).show(ui, |ui, _| {
                    let edit = Select::new(words.knockout.clone(), Bind::given(b.knockout), &knockouts).show(ui);
                    push(style_edit(&edit, style, |v: &Knockout| StyleOp::Blending(BlendingOp::Knockout(*v))));
                });
                for (index, (text, get)) in advanced_labels(&words).into_iter().zip(ADVANCED_GETTERS).enumerate() {
                    let edit = Checkbox::new(Bind::given(get(b))).label(text).show(ui);
                    push(style_edit(&edit, style, |v: &bool| StyleOp::Blending(BlendingOp::Check { index, on: *v })));
                }
            });
        });

        let entries = b.blend_if.len();
        if entries == 0 {
            return;
        }
        let shown = input.blend_if_channel.min(entries - 1);
        Section::fixed(words.blend_if.clone()).strength(HeaderStrength::Caption).show(ui, |ui, _| {
            FieldGroup::new(words.blend_if.clone()).show(ui, |ui, _| {
                let names: Vec<Entry<usize>> =
                    (0..entries).map(|i| Entry::new(i, blend_if_entry_label(i, entries, input.channel_names, &words))).collect();
                Field::new(words.blend_if.clone()).show(ui, |ui, _| {
                    let edit = Select::new(words.blend_if.clone(), Bind::given(shown), &names).show(ui);
                    if let Some(&index) = edit.value() {
                        out.borrow_mut().blend_if_channel = Some(index);
                    }
                });
                let entry = b.blend_if[shown];
                for (caption, underlying) in [(&words.this_layer, false), (&words.underlying_layer, true)] {
                    let range = if underlying { entry.underlying } else { entry.this_layer };
                    Stack::new().id_salt(("blendIf", shown, underlying)).show(ui, |ui, _| {
                        let edit = BlendIfBar::new(caption.clone(), Bind::given(range)).show(ui);
                        push(style_edit(&edit, style, |v: &BlendIfRange| {
                            StyleOp::Blending(BlendingOp::BlendIf { entry: shown, underlying, range: *v })
                        }));
                    });
                }
            });
        });
    });
    out.into_inner()
}

/// The blend modes with `current` added at the end when the list lacks it (a mode an imported document brings), so
/// the select still shows it.
fn with_current(modes: &[Entry<String>], current: &str) -> Vec<Entry<String>> {
    let mut entries = modes.to_vec();
    if !entries.iter().any(|entry| entry.value().is_some_and(|value| value == current)) {
        entries.push(Entry::new(current.to_owned(), current.to_owned()));
    }
    entries
}

/// A 0..=100 % slider and field, whole percent.
fn percent(ui: &mut Ui, label: &Label, value: f32) -> craft_ui::contract::EditResponse<f64> {
    SliderField::new(label.clone(), Bind::given(f64::from(value)), 0.0..=100.0).step(1.0).unit_kind(UnitKind::Percent).show(ui)
}

/// The Advanced Blending checks' values, in Photoshop's order.
const ADVANCED_GETTERS: [fn(&Blending) -> bool; 5] = [
    |b| b.blend_interior_effects_as_group,
    |b| b.blend_clipped_layers_as_group,
    |b| b.transparency_shapes_layer,
    |b| b.layer_mask_hides_effects,
    |b| b.vector_mask_hides_effects,
];

/// The setter of Advanced Blending check `index`.
fn advanced_set(index: usize) -> fn(&mut Blending, bool) {
    match index {
        0 => |b, v| b.blend_interior_effects_as_group = v,
        1 => |b, v| b.blend_clipped_layers_as_group = v,
        2 => |b, v| b.transparency_shapes_layer = v,
        3 => |b, v| b.layer_mask_hides_effects = v,
        _ => |b, v| b.vector_mask_hides_effects = v,
    }
}

/// The Advanced Blending checks' labels, in Photoshop's order.
fn advanced_labels(words: &LayerStyleWords) -> [Label; 5] {
    [
        words.blend_interior_effects.clone(),
        words.blend_clipped_layers.clone(),
        words.transparency_shapes_layer.clone(),
        words.layer_mask_hides_effects.clone(),
        words.vector_mask_hides_effects.clone(),
    ]
}

/// The name of Blend If entry `index` when the style has `entries` entries and the document's channels are
/// `channel_names`: with as many entries as names (a one-channel document) the name itself; otherwise entry 0 is
/// Gray and entry `i` is name `i − 1`.
pub(super) fn blend_if_entry_label(index: usize, entries: usize, channel_names: &[String], words: &LayerStyleWords) -> Label {
    let name = |i: usize| Label::from(channel_names.get(i).cloned().unwrap_or_default());
    if entries == channel_names.len() {
        name(index)
    } else if index == 0 {
        words.gray.clone()
    } else {
        name(index - 1)
    }
}
