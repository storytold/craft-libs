use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use craft_xtask::fetch::{ExtractOptions, FetchError, FetchSummary, Pin, Transport, extract, fetch_archive_with, fetch_pinned_with, sha256_hex};
use tempfile::TempDir;

/// A folder under the system temporary folder, created with a unique name and removed when
/// dropped.
struct Scratch(PathBuf, TempDir);

impl Scratch {
    fn new(label: &str) -> Self {
        let dir = tempfile::Builder::new().prefix(&format!("craft-xtask-{label}-")).tempdir().unwrap();
        Self(dir.path().to_path_buf(), dir)
    }

    /// Every name under the folder, sorted, with `/` between folders.
    fn listing(&self) -> Vec<String> {
        fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
                if entry.file_type().unwrap().is_dir() {
                    walk(&entry.path(), &format!("{name}/"), out);
                }
                out.push(name);
            }
        }
        let mut out = Vec::new();
        walk(self.1.path(), "", &mut out);
        out.sort();
        out
    }
}

/// What the fake server does for one address.
enum Reply {
    Body(Vec<u8>),
    /// Sends these bytes, then fails as a dropped connection does.
    Cut(Vec<u8>),
    /// Sends nothing and refuses the body for its advertised size, as `curl --max-filesize` does.
    Refused,
}

/// An in-process transport: a table from address to reply, with a count of requests.
#[derive(Default)]
struct Fake {
    replies: HashMap<String, Reply>,
    calls: Cell<usize>,
}

impl Fake {
    fn with(mut self, url: &str, reply: Reply) -> Self {
        self.replies.insert(url.to_string(), reply);
        self
    }
}

impl Transport for Fake {
    fn download(&self, url: &str, _max_bytes: u64, out: &mut dyn Write) -> io::Result<()> {
        self.calls.set(self.calls.get() + 1);
        match self.replies.get(url) {
            // Written in small pieces, as a response of unknown length arrives.
            Some(Reply::Body(bytes)) => bytes.chunks(7).try_for_each(|piece| out.write_all(piece)),
            Some(Reply::Cut(bytes)) => {
                out.write_all(bytes)?;
                Err(io::Error::new(io::ErrorKind::ConnectionReset, "connection reset"))
            }
            Some(Reply::Refused) => Err(io::Error::new(io::ErrorKind::FileTooLarge, "maximum file size exceeded")),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "404")),
        }
    }
}

const URL: &str = "https://example.com/corpus/a.png";

fn pin<'a>(name: &'a str, sha256: &'a str) -> Pin<'a> {
    Pin { name, url: URL, sha256, max_bytes: 1024 }
}

#[test]
fn downloads_a_pinned_file_and_reuses_it_on_the_next_run() {
    let dir = Scratch::new("reuse");
    let body = b"pinned corpus bytes".to_vec();
    let sha = sha256_hex(&body);
    let fake = Fake::default().with(URL, Reply::Body(body.clone()));
    let pins = [pin("png/a.png", &sha)];

    let first = fetch_pinned_with(&dir.0, &pins, &fake).unwrap();
    assert_eq!(first, FetchSummary { reused: 0, downloaded: 1 });
    assert_eq!(fs::read(dir.0.join("png/a.png")).unwrap(), body);

    let second = fetch_pinned_with(&dir.0, &pins, &fake).unwrap();
    assert_eq!(second, FetchSummary { reused: 1, downloaded: 0 });
    assert_eq!(fake.calls.get(), 1);
    assert_eq!(dir.listing(), ["png", "png/a.png"]);
}

#[test]
fn a_file_with_another_hash_is_downloaded_again() {
    let dir = Scratch::new("repair");
    let body = b"the right bytes".to_vec();
    let sha = sha256_hex(&body).to_ascii_uppercase();
    fs::write(dir.0.join("a.png"), b"damaged").unwrap();
    let fake = Fake::default().with(URL, Reply::Body(body.clone()));

    let summary = fetch_pinned_with(&dir.0, &[pin("a.png", &sha)], &fake).unwrap();
    assert_eq!(summary.downloaded, 1);
    assert_eq!(fs::read(dir.0.join("a.png")).unwrap(), body);
}

#[test]
fn a_file_over_its_limit_is_downloaded_again() {
    let dir = Scratch::new("oversized-cache");
    let body = b"small".to_vec();
    let sha = sha256_hex(&body);
    fs::write(dir.0.join("a.png"), vec![b'x'; 2048]).unwrap();
    let fake = Fake::default().with(URL, Reply::Body(body.clone()));

    fetch_pinned_with(&dir.0, &[pin("a.png", &sha)], &fake).unwrap();
    assert_eq!(fs::read(dir.0.join("a.png")).unwrap(), body);
}

#[test]
fn a_download_with_the_wrong_hash_leaves_no_file() {
    let dir = Scratch::new("mismatch");
    let fake = Fake::default().with(URL, Reply::Body(b"tampered".to_vec()));
    let wanted = sha256_hex(b"original");

    let err = fetch_pinned_with(&dir.0, &[pin("a.png", &wanted)], &fake).unwrap_err();
    assert!(matches!(&err, FetchError::HashMismatch { expected, actual, .. } if *expected == wanted && *actual == sha256_hex(b"tampered")), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_damaged_file_is_deleted_even_when_its_download_fails() {
    let dir = Scratch::new("delete-bad");
    fs::write(dir.0.join("a.png"), b"damaged").unwrap();
    let fake = Fake::default();

    let err = fetch_pinned_with(&dir.0, &[pin("a.png", &sha256_hex(b"right"))], &fake).unwrap_err();
    assert!(matches!(err, FetchError::Download { .. }), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_response_of_unknown_length_stops_at_the_limit() {
    let dir = Scratch::new("limit");
    let body = vec![b'z'; 1025];
    let fake = Fake::default().with(URL, Reply::Body(body.clone()));

    let err = fetch_pinned_with(&dir.0, &[pin("a.png", &sha256_hex(&body))], &fake).unwrap_err();
    assert!(matches!(&err, FetchError::TooLarge { limit, .. } if limit == "1024 bytes"), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_body_the_transport_refuses_for_its_size_is_too_large() {
    let dir = Scratch::new("refused");
    let fake = Fake::default().with(URL, Reply::Refused);
    let sha = sha256_hex(b"body");

    let err = fetch_pinned_with(&dir.0, &[pin("a.png", &sha)], &fake).unwrap_err();
    assert!(matches!(&err, FetchError::TooLarge { limit, .. } if limit == "1024 bytes"), "{err}");
    let archive = Pin { name: "t.tar", url: URL, sha256: &sha, max_bytes: 1024 };
    let err = fetch_archive_with(&dir.0, &archive, &ExtractOptions::default(), &fake).unwrap_err();
    assert!(matches!(&err, FetchError::TooLarge { limit, .. } if limit == "1024 bytes"), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_response_of_exactly_the_limit_is_accepted() {
    let dir = Scratch::new("exact");
    let body = vec![b'z'; 1024];
    let fake = Fake::default().with(URL, Reply::Body(body.clone()));

    fetch_pinned_with(&dir.0, &[pin("a.png", &sha256_hex(&body))], &fake).unwrap();
    assert_eq!(fs::read(dir.0.join("a.png")).unwrap().len(), 1024);
}

#[test]
fn an_interrupted_transfer_leaves_no_file() {
    let dir = Scratch::new("cut");
    let body = b"complete body".to_vec();
    let fake = Fake::default().with(URL, Reply::Cut(body[..5].to_vec()));

    let err = fetch_pinned_with(&dir.0, &[pin("a.png", &sha256_hex(&body))], &fake).unwrap_err();
    assert!(matches!(err, FetchError::Download { .. }), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn bad_pins_are_refused_before_any_download() {
    let dir = Scratch::new("invalid");
    let sha = sha256_hex(b"x");
    let fake = Fake::default().with(URL, Reply::Body(b"x".to_vec()));
    let named = |name| vec![pin(name, &sha)];
    let mut cases: Vec<Vec<Pin<'_>>> = [
        "",
        "../a.png",
        "a/../b.png",
        "/etc/passwd",
        "a//b.png",
        "a/",
        "./a.png",
        "a\\b.png",
        "C:a.png",
        "nul",
        "Com1.txt",
        "COM\u{b9}.txt",
        "lpt\u{b3}",
        "a.png.",
        "a.png ",
        "a\u{7}.png",
    ]
    .into_iter()
    .map(named)
    .collect();
    cases.push(vec![pin("a.png", &sha), pin("A.PNG", &sha)]);
    cases.push(vec![pin("a", &sha), pin("a/b.png", &sha)]);
    cases.push(vec![pin("a/b.png", &sha), pin("A", &sha)]);
    cases.push(vec![pin("Png/a.png", &sha), pin("png/b.png", &sha)]);
    // `Σ`, `σ` and `ς` are one letter in different case, though `ς` is not the lowercase of `Σ`.
    cases.push(vec![pin("\u{3a3}.png", &sha), pin("\u{3c2}.png", &sha)]);
    cases.push(vec![pin("\u{3c2}/a.png", &sha), pin("\u{3a3}/b.png", &sha)]);
    // A composed `é` and `e` followed by a combining acute accent are one name on macOS.
    cases.push(vec![pin("\u{e9}.png", &sha), pin("e\u{301}.png", &sha)]);
    cases.push(vec![pin("\u{e9}/a.png", &sha), pin("E\u{301}/b.png", &sha)]);
    // `α` with U+0345 and U+0301 in either order is one name once composed.
    cases.push(vec![pin("\u{3b1}\u{345}\u{301}.png", &sha), pin("\u{3b1}\u{301}\u{345}.png", &sha)]);
    cases.push(vec![pin("\u{3b1}\u{345}\u{301}/a.png", &sha), pin("\u{3b1}\u{301}\u{345}/b.png", &sha)]);
    cases.push(vec![Pin { url: "file:///etc/passwd", ..pin("a.png", &sha) }]);
    cases.push(vec![Pin { url: "http://example.com/a.png", ..pin("a.png", &sha) }]);
    cases.push(vec![Pin { url: "https:///a.png", ..pin("a.png", &sha) }]);
    cases.push(vec![Pin { url: "https://example.com/a b.png", ..pin("a.png", &sha) }]);
    cases.push(vec![pin("a.png", "abc")]);
    let not_hex = "g".repeat(64);
    cases.push(vec![pin("a.png", &not_hex)]);
    cases.push(vec![Pin { max_bytes: 0, ..pin("a.png", &sha) }]);

    for pins in &cases {
        let err = fetch_pinned_with(&dir.0, pins, &fake).unwrap_err();
        assert!(matches!(err, FetchError::Invalid { .. }), "{pins:?}: {err}");
    }
    assert_eq!(fake.calls.get(), 0);
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_destination_that_is_a_file_is_refused_before_any_download() {
    let dir = Scratch::new("file-dest");
    let dest = dir.0.join("cache");
    fs::write(&dest, b"not a folder").unwrap();
    let body = b"x".to_vec();
    let sha = sha256_hex(&body);
    let fake = Fake::default().with(URL, Reply::Body(body));

    let err = fetch_pinned_with(&dest, &[pin("a.png", &sha)], &fake).unwrap_err();
    assert!(matches!(err, FetchError::NotRegular(_)), "{err}");
    assert_eq!(fake.calls.get(), 0);
    assert_eq!(fs::read(&dest).unwrap(), b"not a folder");
}

#[test]
fn a_bad_archive_name_is_refused_before_the_download() {
    let dir = Scratch::new("archive-name");
    let archive = gzip(&tarball(&[("a.png", b'0', b"aaa")]));
    let sha = sha256_hex(&archive);
    let fake = Fake::default().with(URL, Reply::Body(archive.clone()));
    for name in ["", "suite\n.tar.gz", "suite\u{1b}[0m.tar.gz"] {
        let pin = Pin { name, url: URL, sha256: &sha, max_bytes: archive.len() as u64 };
        let err = fetch_archive_with(&dir.0, &pin, &ExtractOptions::default(), &fake).unwrap_err();
        assert!(matches!(err, FetchError::Invalid { .. }), "{name:?}: {err}");
    }
    assert_eq!(fake.calls.get(), 0);
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_folder_at_a_file_name_is_refused() {
    let dir = Scratch::new("folder-target");
    fs::create_dir(dir.0.join("a.png")).unwrap();
    let fake = Fake::default().with(URL, Reply::Body(b"x".to_vec()));

    let err = fetch_pinned_with(&dir.0, &[pin("a.png", &sha256_hex(b"x"))], &fake).unwrap_err();
    assert!(matches!(err, FetchError::NotRegular(_)), "{err}");
    assert_eq!(fake.calls.get(), 0);
}

#[test]
fn a_file_at_a_folder_name_is_refused() {
    let dir = Scratch::new("file-parent");
    fs::write(dir.0.join("png"), b"not a folder").unwrap();
    let fake = Fake::default().with(URL, Reply::Body(b"x".to_vec()));

    let err = fetch_pinned_with(&dir.0, &[pin("png/a.png", &sha256_hex(b"x"))], &fake).unwrap_err();
    assert!(matches!(err, FetchError::NotRegular(_)), "{err}");
}

#[cfg(unix)]
#[test]
fn links_under_the_destination_are_refused() {
    let dir = Scratch::new("links");
    let outside = Scratch::new("links-outside");
    std::os::unix::fs::symlink(&outside.0, dir.0.join("png")).unwrap();
    std::os::unix::fs::symlink(outside.0.join("b.png"), dir.0.join("b.png")).unwrap();
    let fake = Fake::default().with(URL, Reply::Body(b"x".to_vec()));
    let sha = sha256_hex(b"x");

    for name in ["png/a.png", "b.png"] {
        let err = fetch_pinned_with(&dir.0, &[pin(name, &sha)], &fake).unwrap_err();
        assert!(matches!(err, FetchError::NotRegular(_)), "{name}: {err}");
    }
    let err = fetch_pinned_with(&dir.0.join("png"), &[pin("a.png", &sha)], &fake).unwrap_err();
    assert!(matches!(err, FetchError::NotRegular(_)), "linked destination: {err}");
    let err = extract(&dir.0.join("png"), "t.tar", &tarball(&[("a.png", b'0', b"1")]), &ExtractOptions::default()).unwrap_err();
    assert!(matches!(err, FetchError::NotRegular(_)), "linked destination: {err}");
    // A trailing separator or `.` makes the system follow the link, so they are checked without it.
    for spelling in ["png/", "png//", "png/."] {
        let dest = PathBuf::from(format!("{}/{spelling}", dir.0.display()));
        let err = fetch_pinned_with(&dest, &[pin("a.png", &sha)], &fake).unwrap_err();
        assert!(matches!(err, FetchError::NotRegular(_)), "{spelling}: {err}");
        let err = extract(&dest, "t.tar", &tarball(&[("a.png", b'0', b"1")]), &ExtractOptions::default()).unwrap_err();
        assert!(matches!(err, FetchError::NotRegular(_)), "{spelling}: {err}");
    }
    assert_eq!(fake.calls.get(), 0);
    assert!(outside.listing().is_empty(), "{:?}", outside.listing());
}

/// A tar header for `name`, written byte for byte so that names the `tar` crate's builder refuses
/// (`..`, absolute paths) can be tested.
fn header(name: &str, kind: u8, size: usize) -> tar::Header {
    let mut header = tar::Header::new_ustar();
    {
        let bytes = header.as_mut_bytes();
        bytes[..100].fill(0);
        bytes[..name.len()].copy_from_slice(name.as_bytes());
    }
    header.set_size(size as u64);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::new(kind));
    header.set_cksum();
    header
}

fn tarball(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, kind, data) in entries {
        builder.append(&header(name, *kind, data.len()), *data).unwrap();
    }
    builder.into_inner().unwrap()
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn an_archive_is_checked_against_its_pin_and_extracted() {
    let dir = Scratch::new("archive");
    let archive = gzip(&tarball(&[
        ("suite-1/", b'5', b""),
        ("suite-1/png/", b'5', b""),
        ("suite-1/png/a.png", b'0', b"aaa"),
        ("suite-1/LICENSE", b'0', b"license text"),
        ("suite-1/empty/", b'5', b""),
    ]));
    let sha = sha256_hex(&archive);
    let fake = Fake::default().with(URL, Reply::Body(archive.clone()));
    let mut options = ExtractOptions::default();
    options.strip_components = 1;
    let pin = Pin { name: "suite.tar.gz", url: URL, sha256: &sha, max_bytes: archive.len() as u64 };

    let written = fetch_archive_with(&dir.0, &pin, &options, &fake).unwrap();
    assert_eq!(written, ["png/a.png", "LICENSE"]);
    assert_eq!(dir.listing(), ["LICENSE", "empty", "png", "png/a.png"]);
    assert_eq!(fs::read(dir.0.join("png/a.png")).unwrap(), b"aaa");

    // A second run replaces the files with the same bytes.
    fs::write(dir.0.join("LICENSE"), b"edited").unwrap();
    fetch_archive_with(&dir.0, &pin, &options, &fake).unwrap();
    assert_eq!(fs::read(dir.0.join("LICENSE")).unwrap(), b"license text");
}

#[test]
fn an_archive_with_the_wrong_hash_or_size_writes_nothing() {
    let dir = Scratch::new("archive-pin");
    let archive = tarball(&[("a.png", b'0', b"aaa")]);
    let fake = Fake::default().with(URL, Reply::Body(archive.clone()));
    let sha = sha256_hex(&archive);
    let wrong = sha256_hex(b"other");

    let err = fetch_archive_with(&dir.0, &Pin { name: "t.tar", url: URL, sha256: &wrong, max_bytes: 1 << 20 }, &ExtractOptions::default(), &fake)
        .unwrap_err();
    assert!(matches!(err, FetchError::HashMismatch { .. }), "{err}");
    let small = Pin { name: "t.tar", url: URL, sha256: &sha, max_bytes: archive.len() as u64 - 1 };
    let err = fetch_archive_with(&dir.0, &small, &ExtractOptions::default(), &fake).unwrap_err();
    assert!(matches!(err, FetchError::TooLarge { .. }), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn unsafe_archive_entries_write_nothing() {
    let cases: [&[(&str, u8, &[u8])]; 22] = [
        &[("ok.png", b'0', b"1"), ("file.png/", b'0', b"2")],
        &[("ok.png", b'0', b"1"), ("foo\\bar.png", b'0', b"2")],
        &[("ok.png", b'0', b"1"), ("foo//", b'5', b"")],
        &[("ok.png", b'0', b"1"), ("foo//bar.png", b'0', b"2")],
        &[("Foo/", b'5', b""), ("foo/", b'5', b"")],
        &[("Foo/a.png", b'0', b"1"), ("foo/b.png", b'0', b"2")],
        &[("ok.png", b'0', b"1"), ("../evil.png", b'0', b"2")],
        &[("ok.png", b'0', b"1"), ("a/../../evil.png", b'0', b"2")],
        &[("ok.png", b'0', b"1"), ("/etc/evil", b'0', b"2")],
        &[("ok.png", b'0', b"1"), ("link", b'2', b"")],
        &[("ok.png", b'0', b"1"), ("hard", b'1', b"")],
        &[("ok.png", b'0', b"1"), ("dev", b'3', b"")],
        &[("ok.png", b'0', b"1"), ("fifo", b'6', b"")],
        &[("ok.png", b'0', b"1"), ("OK.PNG", b'0', b"2")],
        &[("ok.png", b'0', b"1"), ("ok.png/inner", b'0', b"2")],
        &[("\u{3a3}.txt", b'0', b"1"), ("\u{3c2}.txt", b'0', b"2")],
        &[("\u{3a3}/", b'5', b""), ("\u{3c2}/", b'5', b"")],
        &[("\u{3c3}/a.txt", b'0', b"1"), ("\u{3c2}/b.txt", b'0', b"2")],
        &[("\u{e9}.txt", b'0', b"1"), ("e\u{301}.txt", b'0', b"2")],
        &[("e\u{301}/a.txt", b'0', b"1"), ("\u{c9}/b.txt", b'0', b"2")],
        &[("\u{3b1}\u{345}\u{301}.txt", b'0', b"1"), ("\u{3b1}\u{301}\u{345}.txt", b'0', b"2")],
        &[("\u{3b1}\u{345}\u{301}/a.txt", b'0', b"1"), ("\u{3b1}\u{301}\u{345}/b.txt", b'0', b"2")],
    ];
    for entries in cases {
        let dir = Scratch::new("unsafe");
        let err = extract(&dir.0, "t.tar", &tarball(entries), &ExtractOptions::default()).unwrap_err();
        assert!(matches!(err, FetchError::Archive { .. }), "{entries:?}: {err}");
        assert!(dir.listing().is_empty(), "{entries:?}: {:?}", dir.listing());
    }
}

#[test]
fn archive_limits_write_nothing() {
    let three = tarball(&[("a", b'0', b"12"), ("b", b'0', b"34"), ("c", b'0', b"56")]);
    let mut entries = ExtractOptions::default();
    entries.max_entries = 2;
    let mut entry = ExtractOptions::default();
    entry.max_entry_bytes = 1;
    let mut total = ExtractOptions::default();
    total.max_expanded_bytes = 5;
    for options in [entries, entry, total] {
        let dir = Scratch::new("limits");
        let err = extract(&dir.0, "t.tar", &three, &options).unwrap_err();
        assert!(matches!(err, FetchError::TooLarge { .. }), "{options:?}: {err}");
        assert!(dir.listing().is_empty(), "{options:?}: {:?}", dir.listing());
    }
    // A file that `strip_components` leaves out still counts against the limits.
    let mut stripped = ExtractOptions::default();
    stripped.max_entry_bytes = 1;
    stripped.strip_components = 1;
    let dir = Scratch::new("limits-stripped");
    let archive = tarball(&[("top", b'0', b"12"), ("d/a", b'0', b"1")]);
    let err = extract(&dir.0, "t.tar", &archive, &stripped).unwrap_err();
    assert!(matches!(err, FetchError::TooLarge { .. }), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
    let dir = Scratch::new("limits-exact");
    let mut exact = ExtractOptions::default();
    exact.max_entries = 3;
    exact.max_entry_bytes = 2;
    exact.max_expanded_bytes = 6;
    assert_eq!(extract(&dir.0, "t.tar", &three, &exact).unwrap(), ["a", "b", "c"]);
}

#[test]
fn a_compressed_archive_that_expands_past_its_limit_is_refused() {
    // A long-name record of 48 MiB compresses to a few KiB and is not a file, so only the bound on
    // the decompressed tar data stops it.
    let long = vec![b'a'; 48 << 20];
    let mut builder = tar::Builder::new(Vec::new());
    builder.append(&header("././@LongLink", b'L', long.len()), long.as_slice()).unwrap();
    builder.append(&header("a.png", b'0', 1), &b"1"[..]).unwrap();
    let archive = gzip(&builder.into_inner().unwrap());
    assert!(archive.len() < 1 << 20);

    let dir = Scratch::new("bomb");
    let err = extract(&dir.0, "t.tar.gz", &archive, &ExtractOptions::default()).unwrap_err();
    assert!(matches!(err, FetchError::TooLarge { .. }), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn bytes_after_the_tar_end_marker_count_against_the_limit() {
    let mut archive = tarball(&[("a", b'0', b"1")]);
    let mut options = ExtractOptions::default();
    options.max_entries = 1;
    options.max_entry_bytes = 1;
    options.max_expanded_bytes = 1;
    // The limit is 1 byte plus 12 KiB for each of 2 entries.
    archive.resize(1 + 2 * 12 * 1024, 0);
    let dir = Scratch::new("trailing");
    assert_eq!(extract(&dir.0, "t.tar", &archive, &options).unwrap(), ["a"]);
    archive.push(0);
    let dir = Scratch::new("trailing-over");
    let err = extract(&dir.0, "t.tar", &archive, &options).unwrap_err();
    assert!(matches!(err, FetchError::TooLarge { .. }), "{err}");
    assert!(dir.listing().is_empty(), "{:?}", dir.listing());
}

#[test]
fn a_compressed_archive_with_a_damaged_trailer_writes_nothing() {
    let archive = gzip(&tarball(&[("a.png", b'0', b"1")]));
    let mut bad_crc = archive.clone();
    let crc = bad_crc.len() - 8;
    bad_crc[crc] ^= 0xff;
    let cut = archive[..archive.len() - 4].to_vec();
    for (label, bytes) in [("checksum", bad_crc), ("cut", cut)] {
        let dir = Scratch::new("gzip-trailer");
        let err = extract(&dir.0, "t.tar.gz", &bytes, &ExtractOptions::default()).unwrap_err();
        assert!(matches!(err, FetchError::Archive { .. }), "{label}: {err}");
        assert!(dir.listing().is_empty(), "{label}: {:?}", dir.listing());
    }
}

#[cfg(unix)]
#[test]
fn an_archive_does_not_write_through_a_link() {
    let dir = Scratch::new("archive-link");
    let outside = Scratch::new("archive-link-outside");
    std::os::unix::fs::symlink(&outside.0, dir.0.join("png")).unwrap();

    let err = extract(&dir.0, "t.tar", &tarball(&[("png/a.png", b'0', b"1")]), &ExtractOptions::default()).unwrap_err();
    assert!(matches!(err, FetchError::NotRegular(_)), "{err}");
    assert!(outside.listing().is_empty(), "{:?}", outside.listing());
}

/// Reads an archive made by the system `tar`, when there is one, so that the reader is checked
/// against another program's output.
#[test]
fn reads_an_archive_made_by_the_system_tar() {
    let source = Scratch::new("system-tar-source");
    fs::create_dir_all(source.0.join("suite/png")).unwrap();
    fs::write(source.0.join("suite/png/basn0g01.png"), b"first").unwrap();
    fs::write(source.0.join("suite/README"), vec![b'r'; 3000]).unwrap();
    let archive = source.0.join("suite.tgz");
    // COPYFILE_DISABLE keeps macOS tar from adding `._` metadata entries; GNU tar reads extra
    // options from TAR_OPTIONS, so it is removed.
    let made = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .env_remove("TAR_OPTIONS")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&source.0)
        .arg("suite")
        .status();
    match made {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            eprintln!("SKIPPED: no system tar");
            return;
        }
        Err(e) => panic!("cannot run tar: {e}"),
        Ok(status) => assert!(status.success(), "tar failed: {status}"),
    }
    let bytes = fs::read(&archive).unwrap();

    let dir = Scratch::new("system-tar");
    let mut written = extract(&dir.0, "suite.tgz", &bytes, &ExtractOptions::default()).unwrap();
    written.sort();
    assert_eq!(written, ["suite/README", "suite/png/basn0g01.png"]);
    assert_eq!(fs::read(dir.0.join("suite/png/basn0g01.png")).unwrap(), b"first");
    assert_eq!(fs::read(dir.0.join("suite/README")).unwrap(), vec![b'r'; 3000]);
}
