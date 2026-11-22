//! `SwatchPalette`: a wrapping grid of colour cells that chooses one colour. Its table of cells is
//! `craft_ui::choice::SwatchTable`.

use craft_behavior::{EditEvent, Mixed, Selection};
use craft_theme::Srgba;
use craft_ui::choice::{SwatchSpacing, SwatchTable};
use craft_ui::contract::{Bind, Held, ListDrag, NoValue, Outcome, Pattern, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::Label;
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
    /// The pick of this pass as one `Set` of the swatch's colour; `None` when nothing was picked. The palette has
    /// already written it to its binding.
    pub edit: Option<EditEvent<Srgba>>,
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
///   tooltip.
/// - A click, Space or Enter on a swatch, or a screen reader's click, sets its colour as one `Set`.
/// - The palette is one Tab stop: Tab enters on the selected swatch (the first when none is) and the next Tab leaves.
///   Arrows move the focus through the cells as the rows lie, Up and Down to the cell above or below; they choose
///   nothing.
/// - AccessKit: a list box named by the label; each cell a list box option named by its name, with the hex colour as
///   its value.
#[must_use = "show the palette with `show`"]
#[derive(Pattern)]
pub struct SwatchPalette<'a> {
    label: Label,
    color: Bind<'a, Srgba>,
    swatches: &'a [(Label, Srgba)],
    spacing: SwatchSpacing,
    drag: Option<TableMap>,
}

impl<'a> SwatchPalette<'a> {
    /// A palette named `label` of `swatches`, each a name and a colour, choosing the colour bound to `color`.
    pub fn new(label: impl Into<Label>, color: Bind<'a, Srgba>, swatches: &'a [(Label, Srgba)]) -> Self {
        SwatchPalette { label: label.into(), color, swatches, spacing: SwatchSpacing::Touching, drag: None }
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

    /// Shows the palette.
    pub fn show(self, ui: &mut Ui) -> SwatchPaletteOutput {
        let SwatchPalette { label, color, swatches, spacing, drag } = self;
        let mut held = Held::new(color);
        let name = label.clone();
        let mut shown = Stack::new().show(ui, |ui, cx| {
            let before = held.read(cx)?;
            let selected = swatches.iter().position(|(_, c)| c.to_rgba() == before.to_rgba());
            let mut current = selected.map_or_else(Selection::default, Selection::single);
            let table = SwatchTable::new(label.clone(), swatches).tips(true).wells(true).spacing(spacing).selection(Bind::bound(&mut current));
            let table = match &drag {
                Some(map) => map(table),
                None => table,
            };
            let table = table.show_all(ui);
            let picked = table.selected.or(table.open)?;
            let after = swatches.get(picked)?.1;
            if after.to_rgba() == before.to_rgba() {
                return None;
            }
            Some((before, after))
        });
        // Once per pass, after the real run: the picked colour is stored and reported.
        let edit = shown
            .settle(|cx, picked| {
                let (before, after) = picked.take()?;
                held.write(cx, &before, after, name.as_str());
                Some(EditEvent::set(Mixed::Same(before), after, Arc::from(name.as_str()), None))
            })
            .flatten();
        SwatchPaletteOutput { response: shown.output, edit }
    }
}
