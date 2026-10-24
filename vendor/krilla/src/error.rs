//! Error handling.
//!
//! There are a lot of things that can go wrong when writing a PDF, like for example when
//! invalid fonts are provided. This module provides the basic error types krilla uses.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;

use crate::configure::{ValidationError, Validators};
#[cfg(feature = "raster-images")]
use crate::graphics::image::Image;
#[cfg(feature = "pdf")]
use crate::pdf::{PdfDocument, PdfError};
use crate::surface::Location;
use crate::tagging::TagId;
use crate::text::Font;

/// A wrapper type for krilla errors.
pub type KrillaResult<T> = Result<T, KrillaError>;

/// An error in krilla.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum KrillaError {
    /// A page or surface failed during cleanup; discard the complete document.
    Poisoned,
    /// [`GroupColorSpace::Cmyk`] with `no_device_cs` needs a `cmyk_profile` for the group
    /// color space.
    ///
    /// [`GroupColorSpace::Cmyk`]: crate::GroupColorSpace::Cmyk
    MissingGroupCmykProfile,
    /// A mesh gradient's corner colors are in color spaces that can't be written as one
    /// (see [`MeshGradient`]), for example RGB with CMYK.
    ///
    /// [`MeshGradient`]: crate::paint::MeshGradient
    MixedMeshColorSpaces(Option<Location>),
    /// [`Surface::start_optional_content`] was given an [`OcgId`] that this document did
    /// not create.
    ///
    /// [`Surface::start_optional_content`]: crate::surface::Surface::start_optional_content
    /// [`OcgId`]: crate::optional_content::OcgId
    UnknownOptionalContentGroup(Option<Location>),
    /// Optional content marks were not balanced: an end without a start, a mark still open
    /// when its stream finished, or a mark overlapping a tagged section.
    UnbalancedOptionalContent(Option<Location>),
    /// An error while attempting to embed a font.
    Font(Font, String),
    /// A list of validation errors. Can only occur if you set the `validator` in
    /// the [`SerializeSettings`] to something else than the dummy validator.
    ///
    /// [`SerializeSettings`]: crate::SerializeSettings
    Validation(Vec<(ValidationError, Validators)>),
    /// A hard limit of the selected PDF version was exceeded.
    Limit(LimitError),
    /// The same destination name has been associated with two different destinations,
    /// which is prohibited.
    DuplicateNamedDestination(Arc<String>),
    /// A duplicate [`Tag::id`] was provided.
    ///
    /// [`Tag::id`]: crate::interchange::tagging::Tag::id
    DuplicateTagId(TagId, Option<Location>),
    /// A [`TagId`] was not found in the [`TagTree`].
    ///
    /// [`TagTree`]: crate::interchange::tagging::TagTree
    UnknownTagId(TagId, Option<Location>),
    /// An image couldn't be processed properly.
    ///
    /// The third argument contains the error message.
    #[cfg(feature = "raster-images")]
    Image(Image, Option<Location>, String),
    /// An embedded PDF document couldn't be processed properly.
    #[cfg(feature = "pdf")]
    Pdf(PdfDocument, PdfError, Option<Location>),
    /// A sixteen bit image was used, even though it isn't
    /// supported by the used PDF version (only available in PDF 1.5+).
    #[cfg(feature = "raster-images")]
    SixteenBitImage(Image, Option<Location>),
}

impl Display for KrillaError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            KrillaError::Poisoned => f.write_str("document contains a failed page or surface"),
            KrillaError::MissingGroupCmykProfile => {
                f.write_str("a CMYK group color space with no_device_cs needs a CMYK profile")
            }
            KrillaError::MixedMeshColorSpaces(location) => {
                f.write_str("a mesh gradient mixes color spaces that can't be written as one")?;
                write_location(f, *location)
            }
            KrillaError::UnknownOptionalContentGroup(location) => {
                f.write_str("an optional content group from another document was used")?;
                write_location(f, *location)
            }
            KrillaError::UnbalancedOptionalContent(location) => {
                f.write_str("optional content marks are not balanced")?;
                write_location(f, *location)
            }
            KrillaError::Font(_, message) => write!(f, "failed to embed font: {message}"),
            KrillaError::Validation(errors) => {
                let count = errors.len();
                write!(
                    f,
                    "validation failed with {count} {}",
                    if count == 1 { "error" } else { "errors" }
                )
            }
            KrillaError::Limit(error) => write!(f, "PDF version limit exceeded: {error}"),
            KrillaError::DuplicateNamedDestination(name) => {
                write!(f, "duplicate named destination: {name}")
            }
            KrillaError::DuplicateTagId(id, location) => {
                write!(f, "duplicate tag id {id:?}")?;
                write_location(f, *location)
            }
            KrillaError::UnknownTagId(id, location) => {
                write!(f, "unknown tag id {id:?}")?;
                write_location(f, *location)
            }
            #[cfg(feature = "raster-images")]
            KrillaError::Image(_, location, message) => {
                write!(f, "failed to process image")?;
                write_location(f, *location)?;
                write!(f, ": {message}")
            }
            #[cfg(feature = "pdf")]
            KrillaError::Pdf(_, error, location) => {
                write!(f, "failed to process embedded PDF")?;
                write_location(f, *location)?;
                write!(f, ": {error}")
            }
            #[cfg(feature = "raster-images")]
            KrillaError::SixteenBitImage(_, location) => {
                write!(
                    f,
                    "sixteen bit images require PDF 1.5 or newer, but the selected PDF version does not support them"
                )?;
                write_location(f, *location)
            }
        }
    }
}

impl Error for KrillaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            KrillaError::Limit(error) => Some(error),
            #[cfg(feature = "pdf")]
            KrillaError::Pdf(_, error, _) => Some(error),
            _ => None,
        }
    }
}

/// A limit imposed by the selected PDF version.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum LimitError {
    /// A float exceeded the maximum allowed size for the PDF version.
    TooLargeFloat,
    /// An array exceeded the maximum allowed length for the PDF version.
    TooLongArray,
    /// A dictionary exceeded the maximum allowed number of entries for the PDF version.
    TooLongDictionary,
}

impl Display for LimitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            LimitError::TooLargeFloat => write!(f, "a float exceeded the maximum allowed size"),
            LimitError::TooLongArray => write!(f, "an array exceeded the maximum allowed length"),
            LimitError::TooLongDictionary => {
                write!(
                    f,
                    "a dictionary exceeded the maximum allowed number of entries"
                )
            }
        }
    }
}

impl Error for LimitError {}

fn write_location(f: &mut Formatter<'_>, location: Option<Location>) -> fmt::Result {
    if let Some(location) = location {
        write!(f, " at location {location}")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::{KrillaError, LimitError};

    #[test]
    fn krilla_error_implements_error() {
        let error = KrillaError::Limit(LimitError::TooLongArray);

        assert_eq!(
            error.to_string(),
            "PDF version limit exceeded: an array exceeded the maximum allowed length"
        );
        assert!(error.source().is_some());
    }
}
