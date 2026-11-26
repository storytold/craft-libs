//! The optional `.scraftz` ZIP container for a `.scraft` session and its embedded audio files.

use crate::{FileFormat, MAX_SESSION_BYTES, SESSION_EXTENSION, Session};
use craft_audio::{BitDepth, EncodeFormat, EncodeOptions, encode};
use craft_zip::{Archive, Compression, Limits, Writer};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::Path;

/// Extension of a ZIP bundle containing a `.scraft` session and its audio files.
pub const BUNDLE_EXTENSION: &str = "scraftz";

/// The limits applied to a bundle in both directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleLimits {
    /// Maximum ZIP entries, including directory entries.
    pub entries: usize,
    /// Maximum combined uncompressed size of entries.
    pub bytes: u64,
    /// Maximum size of the contained session JSON.
    pub session_bytes: u64,
}

/// SoundCraft's current bundle defaults: 10,000 entries, 1 GiB expanded and 256 MiB per session.
pub const DEFAULT_BUNDLE_LIMITS: BundleLimits = BundleLimits { entries: 10_000, bytes: 1 << 30, session_bytes: MAX_SESSION_BYTES };

/// Errors while reading or writing a session bundle.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    /// The archive is malformed, exceeds a configured limit, or does not contain a session.
    #[error("bundle: {0}")]
    Invalid(String),
    /// The session JSON cannot be serialized.
    #[error("{0}")]
    Session(String),
    /// The embedded audio cannot be encoded as WAV.
    #[error("{0}")]
    Audio(String),
}

/// Write a session and its decoded audio as a deterministic `.scraftz` ZIP bundle.
///
/// Audio entries use the same float WAV representation as SoundCraft's desktop bundle writer.
/// The returned session has each included source's path rewritten to `Audio Files/<name>.wav`,
/// its format set to `Wav`, and `unsaved` cleared.
pub fn write_bundle(session: &Session, limits: BundleLimits) -> Result<(Vec<u8>, Session), BundleError> {
    let mut fresh = session.clone();
    let mut zip = Writer::new();
    let mut names = HashSet::new();
    let mut count = 0usize;
    let mut total = 0u64;

    for source in &mut fresh.sources {
        let Some(audio) = fresh.pool.get(source.id) else { continue };
        let bytes = encode(&audio.buffer, &EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Float32))
            .map_err(|e| BundleError::Audio(e.to_string()))?;
        let base = sanitize_name(&source.name);
        let mut target = format!("Audio Files/{base}.wav");
        let mut suffix = 1usize;
        while names.contains(&target) && suffix <= 9_999 {
            target = format!("Audio Files/{base}_{suffix:02}.wav");
            suffix += 1;
        }
        account(&mut count, &mut total, bytes.len(), limits)?;
        zip.add(&target, &bytes, Compression::Stored).map_err(|e| BundleError::Invalid(e.to_string()))?;
        names.insert(target.clone());
        source.path = target;
        source.unsaved = false;
        source.format = FileFormat::Wav;
    }

    let text = crate::write_session(&fresh, limits.session_bytes)?;
    account(&mut count, &mut total, text.len(), limits)?;
    let session_name = format!("{}.{}", sanitize_name(&fresh.name), SESSION_EXTENSION);
    zip.add(&session_name, text.as_bytes(), Compression::Stored).map_err(|e| BundleError::Invalid(e.to_string()))?;
    let bytes = zip.finish().map_err(|e| BundleError::Invalid(e.to_string()))?;
    Ok((bytes, fresh))
}

/// Read the first root-level `.scraft` and retain other files by their archive path.
///
/// Duplicate entry names, non-text names and entries whose file data overlaps another entry are
/// refused before payloads are copied. The session byte cap is checked against the ZIP's declared
/// size before decompressing its JSON.
pub fn read_bundle(bytes: &[u8], limits: BundleLimits) -> Result<(String, HashMap<String, Vec<u8>>), BundleError> {
    let max_entries = u64::try_from(limits.entries).unwrap_or(u64::MAX);
    let mut archive = Archive::open(bytes, Limits { max_entries, max_total_bytes: limits.bytes }).map_err(zip_error)?;
    if archive.has_duplicate_names() {
        return Err(BundleError::Invalid("files share a name".into()));
    }
    if archive.has_overlapping_files().map_err(zip_error)? {
        return Err(BundleError::Invalid("files share data".into()));
    }
    let len = archive.len();
    let mut session = None;
    let mut files = HashMap::new();
    for index in 0..len {
        let is_dir = archive.entry_is_dir(index).map_err(zip_error)?.ok_or_else(|| BundleError::Invalid("missing entry".into()))?;
        if is_dir {
            continue;
        }
        let name = archive.entry_name(index).map_err(zip_error)?.ok_or_else(|| BundleError::Invalid("missing entry".into()))?;
        let is_session = !name.contains('/') && Path::new(&name).extension().and_then(OsStr::to_str) == Some(SESSION_EXTENSION);
        let size = archive.declared_size(&name).ok_or_else(|| BundleError::Invalid(format!("{name}: missing entry")))?;
        if is_session && session.is_none() {
            check_session_size(&name, size, limits.session_bytes, false).map_err(bundle_context)?;
        }
        let data = archive.read(&name, limits.bytes).map_err(zip_error)?.ok_or_else(|| BundleError::Invalid(format!("{name}: missing entry")))?;
        if is_session && session.is_none() {
            session = Some(String::from_utf8(data).map_err(|_| BundleError::Invalid(format!("{name} is not text")))?);
        } else {
            files.insert(name, data);
        }
    }
    let session = session.ok_or_else(|| BundleError::Invalid("no .scraft session inside".into()))?;
    Ok((session, files))
}

/// Refuse a session that cannot be opened again under the same byte cap.
pub fn check_session_size(name: &str, len: u64, limit: u64, saving: bool) -> Result<(), BundleError> {
    if len <= limit {
        return Ok(());
    }
    let most = if limit >= 1 << 20 { format!("{} MiB", limit >> 20) } else { format!("{limit} bytes") };
    let message = if saving {
        format!("{name} is too large to save: over {most}, more than SoundCraft opens")
    } else {
        format!("{name} is too large to open (over {most})")
    };
    Err(BundleError::Session(message))
}

fn bundle_context(error: BundleError) -> BundleError {
    match error {
        BundleError::Session(message) => BundleError::Invalid(message),
        other => other,
    }
}

fn account(count: &mut usize, total: &mut u64, len: usize, limits: BundleLimits) -> Result<(), BundleError> {
    *count = count.saturating_add(1);
    *total = total.saturating_add(u64::try_from(len).unwrap_or(u64::MAX));
    if *count > limits.entries || *total > limits.bytes {
        return Err(BundleError::Invalid(format!(
            "too large to save (a bundle holds at most {} files and {} MiB, or it can't be opened again)",
            limits.entries,
            limits.bytes >> 20
        )));
    }
    Ok(())
}

fn zip_error(error: craft_zip::Error) -> BundleError {
    match error {
        craft_zip::Error::NotZip(message) => BundleError::Invalid(message),
        craft_zip::Error::TooManyEntries { .. } => BundleError::Invalid("too many files".into()),
        craft_zip::Error::EntryTooLarge { .. } | craft_zip::Error::BudgetSpent { .. } => BundleError::Invalid("too large".into()),
        craft_zip::Error::Entry { name, msg } => BundleError::Invalid(format!("{name}: {msg}")),
        other => BundleError::Invalid(other.to_string()),
    }
}

fn sanitize_name(name: &str) -> String {
    let text: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' }).collect();
    let text = text.trim().trim_start_matches('.').to_string();
    if text.is_empty() { "untitled".into() } else { text.chars().take(100).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    #[test]
    fn bare_session_round_trips_in_a_bundle() {
        let session = Session::default();
        let (bytes, fresh) = write_bundle(&session, DEFAULT_BUNDLE_LIMITS).unwrap();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        assert_eq!(fresh, session);
        assert_eq!(write_bundle(&session, DEFAULT_BUNDLE_LIMITS).unwrap().0, bytes, "same input writes the same ZIP bytes");
        let (text, files) = read_bundle(&bytes, DEFAULT_BUNDLE_LIMITS).unwrap();
        assert!(files.is_empty());
        assert_eq!(Session::from_json(&text).unwrap(), session);
    }

    #[test]
    fn audio_is_stored_as_float_wav_and_the_session_path_is_rewritten() {
        use crate::{FileFormat, Source, SourceAudio, SourceId};
        use craft_audio::AudioBuffer;
        use std::sync::Arc;

        let id = SourceId(1);
        let samples = [-2.0, -0.25, 0.5, 1.5];
        let mut audio = AudioBuffer::new(48_000, 1, samples.len());
        audio.channels[0].copy_from_slice(&samples);
        let mut session = Session::default();
        session.sources.push(Source {
            id,
            name: "Lead / take?".into(),
            path: "elsewhere.wav".into(),
            channels: 1,
            frames: samples.len() as u64,
            sample_rate: 48_000,
            format: FileFormat::Other,
            time_reference: 0,
            unsaved: true,
        });
        session.pool.insert(id, Arc::new(SourceAudio::new(audio)));

        let (bytes, fresh) = write_bundle(&session, DEFAULT_BUNDLE_LIMITS).unwrap();
        let source = fresh.sources.first().unwrap();
        assert_eq!(source.path, "Audio Files/Lead _ take_.wav");
        assert_eq!(source.format, FileFormat::Wav);
        assert!(!source.unsaved);

        let (text, files) = read_bundle(&bytes, DEFAULT_BUNDLE_LIMITS).unwrap();
        let parsed = Session::from_json(&text).unwrap();
        assert_eq!(parsed.sources[0].path, source.path);
        let wav = files.get(&source.path).unwrap();
        let (_, decoded) = craft_audio::decode(wav, Some("wav")).unwrap();
        assert_eq!(decoded.channels[0], samples);
    }

    #[test]
    fn session_size_is_checked_in_both_directions() {
        let too_big = check_session_size("a session", 11, 10, true).unwrap_err();
        assert_eq!(too_big.to_string(), "a session is too large to save: over 10 bytes, more than SoundCraft opens");
        let too_big = check_session_size("a session", 11, 10, false).unwrap_err();
        assert_eq!(too_big.to_string(), "a session is too large to open (over 10 bytes)");
    }

    #[test]
    fn a_bundle_without_a_session_is_refused() {
        let mut zip = Writer::new();
        zip.add("Audio Files/a.wav", b"audio", Compression::Stored).unwrap();
        let bytes = zip.finish().unwrap();
        assert_eq!(read_bundle(&bytes, DEFAULT_BUNDLE_LIMITS).unwrap_err().to_string(), "bundle: no .scraft session inside");
    }

    #[test]
    fn every_truncation_of_a_bundle_is_an_error_not_a_panic() {
        let (bytes, _) = write_bundle(&Session::default(), DEFAULT_BUNDLE_LIMITS).unwrap();
        for cut in 0..bytes.len() {
            assert!(read_bundle(&bytes[..cut], DEFAULT_BUNDLE_LIMITS).is_err(), "accepted truncation at {cut}");
        }
        assert!(read_bundle(&bytes, DEFAULT_BUNDLE_LIMITS).is_ok());
    }
}
