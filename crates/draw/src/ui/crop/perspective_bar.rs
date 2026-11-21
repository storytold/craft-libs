//! [`PerspectiveCropBar`]: the Perspective Crop tool's options.

use super::bar_words::CropBarWords;
use super::size_cells::{self, SizeLimits};
use super::{CropDocument, CropSizeFields};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Gap, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Divider;
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::Env;
use egui::{Response, Ui};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The Perspective Crop options: W × H × Resolution, Front Image, Clear, a divider and Show Grid.
///
/// - The fields hold W to 1 to 300 000 px and the resolution to 1 to 29 999.999 px/in (px/cm divides by 2.54); a
///   typed value outside is kept at the limit and reported as [`PerspectiveCropBarOutput::out_of_range`].
/// - Front Image fills W, H and the resolution from the document, rounded inward to the limits; Clear empties the
///   three and puts the resolution unit back to px/in.
#[must_use = "show the bar's children with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct PerspectiveCropBar<'a> {
    size: &'a mut CropSizeFields,
    document: &'a CropDocument,
    show_grid: Bind<'a, bool>,
}

/// What [`PerspectiveCropBar::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies what the bar reports"]
pub struct PerspectiveCropBarOutput {
    /// The strip's response.
    pub response: Response,
    /// A field changed, or Front Image or Clear was pressed.
    pub changed: bool,
    /// A typed value was out of range: the host opens the range alert.
    pub out_of_range: Option<craft_ui::text::RangeAlert>,
}

impl Outcome for PerspectiveCropBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a> PerspectiveCropBar<'a> {
    /// The bar editing `size` over `document`, with Show Grid bound to `show_grid`.
    pub fn new(size: &'a mut CropSizeFields, document: &'a CropDocument, show_grid: Bind<'a, bool>) -> Self {
        PerspectiveCropBar { size, document, show_grid }
    }

    /// Shows the bar's children.
    pub fn show(self, ui: &mut Ui) -> PerspectiveCropBarOutput {
        let PerspectiveCropBar { size, document, mut show_grid } = self;
        let env = Env::of(ui);
        let words = env.words::<CropBarWords>();
        let units = env.units();
        let has_document = document.size.width > 0.0 && document.size.height > 0.0;
        let mut changed = false;
        let mut out_of_range = None;
        let shown = Row::new().gap(Gap::Item).show(ui, |ui, _| {
            let cells = size_cells::show(ui, size, SizeLimits::PERSPECTIVE, document, &units, &words);
            changed |= cells.changed;
            // The body can run more than once in a pass: a later run does not drop an alert an earlier one raised.
            if cells.out_of_range.is_some() {
                out_of_range = cells.out_of_range;
            }
            if clicked(&Button::new(words.front_image.clone()).enabled(has_document).show(ui)) {
                size.front_image(document, SizeLimits::PERSPECTIVE.front_limits(size.resolution_unit));
                changed = true;
            }
            if clicked(&Button::new(words.clear.clone()).show(ui)) {
                size.clear_size();
                changed = true;
            }
            Divider::vertical().show(ui);
            Checkbox::new(show_grid.reborrowed()).label(words.show_grid.clone()).show(ui);
        });
        PerspectiveCropBarOutput { response: shown.output, changed, out_of_range }
    }
}
