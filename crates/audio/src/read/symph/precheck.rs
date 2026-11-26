//! Container preflight: ID3v2 skipping, the budgeted marker scan and the size checks that run before probing.

use super::super::id3v2_len;
use crate::{AudioError, AudioInfo, ChannelLayout, Container, Result};
use std::io::{Cursor, Seek, SeekFrom};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::{ProbeOptions, ProbeableFormat, Score, Scoreable};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions, ScopedStream};

/// How far [`first_marker`] looks for the first format marker: the same 1 MiB a default symphonia
/// probe scans.
pub(super) const MAX_PREFIX_SCAN: usize = 1 << 20;

/// How many MPEG/ADTS candidates [`first_marker`] may hand to the readers' expensive `Scoreable`
/// implementations. A crafted 1 MiB prefix of repeated sync markers otherwise asks for hundreds
/// of thousands of scores, each allowed up to symphonia's 16 KiB score depth. Once the budget is
/// spent, the scan stops at the next audio candidate: a container-like byte string later in an
/// unscored audio payload must not become authoritative. A container marker reached first is still
/// checked; an unchecked one the probe selects is refused after probing.
pub(super) const MAX_AUDIO_SCORE_ATTEMPTS: usize = 256;

/// A per-scan budget for `Scoreable::score` calls. Taking a slot consumes it even when the score
/// rejects the candidate, so a stream of false syncs cannot multiply the work.
pub(super) struct ScoreBudget {
    pub(super) used: usize,
}

impl ScoreBudget {
    /// Start a scan with no expensive score attempts spent.
    pub(super) fn new() -> Self {
        Self { used: 0 }
    }

    /// Spend one score attempt, returning false once the per-scan budget is exhausted.
    pub(super) fn take(&mut self) -> bool {
        if self.exhausted() {
            return false;
        }
        self.used = self.used.saturating_add(1);
        true
    }

    /// Whether no attempt is left for this scan.
    pub(super) fn exhausted(&self) -> bool {
        self.used >= MAX_AUDIO_SCORE_ATTEMPTS
    }
}

/// The first byte after any ID3v2 tags at the start of `b`. Symphonia's tag reader allocates
/// whatever a frame claims, so the tags are skipped, not read: they are not used here.
pub(super) fn skip_id3v2(b: &[u8]) -> usize {
    let mut at = 0usize;
    loop {
        match b.get(at..).and_then(id3v2_len) {
            Some(n) if n > 0 => at = at.saturating_add(n),
            _ => return at,
        }
    }
}

/// The first format marker the probe can reach from `start`, within its 1 MiB scan limit.
#[derive(Clone, Copy)]
pub(super) enum FirstMarker {
    Flac(usize),
    Caf(usize),
    Mp4(usize),
    Wav(usize),
    /// MPEG audio, Ogg, Matroska or AIFF: formats with no separate container size check.
    Audio,
}

/// What the preflight container checks covered before the probe built a reader.
pub(super) enum Precheck {
    /// A zero-frame FLAC: returned without a reader, because Symphonia 0.6's reader needs a first
    /// audio frame to resynchronize to.
    Empty(AudioInfo),
    /// The size checks ran for the container the probe reports under this short name.
    Checked(&'static str),
    /// As `Checked`, for a FLAC stream with this layout, which symphonia's reader does not parse.
    Flac(&'static str, Option<ChannelLayout>),
    /// No container size check was needed.
    Unchecked,
}

/// Probe short names whose demuxer allocates from sizes in the file, so the matching container
/// check has to run before the stream is decoded.
pub(super) const SIZE_CHECKED_PROBE_FORMATS: &[&str] = &["caf", "flac", "isomp4", "wave"];

/// The container markers to check for the family `detected` names. `None` is a raw audio stream
/// (MP3, Ogg, AIFF), whose payload may contain container-like bytes: the unrestricted scan below
/// finds a container only when one starts the stream, before any audio marker.
fn container_markers(detected: Option<Container>) -> Option<&'static [&'static [u8; 4]]> {
    match detected {
        // RF64 and BW64 are the >4 GiB RIFF forms.
        Some(Container::Wav) => Some(&[b"RIFF", b"RF64", b"BW64"]),
        Some(Container::Flac) => Some(&[b"fLaC"]),
        Some(Container::Caf) => Some(&[b"caff"]),
        // MP4/M4A put `ftyp` after the first atom's size field (an `.aac` name may hide one).
        Some(Container::Mp4 | Container::QuickTime | Container::Adts) => Some(&[b"ftyp"]),
        Some(Container::Aiff | Container::MpegAudio | Container::Ogg) => None,
        // Unknown files may carry junk before the container; symphonia's probe scans for markers.
        _ => Some(&[b"caff", b"ftyp", b"fLaC", b"RIFF"]),
    }
}

/// The probe short name a container marker stands for.
fn expected_probe_format(marker: &[u8; 4]) -> &'static str {
    match marker {
        b"caff" => "caf",
        // The isomp4 reader reads the `ftyp` atom.
        b"ftyp" => "isomp4",
        b"fLaC" => "flac",
        // RIFF, RF64 and BW64.
        _ => "wave",
    }
}

/// Find the first marker of `markers` at or after `at`, within symphonia's probe depth. A RIFF
/// form other than WAVE is skipped, as the probe skips it and reads a WAVE behind it.
fn find_container_marker<'a>(bytes: &[u8], at: usize, markers: &'a [&'static [u8; 4]]) -> Option<(usize, &'a [u8; 4])> {
    let end = bytes.len().min(at.saturating_add(MAX_PREFIX_SCAN));
    for pos in at..end.saturating_sub(3) {
        let window = bytes.get(pos..pos + 4)?;
        if let Some(marker) = markers.iter().find(|marker| window == marker.as_slice()) {
            if matches!(*marker, b"RIFF" | b"RF64" | b"BW64") && bytes.get(pos + 8..pos + 12) != Some(&b"WAVE"[..]) {
                continue;
            }
            return Some((pos, *marker));
        }
    }
    None
}

/// The scan variant for a marker found by [`find_container_marker`], with the probe short name its
/// size check stands for. `ftyp` sits four bytes into its atom, so the MP4 check starts there.
fn marker_kind(at: usize, marker: &[u8; 4]) -> (FirstMarker, &'static str) {
    match marker {
        b"fLaC" => (FirstMarker::Flac(at), expected_probe_format(marker)),
        b"caff" => (FirstMarker::Caf(at), expected_probe_format(marker)),
        b"ftyp" => (FirstMarker::Mp4(at.saturating_sub(4)), expected_probe_format(marker)),
        _ => (FirstMarker::Wav(at), expected_probe_format(marker)),
    }
}

/// The probe short name a container scan variant stands for.
fn marker_name(marker: FirstMarker) -> &'static str {
    match marker {
        FirstMarker::Flac(_) => "flac",
        FirstMarker::Caf(_) => "caf",
        FirstMarker::Mp4(_) => "isomp4",
        FirstMarker::Wav(_) => "wave",
        FirstMarker::Audio => "",
    }
}

/// Whether `rest` starts with one of `R`'s exact registered probe markers.
fn has_probe_marker<R: ProbeableFormat<'static>>(rest: &[u8]) -> bool {
    R::probe_data().iter().any(|data| data.spec.markers.iter().any(|marker| rest.starts_with(marker)))
}

/// Score `R` at `at` exactly as symphonia's private probe does: `Supported` wins, EOF and every
/// other scoring error mean "not this format", while a real I/O error aborts the probe.
fn scores_supported<R: Scoreable>(stream: &mut MediaSourceStream<'_>, at: usize, depth: u16) -> Result<bool> {
    stream.seek(SeekFrom::Start(u64::try_from(at).unwrap_or(u64::MAX)))?;
    match R::score(ScopedStream::new(stream, u64::from(depth))) {
        Ok(Score::Supported(_)) => Ok(true),
        Err(SymError::IoError(e)) if e.kind() != std::io::ErrorKind::UnexpectedEof => Err(AudioError::Malformed(e.to_string())),
        _ => Ok(false),
    }
}

/// Whether `b[at..]` is an MPEG/ADTS candidate the probe would actually select: an exact
/// `probe_data()` marker and a `Supported` score from the same reader. A loose `0xFFEx` pair is
/// not enough: `0xFFE0` is neither a registered marker nor a valid frame, so it must not stop the
/// scan before a later FLAC/CAF/MP4 candidate.
pub(super) fn audio_sync_at<'a>(b: &'a [u8], at: usize, stream: &mut Option<MediaSourceStream<'a>>, budget: &mut ScoreBudget) -> Result<bool> {
    let rest = b.get(at..).unwrap_or_default();
    let adts = has_probe_marker::<symphonia::default::formats::AdtsReader<'static>>(rest);
    let mpa = has_probe_marker::<symphonia::default::formats::MpaReader<'static>>(rest);
    if !adts && !mpa {
        return Ok(false);
    }
    if stream.is_none() {
        *stream = Some(MediaSourceStream::new(Box::new(Cursor::new(b)), MediaSourceStreamOptions::default()));
    }
    let Some(mss) = stream.as_mut() else { return Ok(false) };
    let depth = ProbeOptions::default().max_score_depth;
    if adts && budget.take() && scores_supported::<symphonia::default::formats::AdtsReader<'static>>(mss, at, depth)? {
        return Ok(true);
    }
    if mpa && budget.take() && scores_supported::<symphonia::default::formats::MpaReader<'static>>(mss, at, depth)? {
        return Ok(true);
    }
    Ok(false)
}

/// Find the first marker that can start a container or audio stream. Symphonia's `Probe::probe`
/// constructs the reader itself and exposes no pre-construction hook, while checking every magic
/// in the scan window would validate bytes the probe never reaches (for example a valid MP3's
/// audio payload). Stopping at the first marker avoids that: it is the candidate the probe selects
/// for FLAC, CAF, MP4, Ogg and Matroska, and for WAV/AIFF once their full form is verified below.
/// MPEG/ADTS candidates are checked with the readers' own `probe_data()`/`Scoreable` APIs, so a
/// false sync byte pair cannot stop the scan. If that scoring budget runs out, the scan stops at
/// the next audio candidate instead of trusting a container-like byte string later in the unscored
/// audio payload. A container marker reached before any such candidate is still preflighted; if
/// the probe selects a size-checked container the scan did not check, `open` refuses it.
///
/// WAV candidates include RF64/BW64 so `check_info` can run, although symphonia 0.6 registers only
/// the RIFF marker. Native PCM RF64/BW64 is parsed by `wav::parse` before this path; a foreign
/// RF64/BW64 stream therefore remains unsupported by the probe rather than misdetected.
fn first_marker(b: &[u8], start: usize) -> Result<Option<FirstMarker>> {
    first_marker_with_budget(b, start, &mut ScoreBudget::new())
}

/// [`first_marker`] with an explicit score budget, so a test can prove a prefix crowded with
/// false sync markers performs only the bounded number of reader scores.
pub(super) fn first_marker_with_budget(b: &[u8], start: usize, budget: &mut ScoreBudget) -> Result<Option<FirstMarker>> {
    let end = b.len().min(start.saturating_add(MAX_PREFIX_SCAN));
    let mut at = start;
    let mut audio_probe = None;
    while at.saturating_add(4) <= end {
        let rest = b.get(at..).unwrap_or_default();
        if rest.get(..4) == Some(b"fLaC".as_slice()) {
            return Ok(Some(FirstMarker::Flac(at)));
        }
        if rest.get(..4) == Some(b"caff".as_slice()) {
            return Ok(Some(FirstMarker::Caf(at)));
        }
        if rest.get(..4) == Some(b"OggS".as_slice()) || rest.get(..4) == Some(b"\x1A\x45\xDF\xA3".as_slice()) || super::super::aiff::is_aiff(rest) {
            return Ok(Some(FirstMarker::Audio));
        }
        if super::super::wav::is_wav(rest) {
            return Ok(Some(FirstMarker::Wav(at)));
        }
        if let (Some(&b0), Some(&b1)) = (b.get(at), b.get(at + 1))
            && b0 == 0xFF
            && b1 & 0xE0 == 0xE0
        {
            // A candidate after the cap cannot be scored. Stop rather than let a container-like
            // byte string later in an unscored audio payload become preflight-authoritative; the
            // probe will select the reader, and `open` refuses an unchecked size-checked one.
            if budget.exhausted() {
                return Ok(None);
            }
            if audio_sync_at(b, at, &mut audio_probe, budget)? {
                return Ok(Some(FirstMarker::Audio));
            }
        }
        if rest.get(4..8) == Some(b"ftyp".as_slice()) {
            return Ok(Some(FirstMarker::Mp4(at)));
        }
        at += 1;
    }
    Ok(None)
}

/// Run the early size checks on the container the probe can select. `start` is the first byte
/// after leading ID3v2 tags. A zero-frame FLAC is returned as an empty stream because Symphonia
/// 0.6's reader requires a first audio frame to resynchronize to.
pub(super) fn precheck(b: &[u8], start: usize, detected: Option<Container>) -> Result<Precheck> {
    let any_marker = |b: &[u8], start: usize| -> Result<Option<(FirstMarker, &'static str)>> {
        Ok(match first_marker(b, start)? {
            // Stop at the first marker of any kind, so container magic incidental to an audio
            // payload is not mistaken for the stream's container.
            Some(FirstMarker::Audio) | None => None,
            Some(marker) => Some((marker, marker_name(marker))),
        })
    };
    let found = match container_markers(detected) {
        // Follow the detection: check the first marker of the family it names, so a misleading
        // marker cannot let a differently sized container reach the probe unchecked. A detection
        // is only a hint, though: when the stream holds none of its markers (a FLAC behind a
        // `.mp3` name), the probe can still select another container, which is checked here.
        Some(markers) => match find_container_marker(b, start, markers) {
            Some((at, marker)) => Some(marker_kind(at, marker)),
            None => any_marker(b, start)?,
        },
        None => any_marker(b, start)?,
    };
    let Some((marker, expected)) = found else { return Ok(Precheck::Unchecked) };
    match marker {
        FirstMarker::Flac(at) => {
            super::super::flac::check_metadata(b, at)?;
            if let Some(info) = super::super::flac::empty_stream_info(b, at) {
                return Ok(Precheck::Empty(info));
            }
            Ok(Precheck::Flac(expected, super::super::flac::layout(b, at)))
        }
        FirstMarker::Caf(at) => {
            super::super::caf::check(b, at)?;
            Ok(Precheck::Checked(expected))
        }
        FirstMarker::Mp4(at) => {
            super::super::mp4::check(b, at)?;
            Ok(Precheck::Checked(expected))
        }
        FirstMarker::Wav(at) => {
            super::super::wav::check_info(b, at)?;
            Ok(Precheck::Checked(expected))
        }
        FirstMarker::Audio => Ok(Precheck::Unchecked),
    }
}
