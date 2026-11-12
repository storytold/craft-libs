//! `ListRow`: one row of a list, the item component of `List`, `FindingsList` and `EventLog`.
//!
//! In reading order a row has its toggle cells (in the columns the collection lays out), an indent and a
//! disclosure column for a tree, a clipping mark, an icon or thumbnails, the label with an optional detail line and
//! wrapped lines under it, and trailing items. The row mirrors in a right-to-left interface.
//!
//! Selection, the lead, hover, the drop mark, the drag ghost, the focus ring and the rename editor belong to the
//! collection that shows the row; the row draws the look the item step gives it and reports what its own parts do.

use super::toggle::ToggleCell;
use super::words::ListWords;
use crate::contract::{Common, OpenChange, RowHeight, RowPressed, RowState, Show};
use crate::primitives::{ContentPainterCx, HAlign, IconSpec};
use crate::runtime::env::Label;
use craft_behavior::{ColumnKey, Cursor};
use craft_theme::variant::Tone;
use craft_theme::{LabelColor, TypeRole};
use egui::Rect;
use std::sync::Arc;

mod output;
mod paint;
mod placement;
mod show;
mod style;
mod thumb;
mod trailing;

use crate::contract::Hosting;
pub use output::{RowEvent, RowOutput};
#[cfg(feature = "testing")]
pub use placement::RowPlacement;
#[cfg(feature = "testing")]
#[doc(hidden)]
pub use placement::indent_steps;
#[cfg(feature = "testing")]
#[doc(hidden)]
pub use placement::placement_under_fixed_sizes;
pub use style::{ListRowStyle, RowStyleCx};
pub(crate) use thumb::RowThumb;
pub(super) use thumb::Thumb;
use thumb::ThumbPayload;
pub use thumb::{ThumbAspect, ThumbPaint, ThumbSize, ThumbTarget, ThumbTargets};
use trailing::Hosted;
pub(super) use trailing::{Trailing, TrailingItem};
pub use trailing::{TrailingFace, TrailingPart};

/// Where on a row a drag of its own starts instead of the row's ([`ListRow::part_drag`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RowDragSpot {
    /// A thumbnail.
    Thumb(ThumbTarget),
    /// A trailing part.
    Trailing(TrailingPart),
}

/// The name of a label colour, or of no label, in the language of `words`.
pub fn label_color_label(color: Option<LabelColor>, words: &ListWords) -> Label {
    let word = match color {
        None => &words.label_none,
        Some(LabelColor::Red) => &words.label_red,
        Some(LabelColor::Orange) => &words.label_orange,
        Some(LabelColor::Yellow) => &words.label_yellow,
        Some(LabelColor::Green) => &words.label_green,
        Some(LabelColor::Seafoam) => &words.label_seafoam,
        Some(LabelColor::Blue) => &words.label_blue,
        Some(LabelColor::Indigo) => &words.label_indigo,
        Some(LabelColor::Magenta) => &words.label_magenta,
        Some(LabelColor::Fuchsia) => &words.label_fuchsia,
        Some(LabelColor::Violet) => &words.label_violet,
        Some(LabelColor::Gray) => &words.label_gray,
    };
    word.clone()
}

/// How strongly a row's label reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RowInk {
    /// `text.normal`.
    #[default]
    Normal,
    /// `text.muted`: a secondary row that is still on.
    Muted,
    /// `text.faint`: a hidden layer.
    Dimmed,
}

/// A text shown in one column of a row: the body role in the row's ink, on one elided line or wrapped.
pub(super) struct TextCell {
    pub column: ColumnKey,
    pub text: Label,
    pub wrap: bool,
}

type LeadPaintFn<'a> = Box<dyn FnOnce(&ContentPainterCx, Rect) + 'a>;

/// One row of a list.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct ListRow<'a> {
    #[common]
    pub(super) common: Common,
    /// The label in the strong weight (`body_strong`), for a row that heads the others.
    #[style_key]
    #[prop]
    pub(super) strong: bool,
    /// The row heads a group of rows: the title strong, on no band, the detail at the end of the same line and
    /// the disclosure of a group.
    #[style_key]
    #[prop(flag)]
    pub(super) heading: bool,
    /// The row has `space.gap.row` of empty room above its content, to set a group off from the one before.
    #[prop]
    pub(super) space_above: bool,
    /// Whether the row is its own AccessKit node; a face inside another component's part turns it off.
    #[prop]
    pub(super) node: bool,
    /// How strongly the label reads. A dimmed row wins over a muted one.
    #[style_key]
    #[prop]
    pub(super) ink: RowInk,
    /// The tone of the icon: the status colour for a severity, the row's ink for `Neutral`. A selected row's icon
    /// reads in the colour that reads on the selection.
    #[style_key]
    #[prop]
    pub(super) tone: Tone,
    /// The tone of the trailing icons, with the same meaning as `tone` has for the row's icon.
    #[style_key]
    #[prop]
    pub(super) trailing_tone: Tone,
    /// The layer form, for lists of layers and channels: dividers under the row and at each toggle cell's edge,
    /// the accent tint on the hover and an outline on the selected row.
    #[style_key]
    #[prop]
    pub(super) layer: bool,
    /// The row of a striped list that is tinted: every second row.
    #[style_key]
    #[prop]
    pub(super) striped: bool,
    /// The type role of the label and the text cells; the weight of `strong` and `heading` when unset. A role taller
    /// than a control makes the row as tall as its line.
    #[style_key]
    #[prop]
    pub(super) role: Option<TypeRole>,
    /// How tall the row is: its collection sets it from `CollectionProps::row_height` in `Collection::dress`.
    #[style_key]
    pub(super) rows: RowHeight,
    /// The label in italics.
    #[prop]
    pub(super) italic: bool,
    /// The hover look without the pointer (the canvas hovers the layer).
    #[prop]
    pub(super) highlight: bool,
    /// The row is clipped to the layer below: it is indented one step and shows the clipping mark before its icon
    /// or thumbnail.
    #[prop]
    pub(super) clipped: bool,
    /// A hairline along the row's bottom edge, in a plain row too.
    #[prop]
    pub(super) rule: bool,
    /// The rename editor replaces the label; `state` sets it from the collection's row state.
    pub(super) renaming: bool,
    /// Where the collection holds a press on the row, from its row state: the toggle cell it acts on shows it.
    pub(super) pressed: Option<RowPressed>,
    /// The collection asked for this row's menu and it is open, from its row state: a revealed part stays shown.
    pub(super) menu_open: bool,
    /// The parts that drag instead of the row, each with what starts its drag session.
    pub(super) drags: Vec<(RowDragSpot, ThumbPayload)>,
    /// The cursors over thumbnails.
    pub(super) cursors: Vec<(ThumbTarget, Cursor)>,
    /// The icon or thumbnails come before the disclosure column instead of after it.
    #[prop]
    pub(super) leading_first: bool,
    /// A second line under the label in the caption role and `text.faint`.
    #[prop(into)]
    pub(super) detail: Option<Label>,
    /// Secondary text under the detail line, wrapped to the label band's width. The row's height follows the
    /// wrapped lines.
    #[prop(into)]
    pub(super) lines: Option<Label>,
    /// The most lines of `lines` the row shows: longer text is cut with an ellipsis at the end of the last one, and
    /// the row is no taller than with exactly that many lines. A screen reader reads all of the text.
    #[prop(quantity = "count")]
    pub(super) line_limit: Option<u8>,
    /// A progress bar under the label, `fraction` full (clamped to 0..=1), in a line of its own below the detail. The
    /// row is one caption line taller than its detail makes it, so a list with fixed row heights adds one
    /// (`ListRow::row_height` counts the detail only). A screen reader reads the percentage in the row's description.
    #[prop(quantity = "fraction")]
    pub(super) progress: Option<f32>,
    /// What a screen reader reads as the row's name when it differs from the drawn text.
    #[prop(into)]
    pub(super) accessible_name: Option<String>,
    pub(super) spacer: bool,
    pub(super) cells: Vec<(ColumnKey, Rect)>,
    /// Each column's alignment and whether its figures are tabular, from `columns(..)`.
    pub(super) aligns: Vec<(ColumnKey, HAlign, bool)>,
    /// The columns that read in the muted look.
    pub(super) muted: Vec<ColumnKey>,
    /// The columns whose wrapped text can be selected and copied.
    pub(super) selectable: Vec<ColumnKey>,
    /// The toggle columns of the collection, from the item facts it records for the row: none of them holds the
    /// label, whether or not this row has a cell in it.
    pub(super) toggle_columns: Vec<ColumnKey>,
    /// The keys of those toggle columns that have one, from the same facts: their cells show the key in the tooltip.
    pub(super) toggle_keys: Vec<(ColumnKey, craft_keymap::Chord)>,
    pub(super) toggles: Vec<ToggleCell>,
    pub(super) texts: Vec<TextCell>,
    /// An icon before the label. A thumbnail takes its place.
    #[prop]
    pub(super) icon: Option<IconSpec<'a>>,
    /// The leading slot, one icon wide and as high as the row, painted by the closure into its rect (a timeline rail:
    /// a dot and a line through the row). It sits where the icon does, mirrored in a right-to-left interface, and
    /// replaces the row's icon; thumbnails replace it in turn.
    #[prop(boxed = "FnOnce(&ContentPainterCx, Rect) + 'a", name = "lead_painter")]
    pub(super) lead_paint: Option<LeadPaintFn<'a>>,
    /// The leading text and the widest text its slot is sized for (`lead_text`).
    pub(super) lead_text: Option<(Label, Label)>,
    /// The type role of the leading text: `Caption` unless set.
    #[prop]
    pub(super) lead_role: TypeRole,
    pub(super) thumb: ThumbSize,
    pub(super) thumb_aspect: ThumbAspect,
    pub(super) first: Option<ThumbPaint<'a>>,
    pub(super) extras: Vec<Thumb<'a>>,
    /// The thumbnails the row's editing is aimed at: one target, or several together (`[ThumbTarget; N]`), framed with
    /// corner brackets whether or not the row is selected.
    #[prop(into, name = "thumb_target")]
    pub(super) targets: ThumbTargets,
    pub(super) trailing: Vec<Trailing<'a>>,
    /// The components under the label band's text, in one line (`below`).
    pub(super) below: Vec<Hosted<'a>>,
    /// The components shown in a column's cell in place of its text (`cell_field`).
    pub(super) cell_fields: Vec<(ColumnKey, Hosted<'a>)>,
    /// The row's depth in a tree, which indents it; `None` for a row of a flat list.
    /// The row is at `depth` levels of a tree: indented by it, an `AccessKit` tree item.
    #[prop]
    pub(super) depth: Option<usize>,
    /// `Some(open)` for a tree row that has children, which shows the disclosure part.
    /// The row is a tree group with children, `open` or closed: it shows the disclosure part.
    #[prop]
    pub(super) group: Option<bool>,
    /// The host declares and paints the disclosure part of this group row (a face has no parts of its own): the row
    /// keeps the disclosure column and paints no disclosure.
    pub(super) disclosure_by_host: bool,
    /// The row is an option of a listbox, not an item of a list: inset and rounded, the chosen option outlined.
    #[style_key]
    #[prop(flag)]
    pub(super) option: bool,
    /// On an option that runs a command (a command palette's row): the lead row is the command highlight, as a menu's,
    /// and no edge outlines the lead or the chosen row.
    #[style_key]
    #[prop(flag)]
    pub(super) command: bool,
    /// On an option of a list that picks a value: the row keeps a mark column, with the check mark when `true`.
    #[prop]
    pub(super) check: Option<bool>,
    pub(super) event: Option<RowEvent>,
    pub(super) open: Option<OpenChange<()>>,
    /// The row's menu button was clicked: its rect, where the menu hangs.
    pub(super) menu: Option<Rect>,
    pub(super) label_rect: Rect,
}

impl<'a> ListRow<'a> {
    /// A row showing `label`, which also names it for a screen reader.
    pub fn new(label: impl Into<Label>) -> Self {
        use crate::contract::Props as _;
        ListRow {
            common: Common::new(),
            strong: false,
            heading: false,
            space_above: false,
            node: true,
            ink: RowInk::Normal,
            tone: Tone::Neutral,
            trailing_tone: Tone::Neutral,
            layer: false,
            striped: false,
            role: None,
            rows: RowHeight::Control,
            italic: false,
            highlight: false,
            clipped: false,
            rule: false,
            renaming: false,
            pressed: None,
            menu_open: false,
            drags: Vec::new(),
            cursors: Vec::new(),
            leading_first: false,
            detail: None,
            lines: None,
            line_limit: None,
            progress: None,
            accessible_name: None,
            spacer: false,
            cells: Vec::new(),
            aligns: Vec::new(),
            muted: Vec::new(),
            selectable: Vec::new(),
            toggle_columns: Vec::new(),
            toggle_keys: Vec::new(),
            toggles: Vec::new(),
            texts: Vec::new(),
            icon: None,
            lead_paint: None,
            lead_text: None,
            lead_role: TypeRole::Caption,
            thumb: ThumbSize::None,
            thumb_aspect: ThumbAspect::SQUARE,
            first: None,
            extras: Vec::new(),
            targets: ThumbTargets::default(),
            trailing: Vec::new(),
            below: Vec::new(),
            cell_fields: Vec::new(),
            depth: None,
            group: None,
            disclosure_by_host: false,
            option: false,
            command: false,
            check: None,
            event: None,
            open: None,
            menu: None,
            label_rect: Rect::NOTHING,
        }
        .label(label)
    }

    /// Empty room in a list: a row that takes its height but draws nothing, senses nothing and has no AccessKit
    /// node. Mark its position `ItemSource::is_section` so keys and clicks skip it.
    pub fn spacer() -> Self {
        let mut row = Self::new(Label::Empty);
        row.spacer = true;
        row
    }

    /// The rects of the row's columns, which the item context lists (`ItemCx::cells`): toggle cells and text cells
    /// draw in theirs and the rest of the row lays out in what they leave.
    #[must_use]
    pub fn cells(mut self, cells: &[(ColumnKey, Rect)]) -> Self {
        self.cells = cells.to_vec();
        self
    }

    /// The host declares and paints the disclosure part of this group row at [`ListRow::disclosure_at`]; the row keeps
    /// the column. For a row the host paints as a face, which declares no parts.
    #[must_use]
    pub(crate) fn disclosure_by_host(mut self) -> Self {
        self.disclosure_by_host = true;
        self
    }

    /// The row is `rows` tall: the height its collection's rows take.
    #[must_use]
    pub(crate) fn rows(mut self, rows: RowHeight) -> Self {
        self.rows = rows;
        self
    }

    /// The row state the collection gives the item: the rename editor, the stripe, the app's mark and the press the
    /// collection holds on the row.
    #[must_use]
    pub fn state(mut self, row: RowState) -> Self {
        self.renaming = row.renaming;
        self.pressed = row.pressed;
        self.menu_open |= row.menu_open;
        self.striped = row.striped;
        self.highlight |= row.emphasized;
        self
    }

    /// The column `column` reads in the muted look: its text cell, or the label when it is the label column.
    #[must_use]
    pub fn cell_muted(mut self, column: ColumnKey) -> Self {
        self.muted.push(column);
        self
    }

    /// The wrapped text of the column `column` (`cell_lines`) can be selected and copied: a press on it focuses the
    /// text instead of acting on the row, and Tab passes over it.
    #[must_use]
    pub fn cell_selectable(mut self, column: ColumnKey) -> Self {
        self.selectable.push(column);
        self
    }

    /// A toggle cell in the column `cell.column`; the collection flips it, the row shows it.
    #[must_use]
    pub fn toggle(mut self, cell: ToggleCell) -> Self {
        self.toggles.push(cell);
        self
    }

    /// `text` in the column `column` of `cells(..)`, in the body role and the row's ink on one line, elided. One call
    /// per text column; a column that `cells(..)` does not list (hidden, or a plain list) shows nothing. A press on
    /// it acts on the row.
    #[must_use]
    pub fn cell(mut self, column: ColumnKey, text: impl Into<Label>) -> Self {
        self.texts.push(TextCell { column, text: text.into(), wrap: false });
        self
    }

    /// A component in the cell of `column` in place of a text: a field that edits the row's value in that column (a
    /// tab stop's position or alignment). It fills the cell's width, inset as a text cell is, at its own height up to
    /// the row's. In the column the label takes it stands in the label's place, and the label stays the row's
    /// accessible name. The row shows `field` and hands its output to `output` in the pass it is shown; its stop joins
    /// the row's roving group as a trailing field's does, and it takes its own presses and the keys it declares. A
    /// column that `cells(..)` does not list shows nothing.
    #[must_use]
    pub fn cell_field<C: Show + 'a>(mut self, column: ColumnKey, field: C, output: impl FnOnce(C::Output) + 'a) -> Self {
        self.cell_fields.push((column, Hosted { item: Some(Box::new(Hosting::new(field, output))), size: None }));
        self
    }

    /// Like `cell`, with the text wrapped to the column's width; the row is as high as its tallest wrapped cell.
    #[must_use]
    pub fn cell_lines(mut self, column: ColumnKey, text: impl Into<Label>) -> Self {
        self.texts.push(TextCell { column, text: text.into(), wrap: true });
        self
    }

    /// A thumbnail of `size` before the label, framed just outside; `paint` draws into the thumbnail's rect.
    /// `ThumbSize::None` draws none.
    #[must_use]
    pub fn thumb(mut self, size: ThumbSize, paint: impl FnOnce(&ContentPainterCx, Rect) + 'a) -> Self {
        self.thumb = size;
        self.thumb_aspect = ThumbAspect::SQUARE;
        self.first = Some(Box::new(paint));
        self
    }

    /// A thumbnail as high as `size` and `aspect` times as wide, for a gradient or other strip that is not square:
    /// its height sets the row's height as for any thumbnail. `ThumbSize::None` draws none.
    #[must_use]
    pub fn thumb_aspect(mut self, size: ThumbSize, aspect: ThumbAspect, paint: impl FnOnce(&ContentPainterCx, Rect) + 'a) -> Self {
        self = self.thumb(size, paint);
        self.thumb_aspect = aspect;
        self
    }

    /// A thumbnail after the first and any earlier extra ones, of the same size and frame (a layer's mask). Extra
    /// thumbnails never drop out for width: the label is cut first. Nothing is drawn without a first thumbnail.
    #[must_use]
    pub fn extra_thumb(mut self, paint: impl FnOnce(&ContentPainterCx, Rect) + 'a) -> Self {
        self.extras.push(Thumb { paint: Some(Box::new(paint)), link: None });
        self
    }

    /// A link glyph before the extra thumbnail added last: linked when `linked`. A click reports
    /// [`RowEvent::LinkToggled`]. `name` is its name for a screen reader, a whole pattern with `{row}` for the
    /// row's label. Does nothing before an [`extra_thumb`](ListRow::extra_thumb).
    #[must_use]
    pub fn thumb_link(mut self, linked: bool, name: impl Into<Label>) -> Self {
        if let Some(thumb) = self.extras.last_mut() {
            thumb.link = Some((linked, name.into()));
        }
        self
    }

    /// A text at the trailing end, in the caption role and `text.muted`, as wide as it measures, that always shows.
    /// The first item added is the outermost.
    #[must_use]
    pub fn trailing(mut self, text: impl Into<Label>) -> Self {
        self.trailing.push(Trailing { item: TrailingItem::Text { text: text.into(), name: None, strong: false }, optional: false });
        self
    }

    /// A short mark at the trailing end, shown as [`trailing`](Self::trailing) shows a text, that stands for `name`
    /// (a font class's sign for the class): `name` is its tooltip and part of the row's description.
    #[must_use]
    pub fn trailing_named(mut self, text: impl Into<Label>, name: impl Into<Label>) -> Self {
        self.trailing.push(Trailing { item: TrailingItem::Text { text: text.into(), name: Some(name.into()), strong: false }, optional: false });
        self
    }

    /// A short mark shown as [`trailing_named`](Self::trailing_named) shows one, in the caption role's semibold face
    /// (a layer's `fx`).
    #[must_use]
    pub fn trailing_strong(mut self, text: impl Into<Label>, name: impl Into<Label>) -> Self {
        self.trailing.push(Trailing { item: TrailingItem::Text { text: text.into(), name: Some(name.into()), strong: true }, optional: false });
        self
    }

    /// A text at the trailing end that drops out when the label would get narrower than its least width with it. A row
    /// sized to its content makes room for it.
    #[must_use]
    pub fn optional(mut self, text: impl Into<Label>) -> Self {
        self.trailing.push(Trailing { item: TrailingItem::Text { text: text.into(), name: None, strong: false }, optional: true });
        self
    }

    /// An icon at the trailing end, as wide as the row's icon, in the row's ink (its `trailing_tone` when set) unless the spec names a colour. It
    /// always shows. `name` is its name for a screen reader and its tooltip; the icon is no focus stop and `name` is
    /// part of the row's description.
    #[must_use]
    pub fn trailing_icon(mut self, icon: impl Into<IconSpec<'static>>, name: impl Into<Label>) -> Self {
        self.trailing.push(Trailing { item: TrailingItem::Icon { icon: icon.into(), name: name.into() }, optional: false });
        self
    }

    /// A pressable item at the trailing end, as wide as the row's icon, that always shows. A press reports
    /// [`RowEvent::Trailing`] and leaves the selection as it is; the part is a focus stop reached from the row by
    /// Forward, and Space and Enter activate it. `name` is its name for a screen reader and its tooltip.
    #[must_use]
    pub fn trailing_part(mut self, part: TrailingPart, face: TrailingFace, name: impl Into<Label>) -> Self {
        debug_assert!(
            !self.trailing.iter().any(|held| matches!(&held.item, TrailingItem::Part { part: have, .. } if *have == part)),
            "trailing part {part:?} is already in the row"
        );
        self.trailing.push(Trailing { item: TrailingItem::Part { part, face, name: name.into(), enabled: true, revealed: false }, optional: false });
        self
    }

    /// A trailing part as [`trailing_part`](Self::trailing_part) adds one, shown only on a hovered or selected row,
    /// while it has focus or while the menu it opens (`ItemCx::part_opener`) is open: a row's "More" button. It keeps
    /// its place and takes a press where it is hidden.
    #[must_use]
    pub fn trailing_part_revealed(mut self, part: TrailingPart, face: TrailingFace, name: impl Into<Label>) -> Self {
        self = self.trailing_part(part, face, name);
        if let Some(Trailing { item: TrailingItem::Part { revealed, .. }, .. }) = self.trailing.last_mut() {
            *revealed = true;
        }
        self
    }

    /// The row's "More" button, a [revealed](Self::trailing_part_revealed) part named "More ({row})": a click on it
    /// asks for the row's menu, which the collection reports as `CollectionEvent::ContextMenu` hung from the button.
    #[must_use]
    pub fn row_menu(self) -> Self {
        self.trailing_part_revealed(TrailingPart::MENU, TrailingFace::Icon(IconSpec::svg(craft_behavior::IconId::new("more"))), Label::Empty)
    }

    /// A component at the trailing end, as wide as it measures, that always shows: a field that edits a value of the
    /// row, or a button. The row shows `field` in its slot and hands its output to `output` in the pass it is shown.
    /// The field's stop joins the row's roving group like the row's focusable parts (Forward from the row enters it,
    /// Tab leaves the collection); it takes the keys it declares.
    #[must_use]
    pub fn trailing_field<C: Show + 'a>(self, field: C, output: impl FnOnce(C::Output) + 'a) -> Self {
        self.hosted(Hosting::new(field, output), false)
    }

    /// A component at the trailing end as [`trailing_field`](Self::trailing_field) adds one, that drops out when the
    /// label would get narrower than its least width with it (a binding's badge). A row sized to its content makes
    /// room for it.
    #[must_use]
    pub fn optional_field<C: Show + 'a>(self, field: C, output: impl FnOnce(C::Output) + 'a) -> Self {
        self.hosted(Hosting::new(field, output), true)
    }

    /// Empty room at the trailing end, as wide as `component` measures: a row that lacks a field the other rows have
    /// keeps the items after it in line with theirs. Nothing is shown.
    #[must_use]
    pub fn trailing_space<C: Show + 'a>(self, component: C) -> Self {
        self.hosted(Hosting::new(component, |_| {}).hidden(), false)
    }

    /// A component under the label, detail and wrapped lines, as wide as it measures: the components added this way
    /// stand in one line from the label's start edge, in the order added (a row that asks its question in place shows
    /// the answers here). The row grows by the line's height. The row shows `component` and hands its output to
    /// `output` in the pass it is shown; its stop joins the row's roving group as a trailing field's does.
    #[must_use]
    pub fn below<C: Show + 'a>(mut self, component: C, output: impl FnOnce(C::Output) + 'a) -> Self {
        self.below.push(Hosted { item: Some(Box::new(Hosting::new(component, output))), size: None });
        self
    }

    fn hosted(mut self, hosting: impl for<'r> crate::contract::Hosted<ListRow<'r>> + 'a, optional: bool) -> Self {
        let item = Hosted { item: Some(Box::new(hosting)), size: None };
        self.trailing.push(Trailing { item: TrailingItem::Hosted(item), optional });
        self
    }

    /// A short text in the leading slot (a page number), in `lead_role` and the detail's ink, at the slot's end so the labels
    /// after it line up: the slot is as wide as `widest` measures. It takes the icon's place; thumbnails replace it.
    /// A click on it picks the row as a click on the row does and reports [`RowEvent::LeadClicked`]; a double click
    /// there does not activate the row.
    #[must_use]
    pub fn lead_text(mut self, text: impl Into<Label>, widest: impl Into<Label>) -> Self {
        self.lead_text = Some((text.into(), widest.into()));
        self
    }

    /// A drag that starts on `spot` drags `payload` through the drag session, with a ghost as large as the part, and
    /// reports [`RowEvent::PartDrag`]; the row does not drag.
    ///
    /// A thumbnail keeps its click. A trailing part that drags is a handle on the row: a click on it is a click on the
    /// row and reports no [`RowEvent::Trailing`], and it is no focus stop (a layer's `fx`, whose drag carries the
    /// layer's effects).
    #[must_use]
    pub fn part_drag<P: Clone + Send + Sync + 'static>(mut self, spot: RowDragSpot, payload: P) -> Self {
        self.drags.retain(|(held, _)| *held != spot);
        self.drags.push((spot, Arc::new(payload)));
        self
    }

    /// The cursor while the pointer is over the thumbnail `target` (the load-selection hand while Cmd is held over a
    /// layer's thumbnail).
    #[must_use]
    pub fn thumb_cursor(mut self, target: ThumbTarget, cursor: impl Into<Cursor>) -> Self {
        self.cursors.retain(|(held, _)| *held != target);
        self.cursors.push((target, cursor.into()));
        self
    }

    /// The payload of the drag that starts on `spot`, if it drags.
    pub(super) fn drag_at(&self, spot: RowDragSpot) -> Option<&ThumbPayload> {
        self.drags.iter().find(|(held, _)| *held == spot).map(|(_, payload)| payload)
    }
}
