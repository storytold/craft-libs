use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use craft_raw::{Limits, RawData, RawError, RawFormat, RawImage, Rect};
use craft_tiff::TiffError;
use sha2::{Digest, Sha256};

pub(super) const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
const HEADER: &str = "schema\tinput\tstatus\tformat\tsample_type\tsample_count\tsensor_sha256\twidth\theight\tcpp\tbits\tcfa\tactive_area\tcrop\torientation\tbaseline_exposure_bits";
const LINEARIZATION_HEADER: &str = "\tlinearization_state\tlinearization_table_sha256";

#[derive(Clone, Copy)]
enum Schema {
    Original,
    Linearization,
}

impl Schema {
    fn version(self) -> u8 {
        match self {
            Self::Original => 1,
            Self::Linearization => 2,
        }
    }
}

#[derive(Debug)]
pub(super) enum RunError {
    Usage,
    Output(io::Error),
}

pub(super) fn run(args: impl IntoIterator<Item = OsString>, out: &mut impl Write, max_input_bytes: u64, limits: &Limits) -> Result<(), RunError> {
    let mut paths = args.into_iter().peekable();
    let (schema, extra_header) =
        if paths.next_if(|arg| arg == "--linearization").is_some() { (Schema::Linearization, LINEARIZATION_HEADER) } else { (Schema::Original, "") };
    paths.next_if(|arg| arg == "--");
    if paths.peek().is_none() {
        return Err(RunError::Usage);
    }
    writeln!(out, "{HEADER}{extra_header}").map_err(RunError::Output)?;
    for (index, path) in paths.enumerate() {
        let input = index + 1;
        let result = read_input(Path::new(&path), max_input_bytes)
            .map_err(|failure| match failure {
                InputFailure::Read => "read_error",
                InputFailure::Limit => "input_limit",
            })
            .and_then(|bytes| craft_raw::decode_with(&bytes, limits).map_err(decode_status));
        match result {
            Ok(raw) => write_success(out, schema, input, &raw),
            Err(status) => write_failure(out, schema, input, status),
        }
        .map_err(RunError::Output)?;
    }
    out.flush().map_err(RunError::Output)
}

enum InputFailure {
    Read,
    Limit,
}

fn read_input(path: &Path, max_input_bytes: u64) -> Result<Vec<u8>, InputFailure> {
    // Reject non-regular paths before opening, and check the opened file again.
    check_input(&fs::metadata(path).map_err(|_| InputFailure::Read)?, max_input_bytes)?;
    let file = File::open(path).map_err(|_| InputFailure::Read)?;
    check_input(&file.metadata().map_err(|_| InputFailure::Read)?, max_input_bytes)?;
    let mut bytes = Vec::new();
    // A file may grow after the metadata check. Read at most one extra byte to detect it.
    file.take(max_input_bytes.saturating_add(1)).read_to_end(&mut bytes).map_err(|_| InputFailure::Read)?;
    if bytes.len() as u64 > max_input_bytes {
        return Err(InputFailure::Limit);
    }
    Ok(bytes)
}

fn check_input(metadata: &fs::Metadata, max_input_bytes: u64) -> Result<(), InputFailure> {
    if !metadata.is_file() {
        return Err(InputFailure::Read);
    }
    if metadata.len() > max_input_bytes {
        return Err(InputFailure::Limit);
    }
    Ok(())
}

fn decode_status(error: RawError) -> &'static str {
    match error {
        RawError::NotRaw => "not_raw",
        RawError::Unsupported(_) => "unsupported",
        RawError::Corrupt(_) => "corrupt",
        RawError::Limit(_) | RawError::Tiff(TiffError::Limit(_)) => "decode_limit",
        RawError::Tiff(_) => "tiff_error",
    }
}

fn write_failure(out: &mut impl Write, schema: Schema, input: usize, status: &str) -> io::Result<()> {
    // Keep the success-only fields present and empty in both schemas.
    let extra_fields = if let Schema::Linearization = schema { "\t\t" } else { "" };
    writeln!(out, "{}\t{input}\t{status}\t\t\t\t\t\t\t\t\t\t\t\t\t{extra_fields}", schema.version())
}

fn write_success(out: &mut impl Write, schema: Schema, input: usize, raw: &RawImage) -> io::Result<()> {
    let mut hash = Sha256::new();
    let (sample_type, count) = match &raw.data {
        RawData::U16(samples) => {
            for sample in samples {
                hash.update(sample.to_le_bytes());
            }
            ("u16", samples.len())
        }
        RawData::F32(samples) => {
            for sample in samples {
                hash.update(sample.to_bits().to_le_bytes());
            }
            ("f32", samples.len())
        }
    };
    let digest = hash.finalize();
    write!(
        out,
        "{}\t{input}\tok\t{}\t{sample_type}\t{count}\t{digest:x}\t{}\t{}\t{}\t{}\t",
        schema.version(),
        format_label(raw.format),
        raw.width,
        raw.height,
        raw.cpp,
        raw.bits,
    )?;
    if let Some(cfa) = &raw.cfa {
        write!(out, "{},{},{},{}:", cfa.width, cfa.height, cfa.origin_x, cfa.origin_y)?;
        let mut pattern = cfa.pattern.iter();
        if let Some(first) = pattern.next() {
            write!(out, "{first}")?;
            for channel in pattern {
                write!(out, ",{channel}")?;
            }
        }
    }
    write_rect(out, raw.active_area)?;
    write_rect(out, raw.crop)?;
    write!(out, "\t{}\t{:016x}", raw.orientation.to_exif(), raw.color.baseline_exposure.to_bits())?;
    if let Schema::Linearization = schema {
        write_linearization(out, raw)?;
    }
    writeln!(out)
}

fn write_linearization(out: &mut impl Write, raw: &RawImage) -> io::Result<()> {
    let Some(table) = &raw.linearization_table else {
        return write!(out, "\tabsent\t");
    };
    let state = if raw.linearized { "applied" } else { "deferred" };
    let mut hash = Sha256::new();
    for entry in table {
        hash.update(entry.to_le_bytes());
    }
    write!(out, "\t{state}\t{:x}", hash.finalize())
}

fn write_rect(out: &mut impl Write, rect: Rect) -> io::Result<()> {
    write!(out, "\t{},{},{},{}", rect.x, rect.y, rect.width, rect.height)
}

fn format_label(format: RawFormat) -> &'static str {
    match format {
        RawFormat::Dng => "dng",
        RawFormat::Cr2 => "cr2",
        RawFormat::Cr3 => "cr3",
        RawFormat::Nef => "nef",
        RawFormat::Nrw => "nrw",
        RawFormat::Arw => "arw",
        RawFormat::Raf => "raf",
        RawFormat::Orf => "orf",
        RawFormat::Rw2 => "rw2",
        RawFormat::Pef => "pef",
        RawFormat::Srw => "srw",
        RawFormat::Crw => "crw",
        RawFormat::Mrw => "mrw",
        RawFormat::X3f => "x3f",
        RawFormat::CfaTiff => "cfa_tiff",
        RawFormat::KodakDcs => "kodak_dcs",
        RawFormat::Erf => "erf",
        RawFormat::Mef => "mef",
        RawFormat::OtherTiff => "other_tiff",
    }
}
