//! `SwatchPalette`: a wrapping grid of colour cells that chooses one colour. Its table of cells is
//! `craft_ui::choice::SwatchTable`.

use craft_behavior::{EditEvent, IconId, Mixed, Selection};
use craft_theme::Srgba;
use craft_ui::choice::{SwatchSpacing, SwatchTable};
use craft_ui::contract::{ActivateOn, Bind, Held, ListDrag, NoValue, Outcome, Pattern, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::memory::StageAccess;
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::style::TypeRole;
use egui::{Response, Ui};
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The table's drag mapper, type-erased so the palette needs no payload parameter.
type TableMap = Arc<dyn for<'t> Fn(SwatchTable<'t>) -> SwatchTable<'t>>;

/// What a [`SwatchPalette`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host reads the pick"]
pub struct SwatchPaletteOutput {
    /// The palette's response.
    pub response: Response,
    /// The pick of this pass as one `Set` of the swatch's colour; `None` when nothing was picked or the pick was the
    /// colour already bound. The palette has already written it to its binding.
    pub edit: Option<EditEvent<Srgba>>,
    /// The request of the Other… cell for the host's colour picker, at the bound colour and keyed by the key given to
    /// [`SwatchPalette::other`], when it was activated this pass.
    pub pick_color: Option<ColorRequest>,
    /// Every swatch click of this pass, also a click on the colour already bound, which files no `edit`.
    pub picked: Option<Srgba>,
}

impl Outcome for SwatchPaletteOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A wrapping grid of colour cells from the host's list of named colours that chooses one colour.
///
/// - The swatch equal to the colour (compared in 8 bits, the first when several are) is selected; the name is its
///   tooltip. An [`optional`](Self::optional) palette bound to no colour, or to a mixed one, selects none.
/// - A click, Space or Enter on a swatch, or a screen reader's click, sets its colour as one `Set`; a click on the
///   colour already bound sets nothing and still reports `picked`.
/// - The palette is one Tab stop: Tab enters on the selected swatch (the first when none is) and the next Tab leaves.
///   Arrows move the focus through the cells as the rows lie, Up and Down to the cell above or below; they choose
///   nothing.
/// - [`other`](Self::other) adds an Other… cell after the swatches, in the same Tab stop and arrow walk: activating it
///   files `pick_color` and sets nothing. [`recent`](Self::recent) adds a captioned row of recent colours under the
///   grid, its own Tab stop, whose pick sets the colour as a swatch's does.
/// - AccessKit: a list box named by the label; each cell a list box option named by its name, with the hex colour as
///   its value.
#[must_use = "show the palette with `show`"]
#[derive(Pattern)]
pub struct SwatchPalette<'a> {
    label: Label,
    color: PaletteColor<'a>,
    swatches: &'a [(Label, Srgba)],
    spacing: SwatchSpacing,
    drag: Option<TableMap>,
    /// The Other… cell: the key of its colour request and its name.
    other: Option<(String, Label)>,
    /// The recent row: its caption and colours.
    recent: Option<(Label, &'a [Srgba])>,
}

/// What a palette binds: a colour, or a colour that may be none.
enum PaletteColor<'a> {
    Color(Held<'a, Srgba>),
    Optional(Bind<'a, Option<Srgba>>),
}

impl PaletteColor<'_> {
    /// The bound colour as the palette shows it: `Same(None)` for no colour (or an optional binding that holds
    /// nothing), `Mixed` for several. `None` while a plain colour's binding holds no single colour, and the palette
    /// shows nothing.
    fn read(&mut self, cx: &mut impl StageAccess) -> Option<Mixed<Option<Srgba>>> {
        match self {
            PaletteColor::Color(held) => held.read(cx).map(|color| Mixed::Same(Some(color))),
            PaletteColor::Optional(bind) => Some(bind.reborrow().read(cx).unwrap_or(Mixed::Same(None))),
        }
    }

    /// Writes `after` over `before` as one `Set` named `name`.
    fn write(&mut self, cx: &mut impl StageAccess, before: Mixed<Option<Srgba>>, after: Srgba, name: &str) {
        match self {
            PaletteColor::Color(held) => {
                if let Mixed::Same(Some(before)) = before {
                    held.write(cx, &before, after, name);
                }
            }
            PaletteColor::Optional(bind) => {
                let _ = bind.reborrow().write(cx, &EditEvent::set(before, Some(after), Arc::from(name), None));
            }
        }
    }
}

/// What the body picked in its pass.
#[derive(Clone, Copy)]
enum Pick {
    /// A swatch or a recent colour.
    Color(Srgba),
    /// The Other… cell.
    Other,
}

/// The cell of `swatches` whose colour is `color`, compared in 8 bits: the first when several are.
fn cell_of(swatches: &[(Label, Srgba)], color: Option<Srgba>) -> Selection<usize> {
    color.and_then(|color| swatches.iter().position(|(_, c)| c.to_rgba() == color.to_rgba())).map_or_else(Selection::default, Selection::single)
}

impl<'a> SwatchPalette<'a> {
    /// A palette named `label` of `swatches`, each a name and a colour, choosing the colour bound to `color`.
    pub fn new(label: impl Into<Label>, color: Bind<'a, Srgba>, swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchPalette {
            label: label.into(),
            color: PaletteColor::Color(Held::new(color)),
            swatches,
            spacing: SwatchSpacing::Touching,
            drag: None,
            other: None,
            recent: None,
        }
    }

    /// A palette over a colour that may be none: a host binds its `Mixed<Option<Srgba>>` with `Bind::bound_mixed`.
    /// No swatch is selected for `Same(None)` or `Mixed`, and a pick writes `Mixed::Same(Some(colour))` and reports
    /// the same `edit` as [`new`](Self::new), its `before` the colour or `Mixed` when there was none.
    pub fn optional(label: impl Into<Label>, color: Bind<'a, Option<Srgba>>, swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchPalette {
            label: label.into(),
            color: PaletteColor::Optional(color),
            swatches,
            spacing: SwatchSpacing::Touching,
            drag: None,
            other: None,
            recent: None,
        }
    }

    /// The gap between the cells, passed on to its `SwatchTable`.
    pub fn spacing(mut self, spacing: SwatchSpacing) -> Self {
        self.spacing = spacing;
        self
    }

    /// The payload a cell drags, passed on to its `SwatchTable`.
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<usize>) -> Option<P> + Send + Sync + 'static) -> Self {
        let map = Arc::new(map);
        self.drag = Some(Arc::new(move |table| {
            let map = Arc::clone(&map);
            table.drag_payload(move |drag| map(drag))
        }));
        self
    }

    /// An Other… cell after the swatches, named `label`: activating it files a `ColorRequest` keyed `key` at the bound
    /// colour as `pick_color`, for the host's own picker, and sets nothing.
    pub fn other(mut self, key: impl Into<String>, label: impl Into<Label>) -> Self {
        self.other = Some((key.into(), label.into()));
        self
    }

    /// A row of recent `colors` under `caption`, below the grid, each named by its hex; an empty slice shows no row.
    pub fn recent(mut self, caption: impl Into<Label>, colors: &'a [Srgba]) -> Self {
        self.recent = Some((caption.into(), colors));
        self
    }

    /// Shows the palette.
    pub fn show(self, ui: &mut Ui) -> SwatchPaletteOutput {
        let SwatchPalette { label, mut color, swatches, spacing, drag, other, recent } = self;
        let recent = recent.filter(|(_, colors)| !colors.is_empty());
        let recent_cells: Vec<(Label, Srgba)> =
            recent.iter().flat_map(|(_, colors)| colors.iter()).map(|color| (Label::from(color.to_hex()), *color)).collect();
        let name = label.clone();
        let mut shown = Stack::new().show(ui, |ui, cx| {
            let before = color.read(cx)?;
            let bound = before.same().copied().flatten();
            let mut current = cell_of(swatches, bound);
            // A palette of recent colours alone has no grid: an empty table would be an empty list box in the tree.
            let mut pick = None;
            if !swatches.is_empty() || other.is_some() {
                // Every click activates, so a click on the swatch already selected, which changes no selection, still picks.
                let mut table = SwatchTable::new(label.clone(), swatches)
                    .tips(true)
                    .wells(true)
                    .spacing(spacing)
                    .activate(ActivateOn::Click)
                    .selection(Bind::bound(&mut current));
                if let Some(map) = &drag {
                    table = map(table);
                }
                if let Some((_, other_label)) = &other {
                    table = table.trailing(other_label.clone(), IconId::new("plus"));
                }
                let table = table.show_all(ui);
                // The Other… cell is keyed `swatches.len()`, past the swatches. A click selects on its press and activates
                // on its release, so only the activation files the request: one request for one click.
                pick = table.selected.or(table.open).and_then(|at| match swatches.get(at) {
                    Some((_, picked)) => Some(Pick::Color(*picked)),
                    None => (other.is_some() && table.open == Some(at) && at == swatches.len()).then_some(Pick::Other),
                });
            }
            if let Some((caption, _)) = &recent {
                Text::new(caption.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
                let mut current = cell_of(&recent_cells, bound);
                let row = SwatchTable::new(caption.clone(), &recent_cells)
                    .tips(true)
                    .wells(true)
                    .activate(ActivateOn::Click)
                    .selection(Bind::bound(&mut current))
                    .show_all(ui);
                if let Some((_, picked)) = row.selected.or(row.open).and_then(|at| recent_cells.get(at)) {
                    pick = Some(Pick::Color(*picked));
                }
            }
            Some((before, pick))
        });
        // Once per pass, after the real run: a changed colour is stored and reported, every pick is reported.
        let settled = shown
            .settle(|cx, result| {
                let (before, pick) = result.take()?;
                let bound = before.same().copied().flatten();
                match pick? {
                    Pick::Other => {
                        let key = other.as_ref().map(|(key, _)| key.clone())?;
                        Some((None, None, Some(ColorRequest::new(key, bound).title(name.clone()))))
                    }
                    Pick::Color(after) => {
                        let unchanged = bound.is_some_and(|bound| bound.to_rgba() == after.to_rgba());
                        let edit = (!unchanged).then(|| {
                            color.write(cx, before, after, name.as_str());
                            let reported = bound.map_or(Mixed::Mixed, Mixed::Same);
                            EditEvent::set(reported, after, Arc::from(name.as_str()), None)
                        });
                        Some((Some(after), edit, None))
                    }
                }
            })
            .flatten();
        let (picked, edit, pick_color) = settled.unwrap_or((None, None, None));
        SwatchPaletteOutput { response: shown.output, edit, pick_color, picked }
    }
}
