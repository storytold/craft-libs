//! A dialog-scoped capability cache with one cancellable selection query. UI frameworks only
//! supply a repaint callback; they do not implement cache policy or stale-response handling.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::DuplexSource;

struct Read {
    printer: String,
    cancelled: Arc<AtomicBool>,
    answer: Receiver<Option<bool>>,
}

impl Drop for Read {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

struct Wake(Arc<dyn Fn() + Send + Sync>);

impl Drop for Wake {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.0)()));
    }
}

/// Own one print dialog's capability knowledge. Call [`Self::refresh`] on opening the dialog
/// and [`Self::invalidate`] after a printer's driver settings close or its configuration changes.
/// Call [`Self::select`] when selection changes and after refresh or invalidation, even if the
/// selection stays the same. Call [`Self::poll`] after a wake, and select `None` when the dialog closes.
/// Successful answers are reused within this session; failures retry after reselection or
/// refresh. Dropping this value cancels the active reader. No state is persisted to disk.
#[derive(Default)]
pub struct DuplexSession {
    selected: Option<String>,
    answers: BTreeMap<String, Option<bool>>,
    active: Option<Read>,
}

impl DuplexSession {
    pub fn refresh(&mut self) {
        self.active = None;
        self.selected = None;
        self.answers.clear();
    }

    /// Forget one printer's answer and cancel its read, without affecting other printers.
    pub fn invalidate(&mut self, printer: &str) {
        self.answers.remove(printer);
        if self.selected.as_deref() == Some(printer) {
            self.active = None;
            self.selected = None;
        }
    }

    /// Unknown means either unread or unavailable, never an assertion of simplex support.
    pub fn answer(&self, printer: &str) -> Option<&Option<bool>> {
        self.answers.get(printer)
    }

    pub fn select(&mut self, printer: Option<&str>, source: &DuplexSource, wake: Arc<dyn Fn() + Send + Sync>) {
        if self.selected.as_deref() != printer {
            self.active = None;
            self.selected = printer.map(str::to_owned);
            self.answers.retain(|_, answer| answer.is_some());
        }
        let Some(printer) = printer else { return };
        if self.active.is_some() || self.answers.contains_key(printer) {
            return;
        }
        let name = printer.to_owned();
        let asked = name.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let source = source.clone();
        let wake = Wake(wake);
        let (sender, answer) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("printer capability".into()).spawn(move || {
            let _wake = wake;
            let sender = sender;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source(&asked, &cancel))).ok().flatten();
            let _ = sender.send(result);
        });
        match spawned {
            Ok(_) => self.active = Some(Read { printer: name, cancelled, answer }),
            Err(_) => {
                self.answers.insert(name, None);
            }
        }
    }

    pub fn poll(&mut self) {
        let Some(read) = &self.active else { return };
        let answer = match read.answer.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        };
        if let Some(answer) = answer {
            self.answers.insert(read.printer.clone(), answer);
            self.active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    fn settle(session: &mut DuplexSession, printer: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.answer(printer).is_none() && Instant::now() < deadline {
            session.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(session.answer(printer).is_some());
    }

    #[test]
    fn successes_cache_failures_retry_and_refresh_rereads() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let source: DuplexSource = Arc::new(move |_, _| match count.fetch_add(1, Ordering::SeqCst) {
            0 => None,
            1 => Some(true),
            _ => Some(false),
        });
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut session = DuplexSession::default();
        session.select(Some("Office"), &source, wake.clone());
        settle(&mut session, "Office");
        assert_eq!(session.answer("Office"), Some(&None));
        session.select(None, &source, wake.clone());
        session.select(Some("Office"), &source, wake.clone());
        settle(&mut session, "Office");
        assert_eq!(session.answer("Office"), Some(&Some(true)));
        session.select(None, &source, wake.clone());
        session.select(Some("Office"), &source, wake.clone());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        session.refresh();
        session.select(Some("Office"), &source, wake.clone());
        settle(&mut session, "Office");
        assert_eq!(session.answer("Office"), Some(&Some(false)));
        session.invalidate("Office");
        assert_eq!(session.answer("Office"), None);
        session.select(Some("Office"), &source, wake);
        settle(&mut session, "Office");
        assert_eq!(session.answer("Office"), Some(&Some(false)));
        assert_eq!(calls.load(Ordering::SeqCst), 4, "invalidating the current selection starts a fresh read on select");
    }

    #[test]
    fn invalidation_preserves_another_printers_pending_read_and_cached_answer() {
        let reads = Arc::new(AtomicUsize::new(0));
        let count = reads.clone();
        let released = Arc::new(AtomicBool::new(false));
        let release = released.clone();
        let source: DuplexSource = Arc::new(move |printer, cancel| {
            if printer == "Office" {
                return Some(count.fetch_add(1, Ordering::SeqCst) > 0);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while !release.load(Ordering::Acquire) && !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            (!cancel.load(Ordering::Acquire)).then_some(false)
        });
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut session = DuplexSession::default();
        session.select(Some("Office"), &source, wake.clone());
        settle(&mut session, "Office");
        session.select(Some("Other"), &source, wake.clone());
        session.invalidate("Office");
        assert_eq!(session.answer("Office"), None);
        released.store(true, Ordering::Release);
        settle(&mut session, "Other");
        assert_eq!(session.answer("Other"), Some(&Some(false)), "the pending read was not cancelled");
        session.select(Some("Office"), &source, wake.clone());
        settle(&mut session, "Office");
        assert_eq!(session.answer("Office"), Some(&Some(true)), "the affected printer was read again");
        session.invalidate("Office");
        session.select(Some("Other"), &source, wake);
        assert_eq!(session.answer("Other"), Some(&Some(false)), "the cached answer survives");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn switching_selection_cancels_the_old_read_and_discards_its_late_answer() {
        let (started, ready) = mpsc::channel();
        let (stopped, cancelled) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let released = Arc::new(AtomicBool::new(false));
        let release = released.clone();
        let source: DuplexSource = Arc::new(move |printer, cancel| {
            if printer != "Slow" {
                return Some(true);
            }
            started.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            stopped.send(cancel.load(Ordering::Acquire)).unwrap();
            while !release.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            finished.send(()).unwrap();
            Some(false)
        });
        let mut session = DuplexSession::default();
        session.select(Some("Slow"), &source, Arc::new(|| {}));
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        session.select(Some("Office"), &source, Arc::new(|| {}));
        assert!(cancelled.recv_timeout(Duration::from_secs(5)).unwrap());
        settle(&mut session, "Office");
        released.store(true, Ordering::Release);
        done.recv_timeout(Duration::from_secs(5)).unwrap();
        session.poll();
        assert_eq!(session.answer("Office"), Some(&Some(true)));
        assert_eq!(session.answer("Slow"), None);
    }

    #[test]
    fn dropping_a_session_cancels_its_reader_and_panic_becomes_unknown() {
        let (started, ready) = mpsc::channel();
        let (stopped, cancelled) = mpsc::channel();
        let source: DuplexSource = Arc::new(move |_, cancel| {
            started.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            stopped.send(cancel.load(Ordering::Acquire)).unwrap();
            Some(false)
        });
        let mut session = DuplexSession::default();
        session.select(Some("Slow"), &source, Arc::new(|| {}));
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(session);
        assert!(cancelled.recv_timeout(Duration::from_secs(5)).unwrap());
        let panic: DuplexSource = Arc::new(|_, _| panic!("driver failed"));
        let mut session = DuplexSession::default();
        session.select(Some("Broken"), &panic, Arc::new(|| {}));
        settle(&mut session, "Broken");
        assert_eq!(session.answer("Broken"), Some(&None));
    }
}
