//! Decode path for everything that isn't native PCM WAV/AIFF, backed by `symphonia`.

use std::borrow::Cow;
use std::io::{Cursor, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};

use symphonia::core::codecs::audio::AudioDecoder;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::audio::well_known::CODEC_ID_ALAC;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{TimeBase, Timestamp};

use crate::pcm::{MAX_DECODE_SAMPLES, be_u32};
use crate::{AudioBuffer, AudioError, AudioInfo, ChannelLayout, CodecId, Container, Result};

/// The most frames a packet may declare: decoders allocate a packet's frames up front, and ALAC
/// takes its frame length from the file (encoders use 4096).
const MAX_PACKET_FRAMES: u64 = 1 << 20;

/// The most frames of silence one block holds where a FLAC lost frames; a longer loss is several
/// blocks.
const GAP_BLOCK_FRAMES: u64 = 1 << 16;

/// The most samples (frames x channels) a packet may declare: the PCM decoder allocates
/// that many for its output up front, so a CAF with thousands of channels and a long packet could
/// otherwise claim gigabytes before a sample is decoded (64 MiB of `f32`). Ports soundcraft #1332.
const MAX_PACKET_SAMPLES: u64 = 1 << 24;

/// Turn a symphonia error into the audio layer's error, keeping "unsupported" and "too large"
/// distinct from a malformed file.
fn map_err(e: SymError) -> AudioError {
    match e {
        SymError::Unsupported(m) => AudioError::Unsupported(m.to_string()),
        SymError::LimitError(m) => AudioError::TooLarge(m.to_string()),
        other => AudioError::Malformed(other.to_string()),
    }
}

/// Run `f`, turning a panic inside the third-party decoder into an error.
fn guarded<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(AudioError::Malformed("decoder failed on malformed input".into())))
}

/// What [`open`] read from the file for what symphonia's FLAC and Ogg readers leave out.
enum Preread {
    /// Not a FLAC file or an Ogg file.
    Nothing,
    /// A FLAC file's layout.
    Flac(ChannelLayout),
    /// An Ogg file's audio streams in file order: each one's serial, which is symphonia's track ID,
    /// and its FLAC channel mask. Symphonia orders the tracks by serial, and the tags its reader
    /// collects do not say which track they are from.
    Ogg(Vec<(u32, Option<u32>)>),
}

/// What [`open`] found: a reader, or a stream that holds no audio frames and has no reader.
enum Opened {
    Reader(Box<dyn FormatReader>, Preread),
    Empty(AudioInfo),
}

/// Decoder options for every compressed format. Gapless playback used to be a
/// `FormatOptions` flag in symphonia 0.5; in 0.6 the decoder option (on by default) trims the
/// encoder delay and padding that the demuxer reports on each packet.
fn decoder_options() -> AudioDecoderOptions {
    AudioDecoderOptions::default().gapless(true)
}

mod map;
mod precheck;
use map::{audio_params, audio_tracks, codec_id_of, info_of};
use precheck::{MAX_PREFIX_SCAN, Precheck, SIZE_CHECKED_PROBE_FORMATS, precheck, skip_id3v2};

/// `block` without the frames still to drop after a seek landed before its target; `None` when
/// that is all of it.
fn after_seek(discard: &mut u64, mut block: AudioBuffer) -> Option<AudioBuffer> {
    if *discard > 0 {
        let drop = (*discard as usize).min(block.frames());
        for c in &mut block.channels {
            c.drain(..drop);
        }
        *discard -= drop as u64;
        if block.frames() == 0 {
            return None;
        }
    }
    Some(block)
}

/// Skip leading ID3v2 tags, run the container size preflight, then probe with symphonia; a zero-frame FLAC found by
/// the preflight comes back as [`Opened::Empty`] without probing. A size-checked container is refused when probing
/// selects a different reader or one that was never checked, so incidental payload magic cannot bypass those limits.
fn open(bytes: Cow<'_, [u8]>, detected: Option<Container>) -> Result<Opened> {
    let start = skip_id3v2(&bytes);
    if start > 0 && start >= bytes.len() {
        return Err(AudioError::Malformed("ID3v2 tag extends past the end of the file".into()));
    }
    let (checked, flac_layout) = match precheck(&bytes, start, detected)? {
        Precheck::Empty(info) => return Ok(Opened::Empty(info)),
        Precheck::Flac(expected, layout) => (Precheck::Checked(expected), layout),
        checked => (checked, None),
    };
    let body = bytes.get(start..).unwrap_or_default();
    // symphonia finds the first page past leading bytes, so look for it the same way.
    let first_page = body.windows(4).take(MAX_PREFIX_SCAN).position(|w| w == b"OggS");
    let ogg_streams = first_page.map(|at| super::flac::ogg_audio_streams(&body[at..]));
    // symphonia reads from an owned stream: a borrowed buffer is copied once, a buffer read from
    // a shared source is moved in.
    let mut mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.into_owned())), MediaSourceStreamOptions::default());
    // Seek (rather than pre-positioning the cursor) so the stream's absolute position is `start`
    // too: container offsets such as MP4's `stco` are absolute file offsets.
    mss.seek(SeekFrom::Start(u64::try_from(start).unwrap_or(u64::MAX)))?;
    let reader: Box<dyn FormatReader> =
        symphonia::default::get_probe().probe(&Hint::new(), mss, FormatOptions::default(), MetadataOptions::default()).map_err(map_err)?;
    let probed = reader.format_info().short_name;
    match checked {
        // Only containers whose readers allocate from sizes in the file need the check. A stream
        // the probe reads with another reader (MP3, Ogg, AIFF, ...) has no such size fields, so a
        // marker found in its audio data must not reject it.
        Precheck::Checked(expected) if SIZE_CHECKED_PROBE_FORMATS.contains(&probed) && probed != expected => {
            return Err(AudioError::Malformed(format!("the probed container ({probed}) is not the one whose size checks ran")));
        }
        Precheck::Unchecked if SIZE_CHECKED_PROBE_FORMATS.contains(&probed) => {
            return Err(AudioError::Malformed(format!("the probed container ({probed}) was not size-checked")));
        }
        _ => {}
    }
    let preread = match (flac_layout.filter(|_| probed == "flac"), ogg_streams.filter(|_| probed == "ogg")) {
        (Some(layout), _) => Preread::Flac(layout),
        (None, Some(streams)) => Preread::Ogg(streams),
        _ => Preread::Nothing,
    };
    Ok(Opened::Reader(reader, preread))
}

/// A demux error ends the decode; it is only an error when nothing was decoded yet.
fn demux_failure(frames: u64, e: SymError) -> Result<()> {
    if frames == 0 {
        return Err(map_err(e));
    }
    log::warn!("stopping decode after demux error: {e}");
    Ok(())
}

/// Note a corrupt packet (the `errors`-th); a stream that never decodes is malformed.
fn corrupt_packet(errors: usize, frames: u64, msg: &str) -> Result<()> {
    log::debug!("concealing corrupt packet: {msg}");
    if errors > 1000 && frames == 0 {
        return Err(AudioError::Malformed(format!("no decodable packets: {msg}")));
    }
    Ok(())
}

/// A file decoded by symphonia, block by block.
pub(crate) struct Symph {
    /// `None` for a stream with no audio frames (a zero-frame FLAC).
    reader: Option<Box<dyn FormatReader>>,
    decoder: Option<Box<dyn AudioDecoder>>,
    track_id: u32,
    time_base: Option<TimeBase>,
    info: AudioInfo,
    frames_known: bool,
    /// Frames decoded so far (for the "nothing decoded" checks).
    decoded: u64,
    errors: usize,
    /// Frames of the last decoded packet: the length of the silence that replaces a packet that
    /// fails to decode when the demuxer gives no duration for it, so the audio after it keeps its
    /// timing (0 before the first packet decodes).
    last_frames: usize,
    /// Frames still to drop after a seek landed before its target.
    discard: u64,
    /// A FLAC file: the frame the next packet starts at when none was lost. The FLAC demuxer drops
    /// a frame whose checksum fails and resyncs on the next one, so a later timestamp is audio that
    /// was lost.
    flac_next: Option<u64>,
    /// Frames of silence still to return for lost FLAC frames, then the block decoded after them.
    gap: u64,
    held: Option<Vec<Vec<f32>>>,
    ended: bool,
    interleaved: Vec<f32>,
}

impl Symph {
    /// Open audio track `track` (the first when `None`) of the file in `bytes`.
    pub(crate) fn open(bytes: Cow<'_, [u8]>, detected: Option<Container>, track: Option<usize>) -> Result<Symph> {
        guarded(move || {
            let (reader, preread) = match open(bytes, detected)? {
                Opened::Reader(reader, preread) => (reader, preread),
                Opened::Empty(info) => {
                    // a stream with no audio still has one track, as WAV and AIFF do
                    if let Some(n) = track.filter(|&n| n > 0) {
                        return Err(AudioError::Unsupported(format!("no audio track {n}: the file has one")));
                    }
                    return Ok(Symph {
                        reader: None,
                        decoder: None,
                        track_id: 0,
                        time_base: None,
                        info,
                        frames_known: true,
                        decoded: 0,
                        errors: 0,
                        last_frames: 0,
                        discard: 0,
                        flac_next: None,
                        gap: 0,
                        held: None,
                        ended: true,
                        interleaved: Vec::new(),
                    });
                }
            };
            let ogg_streams = if let Preread::Ogg(streams) = &preread { streams.as_slice() } else { &[] };
            let (track_id, mut info) = info_of(reader.as_ref(), detected, ogg_streams, track)?;
            match &preread {
                Preread::Flac(layout) if layout.channels() == info.channels => info.layout = *layout,
                Preread::Ogg(streams) if info.codec == CodecId::Flac => {
                    let mask = streams.iter().find(|(serial, _)| *serial == track_id).and_then(|&(_, mask)| mask);
                    info.layout = super::flac::layout_from(mask, info.channels);
                }
                _ => {}
            }
            let t = reader.tracks().iter().find(|t| t.id == track_id).ok_or_else(|| AudioError::Malformed("track vanished".into()))?;
            let params = audio_params(t).cloned().ok_or_else(|| AudioError::Malformed("track vanished".into()))?;
            let alac = params.extra_data.as_deref().filter(|_| params.codec == CODEC_ID_ALAC).and_then(|d| be_u32(d, 0));
            let channels = u64::from(info.channels);
            if params
                .max_frames_per_packet
                .into_iter()
                .chain(alac.map(u64::from))
                .any(|n| n > MAX_PACKET_FRAMES || n.saturating_mul(channels) > MAX_PACKET_SAMPLES)
            {
                return Err(AudioError::Malformed("the file declares packets far longer than any codec uses".into()));
            }
            let decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &decoder_options()).map_err(map_err)?;
            let frames_known = info.frames > 0 && info.channels > 0;
            // Native FLAC timestamps count frames from 0.
            let counts_frames = t.time_base.is_none_or(|tb| tb.numer.get() == 1 && tb.denom.get() == info.sample_rate);
            let flac_next = (info.container == Container::Flac && info.codec == CodecId::Flac && counts_frames).then_some(0);
            Ok(Symph {
                time_base: t.time_base,
                reader: Some(reader),
                decoder: Some(decoder),
                track_id,
                info,
                frames_known,
                decoded: 0,
                errors: 0,
                last_frames: 0,
                discard: 0,
                flac_next,
                gap: 0,
                held: None,
                ended: false,
                interleaved: Vec::new(),
            })
        })
    }

    pub(crate) fn info(&self) -> &AudioInfo {
        &self.info
    }

    pub(crate) fn frames_known(&self) -> bool {
        self.frames_known
    }

    /// The BWF metadata of the WAV around the stream (symphonia does not read `bext`).
    pub(crate) fn set_bwf(&mut self, bwf: Option<crate::BwfInfo>) {
        self.info.bwf = bwf;
    }

    /// The next block of decoded audio, or `None` at the end.
    pub(crate) fn next_block(&mut self) -> Result<Option<AudioBuffer>> {
        guarded(|| self.next_block_inner())
    }

    fn next_block_inner(&mut self) -> Result<Option<AudioBuffer>> {
        let (Some(reader), Some(decoder)) = (self.reader.as_mut(), self.decoder.as_mut()) else { return Ok(None) };
        loop {
            // Silence for lost FLAC frames comes first, then the block that was decoded after them.
            let pending = if self.gap > 0 {
                let n = self.gap.min(GAP_BLOCK_FRAMES);
                self.gap -= n;
                Some(crate::pcm::silence(usize::from(self.info.channels), n as usize)?)
            } else {
                self.held.take()
            };
            if let Some(block) = pending {
                match after_seek(&mut self.discard, AudioBuffer { sample_rate: self.info.sample_rate, channels: block }) {
                    Some(block) => return Ok(Some(block)),
                    None => continue,
                }
            }
            if self.ended {
                if self.info.channels == 0 {
                    return Err(AudioError::Malformed("stream contains no audio".into()));
                }
                return Ok(None);
            }
            let packet = match reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) | Err(SymError::ResetRequired) => {
                    self.ended = true;
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.ended = true;
                    continue;
                }
                Err(e) => {
                    demux_failure(self.decoded, e)?;
                    self.ended = true;
                    continue;
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            if let Some(next) = self.flac_next {
                let pts = u64::try_from(packet.pts.get()).unwrap_or(0);
                let lost = pts.saturating_sub(next);
                // Never past the declared length. Without one, a jump longer than any packet is
                // not believed.
                let room = if self.frames_known { self.info.frames.saturating_sub(next) } else { MAX_PACKET_FRAMES };
                if lost > 0 && lost <= room && self.info.channels > 0 {
                    self.gap = lost;
                    self.info.damaged += 1;
                }
                self.flac_next = Some(pts.saturating_add(packet.dur.get()));
            }
            let block = match decoder.decode(&packet) {
                Ok(decoded) => {
                    // The interleaved copy below is laid out by planes: use their count so a
                    // decoder whose channel spec disagrees with its planes cannot trip the copy's
                    // length assertion.
                    let ch = decoded.num_planes();
                    if ch == 0 {
                        continue;
                    }
                    let n = decoded.frames();
                    let needed = n
                        .checked_mul(ch)
                        .filter(|&t| t as u64 <= MAX_DECODE_SAMPLES)
                        .ok_or_else(|| AudioError::TooLarge(format!("a decoded block exceeds the {MAX_DECODE_SAMPLES}-sample decode limit")))?;
                    if self.interleaved.len() < needed {
                        self.interleaved
                            .try_reserve(needed - self.interleaved.len())
                            .map_err(|_| AudioError::TooLarge("cannot allocate decoded audio".into()))?;
                        self.interleaved.resize(needed, 0.0);
                    }
                    decoded.copy_to_slice_interleaved(&mut self.interleaved[..needed]);
                    let mut planar = crate::pcm::planar(ch, n)?;
                    for frame in self.interleaved[..needed].chunks_exact(ch) {
                        for (dst, &s) in planar.iter_mut().zip(frame) {
                            dst.push(if s.is_finite() { s } else { 0.0 });
                        }
                    }
                    // The decoded stream is authoritative for the channel count.
                    if usize::from(self.info.channels) != ch {
                        self.info.channels = u16::try_from(ch).map_err(|_| AudioError::Unsupported("too many channels".into()))?;
                        self.info.layout = ChannelLayout::unordered(crate::layout::count(ch));
                    }
                    self.decoded = self.decoded.saturating_add(n as u64);
                    self.last_frames = n;
                    planar
                }
                Err(SymError::DecodeError(m)) => {
                    self.errors += 1;
                    self.info.damaged += 1;
                    corrupt_packet(self.errors, self.decoded, m)?;
                    // the packet's own duration when the demuxer gives one (Vorbis packets vary in
                    // length; it leaves out priming), otherwise the last packet's: a damaged first
                    // packet is concealed only when its duration is known
                    let rate = u64::from(self.info.sample_rate.max(1));
                    let (numer, denom) = self.time_base.map_or((1, rate), |tb| (u64::from(tb.numer.get()), u64::from(tb.denom.get())));
                    let own = u128::from(packet.dur.get()) * u128::from(numer) * u128::from(rate) / u128::from(denom);
                    let frames = match usize::try_from(own) {
                        Ok(n) if n > 0 && n as u64 <= MAX_PACKET_FRAMES => n,
                        _ => self.last_frames,
                    };
                    if frames == 0 || self.info.channels == 0 {
                        continue;
                    }
                    crate::pcm::silence(usize::from(self.info.channels), frames)?
                }
                Err(SymError::IoError(_)) => {
                    self.ended = true;
                    continue;
                }
                Err(e) => return Err(map_err(e)),
            };
            if self.gap > 0 {
                self.held = Some(block);
                continue;
            }
            if let Some(block) = after_seek(&mut self.discard, AudioBuffer { sample_rate: self.info.sample_rate, channels: block }) {
                return Ok(Some(block));
            }
        }
    }

    /// Continue from frame `frame`: an accurate seek lands at or before it, and the frames up to
    /// it are dropped.
    pub(crate) fn seek(&mut self, frame: u64) -> Result<u64> {
        // At or past a known end: nothing is left to decode.
        if self.frames_known && frame >= self.info.frames {
            self.ended = true;
            self.discard = 0;
            return Ok(self.info.frames);
        }
        guarded(|| {
            let (Some(reader), Some(decoder)) = (self.reader.as_mut(), self.decoder.as_mut()) else { return Ok(0) };
            let rate = u64::from(self.info.sample_rate.max(1));
            // Timestamps count `numer / denom` seconds; without a time base they count frames.
            let (numer, denom) = self.time_base.map_or((1, rate), |tb| (u64::from(tb.numer.get()), u64::from(tb.denom.get())));
            let to_ts = |f: u64| u128::from(f) * u128::from(denom) / (u128::from(numer) * u128::from(rate));
            let ts = i64::try_from(to_ts(frame)).unwrap_or(i64::MAX);
            let seeked = match reader.seek(SeekMode::Accurate, SeekTo::Timestamp { ts: Timestamp::new(ts), track_id: self.track_id }) {
                Ok(s) => s,
                // Past the end of a stream whose length is unknown: the stream is over.
                Err(SymError::SeekError(_)) => {
                    self.ended = true;
                    return Ok(frame);
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.ended = true;
                    return Ok(frame);
                }
                Err(e) => return Err(map_err(e)),
            };
            decoder.reset();
            // the packet before the seek says nothing about the length of one lost after it
            self.last_frames = 0;
            self.gap = 0;
            self.held = None;
            let actual = u64::try_from(seeked.actual_ts.get()).unwrap_or(0);
            let actual = u64::try_from(u128::from(actual) * u128::from(numer) * u128::from(rate) / u128::from(denom)).unwrap_or(u64::MAX);
            // A seek into lost frames lands on the next one that survived: the stream is where the
            // seek says, and that packet's later timestamp is the rest of the loss.
            self.flac_next = self.flac_next.map(|_| actual);
            self.discard = frame.saturating_sub(actual);
            self.ended = false;
            Ok(frame)
        })
    }
}

/// The codec parameters of each audio track symphonia finds, for a track list.
pub(crate) fn tracks(bytes: Cow<'_, [u8]>, detected: Option<Container>) -> Result<Vec<(CodecId, u32, u16)>> {
    guarded(move || {
        let reader = match open(bytes, detected)? {
            Opened::Reader(reader, _) => reader,
            Opened::Empty(info) => return Ok(vec![(info.codec, info.sample_rate, info.channels)]),
        };
        Ok(audio_tracks(reader.as_ref())
            .filter_map(audio_params)
            .map(|p| (codec_id_of(p.codec), p.sample_rate.unwrap_or(0), p.channels.as_ref().map_or(0, |c| u16::try_from(c.count()).unwrap_or(0))))
            .collect())
    })
}
