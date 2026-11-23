//! The VML attributes a page background's fill (`v:background/v:fill`) carries that this crate
//! models: its reader consumes them and its writer writes them (ECMA-376 Part 4 §6.1.2.5).
//! Every other attribute is kept verbatim on the document's [`craft_wordmodel::PageFillExtra`],
//! so a file's Office extensions ([MS-ODRAWXML] §2.2) survive a save.

/// `v:fill` attributes the page fill model holds: its type and colours, its gradient's angle,
/// focus and stops, its picture, and its transparency.
pub(crate) const MODELED_FILL_ATTRS: &[&str] = &[
    "type",
    "color",
    "color2",
    "opacity",
    "o:opacity2",
    "angle",
    "colors",
    "focus",
    "focusposition",
    "focussize",
    "rotate",
    "method",
    "r:id",
    "r:embed",
    "o:relid",
    "o:title",
];
