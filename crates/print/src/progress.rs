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
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn progress_arrives_on_the_calling_thread_before_submission_finishes() {
        let caller = std::thread::current().id();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let observations = Rc::clone(&seen);
        let (observed, waiting) = sync_channel(0);
        let result = run(
            &mut move |processed, sheets| {
                assert_eq!(std::thread::current().id(), caller);
                observations.borrow_mut().push((processed, sheets));
                observed.send(()).unwrap();
            },
            move |events| {
                assert_ne!(std::thread::current().id(), caller);
                for completed in 0..=3 {
                    events.send((completed, 3)).unwrap();
                    // No terminal result until the caller processes this intermediate event.
                    waiting.recv().unwrap();
                }
                Ok("accepted")
            },
        );
        assert_eq!(result, Ok("accepted"));
        assert_eq!(*seen.borrow(), [(0, 3), (1, 3), (2, 3), (3, 3)]);
    }

    #[test]
    fn complete_progress_does_not_hide_a_terminal_failure() {
        let mut seen = Vec::new();
        let error = Error::Spool("the native print package failed".into());
        let result = run(&mut |done, total| seen.push((done, total)), |events| {
            events.send((0, 1)).unwrap();
            events.send((1, 1)).unwrap();
            Err::<(), _>(error.clone())
        });
        assert_eq!(seen, [(0, 1), (1, 1)]);
        assert_eq!(result, Err(error));
    }

    #[test]
    fn a_partial_job_keeps_its_failure_and_last_processed_sheet() {
        let mut seen = Vec::new();
        let result = run(&mut |done, total| seen.push((done, total)), |events| {
            events.send((0, 3)).unwrap();
            events.send((1, 3)).unwrap();
            Err::<(), _>(Error::Spool("the second sheet could not be rendered".into()))
        });
        assert_eq!(seen, [(0, 3), (1, 3)]);
        assert_eq!(result, Err(Error::Spool("the second sheet could not be rendered".into())));
    }

    #[test]
    fn worker_failure_is_reported_without_inventing_progress() {
        let mut seen = Vec::new();
        let result = run(&mut |done, total| seen.push((done, total)), |_| -> Result<(), Error> {
            panic!("synthetic native worker failure");
        });
        assert!(seen.is_empty());
        assert!(result.is_err_and(|e| e.to_string().contains("check the printer's queue")));
    }

    #[test]
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
