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

// Directives follow env_logger: the most specific matching one wins, at `::` boundaries or as a
// `*` prefix, a bare target turns everything on, levels are case-blind; a hostile spec never
// panics and an unusable part is skipped, leaving errors only for the targets it would have named.
#[test]
fn filter_directives_follow_env_logger_and_hostile_specs_never_panic() {
    let f = Filter::parse("info,wgpu_core=error,photocraft_pdf=trace,photocraft_pdf::fonts=off");
    assert_eq!(f.level_for("eframe"), LevelFilter::Info);
    assert_eq!(f.level_for("wgpu_core::instance"), LevelFilter::Error);
    assert_eq!(f.level_for("photocraft_pdf::images"), LevelFilter::Trace);
    assert_eq!(f.level_for("photocraft_pdf::fonts"), LevelFilter::Off, "the longer directive wins");
    assert_eq!(f.max(), LevelFilter::Trace);
    assert_eq!(f.level_for("wgpu_core_extra"), LevelFilter::Info, "a module name matches at `::` boundaries, not as a bare prefix");
    assert_eq!(Filter::parse("photocraft*=debug").level_for("photocraft_layout::lines"), LevelFilter::Debug);
    assert_eq!(Filter::parse("photocraft*=debug").level_for("eframe"), LevelFilter::Error);
    assert_eq!(Filter::parse("naga").level_for("naga::front"), LevelFilter::Trace, "a bare target turns everything from it on");
    assert_eq!(Filter::parse("naga").level_for("eframe"), LevelFilter::Error);
    assert_eq!(Filter::parse(" WARN , photocraft = Debug ").level_for("photocraft"), LevelFilter::Debug);
    assert_eq!(Filter::parse(" WARN , photocraft = Debug ").level_for("eframe"), LevelFilter::Warn);
    let defaults = Filter::parse(&NAMES.default_filter());
    assert_eq!(defaults.level_for("photocraft_gpu::device"), LevelFilter::Info);
    assert_eq!(defaults.level_for("wgpu_core::device"), LevelFilter::Warn);

    let long = "x".repeat(100_000);
    let many = "a=info,".repeat(10_000);
    for spec in
        ["", ",,,", "=", "==", "=debug", "photocraft=", "nonsense=loud", "🦀=info", "*", "*=", "=*", "a=b=c", "\0", "é::ü=trace", &long, &many]
    {
        let f = Filter::parse(spec);
        for target in ["", "photocraft", "🦀", "a::b", "*"] {
            let _ = f.level_for(target);
        }
        let _ = f.max();
    }
    assert_eq!(Filter::parse("warn,,photocraft=DEBUG").level_for("photocraft"), LevelFilter::Debug);
    assert_eq!(Filter::parse("nonsense=loud").level_for("nonsense"), LevelFilter::Error, "an unknown level is skipped");
    assert_eq!(Filter::parse("photocraft=").level_for("photocraft"), LevelFilter::Error);
    assert_eq!(Filter::parse("=debug").max(), LevelFilter::Error, "a directive without a target is skipped");
    assert_eq!(Filter::parse("🦀=info").level_for("🦀::claw"), LevelFilter::Info);
}

// The hand-rolled civil-from-days calendar gives UTC dates with milliseconds at the epoch, leap
// days (2000 is a leap year, 2100 is not), year ends and years past 9999; a clock before 1970
// reads as the epoch.
#[test]
fn timestamps_are_utc_with_milliseconds() {
    use std::time::Duration;
    use web_time::UNIX_EPOCH;
    assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_millis(1_791_446_357_728)), "2026-10-08T07:59:17.728Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_709_164_800)), "2024-02-29T00:00:00.000Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_709_251_199)), "2024-02-29T23:59:59.000Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_709_251_200)), "2024-03-01T00:00:00.000Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_millis(1_735_689_599_999)), "2024-12-31T23:59:59.999Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_735_689_600)), "2025-01-01T00:00:00.000Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29T00:00:00.000Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(4_107_542_400)), "2100-03-01T00:00:00.000Z");
    assert_eq!(timestamp(UNIX_EPOCH - Duration::from_secs(5)), "1970-01-01T00:00:00.000Z");
    assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(253_402_300_800)), "10000-01-01T00:00:00.000Z");
}

// Before attach, lines up to MAX_PENDING are kept and written on attach, the overflow becomes one
// drop note, exactly a full buffer drops nothing, later lines follow, and once no file will come
// nothing is kept.
#[test]
fn the_pending_buffer_is_bounded_and_says_how_much_it_dropped() {
    let dir = temp_dir("bounded");
    let LogFile { path, file, .. } = rotate(&dir, NAMES).expect("rotate");
    let overflow = 5;
    let mut sink = Sink::new(MAX_FILE_BYTES);
    for i in 0..MAX_PENDING + overflow {
        sink.write(&format!("line {i}\n"));
    }
    sink.attach(file);
    sink.write("late\n");
    let text = read(&path);
    assert_eq!(text.lines().filter(|l| l.starts_with("line ")).count(), MAX_PENDING);
    assert_eq!(text.matches("earlier log lines were dropped").count(), 1, "{text}");
    assert!(text.contains(&format!("{overflow} earlier log lines were dropped")), "{text}");
    assert!(text.starts_with("line 0\n") && !text.contains(&format!("line {MAX_PENDING}\n")), "the first lines are kept");
    assert!(text.ends_with("late\n"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);

    let dir = temp_dir("full");
    let LogFile { path, file, .. } = rotate(&dir, NAMES).expect("rotate");
    let mut sink = Sink::new(MAX_FILE_BYTES);
    for i in 0..MAX_PENDING {
        sink.write(&format!("line {i}\n"));
    }
    sink.attach(file);
    let text = read(&path);
    assert_eq!(text.lines().count(), MAX_PENDING);
    assert!(!text.contains("dropped"), "exactly a full buffer drops nothing");
    let _ = std::fs::remove_dir_all(&dir);

    let mut sink = Sink::new(MAX_FILE_BYTES);
    sink.write("early\n");
    sink.close();
    for _ in 0..MAX_PENDING + 1 {
        sink.write("late\n");
    }
    assert!(sink.pending.is_empty() && sink.dropped == 0, "without a file nothing is kept");
}

// A sink attached to a file stops at max_bytes: the line that would pass it is replaced by one
// note and nothing follows.
#[test]
fn the_file_stops_at_its_size_cap_with_one_note() {
    let dir = temp_dir("cap");
    let LogFile { path, file, .. } = rotate(&dir, NAMES).expect("rotate");
    let mut sink = Sink::new(100);
    sink.attach(file);
    for i in 0..50 {
        sink.write(&format!("line {i:02} with some padding\n"));
    }
    let text = read(&path);
    let written: usize = text.lines().filter(|l| l.starts_with("line ")).map(|l| l.len() + 1).sum();
    assert!(written > 0 && written <= 100, "the lines before the note fit the cap: {text}");
    assert!(text.lines().filter(|l| l.starts_with("line ")).count() < 50, "the cap cut the writing off: {text}");
    assert_eq!(text.matches("log file reached").count(), 1, "{text}");
    let after_note = text.lines().skip_while(|l| !l.contains("log file reached")).skip(1).count();
    assert_eq!(after_note, 0, "nothing follows the note: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

// Eight threads logging through one AppLogger leave 800 whole lines in its file, none torn.
#[test]
fn records_from_many_threads_are_whole_lines() {
    use log::Log;
    let dir = temp_dir("threads");
    let logger = std::sync::Arc::new(AppLogger::new(NAMES, Filter::parse("info"), false));
    let path = logger.attach_dir(&dir).expect("attach");
    let workers: Vec<_> = (0..8)
        .map(|t| {
            let l = std::sync::Arc::clone(&logger);
            std::thread::spawn(move || {
                for i in 0..100 {
                    l.log(&log::Record::builder().level(log::Level::Info).target("photocraft").args(format_args!("thread {t} record {i}")).build());
                }
            })
        })
        .collect();
    for w in workers {
        w.join().expect("worker");
    }
    let text = read(&path);
    assert_eq!(text.lines().count(), 800);
    assert!(text.lines().all(|l| l.contains(" INFO  [") && l.contains(" photocraft: thread ") && l.contains(" record ")), "{text}");
    for t in 0..8 {
        assert_eq!(text.matches(&format!(": thread {t} record ")).count(), 100, "thread {t}");
    }
    let _ = std::fs::remove_dir_all(&dir);
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
