//! Bridge native worker events to callbacks on the submission thread.

use std::sync::mpsc::{SyncSender, sync_channel};

use crate::Error;

pub(crate) fn run<T: Send>(
    progress: &mut dyn FnMut(usize, usize),
    work: impl FnOnce(SyncSender<(usize, usize)>) -> Result<T, Error> + Send,
) -> Result<T, Error> {
    // A slow UI callback must not accumulate one queued event per PDF sheet. The sender must
    // ignore disconnection: dropping a progress receiver does not cancel an already-started job.
    let (sender, receiver) = sync_channel(1);
    std::thread::scope(move |scope| {
        let worker = std::thread::Builder::new()
            .name("print job".into())
            .spawn_scoped(scope, move || work(sender))
            .map_err(|e| Error::Spool(format!("the print job could not be started: {e}")))?;
        // Own the receiver here so unwinding a callback closes it before scope joins the
        // worker; otherwise a bounded sender could block forever during that join.
        for (processed, sheets) in receiver {
            progress(processed, sheets);
        }
        worker.join().map_err(|_| Error::Spool("the print job stopped unexpectedly; check the printer's queue before printing again".into()))?
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::testing::trial;

    #[trial]
    fn callback_unwind_closes_progress_and_reaps_the_worker() {
        let finished = std::sync::atomic::AtomicBool::new(false);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&mut |_, _| panic!("synthetic caller callback failure"), |events| {
                for processed in 0..=20 {
                    let _ = events.send((processed, 20));
                }
                finished.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        }));
        assert!(caught.is_err());
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
    }
}
