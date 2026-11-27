//! The window manager's GSettings `button-layout` (GNOME's, Cinnamon's or MATE's schema) read with `gsettings get` and
//! followed with `gsettings monitor`, both documented commands of GLib's `gsettings`. The monitor's output only says
//! that the key changed; each change is read again with `get`, whose output is one GVariant string.
//!
//! Not in a sandbox: there `gsettings` reads the sandbox's own settings, which hold the schema's default, not the
//! user's choice, and the settings portal (`portal.rs`) answers instead. Nor while the portal exports the same key
//! (GNOME's portal does): the source waits for the portal's first answer, at most `PORTAL_FIRST`, and runs only while
//! the portal has none, looking again every `PORTAL_AGAIN`.

use super::super::{LayoutSource, RawLayout, Shared};
use super::{Env, RETRY, back_off};
use craft_host::caption_layout::{LayoutDesktop, LayoutReading, MAX_LAYOUT_BYTES};
use futures_lite::io::AsyncReadExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const KEY: &str = "button-layout";
/// How long one `gsettings get` may take.
const TIMEOUT: Duration = Duration::from_secs(2);
/// A burst of monitor output is one change once it has been quiet this long, or after `BURST_MAX` at most.
const QUIET: Duration = Duration::from_millis(50);
const BURST_MAX: Duration = Duration::from_millis(500);
/// A monitor that ran this long before it ended was working: the next one starts after the shortest wait again.
const STEADY: Duration = Duration::from_secs(60);
/// How long the source waits for the portal's first answer, and how often it looks whether the portal still has one.
const PORTAL_FIRST: Duration = Duration::from_secs(5);
const PORTAL_AGAIN: Duration = Duration::from_secs(60);

/// Follows `desktop`'s GSettings key until the watcher is dropped; returns at once in a sandbox, on a desktop without
/// such a key and where no `gsettings` is found.
pub(super) async fn watch(shared: &Shared, env: &Env, desktop: LayoutDesktop) {
    if env.sandboxed {
        return;
    }
    let (Some(schema), Some(exe)) = (desktop.gsettings_schema(), super::super::portal::gsettings_exe(env.path.as_deref())) else { return };
    let asked = Instant::now() + PORTAL_FIRST;
    while !shared.captions_asked.load(Ordering::Acquire) && Instant::now() < asked {
        async_io::Timer::after(Duration::from_millis(100)).await;
    }
    // The portal's export of the same key is live and ranks first; one process less while it answers.
    while shared.has_caption(LayoutSource::DesktopPortal) {
        async_io::Timer::after(PORTAL_AGAIN).await;
    }
    follow(shared, &exe, schema, LayoutSource::DesktopService, RETRY).await;
}

/// Reads `schema`'s key with `exe` into `source`'s place and reads it again at each change the monitor reports. A
/// monitor that ends (the schema is not installed, the helper fails) is started again after `retry`, doubling.
pub(super) async fn follow(shared: &Shared, exe: &Path, schema: &str, source: LayoutSource, mut retry: Duration) {
    let first = retry;
    loop {
        let started = Instant::now();
        // The monitor starts before the read, so a change between the two is not missed.
        let mut monitor = async_process::Command::new(exe)
            .args(["monitor", schema, KEY])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .reap_on_drop(true)
            .spawn()
            .ok();
        publish(shared, exe, schema, source).await;
        if let Some(mut stdout) = monitor.as_mut().and_then(|child| child.stdout.take()) {
            while changed(&mut stdout).await {
                publish(shared, exe, schema, source).await;
            }
        }
        // Killed and reaped here, whether it ended or not.
        drop(monitor);
        if started.elapsed() >= STEADY {
            retry = first;
        }
        back_off(&mut retry).await;
    }
}

/// Reads the key and publishes it. A read that fails or times out changes nothing: the last answer stays.
async fn publish(shared: &Shared, exe: &Path, schema: &str, source: LayoutSource) {
    let Some(text) = crate::process::capture(exe, &["get", schema, KEY], TIMEOUT).await else { return };
    if let Some(value) = gvariant_string(&text) {
        let origin = format!("gsettings, {schema} {KEY}");
        shared.publish_caption(source, Some(LayoutReading::new(source, RawLayout::Names(value), origin)));
    }
}

/// The string `gsettings get` prints for a string key, quoted (`'menu:minimize,maximize,close'`); `None` for anything
/// else (another type, a longer value than a layout holds).
pub(super) fn gvariant_string(text: &str) -> Option<String> {
    let inner = text.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    (inner.len() <= MAX_LAYOUT_BYTES && !inner.contains(['\'', '\\'])).then(|| inner.to_owned())
}

/// Waits for the monitor's next output and takes the rest of its burst. `false` when the monitor ended (end of output
/// or a failed read).
async fn changed(stdout: &mut (impl futures_lite::AsyncRead + Unpin)) -> bool {
    let mut buffer = [0; 4096];
    if !matches!(stdout.read(&mut buffer).await, Ok(1..)) {
        return false;
    }
    let until = Instant::now() + BURST_MAX;
    while Instant::now() < until {
        let more = futures_lite::future::race(async { Some(stdout.read(&mut buffer).await) }, async {
            async_io::Timer::after(QUIET).await;
            None
        })
        .await;
        // Quiet, or the monitor ended after its output: this change is read now, the end found at the next call.
        if !matches!(more, Some(Ok(1..))) {
            break;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{LayoutSource, RawLayout, changed, follow};
    use crate::appearance::{Shared, Watcher, fake_bus::eventually};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    const SCHEMA: &str = "org.cinnamon.desktop.wm.preferences";

    /// A `gsettings` stand-in in `dir`: `get` prints the quoted contents of `value` (and counts itself in `gets`),
    /// `monitor` writes its pid to `monitor.pid` and runs `monitor` (a shell fragment).
    fn stand_in(dir: &Path, get: &str, monitor: &str) -> std::io::Result<PathBuf> {
        let exe = dir.join("gsettings");
        let script = format!(
            "#!/bin/sh\ndir=\"{dir}\"\ncase \"$1\" in\n  get) echo x >> \"$dir/gets\"; {get} ;;\n  monitor) echo $$ > \"$dir/monitor.pid\"; {monitor} ;;\nesac\n",
            dir = dir.display()
        );
        std::fs::write(&exe, script)?;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
        Ok(exe)
    }

    /// A monitor that shows a change each time `value` changes, once it has read the first value (`monitor.ready`).
    const FOLLOWS: &str = "last=$(cat \"$dir/value\"); echo > \"$dir/monitor.ready\"; while sleep 0.05; do now=$(cat \"$dir/value\"); if [ \"$now\" != \"$last\" ]; then echo \"button-layout: '$now'\"; last=$now; fi; done";

    fn spawn(exe: PathBuf) -> Option<Watcher> {
        Watcher::spawn_shared(Shared::new(false, Box::new(|| {})), move |shared| async move {
            follow(&shared, &exe, SCHEMA, LayoutSource::DesktopService, Duration::from_millis(50)).await;
        })
    }

    fn names(watcher: &Watcher) -> Option<RawLayout> {
        watcher.caption_layout().map(|reading| reading.raw)
    }

    fn alive(pid: &str) -> bool {
        // A zombie has been killed but not reaped yet; `reap_on_drop` reaps it.
        std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())).is_ok_and(|stat| !stat.contains(") Z "))
    }

    // The key is read at start and again at each change its one monitor reports, and the monitor ends with the
    // watcher: no `gsettings` is left running.
    #[test]
    fn the_key_is_followed_and_the_monitor_goes_with_the_watcher() -> Result<(), Box<dyn std::error::Error>> {
        let dir = crate::temp::TempDir::new("craft-caption-gsettings")?;
        std::fs::write(dir.path().join("value"), "menu:minimize,maximize,close")?;
        let exe = stand_in(dir.path(), "echo \"'$(cat \"$dir/value\")'\"", FOLLOWS)?;
        let watcher = spawn(exe).ok_or("worker creation failed")?;
        assert!(eventually(|| names(&watcher) == Some(RawLayout::Names("menu:minimize,maximize,close".into()))), "read at start");
        let pid = std::fs::read_to_string(dir.path().join("monitor.pid"))?;
        assert!(alive(&pid), "the monitor runs");
        assert!(eventually(|| dir.path().join("monitor.ready").exists()), "the monitor read the first value");
        std::fs::write(dir.path().join("value"), "close,minimize:maximize")?;
        assert!(eventually(|| names(&watcher) == Some(RawLayout::Names("close,minimize:maximize".into()))), "the change is read");
        drop(watcher);
        assert!(eventually(|| !alive(&pid)), "the monitor ended with the watcher");
        Ok(())
    }

    // A helper that hangs on `get` gives no layout and holds nothing up: the read is ended at its time limit, and the
    // monitor with the watcher. A couple of seconds: the read's limit is 2 s.
    #[test]
    fn a_hung_helper_gives_no_layout_and_is_ended() -> Result<(), Box<dyn std::error::Error>> {
        let hung = crate::temp::TempDir::new("craft-caption-gsettings-hung")?;
        let watcher = spawn(stand_in(hung.path(), "echo $$ > \"$dir/get.pid\"; exec sleep 30", "exec sleep 30")?).ok_or("worker creation failed")?;
        let pid = |file: &str| std::fs::read_to_string(hung.path().join(file)).unwrap_or_default();
        assert!(eventually(|| !pid("get.pid").is_empty()), "the read started");
        assert!(eventually(|| !alive(&pid("get.pid"))), "the read was ended at its limit");
        assert_eq!(names(&watcher), None, "a read that never ends gives no layout");
        let monitor = pid("monitor.pid");
        assert!(alive(&monitor), "the monitor runs");
        drop(watcher);
        assert!(eventually(|| !alive(&monitor)), "nothing is left running");
        Ok(())
    }

    // A flood of monitor output is one change: what is there is taken in one go and read once, not once a line.
    #[test]
    fn a_flood_of_monitor_output_is_one_change() {
        let flood = "button-layout: 'close:'\n".repeat(1000);
        let mut output = futures_lite::io::Cursor::new(flood.into_bytes());
        assert!(futures_lite::future::block_on(changed(&mut output)), "the flood is a change");
        assert!(!futures_lite::future::block_on(changed(&mut output)), "and all of it: what follows is the monitor's end");
    }

    // A monitor that ends at once is started again, after a wait that doubles each time, so a helper that can't
    // monitor (a schema that is not installed) is not restarted in a loop. The waits here start at 100 ms; the gaps
    // are only bounded below, as a slow machine makes them longer.
    #[test]
    fn an_ending_monitor_is_started_again_after_ever_longer_waits() -> Result<(), Box<dyn std::error::Error>> {
        let ending = crate::temp::TempDir::new("craft-caption-gsettings-ending")?;
        let exe = stand_in(ending.path(), "echo \"'close:'\"", "date +%s%N >> \"$dir/starts\"; exit 1")?;
        let watcher = Watcher::spawn_shared(Shared::new(false, Box::new(|| {})), move |shared| async move {
            follow(&shared, &exe, SCHEMA, LayoutSource::DesktopService, Duration::from_millis(100)).await;
        })
        .ok_or("worker creation failed")?;
        let starts = || -> Vec<u128> {
            std::fs::read_to_string(ending.path().join("starts")).unwrap_or_default().lines().filter_map(|line| line.trim().parse().ok()).collect()
        };
        assert!(eventually(|| starts().len() >= 4), "started again: {:?}", starts());
        drop(watcher);
        let starts = starts();
        for (gap, wait) in starts.windows(2).map(|pair| pair[1].saturating_sub(pair[0])).zip([100_000_000, 200_000_000, 400_000_000]) {
            assert!(gap >= wait, "a restart came {gap} ns after the last, before its wait of {wait} ns: {starts:?}");
        }
        Ok(())
    }
}
