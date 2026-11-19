//! Sony's in-file lens-correction records (shading, lateral chromatic aberration, distortion): bounded decoding of
//! the raw image IFD's fields and their encrypted `SR2SubIFD` copies into [`SonyLensCorrections`], and validated
//! ILCE-7RM4A, ILCE-7M3 and ILCE-7CR distortion tables into DNG warps for the shared optics pipeline.
//!
//! Tag identity/layout: https://exiftool.org/TagNames/EXIF.html (`0x7031` to `0x7037`, signed shorts) and
//! https://exiftool.org/TagNames/Sony.html#SR2SubIFD (`0x797d`, `0x7980`, `0x7982`). The `SR2SubIFD` setting tags
//! `0x797c`, `0x797f` and `0x7981` sit next to those tables, unnamed there; they equal the raw IFD's settings in every
//! file inspected. Geometry inferred independently from files and Sony Imaging Edge exports; see
//! docs/sony-lens-corrections.md. No lens database and no external decoder code. The MakerNote's similarly named
//! tables have another scale and layout and are never read here.
//!
//! A raw-IFD field is the one the TIFF reader parsed, the copy the image itself uses (its first occurrence, within
//! the reader's limits). The directory is also scanned, for diagnostics only: whether a tag is repeated, and why an
//! entry the reader left out was dropped (values outside the file, an unknown type, or past the reader's limits).
//! `SR2SubIFD` fields are read straight from their first entry. Only the exact layouts are accepted: SHORT\[1\] (or
//! nonnegative SSHORT\[1\]) settings, SSHORT\[17\] headed by 16 for shading and distortion, SSHORT\[33\] headed by
//! 32 for CA. Nothing is allocated from a count. The raw IFD's copy wins; the `SR2SubIFD`'s is used only when the raw
//! IFD lacks the tag, and a malformed raw-IFD copy is reported, not replaced. Decoding grants nothing: only components
//! whose interpretation was independently validated for the model and mode are converted ([`SonyLensStatus`]), and
//! the conversion receives exactly the table the decoder applied before this record existed.

use crate::lens_metadata::{
    SonyComponent, SonyCopy, SonyField, SonyFieldError, SonyLensCorrections, SonyLensGeometry, SonyLensStatus, SonySetting, SonySettingMeaning,
    SonySource,
};
use crate::sr2::SubIfd;
use crate::{Opcode, Rect};
use craft_tiff::{ByteOrder, FieldType, Ifd, Value};

/// Each component's setting and table tags, `(raw IFD, SR2SubIFD)`.
const SHADING: Tags = Tags { setting: (0x7031, 0x797c), table: (0x7032, 0x797d) };
const CA: Tags = Tags { setting: (0x7034, 0x797f), table: (0x7035, 0x7980) };
const DISTORTION: Tags = Tags { setting: (0x7036, 0x7981), table: (0x7037, 0x7982) };
/// TIFF field types.
const SHORT: u16 = 3;
const SSHORT: u16 = 8;
/// Models using the validated A7R IVA conversion (ILCE-7RM4A, ILCE-7M3). The A7CR uses a separate conversion.
/// ILCE-7RM4 and ILCE-7RM4A are distinct models, not interchangeable aliases.
const DISTORTION_MODELS: &[&str] = &["ILCE-7RM4A", "ILCE-7M3"];
/// Most directory entries scanned: a classic TIFF directory's largest count, also bounding a BigTIFF one.
const MAX_ENTRIES: usize = 1 << 16;
const SAMPLES: usize = 1024;
const MAX_ERROR: f64 = 0.0005;
/// Empirical A7CR unit, fixed on four FE 24-50mm files before checking the other files and lenses.
/// The camera-JPEG measurements and remaining coverage limits are in docs/sony-lens-corrections.md.
const A7CR_UNIT: f64 = 15559.300457825264;
const A7CR_FRAME: (usize, usize) = (9504, 6336);
const A7CR_APPROXIMATION_PX: f64 = 0.25;

#[derive(Clone, Copy)]
struct Tags {
    setting: (u16, u16),
    table: (u16, u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Shading,
    Ca,
    Distortion,
}

/// The documented meaning of a setting value (ExifTool EXIF tag-name documentation), compared whole.
fn meaning(kind: Kind, value: u16) -> SonySettingMeaning {
    use Kind::{Ca, Distortion, Shading};
    use SonySettingMeaning::*;
    match (kind, value) {
        (Shading, 256) | (Ca | Distortion, 0) => Off,
        (Shading, 257 | 272) | (Ca | Distortion, 1) => Auto,
        (Distortion, 17) => ForcedAuto,
        (Shading, 511) | (Ca | Distortion, 255) => Unavailable,
        _ => Unknown(value),
    }
}

/// One copy of a field.
#[derive(Clone, Debug, PartialEq)]
enum Read<T> {
    Absent,
    Valid(T),
    Rejected(SonyFieldError),
}

impl<T> Read<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Read<U> {
        match self {
            Read::Absent => Read::Absent,
            Read::Valid(v) => Read::Valid(f(v)),
            Read::Rejected(e) => Read::Rejected(e),
        }
    }
    fn copy(&self) -> SonyCopy {
        match self {
            Read::Absent => SonyCopy::Absent,
            Read::Valid(_) => SonyCopy::Valid,
            Read::Rejected(e) => SonyCopy::Rejected(*e),
        }
    }
}

/// One copy of a field, and whether its directory holds the tag more than once.
struct Found<T> {
    read: Read<T>,
    repeated: bool,
}

impl<T> Found<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Found<U> {
        Found { read: self.read.map(f), repeated: self.repeated }
    }
}

/// A TIFF directory whose out-of-line values sit at `offset - base` in `data`: the raw IFD in the file (`base` 0), or
/// the decrypted `SR2SubIFD`, whose offsets are file offsets into the block itself.
struct Directory<'a> {
    /// From the entry count on.
    entries: &'a [u8],
    data: &'a [u8],
    base: usize,
    order: ByteOrder,
    /// BigTIFF layout: 8-byte entry count, 20-byte entries with an 8-byte count and value field.
    big: bool,
}

impl<'a> Directory<'a> {
    /// The first entry of `tag`, and whether another follows. Only the first counts, as in the TIFF reader and
    /// `sr2.rs`. Reads at most the entries the data holds, and at most [`MAX_ENTRIES`].
    fn entry(&self, tag: u16) -> (Option<&'a [u8]>, bool) {
        let o = self.order;
        let (n, head, size) = if self.big { (o.read_u64(self.entries, 0), 8, 20) } else { (o.read_u16(self.entries, 0).map(u64::from), 2, 12) };
        let Some(n) = n else { return (None, false) };
        let mut found = None;
        for i in 0..usize::try_from(n).unwrap_or(usize::MAX).min(MAX_ENTRIES) {
            let at = i.checked_mul(size).and_then(|at| at.checked_add(head));
            let Some(e) = at.and_then(|at| self.entries.get(at..at.checked_add(size)?)) else { break };
            if o.read_u16(e, 0) == Some(tag) {
                if found.is_some() {
                    return (found, true);
                }
                found = Some(e);
            }
        }
        (found, false)
    }

    /// Field type and count of entry `e`.
    fn field(&self, e: &[u8]) -> (u16, u64) {
        let o = self.order;
        let count = if self.big { o.read_u64(e, 4) } else { o.read_u32(e, 4).map(u64::from) };
        (o.read_u16(e, 2).unwrap_or(0), count.unwrap_or(u64::MAX))
    }

    /// `len` value bytes of entry `e`: inline when they fit the value field, else at the entry's offset.
    fn values(&self, e: &'a [u8], len: usize) -> Option<&'a [u8]> {
        let (field, width) = if self.big { (12, 8) } else { (8, 4) };
        if len <= width {
            return e.get(field..field + len);
        }
        let offset = if self.big { self.order.read_u64(e, field)? } else { u64::from(self.order.read_u32(e, field)?) };
        let at = usize::try_from(offset).ok()?.checked_sub(self.base)?;
        self.data.get(at..at.checked_add(len)?)
    }

    /// Why the TIFF reader left out entry `e`, which the directory holds: a field type it doesn't know, values outside
    /// the data, or else its own limits (the entries it reads per directory, its value budget).
    fn unread(&self, e: &'a [u8]) -> SonyFieldError {
        let (kind, count) = self.field(e);
        let Some(size) = FieldType::from_u16(kind).map(FieldType::size) else { return SonyFieldError::FieldType(kind) };
        match count.checked_mul(size as u64).and_then(|len| usize::try_from(len).ok()).and_then(|len| self.values(e, len)) {
            Some(_) => SonyFieldError::PastReaderLimits,
            None => SonyFieldError::OutsideData,
        }
    }

    /// A radial table of exactly `N` samples in entry `e`: SSHORT\[N + 1\], the first value `N`.
    fn table<const N: usize>(&self, e: &'a [u8]) -> Read<[i16; N]> {
        let (kind, count) = self.field(e);
        if kind != SSHORT {
            return Read::Rejected(SonyFieldError::FieldType(kind));
        }
        if count != (N + 1) as u64 {
            return Read::Rejected(SonyFieldError::Count(count));
        }
        let Some(bytes) = self.values(e, 2 * (N + 1)) else { return Read::Rejected(SonyFieldError::OutsideData) };
        let at = |i: usize| self.order.read_u16(bytes, 2 * i).map_or(0, |v| v as i16);
        let prefix = at(0);
        if usize::try_from(prefix).ok() != Some(N) {
            return Read::Rejected(SonyFieldError::CountPrefix(prefix));
        }
        Read::Valid(std::array::from_fn(|i| at(i + 1)))
    }

    /// A setting in entry `e`: SHORT\[1\], or SSHORT\[1\] when not negative.
    fn setting(&self, e: &'a [u8], kind: Kind) -> Read<SonySetting> {
        let (field_type, count) = self.field(e);
        if field_type != SHORT && field_type != SSHORT {
            return Read::Rejected(SonyFieldError::FieldType(field_type));
        }
        if count != 1 {
            return Read::Rejected(SonyFieldError::Count(count));
        }
        let Some(value) = self.values(e, 2).and_then(|b| self.order.read_u16(b, 0)) else {
            return Read::Rejected(SonyFieldError::OutsideData);
        };
        if field_type == SSHORT && (value as i16) < 0 {
            return Read::Rejected(SonyFieldError::NegativeSetting(value as i16));
        }
        Read::Valid(SonySetting { value, meaning: meaning(kind, value) })
    }
}

/// A table as the TIFF reader parsed it: SSHORT\[N + 1\], the first value `N`.
fn parsed_table<const N: usize>(value: &Value) -> Read<[i16; N]> {
    let Value::SShort(v) = value else { return Read::Rejected(SonyFieldError::FieldType(value.field_type() as u16)) };
    if v.len() != N + 1 {
        return Read::Rejected(SonyFieldError::Count(v.len() as u64));
    }
    let at = |i: usize| v.get(i).copied().unwrap_or(0);
    if usize::try_from(at(0)).ok() != Some(N) {
        return Read::Rejected(SonyFieldError::CountPrefix(at(0)));
    }
    Read::Valid(std::array::from_fn(|i| at(i + 1)))
}

/// A setting as the TIFF reader parsed it: SHORT\[1\], or SSHORT\[1\] when not negative (the documented and the stored
/// type differ).
fn parsed_setting(value: &Value, kind: Kind) -> Read<SonySetting> {
    let number = match value {
        Value::Short(v) if v.len() != 1 => return Read::Rejected(SonyFieldError::Count(v.len() as u64)),
        Value::SShort(v) if v.len() != 1 => return Read::Rejected(SonyFieldError::Count(v.len() as u64)),
        Value::Short(v) => v.first().copied(),
        Value::SShort(v) => match v.first().copied() {
            Some(n) if n < 0 => return Read::Rejected(SonyFieldError::NegativeSetting(n)),
            n => n.map(|n| n as u16),
        },
        other => return Read::Rejected(SonyFieldError::FieldType(other.field_type() as u16)),
    };
    number.map_or(Read::Rejected(SonyFieldError::Count(0)), |value| Read::Valid(SonySetting { value, meaning: meaning(kind, value) }))
}

/// The raw IFD's copy of `tag`: the one the TIFF reader parsed, which is what the image itself uses. The directory
/// itself only adds diagnostics: whether the tag is repeated, and why an entry the reader left out was dropped.
fn raw_copy<T>(ifd: &Ifd, dir: Option<&Directory<'_>>, tag: u16, read: impl Fn(&Value) -> Read<T>) -> Found<T> {
    let (entry, repeated) = dir.map_or((None, false), |d| d.entry(tag));
    let read = match (ifd.get(tag), entry.zip(dir)) {
        (Some(parsed), _) => read(&parsed.value),
        (None, Some((e, d))) => Read::Rejected(d.unread(e)),
        (None, None) => Read::Absent,
    };
    Found { read, repeated }
}

/// The `SR2SubIFD`'s copy of `tag`, read straight from its first entry.
fn sr2_copy<'a, T>(dir: Option<&Directory<'a>>, tag: u16, read: impl Fn(&Directory<'a>, &'a [u8]) -> Read<T>) -> Found<T> {
    let Some(d) = dir else { return Found { read: Read::Absent, repeated: false } };
    let (entry, repeated) = d.entry(tag);
    Found { read: entry.map_or(Read::Absent, |e| read(d, e)), repeated }
}

/// One field from its two copies: the raw IFD's when well-formed, else the `SR2SubIFD`'s when the raw IFD lacks the
/// tag.
fn combine<T: PartialEq>(raw: Found<T>, sr2: Found<T>) -> SonyField<T> {
    let (raw_ifd, sr2_copy) = (raw.read.copy(), sr2.read.copy());
    let copies_agree = match (&raw.read, &sr2.read) {
        (Read::Valid(a), Read::Valid(b)) => Some(a == b),
        _ => None,
    };
    let (value, source) = match (raw.read, sr2.read) {
        (Read::Valid(v), _) => (Some(v), Some(SonySource::RawIfd)),
        (Read::Absent, Read::Valid(v)) => (Some(v), Some(SonySource::Sr2SubIfd)),
        _ => (None, None),
    };
    SonyField { value, source, raw_ifd, sr2: sr2_copy, copies_agree, raw_ifd_repeated: raw.repeated, sr2_repeated: sr2.repeated }
}

/// Whether `r` has the native 3:2 framing, within 0.5 %.
pub(super) fn is_3_2(r: Rect) -> bool {
    r.height > 0 && (r.width as f64 / r.height as f64 - 1.5).abs() <= 0.005
}

/// Decode the file's record and the validated distortion warp. `raw` is the raw image IFD as the TIFF reader parsed
/// it (in a BigTIFF file when `big`), `sr2` the decrypted `SR2SubIFD`. The record is `None` when neither directory
/// holds any of the six fields.
pub(super) fn corrections(
    bytes: &[u8],
    order: ByteOrder,
    (raw, big): (&Ifd, bool),
    sr2: Option<&SubIfd>,
    model: &str,
    geometry: SonyLensGeometry,
) -> (Option<SonyLensCorrections>, Option<Opcode>) {
    let plain = usize::try_from(raw.offset).ok().and_then(|at| Some(Directory { entries: bytes.get(at..)?, data: bytes, base: 0, order, big }));
    let encrypted = sr2.map(|s| {
        let (block, start) = s.block();
        Directory { entries: block, data: block, base: start, order: s.order(), big: false }
    });
    let (p, s) = (plain.as_ref(), encrypted.as_ref());
    // CA: two contiguous 16-sample curves after the count
    let split = |v: [i16; 32]| -> [[i16; 16]; 2] { std::array::from_fn(|c| std::array::from_fn(|i| v[16 * c + i])) };
    let table16 = |tags: (u16, u16)| combine(raw_copy(raw, p, tags.0, parsed_table::<16>), sr2_copy(s, tags.1, |d, e| d.table::<16>(e)));
    let shading_table = table16(SHADING.table);
    let distortion_table = table16(DISTORTION.table);
    let ca_table = combine(raw_copy(raw, p, CA.table.0, parsed_table::<32>).map(split), sr2_copy(s, CA.table.1, |d, e| d.table::<32>(e)).map(split));
    let setting = |tags: Tags, kind: Kind| {
        combine(raw_copy(raw, p, tags.setting.0, |v| parsed_setting(v, kind)), sr2_copy(s, tags.setting.1, |d, e| d.setting(e, kind)))
    };
    let settings = [setting(SHADING, Kind::Shading), setting(CA, Kind::Ca), setting(DISTORTION, Kind::Distortion)];
    if !shading_table.is_present() && !ca_table.is_present() && !distortion_table.is_present() && !settings.iter().any(SonyField::is_present) {
        return (None, None);
    }
    let [shading_setting, ca_setting, distortion_setting] = settings;
    // Only the validated scope reaches the conversion: the raw IFD's table, as the TIFF reader parsed it, of a Bayer
    // image whose image crop and reference are native 3:2 (DefaultCrop can keep native geometry while Sony's tags
    // select an aspect crop).
    let a7cr = model == "ILCE-7CR";
    let validated_model = DISTORTION_MODELS.contains(&model) || a7cr;
    let in_mode = !geometry.linear_ycbcr
        && geometry.native_3_2
        && distortion_table.source == Some(SonySource::RawIfd)
        && (!a7cr || ((geometry.reference.width, geometry.reference.height) == A7CR_FRAME && geometry.image_crop == geometry.reference));
    let a7cr_auto = distortion_setting.value.is_some_and(|s| matches!(s.meaning, SonySettingMeaning::Auto | SonySettingMeaning::ForcedAuto));
    let (w, h) = (geometry.storage_width, geometry.storage_height);
    let warp = match &distortion_table.value {
        Some(table) if in_mode && a7cr && a7cr_auto => a7cr_distortion(table, Rect::new(0, 0, w, h), geometry.reference),
        Some(table) if in_mode && !a7cr => distortion(model, table, Rect::new(0, 0, w, h), geometry.reference),
        _ => None,
    };
    let distortion_status = match &distortion_table.value {
        None => SonyLensStatus::NoTable,
        Some(_) if warp.is_some() => SonyLensStatus::Applied,
        Some(_) if !validated_model => SonyLensStatus::NotValidatedModel,
        Some(_) if !in_mode => SonyLensStatus::NotValidatedMode,
        Some(_) if a7cr && !a7cr_auto => SonyLensStatus::CameraCorrectionDisabled,
        Some(table) if table.iter().all(|v| *v == 0) => SonyLensStatus::Neutral,
        Some(_) => SonyLensStatus::Rejected,
    };
    // No independent validation of shading or CA semantics exists on any body: decoded, never applied.
    let unvalidated = |present: bool| if present { SonyLensStatus::NotValidatedComponent } else { SonyLensStatus::NoTable };
    let record = SonyLensCorrections {
        shading: SonyComponent { status: unvalidated(shading_table.value.is_some()), setting: shading_setting, table: shading_table },
        chromatic_aberration: SonyComponent { status: unvalidated(ca_table.value.is_some()), setting: ca_setting, table: ca_table },
        distortion: SonyComponent { setting: distortion_setting, table: distortion_table, status: distortion_status },
        sr2_read: sr2.is_some(),
        geometry,
    };
    (Some(record), warp)
}

/// A7CR camera-JPEG convention, measured independently of the ILCE-7RM4A conversion below.
/// For output radius rho, the corrected radius is rho / zoom. The table gives source radius at that
/// corrected radius. Its knots use the full-frame diagonal, not storage padding. Off/unknown settings
/// and unvalidated capture modes are excluded by the caller.
fn a7cr_distortion(table: &[i16; 16], active: Rect, crop: Rect) -> Option<Opcode> {
    if (crop.width, crop.height) != A7CR_FRAME
        || crop != crop.clipped(active.width, active.height)
        || table.iter().all(|v| *v == 0)
        || table.iter().any(|v| !(-4096..=4096).contains(v))
    {
        return None;
    }
    let (cw, ch) = (crop.width as f64, crop.height as f64);
    let spacing = cw.hypot(ch) / 30.0;
    let source = |r: f64| {
        let at = r / spacing;
        let i = (at.floor() as usize).min(14);
        let (a, b) = (f64::from(table[i]), f64::from(table[i + 1]));
        r * (1.0 + (a + (b - a) * (at - i as f64)) / A7CR_UNIT)
    };
    let border_min = cw.min(ch) / 2.0;
    let border_max = cw.hypot(ch) / 2.0;
    let border_ratio = |zoom: f64| {
        (0..=4096)
            .map(|j| {
                let r = border_min + (border_max - border_min) * j as f64 / 4096.0;
                source(r / zoom) / r
            })
            .fold(0.0, f64::max)
    };
    let (mut lo, mut hi) = (0.5, 2.0);
    if border_ratio(lo) <= 1.0 || border_ratio(hi) > 1.0 {
        return None;
    }
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if border_ratio(mid) > 1.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let zoom = (lo + hi) / 2.0;
    let radius = (cw - 1.0).hypot(ch - 1.0) / 2.0;
    let basis = |r: f64| {
        let r2 = r * r;
        [r, r * r2, r * r2 * r2, r * r2 * r2 * r2]
    };
    let mut matrix = [[0.0; 4]; 4];
    let mut rhs = [0.0; 4];
    let mut previous = -1.0;
    for j in 0..=SAMPLES {
        let r = j as f64 / SAMPLES as f64;
        let mapped = source(r * radius / zoom) / radius;
        if !mapped.is_finite() || mapped <= previous {
            return None;
        }
        previous = mapped;
        let terms = basis(r);
        for (i, &a) in terms.iter().enumerate() {
            rhs[i] += a * mapped;
            for (k, &b) in terms.iter().enumerate() {
                matrix[i][k] += a * b;
            }
        }
    }
    let mut k = solve4(matrix, rhs)?;
    // Check every displayed radius, separately from the empirical camera-JPEG validation.
    for j in 0..=32768 {
        let r = j as f64 / 32768.0;
        let fit = basis(r).iter().zip(k).map(|(a, b)| a * b).sum::<f64>();
        let r2 = r * r;
        let derivative = k[0] + r2 * (3.0 * k[1] + r2 * (5.0 * k[2] + r2 * 7.0 * k[3]));
        let error_px = (fit * radius - source(r * radius / zoom)).abs() * 1440.0 / cw;
        if !error_px.is_finite() || error_px > A7CR_APPROXIMATION_PX || derivative <= 0.0 {
            return None;
        }
    }
    let (ax, ay) = (active.width as f64 - 1.0, active.height as f64 - 1.0);
    let (cx, cy) = (crop.x as f64 + (cw - 1.0) / 2.0, crop.y as f64 + (ch - 1.0) / 2.0);
    let farthest = cx.max(ax - cx).hypot(cy.max(ay - cy));
    let ratio2 = (farthest / radius).powi(2);
    for (i, coefficient) in k.iter_mut().enumerate() {
        *coefficient *= ratio2.powi(i as i32);
    }
    Some(Opcode::WarpRectilinear { planes: vec![[k[0], k[1], k[2], k[3], 0.0, 0.0]], center: [cx / ax, cy / ay] })
}

/// The validated conversion of a raw-IFD distortion table into a `WarpRectilinear`. Only independently validated
/// camera models and the 16-entry layout; unknown models and implausible or non-smooth maps are left uncorrected.
/// See docs/sony-lens-corrections.md. A camera setting of Off does not remove the lens data: the user can enable it
/// in the developer.
fn distortion(model: &str, table: &[i16; 16], active: Rect, crop: Rect) -> Option<Opcode> {
    // A matching table shape does not establish its scale or radial normalization on another body.
    if !DISTORTION_MODELS.contains(&model) {
        return None;
    }
    if table.iter().all(|v| *v == 0) || table.iter().any(|v| !(-4096..=4096).contains(v)) {
        return None;
    }
    let crop = crop.clipped(active.width, active.height);
    if active.width < 2 || active.height < 2 || crop.width < 2 || crop.height < 2 {
        return None;
    }
    // Only native 3:2 framing has been validated. An in-camera aspect crop may retain a table
    // normalized to the full sensor frame; do not silently reinterpret it as the smaller crop.
    if (crop.width as f64 / crop.height as f64 - 1.5).abs() > 0.005 {
        return None;
    }
    // Interpolate fractional radial scale at evenly spaced radii from centre to cropped corner.
    let scale = |r: f64| {
        let at = r.clamp(0.0, 1.0) * 15.0;
        let i = (at.floor() as usize).min(14);
        let a = f64::from(table[i]);
        let b = f64::from(table[i + 1]);
        1.0 + (a + (b - a) * (at - i as f64)) / 16384.0
    };
    let (cw, ch) = (crop.width as f64 - 1.0, crop.height as f64 - 1.0);
    let diagonal = cw.hypot(ch);
    // Zoom until the entire rectangular output boundary is inside the source. For a piecewise-linear
    // scale its maximum on that boundary occurs at its nearest radius or at one of the remaining knots.
    let nearest = cw.min(ch) / diagonal;
    let max_scale = (0..16).map(|i| i as f64 / 15.0).filter(|&r| r >= nearest).map(scale).fold(scale(nearest), f64::max);
    let zoom = 1.0 / max_scale;
    let basis = |r: f64| {
        let r2 = r * r;
        [r, r * r2, r * r2 * r2, r * r2 * r2 * r2]
    };
    let mut matrix = [[0.0; 4]; 4];
    let mut rhs = [0.0; 4];
    let mut previous = -1.0;
    for j in 0..=SAMPLES {
        let r = j as f64 / SAMPLES as f64;
        let source = r * scale(r) * zoom;
        if source <= previous {
            return None;
        }
        previous = source;
        let terms = basis(r);
        for (i, &a) in terms.iter().enumerate() {
            rhs[i] += a * source;
            for (k, &b) in terms.iter().enumerate() {
                matrix[i][k] += a * b;
            }
        }
    }
    let mut k = solve4(matrix, rhs)?;
    // Keep the polynomial within a fraction of a preview pixel of the file's table, and forbid folds.
    for j in 0..=SAMPLES {
        let r = j as f64 / SAMPLES as f64;
        let fit = basis(r).iter().zip(k).map(|(a, b)| a * b).sum::<f64>();
        let r2 = r * r;
        let derivative = k[0] + r2 * (3.0 * k[1] + r2 * (5.0 * k[2] + r2 * 7.0 * k[3]));
        if (fit - r * scale(r) * zoom).abs() > MAX_ERROR || derivative <= 0.0 {
            return None;
        }
    }
    // DNG uses the farthest *active-area* corner, not the cropped diagonal. Preserve the same mapping
    // after the engine translates the opcode into default-cropped and EXIF-oriented coordinates.
    let (ax, ay) = (active.width as f64 - 1.0, active.height as f64 - 1.0);
    let (cx, cy) = (crop.x as f64 + cw / 2.0, crop.y as f64 + ch / 2.0);
    let farthest = cx.max(ax - cx).hypot(cy.max(ay - cy));
    let ratio2 = (2.0 * farthest / diagonal).powi(2);
    for (i, coefficient) in k.iter_mut().enumerate() {
        *coefficient *= ratio2.powi(i as i32);
    }
    Some(Opcode::WarpRectilinear { planes: vec![[k[0], k[1], k[2], k[3], 0.0, 0.0]], center: [cx / ax, cy / ay] })
}

/// Small fixed-size least-squares system; partial pivoting, no input-sized allocations.
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    for col in 0..4 {
        let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if !a[pivot][col].is_finite() || a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..4 {
            let factor = a[row][col] / a[col][col];
            let (top, bottom) = a.split_at_mut(row);
            for (x, p) in bottom[0][col..].iter_mut().zip(&top[col][col..]) {
                *x -= factor * p;
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = [0.0; 4];
    for i in (0..4).rev() {
        x[i] = (b[i] - (i + 1..4).map(|j| a[i][j] * x[j]).sum::<f64>()) / a[i][i];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lens_metadata::SonyCopy::{Absent, Rejected, Valid};
    use crate::lens_metadata::SonyFieldError::*;
    use crate::{LensMetadata, Limits, Mode, RawImage};
    use craft_tiff::ByteOrder::{Big, Little};
    use craft_tiff::{IfdBuilder, ImageData, Tiff, TiffWriter, Value, tags as t};

    const TELE: [i16; 16] = [-2, 1, 7, 17, 31, 48, 68, 93, 120, 152, 187, 225, 265, 309, 356, 403];
    const WIDE: [i16; 16] = [14, 0, -27, -59, -106, -158, -223, -291, -367, -440, -519, -593, -667, -731, -792, -839];
    const AREA: Rect = Rect { x: 0, y: 0, width: 9504, height: 6336 };
    /// The shading samples of the public ILCE-7CR samples (raw.pixls.us DSC00795/6/8).
    const SHADING_T: [i16; 16] = [0, 64, 128, 224, 320, 480, 672, 896, 1184, 1472, 1824, 2240, 2816, 3552, 4512, 7328];
    const KEY: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
    /// Public storytold/lightcraft#1300, FE 16-25mm F2.8 G at 16 mm, raw-IFD samples.
    const A7CR_WIDE: [i16; 16] = [0, -14, -59, -130, -226, -341, -472, -615, -767, -924, -1084, -1242, -1397, -1549, -1696, -1839];

    /// A CA table with negative and positive samples and two different curves.
    fn ca() -> [i16; 32] {
        std::array::from_fn(|i| (i as i16 - 16) * 128)
    }
    fn ca_curves() -> [[i16; 16]; 2] {
        let v = ca();
        std::array::from_fn(|c| std::array::from_fn(|i| v[16 * c + i]))
    }
    fn sshort(prefix: i16, samples: &[i16]) -> Value {
        Value::SShort(std::iter::once(prefix).chain(samples.iter().copied()).collect())
    }
    /// The six raw-IFD fields: shading 272 (Auto), CA 1 (Auto), and the distortion `setting` and `table`.
    fn plain(table: &[i16; 16], setting: u16) -> Vec<(u16, Value)> {
        vec![
            (SHADING.setting.0, Value::Short(vec![272])),
            (SHADING.table.0, sshort(16, &SHADING_T)),
            (CA.setting.0, Value::Short(vec![1])),
            (CA.table.0, sshort(32, &ca())),
            (DISTORTION.setting.0, Value::Short(vec![setting])),
            (DISTORTION.table.0, sshort(16, table)),
        ]
    }
    /// The same fields as `SR2SubIFD` entries `(tag, type, values)`.
    fn encrypted(table: &[i16; 16], setting: u16) -> Vec<(u16, u16, Vec<u16>)> {
        let t = |prefix: i16, v: &[i16]| std::iter::once(prefix).chain(v.iter().copied()).map(|x| x as u16).collect::<Vec<u16>>();
        vec![
            (SHADING.setting.1, SHORT, vec![272]),
            (SHADING.table.1, SSHORT, t(16, &SHADING_T)),
            (CA.setting.1, SHORT, vec![1]),
            (CA.table.1, SSHORT, t(32, &ca())),
            (DISTORTION.setting.1, SHORT, vec![setting]),
            (DISTORTION.table.1, SSHORT, t(16, table)),
        ]
    }
    fn set(fields: &mut Vec<(u16, Value)>, tag: u16, value: Value) {
        fields.retain(|(t, _)| *t != tag);
        fields.push((tag, value));
    }
    fn word(order: ByteOrder, v: u32) -> Vec<u8> {
        let mut out = Vec::new();
        order.put_u32(&mut out, v);
        out
    }

    /// A small uncompressed Sony CFA file (the raw IFD is IFD0) with the lens fields `plain` in its raw IFD and, when
    /// `sr2` is set, an encrypted `SR2SubIFD` holding those entries, reached as Sony stores it: `DNGPrivateData` ->
    /// `SR2Private` IFD -> block.
    struct Arw {
        model: &'static str,
        order: ByteOrder,
        bigtiff: bool,
        size: (u32, u32),
        sony_crop: Option<[u32; 4]>,
        default_crop: Option<[u32; 4]>,
        plain: Vec<(u16, Value)>,
        sr2: Option<Vec<(u16, u16, Vec<u16>)>>,
        /// The key stored in `SR2SubIFDKey`; the block is always encrypted with the usual one.
        stored_key: [u8; 4],
        /// An edit to the decrypted `SR2SubIFD` before it is encrypted.
        #[allow(clippy::type_complexity)]
        sr2_patch: Option<Box<dyn Fn(&mut [u8])>>,
    }

    impl Arw {
        fn new(model: &'static str, order: ByteOrder) -> Arw {
            Arw {
                model,
                order,
                bigtiff: false,
                size: (32, 24),
                sony_crop: Some([2, 2, 30, 20]),
                default_crop: Some([2, 2, 30, 20]),
                plain: plain(&TELE, 0),
                sr2: Some(encrypted(&TELE, 0)),
                stored_key: KEY,
                sr2_patch: None,
            }
        }

        fn build(&self) -> Vec<u8> {
            let (w, h) = self.size;
            let mut raw = IfdBuilder::new();
            raw.set(t::MAKE, Value::Ascii("SONY".into()));
            raw.set(t::MODEL, Value::Ascii(self.model.into()));
            raw.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
            raw.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
            raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
            raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
            raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
            raw.set(t::COMPRESSION, Value::Short(vec![1]));
            raw.set(t::WHITE_LEVEL, Value::Long(vec![16383]));
            if let Some([x, y, cw, ch]) = self.sony_crop {
                raw.set(0x74c7, Value::Long(vec![x, y]));
                raw.set(0x74c8, Value::Long(vec![cw, ch]));
            }
            if let Some([x, y, cw, ch]) = self.default_crop {
                raw.set(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![x, y]));
                raw.set(t::DEFAULT_CROP_SIZE, Value::Long(vec![cw, ch]));
            }
            if self.sr2.is_some() {
                raw.set(t::DNG_PRIVATE_DATA, Value::Byte(vec![0; 4]));
            }
            for (tag, value) in &self.plain {
                raw.set(*tag, value.clone());
            }
            raw.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![vec![0; (w * h * 2) as usize]] });
            let mut file = TiffWriter::new(self.order, self.bigtiff).write(&[raw]).unwrap();
            let Some(entries) = &self.sr2 else { return file };
            let o = self.order;
            let at = Tiff::parse(&file).unwrap().ifds[0].get(t::DNG_PRIVATE_DATA).unwrap().offset as usize;
            let private = file.len();
            let start = private + 2 + 3 * 12 + 4;
            let refs: Vec<(u16, u16, &[u16])> = entries.iter().map(|(tag, kind, v)| (*tag, *kind, v.as_slice())).collect();
            let mut block = crate::sr2::tests::block(start, &refs, o);
            if let Some(patch) = &self.sr2_patch {
                crate::sr2::tests::encrypt(&mut block);
                patch(&mut block);
                crate::sr2::tests::encrypt(&mut block);
            }
            o.put_u16(&mut file, 3);
            for (tag, value) in [(0x7200u16, start as u32), (0x7201, block.len() as u32)] {
                o.put_u16(&mut file, tag);
                o.put_u16(&mut file, 4);
                o.put_u32(&mut file, 1);
                o.put_u32(&mut file, value);
            }
            o.put_u16(&mut file, 0x7221);
            o.put_u16(&mut file, 7);
            o.put_u32(&mut file, 4);
            file.extend(self.stored_key);
            o.put_u32(&mut file, 0);
            file.extend(block);
            file[at..at + 4].copy_from_slice(&word(o, private as u32));
            file
        }
    }

    /// Byte position of IFD0's entry for `tag` in a classic TIFF `file`.
    fn entry_at(file: &[u8], order: ByteOrder, tag: u16) -> usize {
        let ifd = order.read_u32(file, 4).unwrap() as usize;
        let n = usize::from(order.read_u16(file, ifd).unwrap());
        (0..n).map(|i| ifd + 2 + 12 * i).find(|&e| order.read_u16(file, e) == Some(tag)).unwrap()
    }

    /// Header and full decode of `file`, which must describe the same image; the full one.
    fn decode(file: &[u8]) -> RawImage {
        let header = crate::arw::decode(file, Mode::Header, &Limits::default()).unwrap();
        let full = crate::arw::decode(file, Mode::Full, &Limits::default()).unwrap();
        assert_eq!(header.info(), full.info());
        full
    }
    fn sony(img: &RawImage) -> &SonyLensCorrections {
        img.lens_metadata.sony.as_ref().expect("a Sony lens record")
    }
    /// (has a value, source, raw-IFD copy, SR2SubIFD copy, agreement) of one field.
    fn prov<T>(f: &SonyField<T>) -> (bool, Option<SonySource>, SonyCopy, SonyCopy, Option<bool>) {
        (f.value.is_some(), f.source, f.raw_ifd, f.sr2, f.copies_agree)
    }
    /// [`prov`] of the three settings, then the three tables.
    #[allow(clippy::type_complexity)]
    fn provenance(s: &SonyLensCorrections) -> [(bool, Option<SonySource>, SonyCopy, SonyCopy, Option<bool>); 6] {
        [
            prov(&s.shading.setting),
            prov(&s.chromatic_aberration.setting),
            prov(&s.distortion.setting),
            prov(&s.shading.table),
            prov(&s.chromatic_aberration.table),
            prov(&s.distortion.table),
        ]
    }

    #[test]
    fn decodes_all_three_components_with_settings_and_provenance() {
        let raw = Some(SonySource::RawIfd);
        for order in [Little, Big] {
            for bigtiff in [false, true] {
                for (model, warps, distortion_status) in
                    [("ILCE-7CR", 0, SonyLensStatus::NotValidatedMode), ("ILCE-7RM4A", 1, SonyLensStatus::Applied)]
                {
                    let case = format!("{model} {order:?} bigtiff={bigtiff}");
                    let mut arw = Arw::new(model, order);
                    arw.bigtiff = bigtiff;
                    let img = decode(&arw.build());
                    let s = sony(&img);
                    assert_eq!(img.opcodes.list3.len(), warps, "{case}");
                    assert!(s.sr2_read, "{case}");
                    assert_eq!(provenance(s), [(true, raw, Valid, Valid, Some(true)); 6], "{case}");
                    assert_eq!(s.shading.table.value, Some(SHADING_T), "{case}");
                    assert_eq!(s.chromatic_aberration.table.value, Some(ca_curves()), "{case}");
                    assert_eq!(s.distortion.table.value, Some(TELE), "{case}");
                    assert_eq!(s.shading.setting.value, Some(SonySetting { value: 272, meaning: SonySettingMeaning::Auto }));
                    assert_eq!(s.chromatic_aberration.setting.value, Some(SonySetting { value: 1, meaning: SonySettingMeaning::Auto }));
                    assert_eq!(s.distortion.setting.value, Some(SonySetting { value: 0, meaning: SonySettingMeaning::Off }));
                    // readable shading and CA tables are not corrections
                    assert_eq!(s.shading.status, SonyLensStatus::NotValidatedComponent, "{case}");
                    assert_eq!(s.chromatic_aberration.status, SonyLensStatus::NotValidatedComponent, "{case}");
                    assert_eq!(s.distortion.status, distortion_status, "{case}");
                    let crop = Rect::new(2, 2, 30, 20);
                    let geometry = SonyLensGeometry {
                        storage_width: 32,
                        storage_height: 24,
                        sony_crop: Some(crop),
                        default_crop: Some(crop),
                        image_crop: crop,
                        reference: crop,
                        compression: 1,
                        linear_ycbcr: false,
                        native_3_2: true,
                    };
                    assert_eq!(s.geometry, geometry, "{case}");
                    // the applied warp is the validated conversion of the raw IFD's table, as before
                    let reference: Vec<Opcode> = distortion(model, &TELE, Rect::new(0, 0, 32, 24), crop).into_iter().collect();
                    assert_eq!(img.opcodes.list3, reference, "{case}");
                    // DNG export keeps the warp and invents no Sony record
                    #[cfg(feature = "dng")]
                    {
                        let dng = crate::write_dng(&img, &Default::default()).unwrap();
                        let back = crate::decode(&dng).unwrap();
                        assert_eq!(back.opcodes.list3, img.opcodes.list3, "{case}");
                        assert_eq!(back.lens_metadata, LensMetadata::default(), "{case}");
                    }
                }
            }
        }
    }

    #[test]
    fn settings_keep_their_number_and_unknown_values_are_never_off() {
        use Kind::{Ca, Distortion, Shading};
        use SonySettingMeaning::*;
        for (kind, value, expected) in [
            (Shading, 256, Off),
            (Shading, 257, Auto),
            (Shading, 272, Auto),
            (Shading, 511, Unavailable),
            (Shading, 0, Unknown(0)),
            (Shading, 1, Unknown(1)),
            (Shading, 255, Unknown(255)),
            (Shading, 258, Unknown(258)),
            (Shading, 273, Unknown(273)),
            (Ca, 0, Off),
            (Ca, 1, Auto),
            (Ca, 255, Unavailable),
            (Ca, 2, Unknown(2)),
            (Ca, 17, Unknown(17)),
            (Ca, 256, Unknown(256)),
            (Distortion, 0, Off),
            (Distortion, 1, Auto),
            (Distortion, 17, ForcedAuto),
            (Distortion, 255, Unavailable),
            (Distortion, 2, Unknown(2)),
            (Distortion, 16, Unknown(16)),
            (Distortion, 256, Unknown(256)),
            (Distortion, u16::MAX, Unknown(u16::MAX)),
        ] {
            assert_eq!(meaning(kind, value), expected, "{kind:?} {value}");
        }
        for order in [Little, Big] {
            let read = |value: Value| {
                let mut arw = Arw::new("ILCE-7CR", order);
                arw.sr2 = None;
                set(&mut arw.plain, DISTORTION.setting.0, value);
                sony(&decode(&arw.build())).distortion.setting.clone()
            };
            assert_eq!(read(Value::Short(vec![17])).value, Some(SonySetting { value: 17, meaning: ForcedAuto }));
            assert_eq!(read(Value::Short(vec![9])).value, Some(SonySetting { value: 9, meaning: Unknown(9) }));
            // the documented editor type (SSHORT) is accepted when not negative
            assert_eq!(read(Value::SShort(vec![1])).value, Some(SonySetting { value: 1, meaning: Auto }));
            for (value, error) in [
                (Value::SShort(vec![-1]), NegativeSetting(-1)),
                (Value::SShort(vec![i16::MIN]), NegativeSetting(i16::MIN)),
                (Value::Long(vec![1]), FieldType(4)),
                (Value::Ascii("1".into()), FieldType(2)),
                (Value::Short(vec![1, 1]), Count(2)),
                (Value::Short(vec![0; 40]), Count(40)),
            ] {
                let f = read(value.clone());
                assert_eq!(prov(&f), (false, None, Rejected(error), Absent, None), "{value:?} {order:?}");
            }
        }
    }

    #[test]
    fn malformed_raw_ifd_tables_are_refused_and_not_replaced_by_the_sr2_copy() {
        let d = DISTORTION.table.0;
        let cases = [
            (d, sshort(16, &[]), Count(1)),
            (d, Value::SShort(vec![16, 0]), Count(2)),
            (d, sshort(16, &TELE[..15]), Count(16)),
            (d, Value::SShort(std::iter::once(16).chain(TELE).chain([0]).collect()), Count(18)),
            (d, Value::SShort(vec![15; 17]), CountPrefix(15)),
            (d, sshort(-16, &TELE), CountPrefix(-16)),
            (d, sshort(0, &TELE), CountPrefix(0)),
            (d, Value::Short(vec![16; 17]), FieldType(3)),
            (d, Value::Long(vec![16; 17]), FieldType(4)),
            (d, Value::Double(vec![f64::NAN; 17]), FieldType(12)),
            (d, Value::Undefined(vec![16; 34]), FieldType(7)),
            (SHADING.table.0, sshort(32, &[0; 32]), Count(33)),
            (SHADING.table.0, Value::Short(std::iter::once(16).chain([0; 16]).collect()), FieldType(3)),
            (CA.table.0, sshort(16, &ca()[..16]), Count(17)),
            (CA.table.0, sshort(16, &ca()), CountPrefix(16)),
            (CA.table.0, sshort(33, &ca()), CountPrefix(33)),
        ];
        for order in [Little, Big] {
            for (tag, value, error) in &cases {
                let mut arw = Arw::new("ILCE-7RM4A", order);
                set(&mut arw.plain, *tag, value.clone());
                let img = decode(&arw.build());
                let s = sony(&img);
                let field = if *tag == SHADING.table.0 {
                    prov(&s.shading.table)
                } else if *tag == CA.table.0 {
                    prov(&s.chromatic_aberration.table)
                } else {
                    prov(&s.distortion.table)
                };
                // the SR2SubIFD's good copy does not hide the corruption
                assert_eq!(field, (false, None, Rejected(*error), Valid, None), "{tag:#x} {value:?} {order:?}");
                // the validated distortion needs its own raw-IFD table; a bad unrelated table doesn't remove it
                assert_eq!(img.opcodes.list3.len(), usize::from(*tag != d), "{tag:#x} {value:?}");
                assert_eq!(s.distortion.status == SonyLensStatus::NoTable, *tag == d);
            }
        }
    }

    #[test]
    fn counts_and_offsets_beyond_the_data_are_reported_without_reading_them() {
        for order in [Little, Big] {
            let good = Arw::new("ILCE-7RM4A", order).build();
            let e = entry_at(&good, order, DISTORTION.table.0);
            let len = good.len() as u32;
            for (field, value, error) in [
                // counts: the TIFF reader drops an entry whose values would pass the end of the file
                (4, 0x7fff_ffff, OutsideData),
                (4, u32::MAX, OutsideData),
                (4, 0, Count(0)),
                (4, 16, Count(16)),
                // value offsets past the end, straddling it, at it
                (8, u32::MAX, OutsideData),
                (8, len - 10, OutsideData),
                (8, len, OutsideData),
            ] {
                let mut file = good.clone();
                file[e + field..e + field + 4].copy_from_slice(&word(order, value));
                let img = decode(&file);
                let s = sony(&img);
                assert_eq!(prov(&s.distortion.table), (false, None, Rejected(error), Valid, None), "{field} {value} {order:?}");
                assert!(img.opcodes.list3.is_empty());
                assert_eq!(s.distortion.status, SonyLensStatus::NoTable);
                assert_eq!(s.shading.table.value, Some(SHADING_T));
            }
        }
    }

    #[test]
    fn a_table_past_the_tiff_readers_entry_limit_is_not_applied() {
        // The TIFF reader that decodes the image reads 8,192 entries of a directory; with 8,200 filler entries ahead
        // of them the image never sees the lens fields, so they must not become a correction (main applies none).
        for order in [Little, Big] {
            let mut arw = Arw::new("ILCE-7RM4A", order);
            (arw.size, arw.sony_crop, arw.default_crop, arw.sr2) = ((30, 20), None, None, None);
            let reference = decode(&arw.build());
            assert_eq!(reference.opcodes.list3.len(), 1, "control: the same file without filler is corrected");
            arw.plain = (0..8200u16).map(|i| (0x5000 + i, Value::Byte(vec![0]))).chain(plain(&TELE, 0)).collect();
            let img = decode(&arw.build());
            assert!(img.opcodes.list3.is_empty(), "{order:?}: a table the TIFF reader never read was applied");
            // the directory scan still finds and reports it
            let s = sony(&img);
            assert_eq!(prov(&s.distortion.table), (false, None, Rejected(PastReaderLimits), Absent, None));
            assert_eq!(s.distortion.status, SonyLensStatus::NoTable);
        }
    }

    #[test]
    fn a_repeated_tag_keeps_the_copy_the_tiff_reader_keeps() {
        let crop = Rect::new(2, 2, 30, 20);
        let warp = |table: &[i16; 16]| -> Vec<Opcode> { distortion("ILCE-7RM4A", table, Rect::new(0, 0, 32, 24), crop).into_iter().collect() };
        for order in [Little, Big] {
            // a second distortion table (WIDE) right after the first (TELE), under a placeholder tag renamed to 0x7037
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.plain.push((0x7040, sshort(16, &WIDE)));
            let good = arw.build();
            let mut tag = Vec::new();
            order.put_u16(&mut tag, DISTORTION.table.0);
            let mut twice = good.clone();
            let e = entry_at(&twice, order, 0x7040);
            twice[e..e + 2].copy_from_slice(&tag);
            // both valid: the first one, as the TIFF reader keeps it and main applies it
            let img = decode(&twice);
            assert_eq!(img.opcodes.list3, warp(&TELE), "{order:?}: both copies valid");
            assert_eq!(sony(&img).distortion.table.value, Some(TELE));
            // reported as a diagnostic, which changes nothing else
            let f = &sony(&img).distortion.table;
            assert_eq!((f.raw_ifd_repeated, f.sr2_repeated, f.raw_ifd, f.copies_agree), (true, false, Valid, Some(true)));
            assert_eq!(sony(&img).distortion.status, SonyLensStatus::Applied);
            assert!(!sony(&decode(&good)).distortion.table.raw_ifd_repeated);
            // the first one's values outside the file: the TIFF reader drops it and keeps the second
            let mut dropped = twice.clone();
            let first = entry_at(&dropped, order, DISTORTION.table.0);
            dropped[first + 8..first + 12].copy_from_slice(&word(order, u32::MAX));
            let img = decode(&dropped);
            assert_eq!(img.opcodes.list3, warp(&WIDE), "{order:?}: first copy unreadable");
            assert_eq!(sony(&img).distortion.table.value, Some(WIDE));
            assert!(sony(&img).distortion.table.raw_ifd_repeated);
            // the first one malformed (the setting's SHORT entry renamed): the TIFF reader keeps it, nothing applies
            let mut short = good.clone();
            let e = entry_at(&short, order, DISTORTION.setting.0);
            short[e..e + 2].copy_from_slice(&tag);
            let img = decode(&short);
            assert!(img.opcodes.list3.is_empty(), "{order:?}: first copy malformed");
            assert_eq!(sony(&img).distortion.table.raw_ifd, Rejected(FieldType(SHORT)));
        }
    }

    #[test]
    fn sr2_copies_fill_in_only_missing_raw_ifd_fields_and_are_never_applied() {
        let (raw, sr2) = (Some(SonySource::RawIfd), Some(SonySource::Sr2SubIfd));
        for order in [Little, Big] {
            // nothing in the raw IFD: every field from the SR2SubIFD, decoded, not applied even on the validated
            // model (only the raw IFD's table was validated)
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.plain.clear();
            let img = decode(&arw.build());
            let s = sony(&img);
            assert_eq!(provenance(s), [(true, sr2, Absent, Valid, None); 6]);
            assert_eq!(
                (s.shading.table.value, s.chromatic_aberration.table.value, s.distortion.table.value),
                (Some(SHADING_T), Some(ca_curves()), Some(TELE))
            );
            assert_eq!(s.distortion.setting.value, Some(SonySetting { value: 0, meaning: SonySettingMeaning::Off }));
            assert_eq!(s.distortion.status, SonyLensStatus::NotValidatedMode);
            assert!(img.opcodes.list3.is_empty());
            // only the distortion table in the raw IFD: that one is used and applied, the rest comes from the SR2SubIFD
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.plain.retain(|(tag, _)| *tag == DISTORTION.table.0);
            let img = decode(&arw.build());
            let s = sony(&img);
            let p = provenance(s);
            assert_eq!(p[5], (true, raw, Valid, Valid, Some(true)));
            assert!(p[..5].iter().all(|f| *f == (true, sr2, Absent, Valid, None)), "{p:?}");
            assert_eq!((s.distortion.status, img.opcodes.list3.len()), (SonyLensStatus::Applied, 1));
            // the raw IFD's tag numbers inside the SR2SubIFD are not its tables
            let mut arw = Arw::new("ILCE-7CR", order);
            arw.plain.clear();
            let plain_tags = [SHADING.setting.0, SHADING.table.0, CA.setting.0, CA.table.0, DISTORTION.setting.0, DISTORTION.table.0];
            arw.sr2 = Some(encrypted(&TELE, 0).into_iter().zip(plain_tags).map(|((_, kind, v), tag)| (tag, kind, v)).collect());
            assert_eq!(decode(&arw.build()).lens_metadata, LensMetadata::default());
        }
    }

    #[test]
    fn disagreeing_copies_are_reported_and_the_raw_ifd_wins() {
        for order in [Little, Big] {
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.sr2 = Some(encrypted(&WIDE, 17));
            let img = decode(&arw.build());
            let s = sony(&img);
            let raw = Some(SonySource::RawIfd);
            assert_eq!(prov(&s.distortion.table), (true, raw, Valid, Valid, Some(false)));
            assert_eq!(prov(&s.distortion.setting), (true, raw, Valid, Valid, Some(false)));
            assert_eq!(prov(&s.shading.table), (true, raw, Valid, Valid, Some(true)));
            assert_eq!(s.distortion.table.value, Some(TELE));
            assert_eq!(s.distortion.setting.value.map(|v| v.value), Some(0));
            // the validated conversion is unchanged: the raw IFD's table
            let reference: Vec<Opcode> = distortion("ILCE-7RM4A", &TELE, Rect::new(0, 0, 32, 24), Rect::new(2, 2, 30, 20)).into_iter().collect();
            assert_eq!(img.opcodes.list3, reference);
        }
    }

    #[test]
    fn unreadable_or_hostile_sr2_subifds_give_no_copies() {
        let raw = Some(SonySource::RawIfd);
        for order in [Little, Big] {
            // stored with another key: the block decrypts to noise, no copy is found, nothing falls back on it
            let mut arw = Arw::new("ILCE-7CR", order);
            arw.stored_key = [1, 2, 3, 4];
            let img = decode(&arw.build());
            let s = sony(&img);
            assert!(provenance(s).iter().all(|&(value, source, plain, sr2, agree)| value
                && source == raw
                && plain == Valid
                && sr2 != Valid
                && agree.is_none()));
            arw.plain.clear();
            let img = decode(&arw.build());
            assert!(img.lens_metadata.sony.as_ref().is_none_or(|s| provenance(s).iter().all(|f| !f.0 && f.3 != Valid)));
            // no SR2SubIFD; DNGPrivateData pointing past the file or into the image
            let mut arw = Arw::new("ILCE-7CR", order);
            arw.sr2 = None;
            let s = sony(&decode(&arw.build())).clone();
            assert!(!s.sr2_read);
            assert_eq!(provenance(&s), [(true, raw, Valid, Absent, None); 6]);
            let good = Arw::new("ILCE-7CR", order).build();
            let at = Tiff::parse(&good).unwrap().ifds[0].get(t::DNG_PRIVATE_DATA).unwrap().offset as usize;
            for pointer in [u32::MAX, good.len() as u32, 8] {
                let mut file = good.clone();
                file[at..at + 4].copy_from_slice(&word(order, pointer));
                let img = decode(&file);
                assert!(sony(&img).distortion.table.sr2 != Valid, "{pointer}");
                assert_eq!(sony(&img).distortion.table.value, Some(TELE));
            }
            // hostile entries in a well-encrypted block (the distortion table is its sixth entry): another type, huge
            // count, a value offset before the block or past it
            let sixth = 2 + 12 * 5;
            let put = move |at: usize, bytes: Vec<u8>| move |b: &mut [u8]| b[at..at + bytes.len()].copy_from_slice(&bytes);
            let mut short = Vec::new();
            order.put_u16(&mut short, SHORT);
            for (patch, error) in [
                (Box::new(put(sixth + 2, short.clone())) as Box<dyn Fn(&mut [u8])>, FieldType(SHORT)),
                (Box::new(put(sixth + 4, word(order, u32::MAX))), Count(u64::from(u32::MAX))),
                (Box::new(put(sixth + 4, word(order, 1))), Count(1)),
                (Box::new(put(sixth + 8, word(order, 0))), OutsideData),
                (Box::new(put(sixth + 8, word(order, u32::MAX))), OutsideData),
            ] {
                let mut arw = Arw::new("ILCE-7CR", order);
                arw.sr2_patch = Some(patch);
                let s = sony(&decode(&arw.build())).clone();
                assert_eq!(prov(&s.distortion.table), (true, raw, Valid, Rejected(error), None), "{error:?}");
                assert_eq!(prov(&s.shading.table), (true, raw, Valid, Valid, Some(true)));
            }
            // an entry count past the end of the block: the scan stops where the block does
            let mut count = Vec::new();
            order.put_u16(&mut count, u16::MAX);
            let mut arw = Arw::new("ILCE-7CR", order);
            arw.sr2_patch = Some(Box::new(put(0, count)));
            assert_eq!(provenance(sony(&decode(&arw.build()))), [(true, raw, Valid, Valid, Some(true)); 6]);
        }
    }

    #[test]
    fn framing_and_coding_evidence_gate_the_validated_distortion() {
        use SonyLensStatus::*;
        let r = Rect::new;
        for order in [Little, Big] {
            let run = |arw: &Arw| {
                let img = decode(&arw.build());
                let s = sony(&img).clone();
                (s, img)
            };
            // Sony's crop at x = 0 and the default crop at x = 2 (the A7R IVA's 0 versus 32): the image keeps Sony's
            // crop, the table is centred on and normalised to the default crop
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.sony_crop = Some([0, 2, 30, 20]);
            let (s, img) = run(&arw);
            let g = &s.geometry;
            assert_eq!(
                (g.sony_crop, g.default_crop, g.image_crop, g.reference),
                (Some(r(0, 2, 30, 20)), Some(r(2, 2, 30, 20)), r(0, 2, 30, 20), r(2, 2, 30, 20))
            );
            assert_eq!((img.crop, s.distortion.status), (r(0, 2, 30, 20), Applied));
            let Some(Opcode::WarpRectilinear { center, .. }) = img.opcodes.list3.first() else { panic!("warp") };
            assert!((center[0] - 16.5 / 31.0).abs() < 1e-12 && (center[1] - 11.5 / 23.0).abs() < 1e-12, "{center:?}");
            // agreeing crops (the A7CR's case), in a frame padded beyond them (lossless L): the storage is recorded,
            // the reference stays the crop
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.size = (40, 28);
            let (s, _) = run(&arw);
            assert_eq!((s.geometry.storage_width, s.geometry.storage_height, s.geometry.reference), (40, 28, r(2, 2, 30, 20)));
            assert_eq!(s.distortion.status, Applied);
            // without a default crop the reference is the image crop
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.default_crop = None;
            let (s, _) = run(&arw);
            assert_eq!((s.geometry.default_crop, s.geometry.reference, s.distortion.status), (None, r(2, 2, 30, 20), Applied));
            // non-3:2 framing: an aspect crop in Sony's tags, in the default crop, or a 4:3 frame without crop tags
            for (sony_crop, default_crop) in
                [(Some([2, 2, 30, 16]), Some([2, 2, 30, 20])), (Some([2, 2, 30, 20]), Some([2, 2, 30, 16])), (None, None)]
            {
                let mut arw = Arw::new("ILCE-7RM4A", order);
                (arw.sony_crop, arw.default_crop) = (sony_crop, default_crop);
                let (s, img) = run(&arw);
                assert!(!s.geometry.native_3_2, "{sony_crop:?} {default_crop:?}");
                assert_eq!((s.distortion.status, img.opcodes.list3.len()), (NotValidatedMode, 0), "{sony_crop:?} {default_crop:?}");
            }
            // neutral, out-of-range and folding tables in the validated scope; models outside it
            for (model, table, status) in [
                ("ILCE-7RM4A", [0; 16], Neutral),
                ("ILCE-7RM4A", [i16::MIN; 16], Rejected),
                ("ILCE-7RM4A", [0, 4096, -4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], Rejected),
                ("ILCE-7RM4", TELE, NotValidatedModel),
                ("ILCE-7CR", [0; 16], NotValidatedMode),
                ("", TELE, NotValidatedModel),
            ] {
                let mut arw = Arw::new(model, order);
                arw.plain = plain(&table, 1);
                let (s, img) = run(&arw);
                assert_eq!((s.distortion.status, img.opcodes.list3.len()), (status, 0), "{model} {table:?}");
                assert_eq!(s.distortion.table.value, Some(table));
            }
        }
    }

    #[test]
    fn out_of_range_crop_tags_are_recorded_and_never_applied() {
        for order in [Little, Big] {
            // past the frame, enormous, empty, partly outside
            for crop in [[99999, 0, 30, 20], [u32::MAX; 4], [2, 2, 0, 0], [30, 2, 30, 20]] {
                let mut arw = Arw::new("ILCE-7RM4A", order);
                (arw.sony_crop, arw.default_crop) = (Some(crop), Some(crop));
                let img = decode(&arw.build());
                let s = sony(&img);
                let [x, y, w, h] = crop.map(|v| v as usize);
                assert_eq!((s.geometry.sony_crop, s.geometry.default_crop), (Some(Rect::new(x, y, w, h)), Some(Rect::new(x, y, w, h))), "{crop:?}");
                assert!(!s.geometry.native_3_2, "{crop:?}");
                assert_eq!((s.distortion.status, img.opcodes.list3.len()), (SonyLensStatus::NotValidatedMode, 0), "{crop:?}");
                assert_eq!(s.distortion.table.value, Some(TELE));
            }
        }
    }

    #[test]
    fn exif_orientation_does_not_change_the_record() {
        // the tables and the warp are in sensor coordinates; the engine orients the warp (`engine::files::embedded_lens`)
        let reference = decode(&Arw::new("ILCE-7RM4A", Little).build());
        for orientation in 1..=8u16 {
            let mut arw = Arw::new("ILCE-7RM4A", Little);
            arw.plain.push((t::ORIENTATION, Value::Short(vec![orientation])));
            let img = decode(&arw.build());
            assert_eq!(img.orientation, crate::Orientation::from_exif(orientation));
            assert_eq!((&img.lens_metadata, &img.opcodes), (&reference.lens_metadata, &reference.opcodes), "{orientation}");
        }
    }

    fn a7cr_geometry(storage: (usize, usize)) -> SonyLensGeometry {
        let crop = Rect::new(32, 20, 9504, 6336);
        SonyLensGeometry {
            storage_width: storage.0,
            storage_height: storage.1,
            sony_crop: Some(crop),
            default_crop: Some(crop),
            image_crop: crop,
            reference: crop,
            compression: 7,
            linear_ycbcr: false,
            native_3_2: true,
        }
    }

    #[test]
    fn a7cr_requires_an_enabled_setting_and_validated_full_frame_mode() {
        use SonyLensStatus::*;
        for order in [Little, Big] {
            for (setting, status) in
                [(0, CameraCorrectionDisabled), (1, Applied), (17, Applied), (255, CameraCorrectionDisabled), (2, CameraCorrectionDisabled)]
            {
                let mut arw = Arw::new("ILCE-7CR", order);
                arw.plain = plain(&A7CR_WIDE, setting);
                arw.sr2 = None;
                let bytes = arw.build();
                let tiff = Tiff::parse(&bytes).unwrap();
                let run = |model: &str, geometry: SonyLensGeometry| {
                    let (record, warp) = corrections(&bytes, order, (&tiff.ifds[0], false), None, model, geometry);
                    (record.unwrap().distortion.status, warp.is_some())
                };
                let geometry = a7cr_geometry((9728, 6656));
                assert_eq!(run("ILCE-7CR", geometry.clone()), (status, status == Applied));
                for model in ["ILCE-7CR ", "ILCE-7CR unknown", "ILCE-7C", "ILCE-7CM2", "ILCE-7RM4", "ILCE-7RM6"] {
                    assert_eq!(run(model, geometry.clone()), (NotValidatedModel, false), "{model}");
                }
                let mut linear = geometry.clone();
                linear.linear_ycbcr = true;
                let mut apsc = geometry.clone();
                apsc.reference = Rect::new(32, 20, 6240, 4160);
                apsc.image_crop = apsc.reference;
                let mut aspect = geometry.clone();
                aspect.image_crop.height = 5346;
                aspect.native_3_2 = false;
                let mut offset = geometry.clone();
                offset.image_crop.x = 0;
                for mode in [linear, apsc, aspect, offset] {
                    assert_eq!(run("ILCE-7CR", mode), (NotValidatedMode, false));
                }
                // Missing or malformed settings cannot enable the correction.
                for value in [None, Some(Value::SShort(vec![-1])), Some(Value::Long(vec![17]))] {
                    arw.plain.retain(|(tag, _)| *tag != DISTORTION.setting.0);
                    if let Some(value) = value {
                        arw.plain.push((DISTORTION.setting.0, value));
                    }
                    let bytes = arw.build();
                    let tiff = Tiff::parse(&bytes).unwrap();
                    let (record, warp) = corrections(&bytes, order, (&tiff.ifds[0], false), None, "ILCE-7CR", geometry.clone());
                    assert_eq!(record.unwrap().distortion.status, CameraCorrectionDisabled);
                    assert!(warp.is_none());
                }
            }
        }
    }

    #[test]
    fn a7cr_storage_padding_does_not_change_the_optical_mapping() {
        let crop = Rect::new(32, 20, 9504, 6336);
        let mut maps = Vec::new();
        for (w, h) in [(9600, 6376), (9728, 6656)] {
            let Opcode::WarpRectilinear { planes, center } = a7cr_distortion(&A7CR_WIDE, Rect::new(0, 0, w, h), crop).unwrap() else {
                panic!("warp")
            };
            let (cx, cy) = (center[0] * (w - 1) as f64, center[1] * (h - 1) as f64);
            assert!((cx - 4783.5).abs() < 1e-10 && (cy - 3187.5).abs() < 1e-10);
            let radius = cx.max((w - 1) as f64 - cx).hypot(cy.max((h - 1) as f64 - cy));
            let k = planes[0];
            maps.push(
                (0..=100)
                    .map(|j| {
                        let r = j as f64 / 100.0 * 5700.0;
                        let r2 = (r / radius).powi(2);
                        r * (k[0] + r2 * (k[1] + r2 * (k[2] + r2 * k[3])))
                    })
                    .collect::<Vec<_>>(),
            );
        }
        for (a, b) in maps[0].iter().zip(&maps[1]) {
            assert!((a - b).abs() < 1e-9);
        }
        for bad in [[0; 16], [i16::MIN; 16], [0, 4096, -4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]] {
            assert!(a7cr_distortion(&bad, Rect::new(0, 0, 9728, 6656), crop).is_none());
        }
        assert!(a7cr_distortion(&A7CR_WIDE, AREA, crop).is_none(), "crop outside storage");
        assert!(a7cr_distortion(&A7CR_WIDE, AREA, Rect::new(0, 0, 6240, 4160)).is_none(), "APS-C");
    }

    #[test]
    fn a7cr_matches_independently_measured_public_jpeg_geometry() {
        // Green-channel feature fits, independent of the table, on the public FE 16-25mm (storytold/lightcraft#1300)
        // and FE 50mm files. Check only the supported radii, not extrapolated corners.
        let prime = [-4, -1, 3, 6, 12, 17, 23, 30, 37, 43, 50, 58, 65, 71, 76, 84];
        let cases = [
            (A7CR_WIDE, [1.0606515126386147, -0.24282593993968085, 0.1667903749262344, -0.059882313562606454], 0.93),
            (prime, [0.9951622479344596, 0.00741813858793482, -0.002474290073462703, -0.000235147000260114], 0.96),
        ];
        let crop = Rect::new(32, 20, 9504, 6336);
        let radius = 9503f64.hypot(6335.0) / 2.0;
        for (table, reference, supported) in cases {
            let Opcode::WarpRectilinear { planes, center } = a7cr_distortion(&table, Rect::new(0, 0, 9728, 6656), crop).unwrap() else {
                panic!("warp")
            };
            let (cx, cy) = (center[0] * 9727.0, center[1] * 6655.0);
            let normalization = cx.max(9727.0 - cx).hypot(cy.max(6655.0 - cy));
            let k = planes[0];
            let mut worst = 0.0f64;
            for j in 0..=1000 {
                let r = supported * j as f64 / 1000.0;
                let rho = r * radius;
                let r2 = (rho / normalization).powi(2);
                let mapped = rho * (k[0] + r2 * (k[1] + r2 * (k[2] + r2 * k[3])));
                let expected = rho * (reference[0] + r * r * (reference[1] + r * r * (reference[2] + r * r * reference[3])));
                worst = worst.max((mapped - expected).abs() * 1440.0 / 9504.0);
            }
            // This smooth feature fit has localization noise. The combined budget allows the
            // 0.75 px camera-table residual plus the separately bounded 0.25 px polynomial error.
            assert!(worst < 1.0, "camera-JPEG shape residual {worst} px at 1440");
        }
    }

    #[test]
    fn linear_ycbcr_files_are_decoded_but_not_corrected() {
        // a downsized lossless (M/S) coding, as in arw.rs's test, from the validated model with 3:2 crop tags
        for order in [Little, Big] {
            let mut raw = IfdBuilder::new();
            raw.set(t::MAKE, Value::Ascii("SONY".into()));
            raw.set(t::MODEL, Value::Ascii("ILCE-7RM4A".into()));
            raw.set(t::IMAGE_WIDTH, Value::Long(vec![4]));
            raw.set(t::IMAGE_LENGTH, Value::Long(vec![4]));
            raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![15, 15, 15]));
            raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
            raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::YCBCR]));
            raw.set(t::COMPRESSION, Value::Short(vec![7]));
            raw.set(0x7010, Value::Short(vec![0; 4]));
            raw.set(0x74c7, Value::Long(vec![0, 0]));
            raw.set(0x74c8, Value::Long(vec![3, 2]));
            for (tag, value) in plain(&TELE, 17) {
                raw.set(tag, value);
            }
            raw.set_image(ImageData::Tiles { tile_width: 4, tile_height: 4, tiles: vec![crate::ljpeg::tests::fixture_420()] });
            let img = decode(&TiffWriter::new(order, false).write(&[raw]).unwrap());
            let s = sony(&img);
            assert!(s.geometry.linear_ycbcr && s.geometry.native_3_2);
            assert_eq!((s.geometry.compression, s.distortion.status, img.opcodes.list3.len()), (7, SonyLensStatus::NotValidatedMode, 0));
            assert_eq!(s.distortion.setting.value, Some(SonySetting { value: 17, meaning: SonySettingMeaning::ForcedAuto }));
            assert_eq!(s.distortion.table.value, Some(TELE));
        }
    }

    #[test]
    fn files_without_lens_fields_have_no_record() {
        for order in [Little, Big] {
            let mut arw = Arw::new("ILCE-7RM4A", order);
            arw.plain.clear();
            arw.sr2 = None;
            let img = decode(&arw.build());
            assert_eq!((img.lens_metadata.clone(), img.opcodes.list3.len()), (LensMetadata::default(), 0));
            // an SR2SubIFD holding other tags only
            arw.sr2 = Some(vec![(0x7310, SHORT, vec![512; 4])]);
            assert_eq!(decode(&arw.build()).lens_metadata, LensMetadata::default());
            // one field anywhere is a record
            arw.sr2 = Some(vec![(DISTORTION.setting.1, SHORT, vec![255])]);
            let img = decode(&arw.build());
            let s = sony(&img);
            assert_eq!(s.distortion.setting.value, Some(SonySetting { value: 255, meaning: SonySettingMeaning::Unavailable }));
            assert_eq!(
                (s.distortion.status, s.shading.status, s.chromatic_aberration.status),
                (SonyLensStatus::NoTable, SonyLensStatus::NoTable, SonyLensStatus::NoTable)
            );
        }
    }

    #[test]
    fn corrupt_files_never_panic() {
        for order in [Little, Big] {
            let good = Arw::new("ILCE-7RM4A", order).build();
            let tiff = Tiff::parse(&good).unwrap();
            let strip = tiff.ifds[0].image().unwrap().chunks(good.len() as u64)[0];
            let image = strip.offset as usize..(strip.offset + strip.len) as usize;
            let check = |file: &[u8]| {
                let header = crate::arw::decode(file, Mode::Header, &Limits::default());
                let full = crate::arw::decode(file, Mode::Full, &Limits::default());
                if let (Ok(h), Ok(f)) = (header, full) {
                    assert_eq!((h.lens_metadata, h.opcodes), (f.lens_metadata, f.opcodes));
                }
            };
            for len in 0..good.len() {
                check(&good[..len]);
            }
            for at in (0..good.len()).filter(|at| !image.contains(at)) {
                for v in [0, 0xff, good[at] ^ 0x80, good[at] ^ 0x01] {
                    let mut file = good.clone();
                    file[at] = v;
                    check(&file);
                }
            }
        }
    }

    #[test]
    fn matches_independent_sony_reference_geometry() {
        // On/off feature correspondences from Imaging Edge 4.1 at 600 mm, independently fitted.
        let Opcode::WarpRectilinear { planes, center } = distortion("ILCE-7RM4A", &TELE, AREA, AREA).unwrap() else { panic!("warp") };
        assert_eq!(center, [0.5, 0.5]);
        let reference = [0.9760159044, 0.0254891644, -0.0005984764, -0.0010120638];
        for i in 0..=100 {
            let r = i as f64 / 100.0;
            let evaluate = |k: &[f64]| r * (k[0] + r * r * (k[1] + r * r * (k[2] + r * r * k[3])));
            assert!((evaluate(&planes[0]) - evaluate(&reference)).abs() < 0.00015);
        }
    }

    // ILCE-7M3 raw-IFD tables (FE 16-35mm F4 ZA OSS at 16 and 35 mm, FE 28-70mm F3.5-5.6 OSS at 70 mm) and the
    // geometry of the camera's own JPEG of the same exposure (Lens Comp. Distortion: Auto): checkerboard corners of
    // that JPEG matched with the uncorrected raw and fitted with the warp's polynomial over the 6000×4000 frame (rms
    // 0.11–0.25 px). See docs/sony-lens-corrections.md.
    const A7M3: [([i16; 16], [f64; 4]); 3] = [
        (
            [32, 2, -38, -83, -138, -199, -270, -336, -402, -462, -519, -568, -611, -643, -670, -690],
            [1.0247000859, -0.1092847508, 0.1061422243, -0.0397989678],
        ),
        ([2, 2, 4, 9, 18, 29, 45, 65, 92, 124, 164, 210, 264, 329, 403, 489], [0.9708150714, 0.0157256288, 0.0175613359, -0.004107544]),
        ([4, 7, 18, 40, 68, 108, 155, 212, 280, 359, 453, 557, 679, 819, 978, 1161], [0.9342002611, 0.0560486347, 0.0076184603, 0.0021497933]),
    ];

    #[test]
    fn ilce_7m3_matches_the_cameras_own_jpeg_geometry() {
        let frame = Rect::new(0, 0, 6000, 4000);
        let evaluate = |k: &[f64], r: f64| r * (k[0] + r * r * (k[1] + r * r * (k[2] + r * r * k[3])));
        for (values, reference) in A7M3 {
            let Opcode::WarpRectilinear { planes, center } = distortion("ILCE-7M3", &values, frame, frame).unwrap() else { panic!("warp") };
            assert_eq!(center, [0.5, 0.5]);
            // The camera fills its frame with its own zoom; compare the shape over the measured radii (to 0.97).
            let radii: Vec<f64> = (1..=97).map(|i| i as f64 / 100.0).collect();
            let (ours, camera): (Vec<f64>, Vec<f64>) = radii.iter().map(|&r| (evaluate(&planes[0], r), evaluate(&reference, r))).unzip();
            let worst = |ours: &[f64]| {
                let zoom = ours.iter().zip(&camera).map(|(a, b)| a * b).sum::<f64>() / ours.iter().map(|a| a * a).sum::<f64>();
                ours.iter().zip(&camera).map(|(a, b)| (a * zoom - b).abs()).fold(0.0, f64::max)
            };
            // 0.0008 of the half diagonal is 2.9 px on the 6000 px frame (measured: 0.4–2.4 px); uncorrected is 10×+ off.
            let (corrected, uncorrected) = (worst(&ours), worst(&radii));
            assert!(corrected < 0.0008 && uncorrected > 10.0 * corrected, "{values:?}: {corrected} vs {uncorrected}");
        }
        // Sony's crop and the default crop agree on this body (12, 12, 6000×4000 of 6048×4024), so the warp sits on
        // the image centre.
        let Opcode::WarpRectilinear { center, .. } =
            distortion("ILCE-7M3", &A7M3[0].0, Rect::new(0, 0, 6048, 4024), Rect::new(12, 12, 6000, 4000)).unwrap()
        else {
            panic!("warp")
        };
        assert!((center[0] * 6047.0 - 3011.5).abs() < 1e-9 && (center[1] * 4023.0 - 2011.5).abs() < 1e-9, "{center:?}");
    }

    #[test]
    fn barrel_framing_and_offset_crop_keep_the_same_geometry() {
        let crop = Rect::new(32, 20, 9504, 6336);
        let active = Rect::new(0, 0, 9600, 6376);
        let Opcode::WarpRectilinear { planes: a, .. } = distortion("ILCE-7RM4A", &WIDE, AREA, AREA).unwrap() else { panic!("warp") };
        let Opcode::WarpRectilinear { planes: b, center } = distortion("ILCE-7RM4A", &WIDE, active, crop).unwrap() else { panic!("warp") };
        assert!(a[0][0] > 1.02 && a[0][0] < 1.03, "barrel correction expands the middle");
        assert!((center[0] * 9599.0 - 4783.5).abs() < 1e-10);
        assert!((center[1] * 6375.0 - 3187.5).abs() < 1e-10);
        let ratio = (4815.5f64.hypot(3187.5)) / (4751.5f64.hypot(3167.5));
        for i in 0..4 {
            assert!((a[0][i] - b[0][i] / ratio.powi(2 * i as i32)).abs() < 1e-10);
        }
    }

    /// The conversion's own refusals; missing and malformed layouts are refused by the decoder before it (see
    /// `malformed_raw_ifd_tables_are_refused_and_not_replaced_by_the_sr2_copy`).
    #[test]
    fn refuses_unusable_tables() {
        for bad in [[0; 16], [i16::MIN; 16], [0, 4096, -4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]] {
            assert!(distortion("ILCE-7RM4A", &bad, AREA, AREA).is_none());
        }
        assert!(distortion("ILCE-7RM4A", &TELE, Rect::new(0, 0, 1, 1), AREA).is_none());
        assert!(distortion("ILCE-7RM4A", &TELE, AREA, Rect::new(0, 0, 9504, 5346)).is_none());
        assert!(distortion("ILCE-7RM4A", &TELE, AREA, Rect::new(99999, 0, 10, 10)).is_none());
    }

    #[test]
    fn refuses_unvalidated_models_even_with_a_valid_table() {
        assert!(distortion("ILCE-7RM4A", &TELE, AREA, AREA).is_some());
        assert!(distortion("ILCE-7M3", &TELE, AREA, AREA).is_some());
        for model in ["", "ILCE-7M4", "ILCE-7RM4", "ILCE-7RM5", "ILCE-9M2", "DSC-RX100M3", "ILCE-7RM4A unknown", "ILCE-7CR", "ILCE-7M3 ", "ILCE-7SM3"]
        {
            assert!(distortion(model, &TELE, AREA, AREA).is_none(), "{model}");
        }
    }

    #[test]
    fn header_and_full_decode_preserve_correction_through_dng() {
        for (model, expected) in [("ILCE-7RM4A", 1), ("ILCE-7RM4", 0), ("ILCE-7M3", 1), ("ILCE-7M4", 0), ("", 0)] {
            for (height, expected) in [(20, expected), (16, 0)] {
                for order in [ByteOrder::Little, ByteOrder::Big] {
                    let mut raw = IfdBuilder::new();
                    raw.set(t::MAKE, Value::Ascii("SONY".into()));
                    raw.set(t::MODEL, Value::Ascii(model.into()));
                    raw.set(t::IMAGE_WIDTH, Value::Long(vec![32]));
                    raw.set(t::IMAGE_LENGTH, Value::Long(vec![24]));
                    raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
                    raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
                    raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
                    raw.set(t::COMPRESSION, Value::Short(vec![1]));
                    raw.set(t::WHITE_LEVEL, Value::Long(vec![16383]));
                    raw.set(0x74c7, Value::Long(vec![0, 2]));
                    raw.set(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![2, 2]));
                    raw.set(t::DEFAULT_CROP_SIZE, Value::Long(vec![30, 20]));
                    // An aspect crop must stay excluded even when DefaultCrop still describes 3:2.
                    raw.set(0x74c8, Value::Long(vec![30, height]));
                    raw.set(DISTORTION.table.0, Value::SShort(std::iter::once(16).chain(TELE).collect()));
                    // Zero pixels have the same byte representation in both orders.
                    raw.set_image(ImageData::Strips { rows_per_strip: 24, strips: vec![vec![0; 32 * 24 * 2]] });
                    let file = TiffWriter { order, ..Default::default() }.write(&[raw]).unwrap();
                    let header = crate::arw::decode(&file, crate::Mode::Header, &Limits::default()).unwrap();
                    let full = crate::arw::decode(&file, crate::Mode::Full, &Limits::default()).unwrap();
                    assert_eq!(header.info(), full.info());
                    assert_eq!(full.opcodes.list3.len(), expected, "{model} {order:?} height={height}");
                    // the image keeps Sony's crop tags; the warp is centred on the DNG default crop (x 2..32 of 32)
                    assert_eq!(full.crop, Rect::new(0, 2, 30, height as usize));
                    if let Some(Opcode::WarpRectilinear { center, .. }) = full.opcodes.list3.first() {
                        assert!((center[0] - 16.5 / 31.0).abs() < 1e-12 && (center[1] - 11.5 / 23.0).abs() < 1e-12, "{center:?}");
                    }
                    #[cfg(feature = "dng")]
                    {
                        let dng = crate::write_dng(&full, &Default::default()).unwrap();
                        assert_eq!(crate::decode(&dng).unwrap().opcodes.list3, full.opcodes.list3);
                    }
                }
            }
        }
    }
}
