use std::io;
use std::sync::{Arc, Mutex};

use craft_color::{ColorInfo, Matrix, Range};
use craft_frame::{Chroma, PixelData};
use craft_media::ByteSource;
use craft_media_source::y4m::{Y4mSource, sniff};
use craft_media_source::{DEFAULT_MAX_MEDIA_BYTES, FrameRequest, MediaError, MediaKind, MediaSource, SharedBytes};
use craft_time::{FrameRate, Tick};

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Tracked {
    prefix: Vec<u8>,
    length: u64,
    fail_at: Option<u64>,
    shrink_after: Option<(usize, u64)>,
    reads: Mutex<Vec<(u64, usize)>>,
}

impl Tracked {
    fn new(bytes: Vec<u8>, length: u64) -> Arc<Self> {
        Arc::new(Self { prefix: bytes, length, fail_at: None, shrink_after: None, reads: Mutex::new(Vec::new()) })
    }

    fn take_reads(&self) -> Vec<(u64, usize)> {
        std::mem::take(&mut *self.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl ByteSource for Tracked {
    fn len(&self) -> u64 {
        if let Some((reads, length)) = self.shrink_after
            && self.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len() >= reads
        {
            return length;
        }
        self.length
    }

    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        self.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((offset, out.len()));
        if self.fail_at.is_some_and(|at| offset <= at && at < offset.saturating_add(out.len() as u64)) {
            return Err(io::Error::other("injected second Y4M delimiter read failure"));
        }
        let start = usize::try_from(offset).map_err(io::Error::other)?;
        let available = self.prefix.get(start..).unwrap_or_default();
        let n = out.len().min(available.len());
        out[..n].copy_from_slice(&available[..n]);
        out[n..].fill(0);
        Ok(())
    }
}

#[test]
fn plays_exact_odd_10_bit_frames_on_cfr_boundaries() -> TestResult<()> {
    // Original deterministic Y4M: 3x3 luma and two 2x2 chroma planes, independently authored.
    let first = [64u16, 65, 66, 67, 68, 69, 70, 71, 72, 401, 402, 403, 404, 601, 602, 603, 604];
    let second = [721u16, 722, 723, 724, 725, 726, 727, 728, 729, 801, 802, 803, 804, 201, 202, 203, 204];
    let mut bytes = b"YUV4MPEG2 W3 H3 F30000:1001 A4:3 C420p10 XCOLORRANGE=FULL\nFRAME\n".to_vec();
    let at_first = bytes.len() as u64;
    bytes.extend(first.into_iter().flat_map(u16::to_le_bytes));
    bytes.extend_from_slice(b"FRAME Xtest\n");
    let at_second = bytes.len() as u64;
    bytes.extend(second.into_iter().flat_map(u16::to_le_bytes));
    bytes.extend_from_slice(b"FRAME\n\x01\x02");
    let length = bytes.len() as u64;
    let reader = Tracked::new(bytes, length);
    let input: SharedBytes = reader.clone();
    let source = Y4mSource::open("odd.y4m", input)?;
    let info = source.info();
    let video = info.video.as_ref().ok_or("expected Y4M video")?;
    let period = Tick(8_475_667_200);
    let color = ColorInfo { matrix: Matrix::Bt601, range: Range::Full, ..ColorInfo::REC709 };
    assert_eq!((info.kind, info.duration, info.file_size), (MediaKind::Movie, Tick(2 * period.0), Some(length)));
    assert_eq!(info.container, "Y4M");
    assert!(info.start_timecode.is_none() && !info.has_audio());
    assert!(matches!(source.audio(0, 2, 48_000), Err(MediaError::NoStream("audio"))));
    assert_eq!((video.width, video.height, video.par), (3, 3, (4, 3)));
    assert_eq!((video.frame_rate, video.color), (FrameRate::new(30_000, 1_001), color));
    assert_eq!(video.codec, "Uncompressed YUV");
    assert!(!video.has_alpha);

    assert!(reader.take_reads().into_iter().all(|(_, len)| len <= 128));
    for (ask, pts, offset, expected) in [
        (Tick(-1), Tick::ZERO, at_first, first),
        (Tick(period.0 - 1), Tick::ZERO, at_first, first),
        (period, period, at_second, second),
        (Tick(3 * period.0), period, at_second, second),
        (Tick::ZERO, Tick::ZERO, at_first, first),
    ] {
        let frame = source.video_frame(FrameRequest { time: ask, scale: 0.5 })?;
        assert_eq!((frame.pts, frame.par, frame.color, frame.width, frame.height), (pts, (4, 3), color, 3, 3));
        let PixelData::Yuv16 { planes, chroma, bits, alpha } = &frame.data else {
            return Err("expected YUV16, not a converted RGB frame".into());
        };
        assert_eq!((*chroma, *bits, alpha.is_none()), (Chroma::C420, 10, true));
        assert_eq!(planes.each_ref().map(|plane| plane.as_slice()), [&expected[..9], &expected[9..13], &expected[13..]]);
        assert_eq!(reader.take_reads(), vec![(offset, 34)]);
    }
    Ok(())
}

#[test]
fn defaults_and_special_planes_remain_distinct() -> TestResult<()> {
    let cases = [
        (b"YUV4MPEG2 W1 H1 Cmono\nFRAME\n\x31".as_slice(), Chroma::C420, [0x31, 128, 128], None),
        (b"YUV4MPEG2 W1 H1 C444alpha\nFRAME\n\x45\x67\x89\xab".as_slice(), Chroma::C444, [0x45, 0x67, 0x89], Some(0xab)),
    ];
    for (bytes, expected_chroma, expected, expected_alpha) in cases {
        let memory: Arc<[u8]> = bytes.into();
        let source = Y4mSource::open("one.y4m", Arc::new(memory))?;
        let video = source.info().video.as_ref().ok_or("expected Y4M video")?;
        assert_eq!((source.info().duration, video.frame_rate, video.par), (Tick::from_units(1, 25), FrameRate::FPS_25, (1, 1)));
        assert_eq!(video.color.range, Range::Limited);
        assert_eq!(video.has_alpha, expected_alpha.is_some());
        let frame = source.video_frame(FrameRequest::full(Tick::ZERO))?;
        let PixelData::Yuv8 { planes, chroma, alpha } = &frame.data else {
            return Err("expected YUV8".into());
        };
        assert_eq!(*chroma, expected_chroma);
        assert_eq!(planes.each_ref().map(|plane| plane.as_slice()), expected.each_ref().map(std::slice::from_ref));
        assert_eq!(alpha.as_ref().map(|plane| plane.as_slice()), expected_alpha.as_ref().map(std::slice::from_ref));
    }
    Ok(())
}

#[test]
fn declines_unknown_and_oversize_without_reading_payloads() {
    assert!(!sniff(b"YUV4MPEG2x"));
    let unknown: Arc<[u8]> = b"YUV4MPEG2 W2 H2 C411\nFRAME\n".as_slice().into();
    assert!(sniff(&unknown));
    assert!(matches!(Y4mSource::open("unknown.y4m", Arc::new(unknown)), Err(MediaError::Unsupported(_))));

    // 2^26 pixels, 6 bytes per pixel: within the parser's pixel limit, but above the
    // source's byte-allocation ceiling. Model the payload length; allocate only its header.
    let prefix = b"YUV4MPEG2 W8192 H8192 C444p16\nFRAME\n".to_vec();
    let frame_bytes = 8192u64 * 8192 * 6;
    assert!(frame_bytes > DEFAULT_MAX_MEDIA_BYTES as u64);
    let length = prefix.len() as u64 + frame_bytes;
    let reader = Tracked::new(prefix, length);
    let input: SharedBytes = reader.clone();
    assert!(matches!(Y4mSource::open("large.y4m", input), Err(MediaError::Unsupported(_))));
    let reads = reader.take_reads();
    assert!(!reads.is_empty() && reads.into_iter().all(|(_, len)| len <= 128));
}

fn sparse_mono_pair() -> (Vec<u8>, u64, u64, u64) {
    let header = b"YUV4MPEG2 W16 H16 Cmono\n";
    let mut prefix = header.to_vec();
    prefix.extend_from_slice(b"FRAME\n");
    prefix.resize(prefix.len() + 256, 0);
    let at_second = prefix.len() as u64;
    prefix.extend_from_slice(b"FRAME\n");
    let length = prefix.len() as u64 + 256;
    (prefix, header.len() as u64, at_second, length)
}

#[test]
fn index_preserves_second_frame_delimiter_io_error() -> TestResult<()> {
    let (prefix, first, second, length) = sparse_mono_pair();
    // Only the read starting at the second delimiter fails; the payload was in bounds.
    let reader = Arc::new(Tracked { prefix, length, fail_at: Some(second), shrink_after: None, reads: Mutex::new(Vec::new()) });
    let input: SharedBytes = reader.clone();
    let result = Y4mSource::open("unreadable.y4m", input);
    assert_eq!(reader.take_reads(), vec![(0, 128), (first, 128), (second, 128)]);
    let Err(MediaError::Io(message)) = result else {
        return Err("a second delimiter read failure must not silently shorten the video".into());
    };
    assert!(message.contains("injected second Y4M delimiter read failure"));
    Ok(())
}

#[test]
fn index_reports_short_read_when_source_shrinks_mid_open() -> TestResult<()> {
    let (prefix, first, second, length) = sparse_mono_pair();
    // The opener and the first two range reads see the original extent. The allocator
    // observes the shrink on the next range, just two bytes into the second delimiter.
    let reader = Arc::new(Tracked { prefix, length, fail_at: None, shrink_after: Some((2, second + 2)), reads: Mutex::new(Vec::new()) });
    let input: SharedBytes = reader.clone();
    let result = Y4mSource::open("shrunk.y4m", input);
    assert_eq!(reader.take_reads(), vec![(0, 128), (first, 128), (second, 2)]);
    let Err(MediaError::Io(message)) = result else {
        return Err("a short read from a changing source must not silently shorten the video".into());
    };
    assert!(message.contains("short read while indexing Y4M"));
    Ok(())
}

#[test]
fn nonintegral_tick_pts_selects_the_frame_it_labels() -> TestResult<()> {
    let bytes: Arc<[u8]> = b"YUV4MPEG2 W1 H1 F31:1 Cmono\nFRAME\n\x11FRAME\n\x22".as_slice().into();
    let source = Y4mSource::open("fractional-ticks.y4m", Arc::new(bytes))?;
    let second_tick = Tick(8_194_064_517);
    assert_eq!(source.info().duration, Tick(16_388_129_033));
    for (ask, pts, luma) in [(Tick(second_tick.0 - 1), Tick::ZERO, 0x11), (second_tick, second_tick, 0x22)] {
        let frame = source.video_frame(FrameRequest::full(ask))?;
        let PixelData::Yuv8 { planes, .. } = &frame.data else {
            return Err("expected YUV8".into());
        };
        assert_eq!(frame.pts, pts);
        assert_eq!(planes[0].as_slice(), &[luma]);
    }
    Ok(())
}
