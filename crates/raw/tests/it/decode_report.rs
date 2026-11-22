use super::dng::{base_ifd, dng, synthetic};
use craft_raw::{BlackLevel, Cfa, DngCompression, DngWriteOptions, Limits, Orientation, RawData, RawImage, write_dng};
use craft_tiff::{ByteOrder, ImageData, Value, tags};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[path = "../../examples/decode_report/report.rs"]
mod report;

const HEADER: &str = concat!(
    "schema\tinput\tstatus\tformat\tsample_type\tsample_count\tsensor_sha256\twidth\theight\tcpp\tbits\tcfa\t",
    "active_area\tcrop\torientation\tbaseline_exposure_bits\n",
);

struct FixtureDir(PathBuf);

impl FixtureDir {
    fn new(test: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"));
        std::fs::create_dir_all(root).unwrap_or_else(|error| panic!("create tmp root {}: {error}", root.display()));
        for attempt in 0..1000 {
            let path = root.join(format!("decode-report-{test}-{}-{attempt}", std::process::id()));
            // Only clean up a directory this test created, even if a process ID was reused.
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create fixture directory {}: {error}", path.display()),
            }
        }
        panic!("no free fixture directory for {test}");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn integer_raw() -> RawImage {
    let mut raw = synthetic(16, 16, Some(Cfa::bayer("RGGB").unwrap()), 1);
    raw.data = RawData::U16((256..512).collect());
    raw.color.baseline_exposure = 0.5;
    raw
}

fn float_raw() -> RawImage {
    let mut raw = synthetic(16, 16, None, 3);
    raw.data = RawData::F32((0..16 * 16 * 3).map(|i| (i % 16) as f32 / 16.0).collect());
    raw.black = BlackLevel::uniform(0.0);
    raw.white = vec![1.0];
    raw.color.baseline_exposure = 0.5;
    raw
}

fn write_fixture(dir: &Path, name: &str, raw: &RawImage, order: ByteOrder) -> OsString {
    let bytes = write_dng(raw, &DngWriteOptions { compression: DngCompression::Uncompressed, order, ..Default::default() }).unwrap();
    write_bytes(dir, name, &bytes)
}

fn write_bytes(dir: &Path, name: &str, bytes: &[u8]) -> OsString {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.into_os_string()
}

fn run_report(paths: &[OsString], max_input_bytes: u64, limits: &Limits) -> Vec<u8> {
    let mut output = Vec::new();
    assert!(report::run(paths.iter().cloned(), &mut output, max_input_bytes, limits).is_ok());
    output
}

#[test]
fn reports_exact_samples_shape_and_metadata_in_argument_order() {
    let fixtures = FixtureDir::new("success");
    let dir = fixtures.path();
    let original = integer_raw();
    let integer = write_fixture(dir, "integer.dng", &original, ByteOrder::Little);
    let float = write_fixture(dir, "float.dng", &float_raw(), ByteOrder::Big);

    let mut changed_sample = original.clone();
    let RawData::U16(samples) = &mut changed_sample.data else {
        panic!("integer fixture must contain integer samples");
    };
    samples[127] += 1;
    let changed_sample = write_fixture(dir, "changed-sample.dng", &changed_sample, ByteOrder::Little);

    let mut rotated = original;
    rotated.orientation = Orientation::Normal;
    let rotated = write_fixture(dir, "changed-orientation.dng", &rotated, ByteOrder::Little);

    let paths = [integer.clone(), float, changed_sample, rotated, integer];
    let limits = Limits::default();
    let first = run_report(&paths, report::MAX_INPUT_BYTES, &limits);
    assert_eq!(first, run_report(&paths, report::MAX_INPUT_BYTES, &limits));

    // The digests were generated independently from the known little-endian u16 and f32 bytes.
    let rows = concat!(
        "1\t1\tok\tdng\tu16\t256\ta1894fbab0b9500f55baa534f974a5c943578668482a5d3cb8fa4a7b5267deee\t16\t16\t1\t16\t2,2,0,0:0,1,1,2\t2,1,12,14\t3,2,4,8\t6\t3fe0000000000000\n",
        "1\t2\tok\tdng\tf32\t768\tc50a4d270d6140ad9988fd8002c5b5de4a78b0b8d598b4db1a5a5385bd4119fd\t16\t16\t3\t32\t\t2,1,12,14\t3,2,4,8\t6\t3fe0000000000000\n",
        "1\t3\tok\tdng\tu16\t256\t3ebf95a27aa36848dc2fc39f537625913e29fd1d390c41b50a527f27f1d5ce91\t16\t16\t1\t16\t2,2,0,0:0,1,1,2\t2,1,12,14\t3,2,4,8\t6\t3fe0000000000000\n",
        "1\t4\tok\tdng\tu16\t256\ta1894fbab0b9500f55baa534f974a5c943578668482a5d3cb8fa4a7b5267deee\t16\t16\t1\t16\t2,2,0,0:0,1,1,2\t2,1,12,14\t3,2,4,8\t1\t3fe0000000000000\n",
        "1\t5\tok\tdng\tu16\t256\ta1894fbab0b9500f55baa534f974a5c943578668482a5d3cb8fa4a7b5267deee\t16\t16\t1\t16\t2,2,0,0:0,1,1,2\t2,1,12,14\t3,2,4,8\t6\t3fe0000000000000\n",
    );
    assert_eq!(String::from_utf8(first).unwrap(), format!("{HEADER}{rows}"));
}

fn report_rows(text: &str) -> Vec<Vec<&str>> {
    text.strip_prefix(HEADER)
        .unwrap()
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 16);
            fields
        })
        .collect()
}

#[test]
fn reports_mixed_failures_and_keeps_the_last_success() {
    let fixtures = FixtureDir::new("mixed");
    let dir = fixtures.path();
    let integer = write_fixture(dir, "integer.dng", &integer_raw(), ByteOrder::Little);
    let unreadable = dir.join("missing.dng").into_os_string();
    let text = write_bytes(dir, "private-fixture-name.txt", b"not a recognized RAW header");

    let mut unsupported = base_ifd(8, 6, 16, 2, false);
    unsupported.set_image(ImageData::Strips { rows_per_strip: 6, strips: vec![vec![0; 8 * 6 * 2 * 2]] });
    let unsupported = write_bytes(dir, "unsupported.dng", &dng(unsupported, ByteOrder::Little));

    let mut corrupt = base_ifd(8, 6, 16, 1, true);
    corrupt.set(tags::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 2]));
    corrupt.set_image(ImageData::Strips { rows_per_strip: 6, strips: vec![vec![0; 8 * 6 * 2]] });
    let corrupt = write_bytes(dir, "corrupt.dng", &dng(corrupt, ByteOrder::Little));

    let missing_strips = dng(base_ifd(8, 6, 16, 1, true), ByteOrder::Little);
    let tiff_error = write_bytes(dir, "missing-strips.dng", &missing_strips);
    let paths = [integer.clone(), unreadable, text, unsupported, corrupt, tiff_error, integer];
    let limits = Limits::default();
    let first = run_report(&paths, report::MAX_INPUT_BYTES, &limits);
    assert_eq!(first, run_report(&paths, report::MAX_INPUT_BYTES, &limits));

    let text = String::from_utf8(first).unwrap();
    assert!(!text.contains("private-fixture-name"));
    assert!(!text.contains("CFAPattern size mismatch"));
    let rows = report_rows(&text);
    let statuses = ["ok", "read_error", "not_raw", "unsupported", "corrupt", "tiff_error", "ok"];
    assert_eq!(rows.len(), statuses.len());
    for (index, (row, status)) in rows.iter().zip(statuses).enumerate() {
        assert_eq!(row[0], "1");
        assert_eq!(row[1], (index + 1).to_string());
        assert_eq!(row[2], status);
        if status != "ok" {
            assert!(row[3..].iter().all(|field| field.is_empty()));
        }
    }
    assert_eq!(rows[0][2..], rows[6][2..]);
}

#[test]
fn separates_encoded_file_and_shared_decoder_limits() {
    let fixtures = FixtureDir::new("limits");
    let dir = fixtures.path();
    let integer = write_fixture(dir, "integer.dng", &integer_raw(), ByteOrder::Little);
    let size = std::fs::metadata(Path::new(&integer)).unwrap().len();
    let paths = [integer];
    for (max_input_bytes, max_samples, status) in [(size, 256, "ok"), (size - 1, 256, "input_limit"), (size, 255, "decode_limit")] {
        let mut limits = Limits::default();
        limits.max_samples = max_samples;
        let output = String::from_utf8(run_report(&paths, max_input_bytes, &limits)).unwrap();
        let rows = report_rows(&output);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][..3], ["1", "1", status]);
        if status != "ok" {
            assert!(rows[0][3..].iter().all(|field| field.is_empty()));
        }
    }
}

#[test]
fn empty_arguments_report_usage_without_writing_a_header() {
    let mut output = Vec::new();
    let result = report::run([], &mut output, report::MAX_INPUT_BYTES, &Limits::default());
    assert!(matches!(result, Err(report::RunError::Usage)));
    assert!(output.is_empty());
}

struct BrokenOutput {
    write_error: bool,
}

impl Write for BrokenOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.write_error { Err(io::Error::from(io::ErrorKind::BrokenPipe)) } else { Ok(bytes.len()) }
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::WriteZero))
    }
}

#[test]
fn write_and_flush_errors_fail_the_invocation() {
    let fixtures = FixtureDir::new("output-error");
    let dir = fixtures.path();
    let integer = write_fixture(dir, "integer.dng", &integer_raw(), ByteOrder::Little);
    for (write_error, kind) in [(true, io::ErrorKind::BrokenPipe), (false, io::ErrorKind::WriteZero)] {
        let mut out = BrokenOutput { write_error };
        let result = report::run([integer.clone()], &mut out, report::MAX_INPUT_BYTES, &Limits::default());
        assert!(matches!(result, Err(report::RunError::Output(error)) if error.kind() == kind));
    }
}
