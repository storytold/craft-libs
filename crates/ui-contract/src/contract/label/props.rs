//! What a side label is: its kind, glyph, alignment, layout, `SideLabel` and `Scrub`.

use crate::contract::Tooltip;
use crate::primitives::IconSpec;
use crate::runtime::access::scope::FieldLabelRole;
use crate::runtime::env::{EnvView, Label};
use craft_behavior::{ColumnWidth, PartKey, PartRef, ValueDrag};
use std::sync::Arc;

/// How a side label is shown.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum LabelKind {
    /// The label's text.
    #[default]
    Text,
    /// No visible label: the text still names the control. In a body whose labelled rows share a label column, the
    /// row keeps the column, so its control starts in the control column.
    Hidden,
    /// An icon or a short text shown in place of the label; the text still names the control. Glyph labels share
    /// a column of their own, apart from the text labels'. The label's detail joins the description and is not shown.
    Glyph(GlyphLabel),
}

/// What a glyph label shows.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Glyph {
    Icon(IconSpec<'static>),
    Text(Label),
}

/// The visible form of a glyph label: an icon, or a short text such as "W" or "H". The accessible name stays the
/// label's `Common.label`.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphLabel {
    shown: Glyph,
}

impl GlyphLabel {
    /// Shows `icon`, as large as the control's icons.
    pub fn icon(icon: impl Into<IconSpec<'static>>) -> Self {
        GlyphLabel { shown: Glyph::Icon(icon.into()) }
    }

    /// Shows `text`, resolved in the scope's language.
    pub fn text(text: impl Into<Label>) -> Self {
        GlyphLabel { shown: Glyph::Text(text.into()) }
    }

    /// What the label leaf draws: the icon, or the text resolved in `env`.
    pub(super) fn resolved(&self, env: EnvView<'_>) -> Glyph {
        match &self.shown {
            Glyph::Icon(icon) => Glyph::Icon(icon.clone()),
            Glyph::Text(text) => Glyph::Text(env.resolve(text)),
        }
    }
}

/// Where a beside label sits in its column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LabelAlign {
    /// Against the control: the text ends where the gap before the control begins, on the start side under RTL.
    #[default]
    End,
    /// At the row's start edge.
    Start,
}

/// Where a side label sits relative to its control.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum FieldLayout {
    /// In a column before the control, above it when the row is too narrow.
    #[default]
    Beside,
    /// Above the control.
    Above,
    /// Above the control, at most this wide.
    Caption(CellWidth),
    /// After the control, as check box and radio labels sit: the control takes its narrowest width and the label
    /// follows it, on the end side under RTL. The row never stacks.
    After,
    /// On one line: the label at the start and the control in a `size.length.number` cell at the end, as a number
    /// field sits over its slider. The label elides before the cell. The row never stacks.
    Line,
    /// In the control's box at its start, in the slot a control that offers one shows it (a number field's leading
    /// adornment); the control takes the whole row. For a glyph label of a body that is one such control: the slot
    /// is the label's, so a drag on it scrubs a scrubbing label at any time. A control that offers no slot shows no
    /// label; the label still names it.
    Inside,
}

/// What a control shows in its slot for a side label with `FieldLayout::Inside` (`EnvView::inside_label`): the
/// glyph, the label's tooltip and whether a drag on the slot scrubs. The control declares the slot with
/// `PartsCx::label_part`.
#[derive(Clone, Debug, PartialEq)]
pub struct InsideLabel {
    glyph: Glyph,
    tooltip: Option<Tooltip>,
    scrubs: bool,
}

impl InsideLabel {
    /// The part a control declares its slot as (`PartsCx::label_part`).
    pub const PART: PartKey = PartKey::new("inside_label");

    /// The glyph's icon, for an icon glyph.
    pub fn icon(&self) -> Option<&IconSpec<'static>> {
        match &self.glyph {
            Glyph::Icon(icon) => Some(icon),
            Glyph::Text(_) => None,
        }
    }

    /// The glyph's text, resolved in the scope's language, for a text glyph.
    pub fn text(&self) -> Option<&Label> {
        match &self.glyph {
            Glyph::Text(text) => Some(text),
            Glyph::Icon(_) => None,
        }
    }

    /// The label's tooltip, shown over the slot.
    pub fn tooltip(&self) -> Option<&Tooltip> {
        self.tooltip.as_ref()
    }

    /// A drag on the slot scrubs the control's value.
    pub fn scrubs(&self) -> bool {
        self.scrubs
    }
}

/// The inside label a control under `side` shows in its slot: `Some` for a glyph label with `FieldLayout::Inside`.
pub(crate) fn inside(env: EnvView<'_>, side: &SideLabel, enabled: bool) -> Option<Arc<InsideLabel>> {
    let LabelKind::Glyph(glyph) = &side.kind else { return None };
    (side.layout == FieldLayout::Inside)
        .then(|| Arc::new(InsideLabel { glyph: glyph.resolved(env), tooltip: side.tooltip.clone(), scrubs: side.scrub && enabled }))
}

/// The width of a caption's cell.
pub type CellWidth = ColumnWidth;

/// What `ContainerProps.label` holds: how the label is shown and what it does. The text is `Common.label`.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SideLabel {
    /// How the label is shown.
    pub kind: LabelKind,
    /// Where it sits.
    pub layout: FieldLayout,
    /// Where a beside label sits in its column.
    pub align: LabelAlign,
    /// Whether it names or describes its control.
    pub role: FieldLabelRole,
    /// Dragging the label scrubs the control's value.
    pub scrub: bool,
    /// The label shows a dot at its end while the value differs from its reset value. Off unless a caller asks.
    pub reset_dot: bool,
    /// An icon before the text, as large as the control's icons.
    pub icon: Option<IconSpec<'static>>,
    /// A line after the text in the muted text colour. It joins the control's description, not its name.
    pub detail: Option<Label>,
    /// The label's tooltip, in place of its text: a hint a short or glyph label has no room for.
    pub tooltip: Option<Tooltip>,
}

impl SideLabel {
    /// The label step shows no label leaf: a hidden label, or one the control shows inside its box.
    pub(super) fn shows_no_leaf(&self) -> bool {
        self.kind == LabelKind::Hidden || self.layout == FieldLayout::Inside
    }

    /// A text label beside its control that names it.
    pub const fn new() -> Self {
        SideLabel {
            kind: LabelKind::Text,
            layout: FieldLayout::Beside,
            align: LabelAlign::End,
            role: FieldLabelRole::Names,
            scrub: false,
            reset_dot: false,
            icon: None,
            detail: None,
            tooltip: None,
        }
    }

    /// How the label is shown.
    #[must_use]
    pub fn kind(mut self, kind: LabelKind) -> Self {
        self.kind = kind;
        self
    }

    /// Where the label sits.
    #[must_use]
    pub const fn layout(mut self, layout: FieldLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Where a beside label sits in its column: against the control by default.
    #[must_use]
    pub const fn align(mut self, align: LabelAlign) -> Self {
        self.align = align;
        self
    }

    /// Whether the label names or describes its control.
    #[must_use]
    pub const fn role(mut self, role: FieldLabelRole) -> Self {
        self.role = role;
        self
    }

    /// Lets a drag on the label scrub the control's value.
    #[must_use]
    pub const fn scrub(mut self, on: bool) -> Self {
        self.scrub = on;
        self
    }

    /// Shows a dot after the text while the control's value differs from its reset value; a click on it resets the value.
    #[must_use]
    pub const fn reset_dot(mut self, on: bool) -> Self {
        self.reset_dot = on;
        self
    }

    /// Shows `icon` before the text.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<IconSpec<'static>>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Shows `detail` after the text, in the muted text colour, and adds it to the control's description.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<Label>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Shows `tooltip` over the label in place of its text, and adds it to the label's description.
    #[must_use]
    pub fn tooltip(mut self, tooltip: impl Into<Tooltip>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

/// How a drag changes a control's value: the drag maths over the control's number, and the sources a drag starts on
/// (`by_label`, `on_field`, `part`). A scrub with no source does nothing.
pub struct Scrub<V> {
    pub(crate) drag: ValueDrag,
    pub(crate) by_label: bool,
    pub(crate) on_field: bool,
    pub(crate) parts: Vec<(PartRef, ValueDrag)>,
    pub(crate) read: Arc<dyn Fn(&V) -> f64 + Send + Sync>,
    pub(crate) write: Arc<dyn Fn(f64) -> V + Send + Sync>,
}

impl<V> Clone for Scrub<V> {
    fn clone(&self) -> Self {
        Scrub {
            drag: self.drag.clone(),
            by_label: self.by_label,
            on_field: self.on_field,
            parts: self.parts.clone(),
            read: Arc::clone(&self.read),
            write: Arc::clone(&self.write),
        }
    }
}

impl<V> Scrub<V> {
    /// A scrub by `drag`, reading the number a value holds with `read` and making a value of a number with `write`.
    /// Both may capture the control's decimals and step, so a scrubbed value is rounded the way the control rounds.
    pub fn new(drag: ValueDrag, read: impl Fn(&V) -> f64 + Send + Sync + 'static, write: impl Fn(f64) -> V + Send + Sync + 'static) -> Self {
        Scrub { drag, by_label: false, on_field: false, parts: Vec::new(), read: Arc::new(read), write: Arc::new(write) }
    }

    /// A drag on the control's side label scrubs, with or without the focus in the control; the label senses the
    /// drag from the pass after the control first declared it.
    #[must_use]
    pub fn by_label(mut self) -> Self {
        self.by_label = true;
        self
    }

    /// A drag on the part `part` scrubs by `drag`, with or without the focus in the control: a scrub handle, or an
    /// opener whose drag moves the value as far as its popup's control would.
    #[must_use]
    pub fn part(mut self, part: impl Into<PartRef>, drag: ValueDrag) -> Self {
        self.parts.push((part.into(), drag));
        self
    }

    /// A drag on the control's body scrubs while its text part does not take the pointer: a field that is not in
    /// edit. A click enters edit, and in edit a drag selects text. The control must sense a drag.
    #[must_use]
    pub fn on_field(mut self) -> Self {
        self.on_field = true;
        self
    }
}
