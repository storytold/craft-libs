//! `ConstraintFields`: the Style, Width, swap and Height cells of a marquee's options bar. A pattern over the
//! children of a `Bar` with `BarKind::Options`.

use super::words::BarWords;
use crate::buttons::IconButton;
use crate::contract::{Bind, Entry, Gap, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing, clicked};
use crate::form::Field;
use crate::layout_components::Row;
use crate::menu::Menu;
use crate::pickers::Select;
use crate::runtime::env::{Env, Label, UnitKind};
use crate::text::NumberField;
use craft_behavior::geometry::Constraint;
use craft_behavior::{EditEvent, IconId, Mixed, NumberCodec, PixelSize};
use egui::{Response, Ui};
use std::ops::RangeInclusive;
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// The three kinds of [`Constraint`], as the Style select lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StyleChoice {
    Normal,
    FixedRatio,
    FixedSize,
}

impl StyleChoice {
    fn of(constraint: Constraint) -> Self {
        match constraint {
            Constraint::Ratio(..) => Self::FixedRatio,
            Constraint::Size(..) => Self::FixedSize,
            _ => Self::Normal,
        }
    }

    /// The constraint a pick starts with.
    fn constraint(self) -> Constraint {
        match self {
            Self::Normal => Constraint::Free,
            Self::FixedRatio => Constraint::Ratio(ConstraintFields::RATIO_START.0, ConstraintFields::RATIO_START.1),
            Self::FixedSize => Constraint::Size(ConstraintFields::SIZE_START.0, ConstraintFields::SIZE_START.1),
        }
    }
}

/// The unit of the Fixed Size fields and what converts it.
#[derive(Clone, Copy)]
struct SizeUnits<'a> {
    unit: UnitKind,
    units: &'a [Entry<UnitKind>],
    pixels_per_inch: f64,
    document: PixelSize,
}

/// What the cells report.
#[derive(Debug)]
#[non_exhaustive]
pub struct ConstraintFieldsOutput {
    /// The cells' row.
    pub response: Response,
    /// The constraint changed this pass: a Style pick, the swap button, a typed Width or Height or a scrub of a
    /// caption. The binding holds the new value when it writes through; a given one applies it. It is a tool setting,
    /// not a document edit: no undo step.
    pub changed: bool,
    /// A unit picked from a Fixed Size field's units menu; the host stores it with its settings.
    pub unit_picked: Option<UnitKind>,
}

impl Outcome for ConstraintFieldsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Style, Width, swap and Height cells of an options `Bar` that read and write a marquee's [`Constraint`]
/// (Photoshop's Rectangular Marquee bar, the Slice tool's).
///
/// - Cells, in the bar's reading order: "Style:" with a `Select` of Normal, Fixed Ratio and Fixed Size; "Width:"
///   with a number field; a swap icon button; "Height:" with a number field. Each caption scrubs its field.
/// - Normal ([`Constraint::Free`]): Width, swap and Height are greyed and show 1 and 1. Fixed Ratio
///   ([`Constraint::Ratio`]): plain numbers in [`RATIO_RANGE`](Self::RATIO_RANGE). Fixed Size ([`Constraint::Size`],
///   pixels): lengths in the [size unit](Self::size_units) (pixels by default), typed text may carry another
///   unit's spelling (`2in`), a secondary click opens the units menu, values stay in 1..=[`SIZE_MAX`](Self::SIZE_MAX)
///   pixels, and leaving a field that shows a rounded value is no change.
/// - Picking a style resets the numbers (Fixed Ratio 1 : 1, Fixed Size 64 × 64 px). The swap button exchanges Width
///   and Height.
/// - `enabled(false)` greys every cell (Photoshop greys them for the Single Row and Single Column
///   Marquee).
/// - Keyboard: each cell is its own Tab stop in the bar's order; right to left the cells follow the bar's reading
///   order and Width stays before Height.
#[must_use = "show the cells with `show`"]
#[derive(crate::contract::Pattern)]
pub struct ConstraintFields<'a> {
    constraint: Bind<'a, Constraint>,
    size_units: Option<SizeUnits<'a>>,
    enabled: bool,
}

impl<'a> ConstraintFields<'a> {
    /// The range of the Fixed Ratio fields.
    pub const RATIO_RANGE: RangeInclusive<f64> = 0.001..=999.0;
    /// The largest Fixed Size side in pixels.
    pub const SIZE_MAX: f64 = 300_000.0;
    /// The ratio a switch to Fixed Ratio starts with.
    pub const RATIO_START: (f64, f64) = (1.0, 1.0);
    /// The size in pixels a switch to Fixed Size starts with.
    pub const SIZE_START: (f64, f64) = (64.0, 64.0);

    /// The cells over the host's `constraint`.
    pub fn new(constraint: Bind<'a, Constraint>) -> Self {
        ConstraintFields { constraint, size_units: None, enabled: true }
    }

    /// Whether the cells take input; true by default.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Shows Fixed Size in `unit`, with `units` in the units menu (the host's ruler units), converting through the
    /// document's resolution `pixels_per_inch` and, for a percentage, its width and height `document`. Without it
    /// Fixed Size is in pixels with no menu.
    pub fn size_units(mut self, unit: UnitKind, units: &'a [Entry<UnitKind>], pixels_per_inch: f64, document: PixelSize) -> Self {
        self.size_units = Some(SizeUnits { unit, units, pixels_per_inch, document });
        self
    }

    /// Adds the cells to the bar's row `ui`.
    pub fn show(self, ui: &mut Ui) -> ConstraintFieldsOutput {
        let ConstraintFields { mut constraint, size_units, enabled } = self;
        let env = Env::of(ui);
        let words = env.words::<BarWords>();
        let points_per_inch = env.units().points_per_inch();
        let name: Arc<str> = words.constraint_style.as_str().into();
        let mut next = None;
        let mut unit_picked = None;
        let shown = Row::new().gap(Gap::Item).enabled(enabled).show(ui, |ui, cx| {
            let before = match constraint.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => Constraint::Free,
            };
            let mut now = before;
            let styles = [
                Entry::new(StyleChoice::Normal, words.constraint_normal.clone()),
                Entry::new(StyleChoice::FixedRatio, words.constraint_fixed_ratio.clone()),
                Entry::new(StyleChoice::FixedSize, words.constraint_fixed_size.clone()),
            ];
            Field::new(words.constraint_style.clone()).id_salt("style").sizing(Sizing::Content).show(ui, |ui, _| {
                let pick = Select::new(words.constraint_style.clone(), Bind::given(StyleChoice::of(before)), &styles).show(ui);
                if let Some(picked) = pick.value().copied()
                    && picked != StyleChoice::of(before)
                {
                    now = picked.constraint();
                }
            });
            let free = !matches!(before, Constraint::Ratio(..) | Constraint::Size(..));
            let (width, height) = match before {
                Constraint::Ratio(w, h) | Constraint::Size(w, h) => (w, h),
                _ => (1.0, 1.0),
            };
            let cells = Cells { size_units, points_per_inch };
            let width_cell = Cell { key: "width", caption: &words.constraint_width, first: true, enabled: enabled && !free };
            if let Some(edit) = cells.number(ui, &width_cell, before, width, &mut unit_picked) {
                now = with_side(before, edit, true);
            }
            let swap = IconButton::new(IconId::new("swap_horizontal")).label(words.constraint_swap.clone()).enabled(enabled && !free).show(ui);
            if clicked(&swap) {
                now = match before {
                    Constraint::Ratio(w, h) => Constraint::Ratio(h, w),
                    Constraint::Size(w, h) => Constraint::Size(h, w),
                    other => other,
                };
            }
            let height_cell = Cell { key: "height", caption: &words.constraint_height, first: false, enabled: enabled && !free };
            if let Some(edit) = cells.number(ui, &height_cell, before, height, &mut unit_picked) {
                now = with_side(before, edit, false);
            }
            if now != before {
                let _ = constraint.reborrow().write(cx, &EditEvent::set(Mixed::Same(before), now, name.clone(), None));
                next = Some(now);
            }
        });
        ConstraintFieldsOutput { response: shown.output, changed: next.is_some(), unit_picked }
    }
}

/// One side's value in the field's unit and back.
#[derive(Clone, Copy)]
struct Conversion {
    unit: UnitKind,
    extent: f64,
    pixels_per_inch: f64,
    points_per_inch: f64,
}

impl Conversion {
    /// `pixels` in the field's unit.
    fn shown(&self, pixels: f64) -> f64 {
        self.unit.from_pixels(pixels, self.extent, self.pixels_per_inch, self.points_per_inch).unwrap_or(pixels)
    }

    /// A value in the field's unit as pixels.
    fn pixels(&self, shown: f64) -> f64 {
        self.unit.to_pixels(shown, self.extent, self.pixels_per_inch, self.points_per_inch).unwrap_or(shown)
    }
}

/// `before` with one side set to `value`: Width when `first`, else Height.
fn with_side(before: Constraint, value: f64, first: bool) -> Constraint {
    let set = |w: f64, h: f64| if first { (value, h) } else { (w, value) };
    match before {
        Constraint::Ratio(w, h) => {
            let (w, h) = set(w, h);
            Constraint::Ratio(w, h)
        }
        Constraint::Size(w, h) => {
            let (w, h) = set(w, h);
            Constraint::Size(w, h)
        }
        other => other,
    }
}

/// What the Width and Height cells share.
struct Cells<'a> {
    size_units: Option<SizeUnits<'a>>,
    points_per_inch: f64,
}

/// One number cell's place and state.
struct Cell<'a> {
    key: &'static str,
    caption: &'a Label,
    first: bool,
    enabled: bool,
}

impl Cells<'_> {
    /// One of Width (`cell.first`) and Height: the side's new value (pixels for Fixed Size) when the user edited
    /// it. A unit picked from the field's menu goes to `unit_picked`.
    fn number(&self, ui: &mut Ui, cell: &Cell<'_>, before: Constraint, value: f64, unit_picked: &mut Option<UnitKind>) -> Option<f64> {
        let sized = matches!(before, Constraint::Size(..));
        let units = self.size_units.filter(|_| sized);
        let conversion = sized.then(|| {
            let (unit, pixels_per_inch, extent) = units.map_or((UnitKind::Pixels, 1.0, 1.0), |s| {
                let side = if cell.first { s.document.width } else { s.document.height };
                (s.unit, s.pixels_per_inch, f64::from(side))
            });
            Conversion { unit, extent, pixels_per_inch, points_per_inch: self.points_per_inch }
        });
        let shown = conversion.map_or(value, |c| c.shown(value));
        let range = conversion.map_or(ConstraintFields::RATIO_RANGE, |c| {
            let (low, high) = (c.shown(1.0), c.shown(ConstraintFields::SIZE_MAX));
            low.min(high)..=low.max(high)
        });
        let mut edited = None;
        Field::new(cell.caption.clone()).id_salt(cell.key).enabled(cell.enabled).side(SideLabel::new().scrub(true)).sizing(Sizing::Content).show(
            ui,
            |ui, _| {
                let mut field = NumberField::new(cell.caption.clone(), Bind::given(Some(shown))).range(range.clone()).scrub_by_label();
                if let Some(c) = conversion {
                    field = if units.is_some() {
                        // Fixed Size with the host's units: typed text may carry any unit, read through the document.
                        let reader = Env::of(ui).units().document_reader(c.unit, c.extent, c.pixels_per_inch, c.points_per_inch);
                        field.codec(NumberCodec::new(reader, *c.unit.decimals().end()))
                    } else {
                        field.unit_kind(c.unit)
                    };
                }
                let out = field.show(ui);
                edited = out.value().copied().flatten();
                if let Some(s) = units {
                    let entries: Vec<Entry<UnitKind>> = s.units.iter().map(|entry| entry.clone().checked(entry.value() == Some(&s.unit))).collect();
                    let menu = Menu::context(cell.caption.clone(), &out.response).values().pick(ui, &entries);
                    *unit_picked = unit_picked.or(menu.picked);
                }
            },
        );
        let edited = edited?;
        let Some(c) = conversion else { return Some(edited) };
        // A field that shows a rounded value commits the rounded value when it is left: that is no change.
        let places = i32::try_from(*c.unit.decimals().end()).unwrap_or(0);
        if (edited - shown).abs() < 0.5 * 10f64.powi(-places) {
            return None;
        }
        Some(c.pixels(edited).clamp(1.0, ConstraintFields::SIZE_MAX))
    }
}
