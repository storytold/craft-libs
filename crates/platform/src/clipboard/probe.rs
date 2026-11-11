//! The paste probe: asks a second clipboard handle on a worker thread what it offers.

use craft_host::Wake;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use super::*;

/// How often the probe's worker looks at the clipboard while the UI wants to know: 250 ms.
pub const PROBE_INTERVAL: Duration = Duration::from_millis(250);
/// How long dropping the probe waits for the worker to let go of its handle: 250 ms.
pub const PROBE_STOP_WAIT: Duration = Duration::from_millis(250);

/// Whether Paste has something to take from the system clipboard, asked on a worker thread.
///
/// `has()` can block for seconds when the clipboard owner never answers (an unresponsive X11
/// client), so it never runs on the thread that draws the window: the worker owns a second
/// clipboard handle, looks every [`PROBE_INTERVAL`] while wanted (not at all otherwise), sends
/// its verdict when it changes and calls [`Wake::wake`], so the app runs a frame that reads it
/// with [`pasteable`](Self::pasteable). Dropping the probe stops the worker and waits
/// up to [`PROBE_STOP_WAIT`] for it to drop its handle (on X11 the last handle hands the copy to
/// the clipboard manager before exit).
pub struct ClipboardProbe {
    wants: Option<Sender<bool>>,
    wanted: bool,
    verdicts: Receiver<bool>,
    last: bool,
}

impl ClipboardProbe {
    /// Starts the worker with the backend `make` builds there, asking for `mimes`. `None` on wasm
    /// or when the thread can't be spawned: the app asks [`Clipboard::has`] in line instead.
    // vectorcraft: crates/ui-egui/src/clipboard_probe.rs:33 Probe::start
    pub fn start(make: Box<dyn FnOnce() -> Box<dyn ClipboardBackend> + Send>, mimes: Vec<&'static str>, wake: Wake) -> Option<Self> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        let (wants, wanted_rx) = std::sync::mpsc::channel();
        let (verdict_tx, verdicts) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("craft-clipboard-probe".into())
            .spawn(move || {
                let mut backend = make();
                let (mut wanted, mut sent) = (false, None);
                loop {
                    if wanted {
                        // This thread may wait here for a slow owner; the UI's never does.
                        let verdict = backend.has(&mimes);
                        if sent != Some(verdict) {
                            if verdict_tx.send(verdict).is_err() {
                                break;
                            }
                            sent = Some(verdict);
                            wake.wake();
                        }
                    }
                    // Sleep until the next look (none while unwanted) or the UI's next word.
                    let next =
                        if wanted { wanted_rx.recv_timeout(PROBE_INTERVAL) } else { wanted_rx.recv().map_err(|_| RecvTimeoutError::Disconnected) };
                    match next {
                        Ok(w) => wanted = w,
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                // The handle goes before the probe's drop stops waiting: on X11 the last handle
                // alive hands the app's copy to the clipboard manager, which must finish first.
                drop(backend);
                drop(verdict_tx);
            })
            .ok()?;
        Some(Self { wants: Some(wants), wanted: false, verdicts, last: false })
    }

    /// Paste can take from the system clipboard: `wanted` (nothing copied in the app, a document
    /// open) and the worker's latest verdict says so. Never blocks. `None` once the worker is
    /// gone.
    // vectorcraft: crates/ui-egui/src/clipboard_probe.rs:76 Probe::pasteable
    pub fn pasteable(&mut self, wanted: bool) -> Option<bool> {
        if wanted != self.wanted {
            self.wants.as_ref()?.send(wanted).ok()?;
            self.wanted = wanted;
        }
        loop {
            match self.verdicts.try_recv() {
                Ok(verdict) => self.last = verdict,
                Err(TryRecvError::Empty) => return Some(wanted && self.last),
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

impl Drop for ClipboardProbe {
    // vectorcraft: crates/ui-egui/src/clipboard_probe.rs:94 Probe::drop
    fn drop(&mut self) {
        self.wants = None;
        let end = Instant::now() + PROBE_STOP_WAIT;
        while self.verdicts.recv_timeout(end.saturating_duration_since(Instant::now())).is_ok() {}
    }
}
