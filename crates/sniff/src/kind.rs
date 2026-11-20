//! The kinds [`identify`](crate::identify) names, and what each one is.

/// The broad kind of content a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Family {
    /// Still or layered pictures.
    Image,
    /// Sound with no pictures.
    Audio,
    /// Moving pictures, with or without sound. Container formats that may hold only sound
    /// (Matroska, MP4, Ogg Theora) are named by what their signature promises.
    Video,
    /// 3D scenes and meshes.
    Model,
    /// Several files packed together.
    Archive,
    /// One compressed stream.
    Compressed,
    /// Machine code or bytecode a runtime loads.
    Executable,
    /// Structured stores: databases and binary property lists.
    Database,
    /// Page-description documents.
    Document,
}

/// A file format named by its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    // Images.
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
    Tiff,
    /// Windows icon (`.ico`).
    Ico,
    /// HEIF still images, HEVC-coded (`.heic`) or generic.
    Heif,
    Avif,
    /// JPEG XL, as a bare codestream or in its ISO BMFF container.
    Jxl,
    /// Photoshop document or large document (`.psd`, `.psb`).
    Psd,
    Qoi,
    OpenExr,
    // Audio.
    /// MPEG-1/2 Audio Layer III, with or without an ID3v2 tag.
    Mp3,
    /// AAC in ADTS framing.
    Aac,
    Flac,
    /// RIFF WAVE.
    Wav,
    /// AIFF or AIFF-C.
    Aiff,
    /// Ogg holding anything but Opus or Theora (usually Vorbis).
    Ogg,
    /// Ogg Opus.
    Opus,
    /// MPEG-4 audio (`.m4a`, `.m4b`).
    M4a,
    /// Standard MIDI file.
    Midi,
    /// AMR narrowband or wideband speech.
    Amr,
    // Video.
    /// MPEG-4 Part 14 and the ISO base media brands it shares.
    Mp4,
    /// QuickTime movie.
    QuickTime,
    /// 3GPP or 3GPP2 movie.
    ThreeGp,
    WebM,
    Matroska,
    /// Ogg holding Theora video.
    OggVideo,
    /// RIFF AVI.
    Avi,
    /// Flash video.
    Flv,
    /// MPEG program stream.
    MpegPs,
    /// MPEG transport stream (188-byte packets).
    MpegTs,
    // 3D models.
    /// Binary glTF.
    Glb,
    /// Binary FBX. ASCII FBX has no signature.
    Fbx,
    /// PLY, ASCII or binary.
    Ply,
    // Archives and compressed streams.
    Zip,
    /// POSIX or GNU tar.
    Tar,
    /// cpio, binary or ASCII ("newc", CRC and "odc").
    Cpio,
    SevenZip,
    Rar,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    // Executables.
    Elf,
    /// Portable Executable: a DOS header whose `e_lfanew` reaches a `PE\0\0` signature.
    Pe,
    /// A DOS `MZ` header with no PE signature in the bytes given (a DOS program, or a PE file
    /// whose header lies past them).
    DosExecutable,
    /// A single-architecture Mach-O file.
    MachO,
    /// A universal ("fat") Mach-O file.
    MachOFat,
    /// A Java class file. It shares `CAFEBABE` with universal Mach-O; see [`identify`](crate::identify).
    JavaClass,
    /// WebAssembly binary module.
    Wasm,
    /// Dalvik executable.
    Dex,
    // Databases.
    Sqlite,
    /// Apple binary property list.
    BinaryPlist,
    // Documents.
    Pdf,
}

impl Kind {
    /// Every kind, in the order [`identify`](crate::identify) documents them.
    pub const ALL: [Kind; 56] = [
        Kind::Png,
        Kind::Jpeg,
        Kind::Gif,
        Kind::WebP,
        Kind::Bmp,
        Kind::Tiff,
        Kind::Ico,
        Kind::Heif,
        Kind::Avif,
        Kind::Jxl,
        Kind::Psd,
        Kind::Qoi,
        Kind::OpenExr,
        Kind::Mp3,
        Kind::Aac,
        Kind::Flac,
        Kind::Wav,
        Kind::Aiff,
        Kind::Ogg,
        Kind::Opus,
        Kind::M4a,
        Kind::Midi,
        Kind::Amr,
        Kind::Mp4,
        Kind::QuickTime,
        Kind::ThreeGp,
        Kind::WebM,
        Kind::Matroska,
        Kind::OggVideo,
        Kind::Avi,
        Kind::Flv,
        Kind::MpegPs,
        Kind::MpegTs,
        Kind::Glb,
        Kind::Fbx,
        Kind::Ply,
        Kind::Zip,
        Kind::Tar,
        Kind::Cpio,
        Kind::SevenZip,
        Kind::Rar,
        Kind::Gzip,
        Kind::Bzip2,
        Kind::Xz,
        Kind::Zstd,
        Kind::Elf,
        Kind::Pe,
        Kind::DosExecutable,
        Kind::MachO,
        Kind::MachOFat,
        Kind::JavaClass,
        Kind::Wasm,
        Kind::Dex,
        Kind::Sqlite,
        Kind::BinaryPlist,
        Kind::Pdf,
    ];

    /// The media type to send or store with the file: the IANA registration where one exists,
    /// otherwise the name in common use (`audio/wav`, `video/x-matroska`, `model/fbx`).
    pub fn mime_type(self) -> &'static str {
        self.info().0
    }

    /// The usual file extension, lower case, without the leading dot.
    pub fn extension(self) -> &'static str {
        self.info().1
    }

    /// The broad kind of content.
    pub fn family(self) -> Family {
        self.info().2
    }

    fn info(self) -> (&'static str, &'static str, Family) {
        use Family::{Archive, Audio, Compressed, Database, Document, Executable, Image, Model, Video};
        match self {
            Kind::Png => ("image/png", "png", Image),
            Kind::Jpeg => ("image/jpeg", "jpg", Image),
            Kind::Gif => ("image/gif", "gif", Image),
            Kind::WebP => ("image/webp", "webp", Image),
            Kind::Bmp => ("image/bmp", "bmp", Image),
            Kind::Tiff => ("image/tiff", "tif", Image),
            Kind::Ico => ("image/vnd.microsoft.icon", "ico", Image),
            Kind::Heif => ("image/heif", "heic", Image),
            Kind::Avif => ("image/avif", "avif", Image),
            Kind::Jxl => ("image/jxl", "jxl", Image),
            Kind::Psd => ("image/vnd.adobe.photoshop", "psd", Image),
            Kind::Qoi => ("image/qoi", "qoi", Image),
            Kind::OpenExr => ("image/x-exr", "exr", Image),
            Kind::Mp3 => ("audio/mpeg", "mp3", Audio),
            Kind::Aac => ("audio/aac", "aac", Audio),
            Kind::Flac => ("audio/flac", "flac", Audio),
            Kind::Wav => ("audio/wav", "wav", Audio),
            Kind::Aiff => ("audio/aiff", "aiff", Audio),
            Kind::Ogg => ("audio/ogg", "ogg", Audio),
            Kind::Opus => ("audio/opus", "opus", Audio),
            Kind::M4a => ("audio/mp4", "m4a", Audio),
            Kind::Midi => ("audio/midi", "mid", Audio),
            Kind::Amr => ("audio/amr", "amr", Audio),
            Kind::Mp4 => ("video/mp4", "mp4", Video),
            Kind::QuickTime => ("video/quicktime", "mov", Video),
            Kind::ThreeGp => ("video/3gpp", "3gp", Video),
            Kind::WebM => ("video/webm", "webm", Video),
            Kind::Matroska => ("video/x-matroska", "mkv", Video),
            Kind::OggVideo => ("video/ogg", "ogv", Video),
            Kind::Avi => ("video/x-msvideo", "avi", Video),
            Kind::Flv => ("video/x-flv", "flv", Video),
            Kind::MpegPs => ("video/mpeg", "mpg", Video),
            Kind::MpegTs => ("video/mp2t", "ts", Video),
            Kind::Glb => ("model/gltf-binary", "glb", Model),
            Kind::Fbx => ("model/fbx", "fbx", Model),
            Kind::Ply => ("model/ply", "ply", Model),
            Kind::Zip => ("application/zip", "zip", Archive),
            Kind::Tar => ("application/x-tar", "tar", Archive),
            Kind::Cpio => ("application/x-cpio", "cpio", Archive),
            Kind::SevenZip => ("application/x-7z-compressed", "7z", Archive),
            Kind::Rar => ("application/vnd.rar", "rar", Archive),
            Kind::Gzip => ("application/gzip", "gz", Compressed),
            Kind::Bzip2 => ("application/x-bzip2", "bz2", Compressed),
            Kind::Xz => ("application/x-xz", "xz", Compressed),
            Kind::Zstd => ("application/zstd", "zst", Compressed),
            Kind::Elf => ("application/x-elf", "elf", Executable),
            Kind::Pe => ("application/vnd.microsoft.portable-executable", "exe", Executable),
            Kind::DosExecutable => ("application/x-msdownload", "exe", Executable),
            Kind::MachO | Kind::MachOFat => ("application/x-mach-binary", "macho", Executable),
            Kind::JavaClass => ("application/java-vm", "class", Executable),
            Kind::Wasm => ("application/wasm", "wasm", Executable),
            Kind::Dex => ("application/vnd.android.dex", "dex", Executable),
            Kind::Sqlite => ("application/vnd.sqlite3", "sqlite", Database),
            Kind::BinaryPlist => ("application/x-bplist", "plist", Database),
            Kind::Pdf => ("application/pdf", "pdf", Document),
        }
    }
}
