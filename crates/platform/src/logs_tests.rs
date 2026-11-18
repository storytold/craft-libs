//! Behaviour tests of `logs`.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const NAMES: LogNames = LogNames::new("photocraft");

#[cfg(not(target_arch = "wasm32"))]
fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("craft-logs-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read log")
}

// Each start shifts app.log to app.1.log and app.2.log; while another instance holds the owner lock, a start writes app-<pid>.log and leaves the chain alone.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rotation_keeps_previous_logs_and_a_running_instance_log() {
    let dir = temp_dir("rotate");
    for run in 1..=4 {
        let LogFile { path, .. } = rotate(&dir, NAMES).expect("rotate");
        assert_eq!(path, dir.join("photocraft.log"));
        assert_eq!(read(&path), "");
        std::fs::write(&path, format!("run {run}")).expect("write");
    }
    assert_eq!(read(&dir.join("photocraft.log")), "run 4");
    assert_eq!(read(&dir.join("photocraft.1.log")), "run 3");
    assert_eq!(read(&dir.join("photocraft.2.log")), "run 2");
    assert!(!dir.join("photocraft.3.log").exists(), "only {KEEP} old logs are kept");

    // A start while the first instance runs leaves its log in place.
    let mut first = rotate(&dir, NAMES).expect("first start");
    first.file.write_all(b"before\n").expect("write");
    for _ in 0..=KEEP {
        let second = rotate(&dir, NAMES).expect("second start");
        assert_eq!(second.path, dir.join(format!("photocraft-{}.log", std::process::id())));
    }
    first.file.write_all(b"after\n").expect("write");
    assert_eq!(read(&dir.join("photocraft.log")), "before\nafter\n");
    assert_eq!(read(&dir.join("photocraft.1.log")), "run 4", "the chain did not move");

    // Once it has exited, the next start rotates again.
    drop(first);
    let third = rotate(&dir, NAMES).expect("third start");
    assert_eq!(third.path, dir.join("photocraft.log"));
    assert_eq!(read(&dir.join("photocraft.1.log")), "before\nafter\n");

    assert_eq!(NAMES.instance_pid("photocraft-123.log"), Some(123));
    assert_eq!(NAMES.instance_pid("photocraft-12a.log"), None);
    assert_eq!(NAMES.instance_pid("photocraft.1.log"), None);
    let _ = std::fs::remove_dir_all(&dir);

    // A file where the directory should be is an error.
    let blocked = temp_dir("blocked");
    std::fs::write(&blocked, "a file").expect("block");
    assert!(rotate(&blocked, NAMES).is_err());
    let _ = std::fs::remove_file(&blocked);
}

// RUST_LOG is user input: the longest matching directive decides, `a` covers `a::b` but not
// `ab`, `a*` covers every target starting with `a`, unknown levels and empty parts are skipped,
// and a hostile spec never panics and leaves the default.
#[test]
fn filter_directives_pick_the_most_specific_match_and_hostile_specs_fall_back() {
    for spec in ["warn,a=info,a::b=trace,c*=debug,d=bogus,,=error,e=,zz", "zz , a::b=trace ,a=info,warn, c*=debug"] {
        let filter = Filter::parse(spec);
        assert_eq!(filter.level_for("a::b::c"), LevelFilter::Trace, "{spec}: the longer directive wins");
        assert_eq!(filter.level_for("a::x"), LevelFilter::Info, "{spec}");
        assert_eq!(filter.level_for("a"), LevelFilter::Info, "{spec}");
        assert_eq!(filter.level_for("ab"), LevelFilter::Warn, "{spec}: `a` does not match `ab`");
        assert_eq!(filter.level_for("a::bc"), LevelFilter::Info, "{spec}: `a::b` does not match `a::bc`");
        assert_eq!((filter.level_for("c"), filter.level_for("cat::x")), (LevelFilter::Debug, LevelFilter::Debug), "{spec}");
        assert_eq!(filter.level_for("zz::q"), LevelFilter::Trace, "{spec}: a bare target turns everything on");
        assert!(filter.level_for("a::b") > filter.level_for("a") && filter.level_for("a") > filter.level_for("other"), "{spec}");
        assert_eq!(filter.max(), LevelFilter::Trace, "{spec}");
    }
    let skipped = Filter::parse("warn,d=bogus,e=,=error");
    assert_eq!((skipped.level_for("d"), skipped.level_for("e"), skipped.level_for("")), (LevelFilter::Warn, LevelFilter::Warn, LevelFilter::Warn));
    let default = Filter::parse("");
    for hostile in ["", "=", ",,", "x=bogus", " = , = ", "=trace"] {
        let filter = Filter::parse(hostile);
        assert_eq!(filter, default, "{hostile:?}");
        assert_eq!((filter.level_for("x"), filter.max()), (LevelFilter::Error, LevelFilter::Error), "{hostile:?}");
    }
}

// The date comes from hand-written civil-from-days arithmetic, which nothing else checks and
// whose mistakes look plausible: leap days, a century year without one, the epoch (and a clock
// before it), and the milliseconds of the instant.
#[test]
fn timestamps_are_utc_with_milliseconds_across_leap_days() {
    use std::time::Duration;
    let at = |millis: u64| timestamp(web_time::UNIX_EPOCH + Duration::from_millis(millis));
    // 2024-02-29T00:00:00Z is 1_709_164_800 s after the epoch.
    assert_eq!(at(1_709_164_800_250), "2024-02-29T00:00:00.250Z");
    assert_eq!(at(1_709_251_199_999), "2024-02-29T23:59:59.999Z");
    assert_eq!(at(1_709_251_200_000), "2024-03-01T00:00:00.000Z", "the day after a leap day is 1 March");
    // 2000 is divisible by 400 and has a 29 February; 2100 is a century year that has none.
    assert_eq!(at(951_782_400_000), "2000-02-29T00:00:00.000Z");
    assert_eq!(at(4_107_542_399_000), "2100-02-28T23:59:59.000Z");
    assert_eq!(at(4_107_542_400_000), "2100-03-01T00:00:00.000Z", "2100 has no 29 February");
    assert_eq!(at(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(at(1), "1970-01-01T00:00:00.001Z");
    let before = web_time::UNIX_EPOCH.checked_sub(Duration::from_secs(5)).unwrap();
    assert_eq!(timestamp(before), "1970-01-01T00:00:00.000Z", "a clock before the epoch reads as the epoch");
}

// Lines logged before the file opens reach it in order, a flood before then keeps its first
// MAX_PENDING lines plus one note that the rest were dropped, and a runaway log stops at its size
// cap with one note and adds nothing after it.
#[test]
fn a_sink_keeps_early_lines_bounded_and_stops_at_its_size_cap() {
    let dir = temp_dir("sink");
    std::fs::create_dir_all(&dir).unwrap();
    let early = dir.join("early.log");
    let mut sink = Sink::new(MAX_FILE_BYTES);
    let extra = 3;
    for i in 0..MAX_PENDING + extra {
        sink.write(&format!("line {i}\n"));
    }
    sink.attach(std::fs::File::create(&early).unwrap());
    sink.write("after\n");
    let text = read(&early);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), MAX_PENDING + 2, "the kept lines, one note, the line after attaching");
    let kept: Vec<String> = (0..MAX_PENDING).map(|i| format!("line {i}")).collect();
    assert_eq!(lines[..MAX_PENDING], kept[..]);
    assert_eq!(lines.iter().filter(|line| line.contains("dropped")).count(), 1);
    assert!(lines[MAX_PENDING].starts_with(&format!("{extra} ")), "{}", lines[MAX_PENDING]);
    assert_eq!(lines[MAX_PENDING + 1], "after");

    let capped = dir.join("capped.log");
    let line = "0123456789\n";
    let max_bytes = u64::try_from(line.len() * 3 / 2).unwrap();
    let mut sink = Sink::new(max_bytes);
    sink.attach(std::fs::File::create(&capped).unwrap());
    sink.write(line);
    sink.write(line);
    let at_cap = read(&capped);
    let lines: Vec<&str> = at_cap.lines().collect();
    assert_eq!(lines.len(), 2, "{at_cap}");
    assert_eq!(lines[0], line.trim_end());
    assert!(lines[1].contains(&max_bytes.to_string()), "one note naming the cap: {at_cap}");
    for _ in 0..3 {
        sink.write(line);
    }
    assert_eq!(read(&capped), at_cap, "nothing follows the note");
    let _ = std::fs::remove_dir_all(&dir);
}
