//! `PaintList`: named paints in rows, with their events and thumbnails.

use craft_behavior::{Axis, EditEvent, IconId, Selection};
use craft_behavior::{DisplayOrder, ItemSource, Mixed, SelectionChange};
use craft_theme::Srgba;
use craft_ui::buttons::Button;
use craft_ui::contract::ShowContainer as _;
use craft_ui::contract::{Bind, Outcome, Show as _, ShowCollection as _};
use craft_ui::contract::{CollectionEvent, NoValue, Rename};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow, RowEvent, ThumbAspect, ThumbSize, TrailingFace, TrailingPart};
use craft_ui::primitives::{ContentPainterCx, IconSpec, PaintShape};
use craft_ui::runtime::env::Label;
use egui::{Color32, Rect, Response, Ui};

use super::{Paint, PaintWords, paint_image, ramp_stops};

/// How a paint is bound (FigCraft's colour style or variable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PaintLinkKind {
    /// A colour style.
    Style,
    /// A variable.
    Variable,
}

/// A row's binding: the style's or variable's name, shown after the paint's name, with a Detach button when
/// [`PaintListOptions::bind`] is on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PaintLink {
    /// The style's or variable's name.
    pub name: String,
    /// Whether it is a style or a variable.
    pub kind: PaintLinkKind,
}

impl PaintLink {
    /// A binding to the style or variable `name`.
    pub fn new(name: impl Into<String>, kind: PaintLinkKind) -> Self {
        PaintLink { name: name.into(), kind }
    }
}

/// What a row shows beyond the chip and the name, by index in the list's paints.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PaintRow {
    /// The style or variable the paint is bound to.
    pub link: Option<PaintLink>,
}

impl PaintRow {
    /// A row bound to `link`.
    pub fn linked(link: PaintLink) -> Self {
        PaintRow { link: Some(link) }
    }
}

/// The row actions a [`PaintList`] offers; all off by default (the list is then PhotoCraft's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PaintListOptions {
    /// A click on a row's chip opens the colour picker on it.
    pub edit: bool,
    /// A hex field per row.
    pub hex: bool,
    /// An opacity field per row (percent).
    pub opacity: bool,
    /// Bind and Detach buttons (styles and variables).
    pub bind: bool,
    /// "Select layers with this color".
    pub select_users: bool,
    /// An Add button under the rows.
    pub add: bool,
    /// A Remove button per row.
    pub remove: bool,
}

impl PaintListOptions {
    /// FigCraft's Selection colors: edit, hex, opacity, bind and select users.
    pub const SELECTION_COLORS: PaintListOptions =
        PaintListOptions { edit: true, hex: true, opacity: true, bind: true, select_users: true, add: false, remove: false };

    /// Turns the `edit` action on or off.
    #[must_use]
    pub const fn edit(mut self, on: bool) -> Self {
        self.edit = on;
        self
    }

    /// Turns the `hex` action on or off.
    #[must_use]
    pub const fn hex(mut self, on: bool) -> Self {
        self.hex = on;
        self
    }

    /// Turns the `opacity` action on or off.
    #[must_use]
    pub const fn opacity(mut self, on: bool) -> Self {
        self.opacity = on;
        self
    }

    /// Turns the `bind` action on or off.
    #[must_use]
    pub const fn bind(mut self, on: bool) -> Self {
        self.bind = on;
        self
    }

    /// Turns the `select_users` action on or off.
    #[must_use]
    pub const fn select_users(mut self, on: bool) -> Self {
        self.select_users = on;
        self
    }

    /// Turns the `add` action on or off.
    #[must_use]
    pub const fn add(mut self, on: bool) -> Self {
        self.add = on;
        self
    }

    /// Turns the `remove` action on or off.
    #[must_use]
    pub const fn remove(mut self, on: bool) -> Self {
        self.remove = on;
        self
    }

    /// Whether any row action is on.
    pub fn any(&self) -> bool {
        self.edit || self.hex || self.opacity || self.bind || self.select_users || self.add || self.remove
    }
}

/// A row action of [`PaintList`], by row index.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaintListEvent {
    /// The chip's picker edited the paint (as [`PaintChip`] reports).
    #[non_exhaustive]
    Edit {
        /// The row's index.
        row: usize,
        /// The edit of the row's paint.
        event: EditEvent<Paint>,
    },
    /// A hex typed and committed.
    #[non_exhaustive]
    Hex {
        /// The row's index.
        row: usize,
        /// The committed colour.
        color: Srgba,
    },
    /// The opacity field: `Set` when typed, a gesture when scrubbed; 0–1.
    #[non_exhaustive]
    Opacity {
        /// The row's index.
        row: usize,
        /// The edit of the opacity.
        event: EditEvent<f32>,
    },
    /// Bind the paint of the row at this index to a style or variable: the app opens its picker.
    Bind(usize),
    /// Detach the style or variable of the row at this index.
    Detach(usize),
    /// Select the layers that use the paint of the row at this index.
    SelectUsers(usize),
    /// Add a paint under the rows.
    Add,
    /// Remove the paint of the row at this index.
    Remove(usize),
    /// Renamed in place.
    #[non_exhaustive]
    Rename {
        /// The row's index.
        row: usize,
        /// The new name.
        name: String,
    },
}

/// What a paint list reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PaintListOutput {
    /// The list's response.
    pub response: Response,
    /// The row a click or key selected in this pass.
    pub selected: Option<usize>,
    /// The row actions of this pass, in order; a rename is here too.
    pub events: Vec<PaintListEvent>,
}

impl Outcome for PaintListOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The paints as a list source, keyed by position.
struct Paints<'a>(&'a [(String, Paint)]);

impl DisplayOrder<usize> for Paints<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|(name, _)| std::borrow::Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for Paints<'_> {}

const REMOVE: TrailingPart = TrailingPart::new(0);
const SELECT_USERS: TrailingPart = TrailingPart::new(1);
const BIND: TrailingPart = TrailingPart::new(2);

/// The chip's width to height in a row.
const ROW_CHIP: (u16, u16) = (3, 2);

/// Named paints in rows: the chip, then the name; a click on a row selects it, a double-click or F2 renames it in
/// place. The Paints list of a document (PhotoCraft's shape paints; FigCraft's and VectorCraft's document colours).
///
/// With [`options`](Self::options) a row also offers the binding's name with Bind and Detach ([`rows`](Self::rows)),
/// "Select layers with this color" and Remove after the name, and Add under the rows. Each reports a
/// [`PaintListEvent`]. The per-row chip edit, hex field and opacity field wait for a row slot that holds field
/// controls.
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct PaintList<'a> {
    label: Label,
    paints: &'a [(String, Paint)],
    selected: Bind<'a, Option<usize>>,
    /// The row actions offered; none by default.
    #[prop]
    options: PaintListOptions,
    /// Each paint's row data (its binding), by index; rows past the end have none.
    #[prop]
    rows: &'a [PaintRow],
    /// The solid colour a row's switch to Solid starts from when the row was never solid.
    #[prop]
    fallback: Option<Srgba>,
}

impl<'a> PaintList<'a> {
    /// Black: a switch to Solid in a list with no solid paint and no [`fallback`](Self::fallback).
    pub const DEFAULT_SOLID: Srgba = Srgba::rgb(0.0, 0.0, 0.0);

    /// A list of `paints` (name and paint per row) named `label`; `selected` is the selected row.
    pub fn new(label: impl Into<Label>, paints: &'a [(String, Paint)], selected: Bind<'a, Option<usize>>) -> Self {
        PaintList { label: label.into(), paints, selected, options: PaintListOptions::default(), rows: &[], fallback: None }
    }

    /// Draws the rows and returns the selection, any rename and the row actions of this pass.
    pub fn show(self, ui: &mut Ui) -> PaintListOutput {
        let words = craft_ui::runtime::env::Env::of(ui).words::<PaintWords>();
        let mut selected_bind = self.selected;
        let (paints, rows, options, label) = (self.paints, self.rows, self.options, self.label.clone());
        let mut output = None;
        let mut add = false;
        let shown = Stack::new().show(ui, |ui, cx| {
            let held = match selected_bind.reborrow().read(cx) {
                Some(Mixed::Same(Some(row))) if row < paints.len() => Selection::single(row),
                _ => Selection::default(),
            };
            let list = List::<usize>::new(label.clone()).selection(Bind::given(held)).can_rename(|_| true);
            output = Some(list.show_items(ui, &Paints(paints), &mut |row, _| {
                let (name, paint) = &paints[*row];
                let mut item =
                    ListRow::new(name.clone()).thumb_aspect(ThumbSize::Small, ThumbAspect::new(ROW_CHIP.0, ROW_CHIP.1), move |painter, rect| {
                        paint_thumb(painter, rect, paint);
                    });
                if let Some(link) = rows.get(*row).and_then(|data| data.link.as_ref()) {
                    item = item.trailing(link.name.clone());
                }
                if options.bind {
                    let (name, icon) = match rows.get(*row).and_then(|data| data.link.as_ref()) {
                        Some(PaintLink { kind: PaintLinkKind::Style, .. }) => (words.detach_style.clone(), "unlink"),
                        Some(PaintLink { kind: PaintLinkKind::Variable, .. }) => (words.detach_variable.clone(), "unlink"),
                        None => (words.styles.clone(), "palette"),
                    };
                    item = item.trailing_part(BIND, TrailingFace::Icon(IconSpec::svg(IconId::new(icon))), name);
                }
                if options.select_users {
                    item = item.trailing_part(
                        SELECT_USERS,
                        TrailingFace::Icon(IconSpec::svg(IconId::new("layers"))),
                        Label::from(words.select_users.fill(&[("name", name.as_str())])),
                    );
                }
                if options.remove {
                    item = item.trailing_part(
                        REMOVE,
                        TrailingFace::Icon(IconSpec::svg(IconId::new("minus"))),
                        Label::from(words.remove.fill(&[("name", name.as_str())])),
                    );
                }
                item
            }));
            if options.add {
                add = craft_ui::contract::clicked(&Button::new(words.add.clone()).icon(IconId::new("plus")).show(ui));
            }
        });
        // A parent's measuring run reuses what it measured and does not run the body.
        let Some(list) = output else { return PaintListOutput { response: shown.output, selected: None, events: Vec::new() } };
        let mut selected = None;
        let mut events = Vec::new();
        for event in list.events {
            match event {
                CollectionEvent::Selection(SelectionChange::Set(keys)) => selected = keys.last().copied().or(selected),
                CollectionEvent::Selection(SelectionChange::Toggle(row)) => selected = Some(row),
                CollectionEvent::Rename(event) if matches!(event, EditEvent::Commit { .. }) => {
                    if let Some(Rename { key, name, .. }) = event.value() {
                        events.push(PaintListEvent::Rename { row: *key, name: name.clone() });
                    }
                }
                CollectionEvent::Item(row, RowEvent::Trailing { part, .. }) => {
                    let linked = self.rows.get(row).is_some_and(|data| data.link.is_some());
                    events.push(match part {
                        REMOVE => PaintListEvent::Remove(row),
                        SELECT_USERS => PaintListEvent::SelectUsers(row),
                        _ if linked => PaintListEvent::Detach(row),
                        _ => PaintListEvent::Bind(row),
                    });
                }
                _ => {}
            }
        }
        if add {
            events.push(PaintListEvent::Add);
        }
        PaintListOutput { response: list.response, selected, events }
    }
}

/// Draws `paint` into a row's thumbnail: a solid paint over the checkerboard where it has alpha, a gradient ramp,
/// the well for No Color with a diagonal, a pattern tiled or a picture stretched.
fn paint_thumb(painter: &ContentPainterCx<'_>, rect: Rect, paint: &Paint) {
    match paint {
        Paint::Solid(color) => {
            if color.a < 1.0 {
                painter.checker(rect);
            }
            painter.painter.rect_filled(rect, 0.0, Color32::from(*color));
        }
        Paint::None => {
            let inner = painter.plot_frame(rect);
            painter.outline(&PaintShape::Path(vec![inner.left_bottom(), inner.right_top()]));
        }
        Paint::Gradient(preview) => {
            painter.checker(rect);
            if !preview.stops.is_empty() {
                painter.ramp(rect, &ramp_stops(preview, false), Axis::X);
            }
        }
        Paint::Pattern(image) | Paint::Picture(image) => {
            painter.checker(rect);
            if let Some(texture) = image.texture {
                paint_image(painter, rect, texture, matches!(paint, Paint::Pattern(_)));
            }
        }
    }
}
