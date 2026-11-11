//! The glyph of the controls that collapse and expand a column of panels or tools.

use crate::primitives::{IconSpec, Mirror};
use craft_behavior::IconId;

/// The double chevron of a collapse or expand control, pointing to the right of the screen or to the left. It names a
/// direction on screen, where the column goes, so it never mirrors under right-to-left.
pub(crate) fn chevrons(right: bool) -> IconSpec<'static> {
    IconSpec::svg(IconId::new(if right { "chevrons_right" } else { "chevrons_left" })).mirror(Mirror::Never)
}
