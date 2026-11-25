//! Job specifications, output targets, and queue status data structures.
//!
//! Moved from `encoderCraft`'s `encodercraft-core` (`storytold/craft-libs#505`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::DispatchError;
use crate::audio::{AudioChannelMapping, ChannelLayout};
use crate::metadata::XmpMetadata;
use crate::timecode::Rational64;

/// Pixel format layout and component organization requested for an output target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PixelFormat {
    /// 8-bit per channel Red, Green, Blue, Alpha (packed 32-bit).
    Rgba8,
    /// 8-bit per channel Blue, Green, Red, Alpha (packed 32-bit).
    Bgra8,
    /// 8-bit per channel Red, Green, Blue (packed 24-bit).
    Rgb8,
    /// 16-bit unsigned integer per channel RGBA (packed 64-bit).
    Rgba16,
    /// 32-bit IEEE float per channel RGBA (packed 128-bit, HDR / compositing).
    RgbaF32,
    /// Planar YUV 4:2:0 8-bit (standard H.264 / WebM / JPEG).
    Yuv420p,
    /// Planar YUV 4:2:2 8-bit (Avid DNxHD/DNxHR SQ/HQ, broadcast standard).
    Yuv422p,
    /// Planar YUV 4:2:0 10-bit little-endian (HDR10, HEVC Main 10).
    Yuv420p10,
    /// Planar YUV 4:2:2 10-bit little-endian (Apple ProRes 422, broadcast MXF).
    Yuv422p10,
    /// Planar YUV 4:4:4 12-bit little-endian (ProRes 4444, master archival).
    Yuv444p12,
    /// Single channel 8-bit grayscale.
    Gray8,
    /// Single channel 16-bit grayscale.
    Gray16,
}

impl PixelFormat {
    /// Returns true if this format includes an alpha channel.
    pub fn has_alpha(&self) -> bool {
        matches!(self, Self::Rgba8 | Self::Bgra8 | Self::Rgba16 | Self::RgbaF32)
    }

    /// Returns true if this format is planar (Y, U, V in separate memory buffers).
    pub fn is_planar(&self) -> bool {
        matches!(self, Self::Yuv420p | Self::Yuv422p | Self::Yuv420p10 | Self::Yuv422p10 | Self::Yuv444p12)
    }

    /// Bit depth per color component.
    pub fn bits_per_component(&self) -> u8 {
        match self {
            Self::Rgba8 | Self::Bgra8 | Self::Rgb8 | Self::Yuv420p | Self::Yuv422p | Self::Gray8 => 8,
            Self::Yuv420p10 | Self::Yuv422p10 => 10,
            Self::Yuv444p12 => 12,
            Self::Rgba16 | Self::Gray16 => 16,
            Self::RgbaF32 => 32,
        }
    }
}

/// Mode used to scale a source image or video frame into destination dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScaleMode {
    /// Fit within destination bounds, maintaining aspect ratio (adds letterbox or pillarbox padding).
    #[default]
    Fit,
    /// Fill the destination bounds entirely, maintaining aspect ratio (crops excess source content).
    Fill,
    /// Stretch non-uniformly to exactly match target width and height.
    Stretch,
    /// Center 1:1 without scaling; pad with black if destination is larger, or crop if smaller.
    Center,
}

/// Execution priority of a queued job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JobPriority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

/// Execution start policy upon submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StartPolicy {
    /// Begins encoding as soon as a worker becomes available.
    #[default]
    AutoStart,
    /// Adds to queue in a paused state until explicitly started by the user.
    Paused,
}

/// Current status of a job or output target in the queue.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JobStatus {
    #[default]
    Queued,
    Processing,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

/// Identifying metadata about the submitting client application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub app_name: String,
    pub app_version: String,
    pub pid: u32,
    #[serde(default)]
    pub user_notes: Option<String>,
}

/// Single clip descriptor inside a multi-clip stitched sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StitchedClipSource {
    pub path: PathBuf,
    #[serde(default)]
    pub in_point_ticks: Option<u64>,
    #[serde(default)]
    pub out_point_ticks: Option<u64>,
}

/// Caption export format for sidecar generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptionExportFormat {
    Srt,
    WebVtt,
    Scc,
    Mcc,
    Ttml,
    Dfxp,
    Ass,
    Stl,
    Pjs,
    RealText,
    JacoSub,
}

impl CaptionExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::WebVtt => "vtt",
            Self::Scc => "scc",
            Self::Mcc => "mcc",
            Self::Ttml => "ttml",
            Self::Dfxp => "dfxp",
            Self::Ass => "ass",
            Self::Stl => "stl",
            Self::Pjs => "pjs",
            Self::RealText => "rt",
            Self::JacoSub => "jss",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "srt" => Some(Self::Srt),
            "vtt" => Some(Self::WebVtt),
            "scc" => Some(Self::Scc),
            "mcc" => Some(Self::Mcc),
            "ttml" => Some(Self::Ttml),
            "dfxp" => Some(Self::Dfxp),
            "ass" | "ssa" => Some(Self::Ass),
            "stl" => Some(Self::Stl),
            "pjs" => Some(Self::Pjs),
            "rt" => Some(Self::RealText),
            "jss" | "js" => Some(Self::JacoSub),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Srt => "SubRip (.srt)",
            Self::WebVtt => "WebVTT (.vtt)",
            Self::Scc => "Scenarist SCC (.scc)",
            Self::Mcc => "MacCaption MCC (.mcc)",
            Self::Ttml => "TTML IMSC1 (.ttml)",
            Self::Dfxp => "TTML / DFXP (.dfxp)",
            Self::Ass => "SubStation Alpha (.ass)",
            Self::Stl => "EBU STL (.stl)",
            Self::Pjs => "Phoenix Subtitle (.pjs)",
            Self::RealText => "RealText (.rt)",
            Self::JacoSub => "JACOsub (.jss)",
        }
    }
}

/// Audio stem export mode for multichannel workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AudioStemMode {
    /// No stem splitting (default single master output).
    #[default]
    None,
    /// Export each channel in the mix as a separate discrete mono BWF WAV file.
    DiscreteMono,
    /// Export channel pairs (e.g. 1-2, 3-4) as stereo BWF WAV files.
    StereoPairs,
}

/// Classification of media payload in a container or stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MediaStreamKind {
    /// Audio-only media (e.g. MP3, WAV, FLAC, AIFF, Ogg, AAC, M4A).
    AudioOnly,
    /// Video containing one or more audio tracks.
    VideoWithAudio,
    /// Video-only media (no audio tracks present).
    VideoOnly,
    /// Single still image (PNG, TIFF, JPEG, EXR, WebP).
    StillImage,
    /// Sequence of still images.
    ImageSequence,
    /// 2D vector graphic (SVG, AI, EPS).
    Vector2D,
    /// Digital document or page layout (PDF, DOCX, IDML, INDD, EPUB, PPTX, XLSX).
    Document,
    /// 3D spatial asset (glTF, GLB, OBJ, STL, PLY).
    Spatial3D,
    /// 2D CAD blueprint or technical drawing (AutoCAD DXF, DWG, HPGL).
    Cad2D,
    /// Multi-layer composite image with masks, layers, and adjustment channels (PSD, PSB, PhotoCraft).
    LayeredComposite,
    /// Procedural motion graphics, composition DAG, keyframed curves (After Effects AEP, EffectCraft).
    MotionGraph,
    /// Non-linear editing video timeline with cuts, tracks, and transitions (Premiere Pro PRPROJ, FilmCraft).
    ProjectTimeline,
    /// Unknown or unprobed format.
    #[default]
    Unknown,
}

impl MediaStreamKind {
    /// Returns true if this media kind contains a video stream.
    pub fn has_video(&self) -> bool {
        matches!(self, Self::VideoWithAudio | Self::VideoOnly | Self::StillImage | Self::ImageSequence | Self::MotionGraph | Self::ProjectTimeline)
    }

    /// Returns true if this media kind is strictly audio.
    pub fn is_audio_only(&self) -> bool {
        matches!(self, Self::AudioOnly)
    }

    /// Returns true if this media kind is a still image or image sequence.
    pub fn is_image(&self) -> bool {
        matches!(self, Self::StillImage | Self::ImageSequence)
    }

    /// Returns true if this media kind is a 2D vector asset.
    pub fn is_vector(&self) -> bool {
        matches!(self, Self::Vector2D)
    }

    /// Returns true if this media kind is a digital document or page layout.
    pub fn is_document(&self) -> bool {
        matches!(self, Self::Document)
    }

    /// Returns true if this media kind is a 3D asset.
    pub fn is_spatial_3d(&self) -> bool {
        matches!(self, Self::Spatial3D)
    }

    /// Returns true if this media kind is a 2D CAD technical drawing.
    pub fn is_cad(&self) -> bool {
        matches!(self, Self::Cad2D)
    }

    /// Returns true if this media kind is a layered image composite.
    pub fn is_layered_composite(&self) -> bool {
        matches!(self, Self::LayeredComposite)
    }

    /// Returns true if this media kind is a motion graphics composition.
    pub fn is_motion_graph(&self) -> bool {
        matches!(self, Self::MotionGraph)
    }

    /// Returns true if this media kind is an NLE project timeline.
    pub fn is_project_timeline(&self) -> bool {
        matches!(self, Self::ProjectTimeline)
    }
}

/// Mode of media source ingestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SourceDescriptor {
    /// File source already present on disk.
    File {
        path: PathBuf,
        #[serde(default)]
        start_tick: Option<i64>,
        #[serde(default)]
        duration_ticks: Option<i64>,
        #[serde(default)]
        timebase: Option<Rational64>,
    },
    /// Multiple sequential clips stitched together into a single continuous sequence.
    StitchedClips { clips: Vec<StitchedClipSource> },
    /// Live frame streaming over local IPC domain socket or named pipe.
    Pipe { endpoint: String, width: u32, height: u32, frame_rate: Rational64, sample_rate: u32, channels: u32 },
    /// Headless project delegation (renders via origin app's CLI).
    ProjectReference { project_path: PathBuf, composition_id: String, host_cli_path: Option<PathBuf> },
}

/// High-level output media container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerType {
    IsoBmffMp4,
    QuickTimeMov,
    MatroskaMkv,
    WebM,
    MpegTs,
    MxfOp1a,
    MxfOpAtom,
    Avi,
    Wave,
    Aiff,
    Flac,
    OggAudio,
    Mp3,
    Caf,
    Opus,
    Ac3,
    Eac3,
    Alac,
    Qoa,
    Aaf,
    Omf,
    Edl,
    Otio,
    StillSequence,
    AnimatedGif,
    Png,
    Jpeg,
    WebP,
    Tiff,
    OpenExr,
    Bmp,
    Dds,
    Dpx,
    Jxl,
    RadianceHdr,
    Dicom,
    Fits,
    Svg,
    PostScriptEps,
    Gltf,
    Usdz,
    Usdc,
    Obj,
    Stl,
    Ply,
    Fbx,
    Alembic,
    Collada,
    Splat,
    Step,
    Pdf,
    Docx,
    Markdown,
    Html,
    Idml,
    Epub,
    Pptx,
    LottieJson,
    WindowsMetafile,
    AdobeInDesignIndd,
    AdobePhotoshopPsd,
    AdobePhotoshopPsb,
    AdobeIllustratorAi,
    AdobeAfterEffectsAep,
    AdobePremierePrproj,
    AdobeAuditionSesx,
    AutocadDxf,
    AutocadDwg,
    HpglPlotter,
    ArtcraftDesigncraft,
    ArtcraftPhotocraft,
    ArtcraftVectorcraft,
    ArtcraftEffectcraft,
    ArtcraftFilmcraft,
    ArtcraftSoundcraft,
    ArtcraftCadcraft,
    ArtcraftAnicraft,
}

impl ContainerType {
    /// Every container variant, for lookups and enumeration.
    pub const ALL: &'static [ContainerType] = &[
        Self::IsoBmffMp4,
        Self::QuickTimeMov,
        Self::MatroskaMkv,
        Self::WebM,
        Self::MpegTs,
        Self::MxfOp1a,
        Self::MxfOpAtom,
        Self::Avi,
        Self::Wave,
        Self::Aiff,
        Self::Flac,
        Self::OggAudio,
        Self::Mp3,
        Self::Caf,
        Self::Opus,
        Self::Ac3,
        Self::Eac3,
        Self::Alac,
        Self::Qoa,
        Self::Aaf,
        Self::Omf,
        Self::Edl,
        Self::Otio,
        Self::StillSequence,
        Self::AnimatedGif,
        Self::Png,
        Self::Jpeg,
        Self::WebP,
        Self::Tiff,
        Self::OpenExr,
        Self::Bmp,
        Self::Dds,
        Self::Dpx,
        Self::Jxl,
        Self::RadianceHdr,
        Self::Dicom,
        Self::Fits,
        Self::Svg,
        Self::PostScriptEps,
        Self::Gltf,
        Self::Usdz,
        Self::Usdc,
        Self::Obj,
        Self::Stl,
        Self::Ply,
        Self::Fbx,
        Self::Alembic,
        Self::Collada,
        Self::Splat,
        Self::Step,
        Self::Pdf,
        Self::Docx,
        Self::Markdown,
        Self::Html,
        Self::Idml,
        Self::Epub,
        Self::Pptx,
        Self::LottieJson,
        Self::WindowsMetafile,
        Self::AdobeInDesignIndd,
        Self::AdobePhotoshopPsd,
        Self::AdobePhotoshopPsb,
        Self::AdobeIllustratorAi,
        Self::AdobeAfterEffectsAep,
        Self::AdobePremierePrproj,
        Self::AdobeAuditionSesx,
        Self::AutocadDxf,
        Self::AutocadDwg,
        Self::HpglPlotter,
        Self::ArtcraftDesigncraft,
        Self::ArtcraftPhotocraft,
        Self::ArtcraftVectorcraft,
        Self::ArtcraftEffectcraft,
        Self::ArtcraftFilmcraft,
        Self::ArtcraftSoundcraft,
        Self::ArtcraftCadcraft,
        Self::ArtcraftAnicraft,
    ];

    /// Canonical file extension without leading dot.
    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::IsoBmffMp4 => "mp4",
            Self::QuickTimeMov => "mov",
            Self::MatroskaMkv => "mkv",
            Self::WebM => "webm",
            Self::MpegTs => "ts",
            Self::MxfOp1a | Self::MxfOpAtom => "mxf",
            Self::Avi => "avi",
            Self::Wave => "wav",
            Self::Aiff => "aif",
            Self::Flac => "flac",
            Self::OggAudio => "ogg",
            Self::Mp3 => "mp3",
            Self::Caf => "caf",
            Self::Opus => "opus",
            Self::Ac3 => "ac3",
            Self::Eac3 => "eac3",
            Self::Alac => "m4a",
            Self::Qoa => "qoa",
            Self::Aaf => "aaf",
            Self::Omf => "omf",
            Self::Edl => "edl",
            Self::Otio => "otio",
            Self::StillSequence => "png",
            Self::AnimatedGif => "gif",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
            Self::Tiff => "tif",
            Self::OpenExr => "exr",
            Self::Bmp => "bmp",
            Self::Dds => "dds",
            Self::Dpx => "dpx",
            Self::Jxl => "jxl",
            Self::RadianceHdr => "hdr",
            Self::Dicom => "dcm",
            Self::Fits => "fits",
            Self::Svg => "svg",
            Self::PostScriptEps => "eps",
            Self::Gltf => "glb",
            Self::Usdz => "usdz",
            Self::Usdc => "usdc",
            Self::Obj => "obj",
            Self::Stl => "stl",
            Self::Ply => "ply",
            Self::Fbx => "fbx",
            Self::Alembic => "abc",
            Self::Collada => "dae",
            Self::Splat => "splat",
            Self::Step => "step",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Idml => "idml",
            Self::Epub => "epub",
            Self::Pptx => "pptx",
            Self::LottieJson => "json",
            Self::WindowsMetafile => "wmf",
            Self::AdobeInDesignIndd => "indd",
            Self::AdobePhotoshopPsd => "psd",
            Self::AdobePhotoshopPsb => "psb",
            Self::AdobeIllustratorAi => "ai",
            Self::AdobeAfterEffectsAep => "aep",
            Self::AdobePremierePrproj => "prproj",
            Self::AdobeAuditionSesx => "sesx",
            Self::AutocadDxf => "dxf",
            Self::AutocadDwg => "dwg",
            Self::HpglPlotter => "plt",
            Self::ArtcraftDesigncraft => "designcraft",
            Self::ArtcraftPhotocraft => "photocraft",
            Self::ArtcraftVectorcraft => "vectorcraft",
            Self::ArtcraftEffectcraft => "effectcraft",
            Self::ArtcraftFilmcraft => "filmcraft",
            Self::ArtcraftSoundcraft => "soundcraft",
            Self::ArtcraftCadcraft => "cadcraft",
            Self::ArtcraftAnicraft => "anicraft",
        }
    }

    /// Resolves a container from a file extension (case-insensitive, with or without a leading dot) or a common alias.
    pub fn from_extension(ext: &str) -> Option<ContainerType> {
        let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        let alias = match ext.as_str() {
            "m4v" => Some(Self::IsoBmffMp4),
            "ts" | "m2ts" | "mts" => Some(Self::MpegTs),
            "aiff" => Some(Self::Aiff),
            "jpeg" => Some(Self::Jpeg),
            "tiff" => Some(Self::Tiff),
            "gltf" => Some(Self::Gltf),
            "oga" | "ogg" => Some(Self::OggAudio),
            "opus" => Some(Self::Opus),
            "ec3" => Some(Self::Eac3),
            "m4a" | "alac" => Some(Self::Alac),
            "caf" => Some(Self::Caf),
            "bwf" => Some(Self::Wave),
            "aif" => Some(Self::Aiff),
            "jpg" => Some(Self::Jpeg),
            "tif" => Some(Self::Tiff),
            "glb" => Some(Self::Gltf),
            "ps" => Some(Self::PostScriptEps),
            "hdr" | "pic" => Some(Self::RadianceHdr),
            "dcm" | "dicom" => Some(Self::Dicom),
            "fits" | "fit" | "fts" => Some(Self::Fits),
            "fbx" => Some(Self::Fbx),
            "abc" | "alembic" => Some(Self::Alembic),
            "dae" | "collada" => Some(Self::Collada),
            "usdc" => Some(Self::Usdc),
            "splat" => Some(Self::Splat),
            "stp" => Some(Self::Step),
            "markdown" => Some(Self::Markdown),
            "htm" => Some(Self::Html),
            "epub" => Some(Self::Epub),
            "pptx" => Some(Self::Pptx),
            "json" | "lottie" => Some(Self::LottieJson),
            "wmf" | "emf" => Some(Self::WindowsMetafile),
            _ => None,
        };
        if alias.is_some() {
            return alias;
        }
        Self::ALL.iter().copied().find(|c| c.file_extension() == ext && !matches!(c, Self::MxfOpAtom | Self::StillSequence))
    }

    /// Whether converting a `self` source into `target` is a supported conversion.
    pub fn can_convert_to(&self, target: ContainerType) -> bool {
        self.valid_conversion_targets().contains(&target)
    }

    /// Returns true if this container format can carry video streams.
    pub fn supports_video(&self) -> bool {
        matches!(
            self,
            Self::IsoBmffMp4
                | Self::QuickTimeMov
                | Self::MatroskaMkv
                | Self::WebM
                | Self::MpegTs
                | Self::MxfOp1a
                | Self::MxfOpAtom
                | Self::Avi
                | Self::AnimatedGif
                | Self::StillSequence
                | Self::Aaf
                | Self::Omf
                | Self::Edl
                | Self::Otio
                | Self::AdobePremierePrproj
                | Self::AdobeAfterEffectsAep
                | Self::ArtcraftFilmcraft
                | Self::ArtcraftEffectcraft
                | Self::ArtcraftAnicraft
        )
    }

    /// Returns true if this container format can carry audio streams.
    pub fn supports_audio(&self) -> bool {
        matches!(
            self,
            Self::IsoBmffMp4
                | Self::QuickTimeMov
                | Self::MatroskaMkv
                | Self::WebM
                | Self::MpegTs
                | Self::MxfOp1a
                | Self::MxfOpAtom
                | Self::Avi
                | Self::Wave
                | Self::Aiff
                | Self::Flac
                | Self::OggAudio
                | Self::Mp3
                | Self::Caf
                | Self::Opus
                | Self::Ac3
                | Self::Eac3
                | Self::Alac
                | Self::Qoa
                | Self::Aaf
                | Self::Omf
                | Self::Edl
                | Self::Otio
                | Self::AdobeAuditionSesx
                | Self::AdobePremierePrproj
                | Self::AdobeAfterEffectsAep
                | Self::ArtcraftSoundcraft
                | Self::ArtcraftFilmcraft
                | Self::ArtcraftEffectcraft
        )
    }

    /// User-friendly display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::IsoBmffMp4 => "MP4 (ISO BMFF)",
            Self::QuickTimeMov => "QuickTime Movie (MOV)",
            Self::MatroskaMkv => "Matroska Video (MKV)",
            Self::WebM => "WebM",
            Self::MpegTs => "MPEG Transport Stream (TS)",
            Self::MxfOp1a => "MXF OP1a Broadcast",
            Self::MxfOpAtom => "MXF OP-Atom",
            Self::Avi => "Audio Video Interleave (AVI)",
            Self::Wave => "Waveform Audio (WAV / BWF)",
            Self::Aiff => "Audio Interchange (AIFF)",
            Self::Flac => "Free Lossless Audio Codec (FLAC)",
            Self::OggAudio => "Ogg Vorbis Audio",
            Self::Mp3 => "MPEG Audio Layer 3 (MP3)",
            Self::Caf => "Apple Core Audio Format (CAF)",
            Self::Opus => "Ogg Opus Audio (OPUS)",
            Self::Ac3 => "Dolby Digital Audio (AC-3)",
            Self::Eac3 => "Dolby Digital Plus Audio (E-AC-3)",
            Self::Alac => "Apple Lossless Audio (ALAC / M4A)",
            Self::Qoa => "Quite OK Audio (QOA)",
            Self::Aaf => "Advanced Authoring Format (AAF)",
            Self::Omf => "Open Media Framework Interchange (OMF)",
            Self::Edl => "CMX 3600 Edit Decision List (EDL)",
            Self::Otio => "OpenTimelineIO Editorial Timeline (OTIO)",
            Self::StillSequence => "Numbered Image Sequence",
            Self::AnimatedGif => "Animated GIF",
            Self::Png => "Portable Network Graphics (PNG)",
            Self::Jpeg => "JPEG Image",
            Self::WebP => "WebP Image",
            Self::Tiff => "Tagged Image File Format (TIFF)",
            Self::OpenExr => "ILM OpenEXR (Float)",
            Self::Bmp => "Windows Bitmap (BMP)",
            Self::Dds => "DirectDraw Surface Texture (DDS)",
            Self::Dpx => "SMPTE 268M Digital Picture Exchange (DPX)",
            Self::Jxl => "JPEG XL Image (JXL)",
            Self::RadianceHdr => "Radiance HDR RGBE (.hdr)",
            Self::Dicom => "Medical DICOM Image (.dcm)",
            Self::Fits => "Astronomical FITS Data (.fits)",
            Self::Svg => "Scalable Vector Graphics (SVG)",
            Self::PostScriptEps => "Encapsulated PostScript (EPS)",
            Self::Gltf => "glTF 2.0 Binary (GLB)",
            Self::Usdz => "Apple AR Quick Look (USDZ)",
            Self::Usdc => "Universal Scene Description Crate (USDC)",
            Self::Obj => "Wavefront 3D (OBJ)",
            Self::Stl => "Stereolithography 3D (STL)",
            Self::Ply => "Stanford Polygon 3D (PLY)",
            Self::Fbx => "Autodesk Filmbox 3D (FBX)",
            Self::Alembic => "Alembic VFX Animation Cache (ABC)",
            Self::Collada => "Collada Digital Asset Exchange (DAE)",
            Self::Splat => "3D Gaussian Splat Point Cloud (SPLAT)",
            Self::Step => "ISO 10303 STEP CAD (.step)",
            Self::Pdf => "Portable Document Format (PDF)",
            Self::Docx => "Microsoft Word Document (DOCX)",
            Self::Markdown => "Markdown Document (.md)",
            Self::Html => "HTML Document (.html)",
            Self::Idml => "InDesign Markup Language (IDML)",
            Self::Epub => "EPUB 3 Digital Publication (EPUB)",
            Self::Pptx => "Microsoft PowerPoint Presentation (PPTX)",
            Self::LottieJson => "Lottie Animation JSON (JSON)",
            Self::WindowsMetafile => "Windows Metafile (WMF / EMF)",
            Self::AdobeInDesignIndd => "Adobe InDesign Document (INDD)",
            Self::AdobePhotoshopPsd => "Adobe Photoshop Document (PSD)",
            Self::AdobePhotoshopPsb => "Adobe Photoshop Large Document (PSB)",
            Self::AdobeIllustratorAi => "Adobe Illustrator Artwork (AI)",
            Self::AdobeAfterEffectsAep => "Adobe After Effects Project (AEP)",
            Self::AdobePremierePrproj => "Adobe Premiere Pro Project (PRPROJ)",
            Self::AdobeAuditionSesx => "Adobe Audition Session (SESX)",
            Self::AutocadDxf => "AutoCAD Drawing Exchange (DXF)",
            Self::AutocadDwg => "AutoCAD Drawing (DWG)",
            Self::HpglPlotter => "HPGL / PLT Plotter Stream",
            Self::ArtcraftDesigncraft => "DesignCraft Publication (.designcraft)",
            Self::ArtcraftPhotocraft => "PhotoCraft Composite (.photocraft)",
            Self::ArtcraftVectorcraft => "VectorCraft Artwork (.vectorcraft)",
            Self::ArtcraftEffectcraft => "EffectCraft Composition (.effectcraft)",
            Self::ArtcraftFilmcraft => "FilmCraft Timeline (.filmcraft)",
            Self::ArtcraftSoundcraft => "SoundCraft Session (.soundcraft)",
            Self::ArtcraftCadcraft => "CadCraft Blueprint (.cadcraft)",
            Self::ArtcraftAnicraft => "AniCraft Animation (.anicraft)",
        }
    }

    /// Primary media domain for this container.
    pub fn media_kind(&self) -> MediaStreamKind {
        match self {
            Self::IsoBmffMp4 | Self::QuickTimeMov | Self::MatroskaMkv | Self::WebM | Self::MpegTs | Self::MxfOp1a | Self::MxfOpAtom | Self::Avi => {
                MediaStreamKind::VideoWithAudio
            }
            Self::AnimatedGif => MediaStreamKind::VideoOnly,
            Self::Wave
            | Self::Aiff
            | Self::Flac
            | Self::OggAudio
            | Self::Mp3
            | Self::Caf
            | Self::Opus
            | Self::Ac3
            | Self::Eac3
            | Self::Alac
            | Self::Qoa
            | Self::AdobeAuditionSesx
            | Self::ArtcraftSoundcraft => MediaStreamKind::AudioOnly,
            Self::Png
            | Self::Jpeg
            | Self::WebP
            | Self::Tiff
            | Self::OpenExr
            | Self::Bmp
            | Self::Dds
            | Self::Dpx
            | Self::Jxl
            | Self::RadianceHdr
            | Self::Dicom
            | Self::Fits => MediaStreamKind::StillImage,
            Self::StillSequence | Self::ArtcraftAnicraft => MediaStreamKind::ImageSequence,
            Self::Svg | Self::AdobeIllustratorAi | Self::ArtcraftVectorcraft | Self::PostScriptEps | Self::WindowsMetafile => {
                MediaStreamKind::Vector2D
            }
            Self::Pdf
            | Self::Docx
            | Self::Markdown
            | Self::Html
            | Self::Idml
            | Self::Epub
            | Self::Pptx
            | Self::AdobeInDesignIndd
            | Self::ArtcraftDesigncraft => MediaStreamKind::Document,
            Self::Gltf
            | Self::Usdz
            | Self::Usdc
            | Self::Obj
            | Self::Stl
            | Self::Ply
            | Self::Fbx
            | Self::Alembic
            | Self::Collada
            | Self::Splat
            | Self::Step => MediaStreamKind::Spatial3D,
            Self::AutocadDxf | Self::AutocadDwg | Self::HpglPlotter | Self::ArtcraftCadcraft => MediaStreamKind::Cad2D,
            Self::AdobePhotoshopPsd | Self::AdobePhotoshopPsb | Self::ArtcraftPhotocraft => MediaStreamKind::LayeredComposite,
            Self::AdobeAfterEffectsAep | Self::ArtcraftEffectcraft | Self::LottieJson => MediaStreamKind::MotionGraph,
            Self::AdobePremierePrproj | Self::ArtcraftFilmcraft | Self::Aaf | Self::Omf | Self::Edl | Self::Otio => MediaStreamKind::ProjectTimeline,
        }
    }

    /// Returns valid, compatible target container types for transcode or export from this source container.
    pub fn valid_conversion_targets(&self) -> Vec<ContainerType> {
        match self.media_kind() {
            MediaStreamKind::AudioOnly if matches!(self, Self::AdobeAuditionSesx | Self::ArtcraftSoundcraft) => {
                vec![
                    Self::Wave,
                    Self::Aiff,
                    Self::Flac,
                    Self::Ac3,
                    Self::Eac3,
                    Self::Alac,
                    Self::Qoa,
                    Self::Aaf,
                    Self::Omf,
                    Self::AdobeAuditionSesx,
                    Self::ArtcraftSoundcraft,
                ]
            }
            MediaStreamKind::AudioOnly => vec![
                Self::Wave,
                Self::Aiff,
                Self::Flac,
                Self::Mp3,
                Self::OggAudio,
                Self::Caf,
                Self::Opus,
                Self::Ac3,
                Self::Eac3,
                Self::Alac,
                Self::Qoa,
                Self::Aaf,
                Self::Omf,
                Self::ArtcraftSoundcraft,
            ],
            MediaStreamKind::VideoOnly => vec![
                Self::IsoBmffMp4,
                Self::QuickTimeMov,
                Self::WebM,
                Self::MatroskaMkv,
                Self::MpegTs,
                Self::MxfOp1a,
                Self::MxfOpAtom,
                Self::Avi,
                Self::AnimatedGif,
                Self::StillSequence,
                Self::Edl,
                Self::Otio,
                Self::Aaf,
                Self::Omf,
            ],
            MediaStreamKind::VideoWithAudio => vec![
                Self::IsoBmffMp4,
                Self::QuickTimeMov,
                Self::WebM,
                Self::MatroskaMkv,
                Self::MpegTs,
                Self::MxfOp1a,
                Self::MxfOpAtom,
                Self::Avi,
                Self::AnimatedGif,
                Self::StillSequence,
                Self::Wave,
                Self::Aiff,
                Self::Flac,
                Self::Mp3,
                Self::Caf,
                Self::Opus,
                Self::Ac3,
                Self::Eac3,
                Self::Alac,
                Self::Qoa,
                Self::Edl,
                Self::Otio,
                Self::Aaf,
                Self::Omf,
            ],
            MediaStreamKind::StillImage => {
                vec![
                    Self::Png,
                    Self::Jpeg,
                    Self::WebP,
                    Self::Tiff,
                    Self::OpenExr,
                    Self::Bmp,
                    Self::Dds,
                    Self::Dpx,
                    Self::Jxl,
                    Self::RadianceHdr,
                    Self::Dicom,
                    Self::Fits,
                    Self::AdobePhotoshopPsd,
                    Self::ArtcraftPhotocraft,
                ]
            }
            MediaStreamKind::LayeredComposite => vec![
                Self::ArtcraftPhotocraft,
                Self::AdobePhotoshopPsd,
                Self::AdobePhotoshopPsb,
                Self::OpenExr,
                Self::Tiff,
                Self::Png,
                Self::Jpeg,
                Self::WebP,
                Self::Bmp,
                Self::Dds,
                Self::Dpx,
                Self::Jxl,
                Self::RadianceHdr,
                Self::Fits,
            ],
            MediaStreamKind::Vector2D => {
                vec![
                    Self::ArtcraftVectorcraft,
                    Self::Svg,
                    Self::Pdf,
                    Self::PostScriptEps,
                    Self::AutocadDxf,
                    Self::WindowsMetafile,
                    Self::Png,
                    Self::Jpeg,
                    Self::WebP,
                    Self::Tiff,
                    Self::Bmp,
                    Self::AdobeIllustratorAi,
                ]
            }
            MediaStreamKind::Document => {
                vec![
                    Self::ArtcraftDesigncraft,
                    Self::Pdf,
                    Self::Idml,
                    Self::Docx,
                    Self::Markdown,
                    Self::Html,
                    Self::Epub,
                    Self::Pptx,
                    Self::Png,
                    Self::Jpeg,
                    Self::WebP,
                    Self::Tiff,
                    Self::Bmp,
                ]
            }
            MediaStreamKind::Spatial3D => vec![
                Self::Gltf,
                Self::Usdz,
                Self::Usdc,
                Self::Obj,
                Self::Stl,
                Self::Ply,
                Self::Fbx,
                Self::Alembic,
                Self::Collada,
                Self::Splat,
                Self::Step,
            ],
            MediaStreamKind::Cad2D => {
                vec![
                    Self::ArtcraftCadcraft,
                    Self::AutocadDxf,
                    Self::AutocadDwg,
                    Self::Svg,
                    Self::Pdf,
                    Self::HpglPlotter,
                    Self::Png,
                    Self::Jpeg,
                    Self::WebP,
                    Self::Tiff,
                    Self::Bmp,
                ]
            }
            MediaStreamKind::MotionGraph => vec![
                Self::ArtcraftEffectcraft,
                Self::IsoBmffMp4,
                Self::QuickTimeMov,
                Self::WebM,
                Self::MatroskaMkv,
                Self::AnimatedGif,
                Self::StillSequence,
                Self::Svg,
                Self::LottieJson,
            ],
            MediaStreamKind::ProjectTimeline => vec![
                Self::ArtcraftFilmcraft,
                Self::IsoBmffMp4,
                Self::QuickTimeMov,
                Self::MxfOp1a,
                Self::MatroskaMkv,
                Self::Avi,
                Self::Wave,
                Self::Aiff,
                Self::Flac,
                Self::Edl,
                Self::Otio,
                Self::Aaf,
                Self::Omf,
            ],
            MediaStreamKind::ImageSequence => vec![
                Self::IsoBmffMp4,
                Self::QuickTimeMov,
                Self::WebM,
                Self::MatroskaMkv,
                Self::Avi,
                Self::AnimatedGif,
                Self::Png,
                Self::Tiff,
                Self::OpenExr,
                Self::ArtcraftAnicraft,
            ],
            MediaStreamKind::Unknown => vec![Self::IsoBmffMp4, Self::QuickTimeMov, Self::Wave, Self::Png],
        }
    }
}

impl std::fmt::Display for ContainerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Which part of a multi-item source a target exports. All-`None` exports the whole source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSelection {
    /// Sequence or composition of a project, by exact name or zero-based index.
    #[serde(default)]
    pub sequence: Option<String>,
    /// Inclusive 1-based page range of a document.
    #[serde(default)]
    pub pages: Option<(u32, u32)>,
    /// Names of the layers to keep (CAD and layered raster sources).
    #[serde(default)]
    pub layers: Option<Vec<String>>,
}

impl SourceSelection {
    pub fn is_empty(&self) -> bool {
        self.sequence.is_none() && self.pages.is_none() && self.layers.is_none()
    }

    /// Rejects selections that can never be satisfied.
    pub fn validate(&self) -> Result<(), DispatchError> {
        if let Some((first, last)) = self.pages
            && (first == 0 || last < first)
        {
            return Err(DispatchError::InvalidJobSpec(format!(
                "invalid page range {first}-{last}: pages are 1-based and the range must not be reversed"
            )));
        }
        if let Some(layers) = &self.layers
            && layers.is_empty()
        {
            return Err(DispatchError::InvalidJobSpec("layer selection must name at least one layer".into()));
        }
        if let Some(seq) = &self.sequence
            && seq.trim().is_empty()
        {
            return Err(DispatchError::InvalidJobSpec("sequence selection cannot be empty".into()));
        }
        Ok(())
    }

    /// Parses `N` or `A-B` (1-based, inclusive).
    pub fn parse_pages(text: &str) -> Result<(u32, u32), DispatchError> {
        let bad = || DispatchError::InvalidJobSpec(format!("invalid page range '{text}' (use N or A-B, 1-based)"));
        let (a, b) = match text.split_once('-') {
            Some((a, b)) => (a.trim().parse::<u32>().map_err(|_| bad())?, b.trim().parse::<u32>().map_err(|_| bad())?),
            None => {
                let n = text.trim().parse::<u32>().map_err(|_| bad())?;
                (n, n)
            }
        };
        if a == 0 || b < a {
            return Err(bad());
        }
        Ok((a, b))
    }
}

/// Rate-control strategy requested for a video encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RateMode {
    /// Constant bitrate when a bitrate is given, constant quality otherwise.
    #[default]
    Auto,
    Cbr,
    Vbr,
    /// Constant quality; the bitrate is ignored.
    ConstantQuality,
}

/// Colour description written into the video bitstream (ITU-T H.273 code points).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorTag {
    /// BT.709 primaries, transfer and matrix, limited range.
    #[default]
    Bt709,
    /// BT.2020 with the PQ (SMPTE ST 2084) transfer.
    Pq,
    /// BT.2020 with the HLG (ARIB STD-B67) transfer.
    Hlg,
}

impl ColorTag {
    /// `(primaries, transfer, matrix)` per ITU-T H.273.
    pub const fn code_points(self) -> (u8, u8, u8) {
        match self {
            Self::Bt709 => (1, 1, 1),
            Self::Pq => (9, 16, 9),
            Self::Hlg => (9, 18, 9),
        }
    }
}

/// Clockwise rotation applied to the picture before scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    /// Whether the rotation swaps width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Self::Cw90 | Self::Cw270)
    }
}

/// Flip (mirroring) applied to the video frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlipMode {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

/// Deinterlacing method applied to interlaced video frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeinterlaceMode {
    #[default]
    None,
    /// Discard lower field and linearly interpolate missing scanlines.
    TopFieldFirst,
    /// Discard upper field and linearly interpolate missing scanlines.
    BottomFieldFirst,
    /// Bob deinterlacing: duplicate lines to preserve vertical resolution.
    Bob,
}

/// Pixel Aspect Ratio (PAR) conform behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelAspectRatioConform {
    #[default]
    Source,
    /// Rescale horizontally so pixels become 1.0 square.
    Square,
    /// Custom PAR numerator and denominator.
    Custom(u32, u32),
}

/// How frames are chosen when the output frame rate differs from the source's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FrameRateMode {
    /// Drop or repeat the nearest source frame.
    #[default]
    Nearest,
    /// Blend the two source frames around each output instant.
    Blend,
}

/// Spatial resampling filter used when frame dimensions change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScalingFilter {
    /// Two-tap linear interpolation.
    #[default]
    Bilinear,
    /// Cubic convolution with a Catmull-Rom kernel.
    Bicubic,
    /// Windowed-sinc reconstruction with a three-lobe Lanczos kernel.
    Lanczos3,
}

/// Geometry and timing conversion of the video stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoTransform {
    pub scale_mode: ScaleMode,
    pub scaling_filter: ScalingFilter,
    pub rotation: Rotation,
    pub flip: FlipMode,
    pub deinterlace: DeinterlaceMode,
    pub par_conform: PixelAspectRatioConform,
    pub frame_rate_mode: FrameRateMode,
}

/// Video encoder settings of a target; every field falls back to a codec default when `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoEncodeSettings {
    /// Codec identifier (`h264`, `hevc`, `av1`, `vp9`, `prores`, `dnxhr`, ...). `None` infers it from the preset name.
    pub codec: Option<String>,
    /// Maximum distance between keyframes in frames.
    pub keyframe_interval: Option<u32>,
    /// Codec profile: ProRes `proxy|lt|422|hq|4444|4444xq`, DNx `lb|sq|hq|hqx|444`, H.264 `baseline|main|high`.
    pub profile: Option<String>,
    /// Consecutive B-frames, for codecs that support them.
    pub bframes: Option<u8>,
    pub rate_mode: RateMode,
    pub color: Option<ColorTag>,
    /// Requested encoded pixel format; `None` selects the codec/container default.
    #[serde(default)]
    pub pixel_format: Option<PixelFormat>,
}

/// Screen anchoring position for visual overlays (timecode, watermark, name overlay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayAnchor {
    TopLeft,
    TopRight,
    #[default]
    BottomLeft,
    BottomRight,
    Center,
}

/// Burn-in SMPTE timecode overlay configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimecodeOverlayConfig {
    pub enabled: bool,
    pub anchor: OverlayAnchor,
    pub scale: usize,
    pub text_opacity: f32,
    pub bg_opacity: f32,
    pub starting_timecode: Option<String>,
}

impl Default for TimecodeOverlayConfig {
    fn default() -> Self {
        Self { enabled: false, anchor: OverlayAnchor::BottomLeft, scale: 2, text_opacity: 1.0, bg_opacity: 0.65, starting_timecode: None }
    }
}

/// Image watermark overlay configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WatermarkOverlayConfig {
    pub enabled: bool,
    pub path: Option<PathBuf>,
    pub anchor: OverlayAnchor,
    pub scale: f32,
    pub opacity: f32,
}

impl Default for WatermarkOverlayConfig {
    fn default() -> Self {
        Self { enabled: false, path: None, anchor: OverlayAnchor::BottomRight, scale: 0.2, opacity: 0.85 }
    }
}

/// Dailies text and name overlay configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NameOverlayConfig {
    pub enabled: bool,
    pub prefix: String,
    pub suffix: String,
    pub use_source_name: bool,
    pub anchor: OverlayAnchor,
    pub opacity: f32,
}

impl Default for NameOverlayConfig {
    fn default() -> Self {
        Self { enabled: false, prefix: String::new(), suffix: String::new(), use_source_name: true, anchor: OverlayAnchor::TopLeft, opacity: 1.0 }
    }
}

/// Broadcast video limiter clipping ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoLimiterClipLevel {
    #[default]
    Ire100,
    Ire103,
}

/// Broadcast video limiter for broadcast compliance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoLimiterConfig {
    pub enabled: bool,
    pub clip_level: VideoLimiterClipLevel,
    pub soft_knee_percent: f32,
}

impl Default for VideoLimiterConfig {
    fn default() -> Self {
        Self { enabled: false, clip_level: VideoLimiterClipLevel::Ire100, soft_knee_percent: 5.0 }
    }
}

/// SDR conform tone mapping for HDR sources (HLG/PQ to Rec.709).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SdrConformConfig {
    pub enabled: bool,
    pub target_nits: f32,
}

/// Source cropping margins in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CropConfig {
    pub enabled: bool,
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// Audio encoder settings of a target.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioEncodeSettings {
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub layout: Option<ChannelLayout>,
    pub bitrate_kbps: Option<u32>,
    /// Bit depth for uncompressed PCM or lossless formats (16, 24, 32).
    #[serde(default)]
    pub bit_depth: Option<u8>,
    /// Delay or advance applied to the audio stream in milliseconds (lip-sync compensation).
    #[serde(default)]
    pub delay_ms: i32,
}

const fn default_true() -> bool {
    true
}

/// Output target specifications.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputTarget {
    pub target_id: String,
    pub output_path: PathBuf,
    pub preset_name: String,
    pub container: ContainerType,
    /// Master toggle for video stream export.
    #[serde(default = "default_true")]
    pub export_video: bool,
    /// Master toggle for audio stream export.
    #[serde(default = "default_true")]
    pub export_audio: bool,
    #[serde(default)]
    pub custom_width: Option<u32>,
    #[serde(default)]
    pub custom_height: Option<u32>,
    #[serde(default)]
    pub custom_frame_rate: Option<Rational64>,
    #[serde(default)]
    pub target_bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub target_lufs: Option<f32>,
    #[serde(default)]
    pub lut_path: Option<PathBuf>,
    #[serde(default)]
    pub burn_timecode: bool,
    #[serde(default)]
    pub caption_path: Option<PathBuf>,
    #[serde(default)]
    pub caption_format: Option<CaptionExportFormat>,
    #[serde(default)]
    pub burn_captions: bool,
    #[serde(default)]
    pub caption_offset_ms: Option<i64>,
    #[serde(default)]
    pub audio_stems: Option<AudioStemMode>,
    #[serde(default)]
    pub metadata: Option<XmpMetadata>,
    #[serde(default)]
    pub hardware_acceleration: bool,
    /// Still-image quality 1..=100 (JPEG and lossy WebP); `None` uses the encoder default.
    #[serde(default)]
    pub image_quality: Option<u8>,
    /// Sequence, page range or layers of the source to export.
    #[serde(default)]
    pub selection: SourceSelection,
    /// Codec, GOP, B-frame, rate-control and colour settings of the video stream.
    #[serde(default)]
    pub video: VideoEncodeSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioEncodeSettings>,
    /// Scale mode, rotation and frame-rate conversion.
    #[serde(default)]
    pub transform: VideoTransform,
    /// Burn-in SMPTE timecode overlay configuration.
    #[serde(default)]
    pub timecode_overlay: Option<TimecodeOverlayConfig>,
    /// Image watermark overlay configuration.
    #[serde(default)]
    pub watermark: Option<WatermarkOverlayConfig>,
    /// Dailies name & text overlay configuration.
    #[serde(default)]
    pub name_overlay: Option<NameOverlayConfig>,
    /// Broadcast video limiter configuration.
    #[serde(default)]
    pub video_limiter: Option<VideoLimiterConfig>,
    /// SDR conform tone mapping for HDR sources.
    #[serde(default)]
    pub sdr_conform: Option<SdrConformConfig>,
    /// Source frame crop margins.
    #[serde(default)]
    pub crop: Option<CropConfig>,
    /// Multichannel matrix audio routing and track mapping.
    #[serde(default)]
    pub audio_mapping: Option<AudioChannelMapping>,
    /// Optional ICC color profile path applied via `craft-cms` during pixel conversion.
    #[serde(default)]
    pub icc_profile_path: Option<PathBuf>,
    /// Whether to apply ISO 21496-1 / HDR gain-map tone mapping.
    #[serde(default)]
    pub apply_gainmap: bool,
    /// Target display HDR headroom (in stops, e.g. `0.0` = SDR, `1.0..=4.0` = HDR) when `apply_gainmap` is enabled.
    #[serde(default)]
    pub gainmap_headroom: Option<f32>,
}

impl OutputTarget {
    /// Returns true if video stream export is active and supported by the container.
    pub fn is_video_enabled(&self) -> bool {
        self.export_video && self.container.supports_video()
    }

    /// Returns true if audio stream export is active and supported by the container.
    pub fn is_audio_enabled(&self) -> bool {
        self.export_audio && self.container.supports_audio()
    }
}

impl Default for OutputTarget {
    fn default() -> Self {
        Self {
            target_id: String::new(),
            output_path: PathBuf::new(),
            preset_name: String::new(),
            container: ContainerType::IsoBmffMp4,
            export_video: true,
            export_audio: true,
            custom_width: None,
            custom_height: None,
            custom_frame_rate: None,
            target_bitrate_kbps: None,
            target_lufs: None,
            lut_path: None,
            burn_timecode: false,
            caption_path: None,
            caption_format: None,
            burn_captions: false,
            caption_offset_ms: None,
            audio_stems: None,
            metadata: None,
            hardware_acceleration: false,
            image_quality: None,
            selection: SourceSelection::default(),
            video: VideoEncodeSettings::default(),
            audio: None,
            transform: VideoTransform::default(),
            timecode_overlay: None,
            watermark: None,
            name_overlay: None,
            video_limiter: None,
            sdr_conform: None,
            crop: None,
            audio_mapping: None,
            icc_profile_path: None,
            apply_gainmap: false,
            gainmap_headroom: None,
        }
    }
}

/// Complete, self-contained transcode job specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CraftJobSpec {
    pub version: String,
    pub job_id: String,
    pub client_info: ClientInfo,
    #[serde(default)]
    pub priority: JobPriority,
    #[serde(default)]
    pub start_policy: StartPolicy,
    pub source: SourceDescriptor,
    pub targets: Vec<OutputTarget>,
}

/// Major version of the job schema this crate reads and writes.
const SUPPORTED_SCHEMA_MAJOR: &str = "1";

impl SourceDescriptor {
    /// Checks the frame rates and timebases the source carries.
    pub fn validate(&self) -> Result<(), DispatchError> {
        match self {
            Self::Pipe { frame_rate, .. } => frame_rate.validate_positive("frameRate"),
            Self::File { timebase: Some(timebase), .. } => timebase.validate_positive("timebase"),
            _ => Ok(()),
        }
    }
}

impl CraftJobSpec {
    /// Validates the internal consistency of the job specification.
    pub fn validate(&self) -> Result<(), DispatchError> {
        if self.job_id.trim().is_empty() {
            return Err(DispatchError::InvalidJobSpec("jobId cannot be empty".into()));
        }
        if self.version.split('.').next() != Some(SUPPORTED_SCHEMA_MAJOR) {
            return Err(DispatchError::InvalidJobSpec(format!(
                "unsupported schema version `{}` (expected {SUPPORTED_SCHEMA_MAJOR}.x)",
                self.version
            )));
        }
        self.source.validate()?;
        for target in &self.targets {
            target.selection.validate()?;
        }

        if self.targets.is_empty() {
            return Err(DispatchError::InvalidJobSpec("job must define at least one output target".into()));
        }

        for (idx, target) in self.targets.iter().enumerate() {
            if target.target_id.trim().is_empty() {
                return Err(DispatchError::InvalidJobSpec(format!("target at index {idx} has empty targetId")));
            }
            if target.output_path.as_os_str().is_empty() {
                return Err(DispatchError::InvalidJobSpec(format!("target `{}` has empty outputPath", target.target_id)));
            }
            if target.image_quality.is_some_and(|q| !(1..=100).contains(&q)) {
                return Err(DispatchError::InvalidJobSpec(format!("target `{}` has imageQuality outside 1..=100", target.target_id)));
            }
            if let Some(rate) = target.custom_frame_rate {
                rate.validate_positive("customFrameRate")?;
            }
        }

        Ok(())
    }

    /// Serializes this job specification to a JSON string.
    pub fn to_json(&self) -> Result<String, DispatchError> {
        serde_json::to_string_pretty(self).map_err(|e| DispatchError::Json(format!("JSON serialization failed: {e}")))
    }

    /// Deserializes a job specification from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self, DispatchError> {
        let spec: Self = serde_json::from_str(json_str).map_err(|e| DispatchError::Json(format!("JSON deserialization failed: {e}")))?;
        spec.validate()?;
        Ok(spec)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ClientInfo, ContainerType, CraftJobSpec, CropConfig, JobPriority, NameOverlayConfig, OutputTarget, OverlayAnchor, Rational64,
        SdrConformConfig, SourceDescriptor, SourceSelection, StartPolicy, TimecodeOverlayConfig, VideoLimiterClipLevel, VideoLimiterConfig,
        WatermarkOverlayConfig,
    };

    #[test]
    fn test_job_spec_json_roundtrip() {
        let spec = CraftJobSpec {
            version: "1.0.0".into(),
            job_id: "test_job_001".into(),
            client_info: ClientInfo { app_name: "FilmCraft".into(), app_version: "0.9.4".into(), pid: 1234, user_notes: Some("Test export".into()) },
            priority: JobPriority::Normal,
            start_policy: StartPolicy::AutoStart,
            source: SourceDescriptor::File { path: PathBuf::from("/media/test.mov"), start_tick: None, duration_ticks: None, timebase: None },
            targets: vec![OutputTarget {
                target_id: "tgt_01".into(),
                output_path: PathBuf::from("/media/out.mp4"),
                preset_name: "YouTube 4K".into(),
                container: ContainerType::IsoBmffMp4,
                custom_width: Some(3840),
                custom_height: Some(2160),
                custom_frame_rate: Some(Rational64::FPS_60),
                target_bitrate_kbps: Some(45000),
                target_lufs: Some(-14.0),
                lut_path: None,
                burn_timecode: false,
                ..Default::default()
            }],
        };

        let json = spec.to_json().unwrap();
        assert!(!json.contains("\"audio\""), "targets without audio settings should preserve the legacy JSON shape");
        let deserialized = CraftJobSpec::from_json(&json).unwrap();
        assert_eq!(spec, deserialized);
    }

    #[test]
    fn test_effects_and_crop_json_roundtrip() {
        let target = OutputTarget {
            target_id: "tgt_effects".into(),
            output_path: PathBuf::from("/media/out_effects.mp4"),
            preset_name: "YouTube 1080p Dailies".into(),
            container: ContainerType::IsoBmffMp4,
            timecode_overlay: Some(TimecodeOverlayConfig {
                enabled: true,
                anchor: OverlayAnchor::BottomRight,
                scale: 3,
                text_opacity: 0.9,
                bg_opacity: 0.5,
                starting_timecode: Some("01:00:00:00".into()),
            }),
            watermark: Some(WatermarkOverlayConfig {
                enabled: true,
                path: Some(PathBuf::from("/branding/logo.png")),
                anchor: OverlayAnchor::TopRight,
                scale: 0.15,
                opacity: 0.75,
            }),
            name_overlay: Some(NameOverlayConfig {
                enabled: true,
                prefix: "SCENE 12 - ".into(),
                suffix: " - TAKE 1".into(),
                use_source_name: true,
                anchor: OverlayAnchor::TopLeft,
                opacity: 0.95,
            }),
            video_limiter: Some(VideoLimiterConfig { enabled: true, clip_level: VideoLimiterClipLevel::Ire103, soft_knee_percent: 4.5 }),
            sdr_conform: Some(SdrConformConfig { enabled: true, target_nits: 100.0 }),
            crop: Some(CropConfig { enabled: true, left: 140, top: 0, right: 140, bottom: 0 }),
            ..Default::default()
        };

        let spec = CraftJobSpec {
            version: "1.0".into(),
            job_id: "test_effects_job".into(),
            client_info: ClientInfo { app_name: "FilmCraft".into(), app_version: "1.0.0".into(), pid: 42, user_notes: None },
            priority: JobPriority::High,
            start_policy: StartPolicy::AutoStart,
            source: SourceDescriptor::File { path: PathBuf::from("/media/raw.mov"), start_tick: None, duration_ticks: None, timebase: None },
            targets: vec![target],
        };

        let json = spec.to_json().unwrap();
        let deserialized = CraftJobSpec::from_json(&json).unwrap();
        assert_eq!(spec, deserialized);
    }

    #[test]
    fn test_container_from_extension_and_conversion_rules() {
        assert_eq!(ContainerType::from_extension(".PSD"), Some(ContainerType::AdobePhotoshopPsd));
        assert_eq!(ContainerType::from_extension("aiff"), Some(ContainerType::Aiff));
        assert_eq!(ContainerType::from_extension("mxf"), Some(ContainerType::MxfOp1a));
        assert_eq!(ContainerType::from_extension("png"), Some(ContainerType::Png));
        assert_eq!(ContainerType::from_extension("epub"), Some(ContainerType::Epub));
        assert_eq!(ContainerType::from_extension("pptx"), Some(ContainerType::Pptx));
        assert_eq!(ContainerType::from_extension("lottie"), Some(ContainerType::LottieJson));
        assert_eq!(ContainerType::from_extension("wmf"), Some(ContainerType::WindowsMetafile));
        assert_eq!(ContainerType::from_extension("emf"), Some(ContainerType::WindowsMetafile));
        assert_eq!(ContainerType::from_extension("nope"), None);
        for c in ContainerType::ALL {
            if !matches!(c, ContainerType::MxfOpAtom | ContainerType::StillSequence) {
                assert_eq!(ContainerType::from_extension(c.file_extension()), Some(*c), "{c:?}");
            }
        }
        assert!(ContainerType::AdobeInDesignIndd.can_convert_to(ContainerType::Pdf));
        assert!(!ContainerType::AdobeInDesignIndd.can_convert_to(ContainerType::Wave));
        assert!(!ContainerType::AutocadDwg.can_convert_to(ContainerType::IsoBmffMp4));
        assert!(ContainerType::AdobeAuditionSesx.can_convert_to(ContainerType::ArtcraftSoundcraft));
        assert!(ContainerType::ArtcraftSoundcraft.can_convert_to(ContainerType::AdobeAuditionSesx));
    }

    #[test]
    fn test_source_selection_validation_and_page_parsing() {
        assert_eq!(SourceSelection::parse_pages("3").ok(), Some((3, 3)));
        assert_eq!(SourceSelection::parse_pages("2-5").ok(), Some((2, 5)));
        for bad in ["0", "5-2", "a-b", "", "1-", "-3"] {
            assert!(SourceSelection::parse_pages(bad).is_err(), "{bad}");
        }
        assert!(SourceSelection::default().is_empty());
        assert!(SourceSelection { pages: Some((0, 1)), ..Default::default() }.validate().is_err());
        assert!(SourceSelection { layers: Some(vec![]), ..Default::default() }.validate().is_err());
        assert!(SourceSelection { sequence: Some(" ".into()), ..Default::default() }.validate().is_err());
        assert!(SourceSelection { sequence: Some("Main".into()), pages: Some((1, 2)), layers: Some(vec!["a".into()]) }.validate().is_ok());
        let json = serde_json::to_string(&OutputTarget::default()).unwrap_or_default();
        let back: OutputTarget =
            serde_json::from_str(&json.replace(",\"selection\":{\"sequence\":null,\"pages\":null,\"layers\":null}", "")).unwrap();
        assert!(back.selection.is_empty(), "selection must default when absent from older job files");
    }

    #[test]
    fn test_unknown_fields_are_tolerated_for_forward_compatibility() {
        let json = r#"{
            "version": "1.1.0",
            "jobId": "job_future",
            "futureTopLevelField": true,
            "clientInfo": {
                "appName": "FilmCraft",
                "appVersion": "1.2.0",
                "pid": 99,
                "futureClientField": "ignored"
            },
            "source": {
                "type": "file",
                "path": "/media/clip.mov",
                "futureSourceField": 42
            },
            "targets": [{
                "targetId": "t1",
                "outputPath": "/media/out.mp4",
                "presetName": "Web 1080p",
                "container": "IsoBmffMp4",
                "futureTargetField": [1, 2, 3]
            }]
        }"#;
        let spec = CraftJobSpec::from_json(json).unwrap();
        assert_eq!(spec.job_id, "job_future");
        assert_eq!(spec.targets.len(), 1);
    }

    fn minimal_spec(source: &str, extra_target: &str, version: &str) -> String {
        format!(
            r#"{{"version":"{version}","jobId":"j","clientInfo":{{"appName":"a","appVersion":"1","pid":1}},
            "source":{source},"targets":[{{"targetId":"t","outputPath":"/o.mp4","presetName":"p","container":"IsoBmffMp4"{extra_target}}}]}}"#
        )
    }

    #[test]
    fn test_source_fields_use_camel_case_keys() {
        let json = minimal_spec(r#"{"type":"file","path":"/a.mov","startTick":5,"durationTicks":10}"#, "", "1.0.0");
        let spec = CraftJobSpec::from_json(&json).unwrap();
        assert!(matches!(spec.source, SourceDescriptor::File { start_tick: Some(5), duration_ticks: Some(10), .. }));
        assert!(spec.to_json().unwrap().contains("startTick"));
    }

    #[test]
    fn test_validation_rejects_bad_version_quality_and_rates() {
        let file = r#"{"type":"file","path":"/a.mov"}"#;
        assert!(CraftJobSpec::from_json(&minimal_spec(file, "", "1.7.2")).is_ok());
        for version in ["", "2.0.0", "x"] {
            assert!(CraftJobSpec::from_json(&minimal_spec(file, "", version)).is_err(), "version {version:?}");
        }
        assert!(CraftJobSpec::from_json(&minimal_spec(file, r#","imageQuality":100"#, "1.0.0")).is_ok());
        for q in [0, 101] {
            assert!(CraftJobSpec::from_json(&minimal_spec(file, &format!(r#","imageQuality":{q}"#), "1.0.0")).is_err(), "quality {q}");
        }
        let pipe = r#"{"type":"pipe","endpoint":"e","width":1,"height":1,"frameRate":{"num":25,"den":0},"sampleRate":48000,"channels":2}"#;
        assert!(CraftJobSpec::from_json(&minimal_spec(pipe, "", "1.0.0")).is_err());
        assert!(CraftJobSpec::from_json(&minimal_spec(file, r#","customFrameRate":{"num":25,"den":0}"#, "1.0.0")).is_err());
    }

    #[test]
    fn test_animated_gif_offers_no_audio_extraction() {
        assert!(!ContainerType::AnimatedGif.can_convert_to(ContainerType::Wave));
        assert!(ContainerType::AnimatedGif.can_convert_to(ContainerType::IsoBmffMp4));
        assert!(ContainerType::IsoBmffMp4.can_convert_to(ContainerType::Wave));
        assert!(!super::PixelFormat::Yuv444p12.has_alpha());
    }
}
