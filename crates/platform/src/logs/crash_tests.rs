//! Behaviour tests of `logs::crash`.
use super::*;

/// The hook, its crash log directory and the last-panic slot are process-wide; trials that
/// install and panic take turns.
#[cfg(not(target_arch = "wasm32"))]
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    lock(&SERIAL)
}

// A panic on a named worker thread is kept for take_last with its message and thread, and appended to the crash log.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_worker_panic_is_kept_and_written_to_the_crash_log() {
    let _serial = serial();
    let dir = std::env::temp_dir().join(format!("craft-crash-{}", std::process::id()));
    install("CrashTest", "1.2.3", Some(dir.clone()));
    let _ = take_last();
    let joined = std::thread::Builder::new().name("crash-test-worker".into()).spawn(|| panic!("boom in a worker")).expect("spawn").join();
    assert!(joined.is_err());
    let last = take_last().expect("the hook kept the panic");
    assert!(last.contains("boom in a worker") && last.contains("thread crash-test-worker"), "{last}");
    assert!(take_last().is_none(), "take_last empties the slot");
    let path = log_path().expect("crash log path");
    assert_eq!(path.parent(), Some(dir.as_path()));
    let log = std::fs::read_to_string(&path).expect("crash log");
    assert!(log.contains("=== CrashTest 1.2.3 (unix ") && log.contains("boom in a worker"), "{log}");
    record("window could not be created");
    assert!(std::fs::read_to_string(&path).expect("crash log").contains("window could not be created"));
    let _ = std::fs::remove_dir_all(dir);
}

// Text a panic quotes between backticks (std quotes the string it was slicing, which can be the
// user's writing) reaches neither take_last, the crash log nor the Panicked that catch returns;
// an unpaired backtick hides the rest, a message without backticks is unchanged, and catch still
// leaves a quoted panic out of take_last.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_panic_report_leaves_out_quoted_text() {
    let _serial = serial();
    let dir = std::env::temp_dir().join(format!("craft-crash-quoted-{}", std::process::id()));
    install("CrashTest", "1.2.3", Some(dir.clone()));
    let _ = take_last();
    let secret = "Dear Ms Smith";
    let joined = std::thread::Builder::new()
        .name("crash-quoted-worker".into())
        .spawn(move || panic!("byte index 5 is not a char boundary; it is inside 'é' (bytes 4..6) of `{secret}`"))
        .expect("spawn")
        .join();
    assert!(joined.is_err());
    let last = take_last().expect("the hook kept the panic");
    assert!(last.starts_with("byte index 5 is not a char boundary") && last.contains("of `…` (at "), "{last}");
    assert!(!last.contains(secret), "{last}");
    let log = std::fs::read_to_string(log_path().expect("crash log path")).expect("crash log");
    assert!(log.contains("of `…` (at ") && !log.contains(secret), "{log}");

    let caught = catch(|| -> u8 { panic!("of `{secret}` and `more`") });
    let message = caught.expect_err("the closure panicked").message;
    assert!(message.starts_with("of `…`") && message.contains(" and "), "the text outside the backticks stays: {message}");
    assert!(!message.contains(secret) && !message.contains("more"), "{message}");
    assert!(take_last().is_none(), "a caught quoted panic is reported once, by the caller");
    let caught = catch(|| -> u8 { panic!("of `half a sentence that {secret} wrote") });
    let message = caught.expect_err("the closure panicked").message;
    assert!(
        message.starts_with("of `") && !message.contains("half a sentence") && !message.contains(secret),
        "an unpaired backtick hides the rest: {message}"
    );
    assert!(take_last().is_none());
    let plain = "index out of bounds: the len is 3 but the index is 5";
    let caught = catch(|| -> u8 { panic!("{plain}") });
    assert_eq!(caught.map_err(|p| p.message), Err(plain.to_owned()), "a message without backticks is unchanged");
    assert!(take_last().is_none());
    let _ = std::fs::remove_dir_all(dir);
}
