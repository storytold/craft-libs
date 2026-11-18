//! Serialization settings for CMYK print output.
//!
//! The document's CMYK profile, its working space, describes the printing condition.
//! [`cmyk_blending`] blends transparency in CMYK and writes no output intent, like a plain
//! print PDF; [`settings`] also writes the profile as the `GTS_PDFX` output intent, through
//! which DeviceCMYK content and the CMYK blending groups are interpreted, as PDF/X needs. This module takes the profile bytes and has no
//! color-management dependency; craft-cms's `builtin::COATED_FOGRA39_ICC` is a suitable
//! default profile, described by [`PrintColor::fogra39`].
//!
//! The settings carry no PDF/X version and are not checked against PDF/X: callers that claim
//! conformance set [`SerializeSettings::pdfx_version`] and run their own checks.

use super::backend::icc::ICCProfile;
use super::backend::{GroupColorSpace, OutputIntent, SerializeSettings, Trapped};
use super::{WriteError, guard};

/// The printing condition of a CMYK export.
#[derive(Clone, Copy, Debug)]
pub struct PrintColor<'a> {
    /// The CMYK ICC profile, written as `/DestOutputProfile`.
    pub profile: &'a [u8],
    /// `/OutputConditionIdentifier`: a registered name such as `FOGRA39`, or `Custom`.
    pub condition_identifier: &'a str,
    /// `/OutputCondition`: a human-readable description.
    pub condition: Option<&'a str>,
    /// `/RegistryName`, such as `http://www.color.org` for registered conditions.
    pub registry_name: Option<&'a str>,
    /// `/Info`: PDF/X requires it when the condition is not registered.
    pub info: Option<&'a str>,
}

impl<'a> PrintColor<'a> {
    /// A profile for the registered FOGRA39 condition (ISO 12647-2:2004 coated paper), such
    /// as craft-cms's `builtin::COATED_FOGRA39_ICC`.
    pub fn fogra39(profile: &'a [u8]) -> Self {
        Self {
            profile,
            condition_identifier: "FOGRA39",
            condition: Some("Coated FOGRA39 (ISO 12647-2:2004)"),
            registry_name: Some("http://www.color.org"),
            info: Some("Coated FOGRA39 (ISO 12647-2:2004)"),
        }
    }
}

/// Settings for a CMYK print PDF without an output intent: DeviceCMYK color and CMYK
/// blending groups, with `profile` as `cmyk_profile`. Other fields keep their defaults;
/// override them with struct update syntax.
///
/// Fails with [`WriteError::InvalidProfile`] when `profile` is not a readable CMYK ICC
/// profile.
pub fn cmyk_blending(profile: &[u8]) -> Result<SerializeSettings, WriteError> {
    guard(|| {
        let profile = ICCProfile::<4>::new(profile).ok_or(WriteError::InvalidProfile)?;
        Ok(SerializeSettings {
            no_device_cs: false,
            cmyk_profile: Some(profile),
            group_color_space: GroupColorSpace::Cmyk,
            ..SerializeSettings::default()
        })
    })
}

/// [`cmyk_blending`] plus the profile as the `GTS_PDFX` output intent and `/Trapped /False`,
/// for PDF/X export.
///
/// Fails with [`WriteError::InvalidProfile`] when `profile` is not a readable CMYK ICC
/// profile.
pub fn settings(color: PrintColor<'_>) -> Result<SerializeSettings, WriteError> {
    let settings = cmyk_blending(color.profile)?;
    let profile = settings.cmyk_profile.clone().ok_or(WriteError::InvalidProfile)?;
    Ok(SerializeSettings {
        output_intent: Some(OutputIntent {
            profile,
            condition_identifier: color.condition_identifier.to_owned(),
            condition: color.condition.map(str::to_owned),
            registry_name: color.registry_name.map(str::to_owned),
            info: color.info.map(str::to_owned),
        }),
        trapped: Some(Trapped::False),
        ..settings
    })
}
