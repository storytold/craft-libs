//! Pictures ([MS-ODRAW] §2.2.32–2.2.45): the drawing group's blip store lists every picture
//! (`FBSE`), either holding its blip or pointing into the `Pictures` stream. Shapes name a picture
//! by its 1-based index in the store. The decoder is `craft-officeart`'s.

pub use craft_officeart::blip::*;
