//! Modalities and capabilities: what kind of data, and what can be done to it.
//!
//! A capability id is `<modality>.<task>`, for example `image.inpaint` or `text.complete`. The set
//! is open: any well-formed id is valid, so an app can add `cad.explain` without a library change.
//! [`well_known`] lists the ones the suite expects to share, so providers and apps agree on
//! spelling.

use serde::{Deserialize, Serialize};

use crate::ids::{IdError, open_enum, validate_token};

open_enum!(
    /// The kind of data a capability works on.
    Modality {
        Text => "text",
        Image => "image",
        Video => "video",
        Audio => "audio",
        Document => "document",
        Tabular => "tabular",
        Vector => "vector",
        Mesh => "mesh",
    }
);

/// `<modality>.<task>`, validated. Both halves are tokens (no dots).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityId(String);

impl CapabilityId {
    /// Parses `"image.inpaint"`.
    pub fn parse(s: &str) -> Result<Self, IdError> {
        let (modality, task) = s.split_once('.').ok_or(IdError::Malformed("expected modality.task"))?;
        validate_token(modality)?;
        validate_token(task)?;
        Ok(Self(s.to_owned()))
    }

    /// Builds an id from a modality and a task token. Fails when `task` is not a valid token.
    pub fn new(modality: &Modality, task: &str) -> Result<Self, IdError> {
        validate_token(task)?;
        Ok(Self(format!("{}.{}", modality.as_str(), task)))
    }

    /// The full `modality.task` text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The modality half. Always present because construction guarantees the dot.
    pub fn modality(&self) -> Modality {
        let head = self.0.split_once('.').map_or("", |(m, _)| m);
        Modality::parse(head).unwrap_or_else(|_| Modality::Other(head.to_owned()))
    }

    /// The task half, for example `inpaint` in `image.inpaint`.
    pub fn task(&self) -> &str {
        self.0.split_once('.').map_or("", |(_, t)| t)
    }
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CapabilityId({:?})", self.0)
    }
}

impl std::str::FromStr for CapabilityId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for CapabilityId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CapabilityId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <String as Deserialize>::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Capability ids the suite expects to share. Not a closed list.
pub const WELL_KNOWN: &[&str] = &[
    "text.complete",
    "text.embed",
    "image.generate",
    "image.detect",
    "image.edit",
    "image.inpaint",
    "image.outpaint",
    "image.segment",
    "image.matte",
    "image.upscale",
    "image.vectorize",
    "audio.transcribe",
    "audio.generate",
    "audio.separate",
    "audio.enhance",
    "video.generate",
    "video.segment",
    "video.upscale",
    "document.ocr",
    "document.extract",
    "mesh.generate",
];

/// [`WELL_KNOWN`] as parsed ids.
pub fn well_known() -> Vec<CapabilityId> {
    WELL_KNOWN.iter().filter_map(|s| CapabilityId::parse(s).ok()).collect()
}
