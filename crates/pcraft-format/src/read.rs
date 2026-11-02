use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{COMPOSITE, Error, Limits, Manifest, Step, THUMB, migrate, parse_manifest};

/// Reads manifest JSON into a [`Manifest`] whose document is `D`.
///
/// 1. [`parse_manifest`] checks `json` against `limits` and parses it.
/// 2. [`migrate`] refuses a `format_version` newer than `target` with [`Error::TooNew`], which
///    holds the manifest's `generator`, and upgrades an older one with `steps`.
/// 3. The members are read in the order the parsed JSON object holds them, the document as `D`.
///
/// A missing `thumbnail` or `composite` reads as `None`, and members the envelope does not name
/// are skipped. A missing or invalid `format_version` is [`Error::Corrupt`], from [`migrate`].
/// Any other member of the wrong type is [`Error::Json`], and so is a missing `generator` or
/// `document`, which is reported after the members that are present, in the order serde's derived
/// `Deserialize` reports them. A `thumbnail` other than [`THUMB`] or a `composite` other than
/// [`COMPOSITE`] is [`Error::Corrupt`], checked after the members are read.
pub fn read_manifest<D: DeserializeOwned, E: From<Error>>(json: &[u8], limits: &Limits, steps: &[Step<E>], target: u32) -> Result<Manifest<D>, E> {
    read_manifest_with(json, limits, steps, target, |document| serde_json::from_value(document).map_err(|e| E::from(Error::Json(e))))
}

/// [`read_manifest`], with the document read by `document` from its JSON value instead of by
/// `D`'s `Deserialize`, for example to decode a deep tree a level at a time.
pub fn read_manifest_with<D, E: From<Error>>(
    json: &[u8],
    limits: &Limits,
    steps: &[Step<E>],
    target: u32,
    document: impl FnOnce(Value) -> Result<D, E>,
) -> Result<Manifest<D>, E> {
    let mut read_document = Some(document);
    let mut value = parse_manifest(json, limits)?;
    migrate(&mut value, steps, target)?;
    let Value::Object(members) = value else {
        return Err(Error::Corrupt("the manifest is not a JSON object".to_owned()).into());
    };
    let (mut format_version, mut generator, mut doc, mut thumbnail, mut composite) = (None, None, None, None, None);
    for (name, member) in members {
        match name.as_str() {
            "format_version" => format_version = Some(u32::deserialize(member).map_err(Error::Json)?),
            "generator" => generator = Some(String::deserialize(member).map_err(Error::Json)?),
            "document" => {
                if let Some(read) = read_document.take() {
                    doc = Some(read(member)?);
                }
            }
            "thumbnail" => thumbnail = Option::<String>::deserialize(member).map_err(Error::Json)?,
            "composite" => composite = Option::<String>::deserialize(member).map_err(Error::Json)?,
            _ => {}
        }
    }
    let missing = |name| Error::Json(serde::de::Error::missing_field(name));
    let manifest = Manifest {
        format_version: format_version.ok_or_else(|| missing("format_version"))?,
        generator: generator.ok_or_else(|| missing("generator"))?,
        document: doc.ok_or_else(|| missing("document"))?,
        thumbnail,
        composite,
    };
    check_preview("thumbnail", manifest.thumbnail.as_deref(), THUMB)?;
    check_preview("composite", manifest.composite.as_deref(), COMPOSITE)?;
    Ok(manifest)
}

/// Refuses a preview member that names an entry other than `expected`.
fn check_preview(member: &str, entry: Option<&str>, expected: &str) -> Result<(), Error> {
    match entry {
        Some(entry) if entry != expected => Err(Error::Corrupt(format!("`{member}` names {entry:?}, not {expected:?}"))),
        _ => Ok(()),
    }
}
