//! Integration tests of craft-platform: dialog lifecycle.

#![cfg(not(target_arch = "wasm32"))]
use craft_platform::{dialog_outcome::Outcome, dialogs::NativeDialogs};
use craft_testkit::deadline::with_deadline;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;
// The service wakes the host once per request, once when its worker is ready for the picker and once
// when the picker has answered. The test pumps after each wake instead of polling against a clock.
#[test]
fn completion_between_pumps_cannot_reject_a_chained_request() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // A hang guard only: every wait below ends at a wake, so a run that works never reaches the limit.
    with_deadline(Duration::from_secs(60), "the dialog lifecycle", || {
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let (wake, woken) = mpsc::channel();
        let (client, mut service) = NativeDialogs::<u8, u8>::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            let _ = wake.send(());
        });
        let first = client.request(1);
        woken.recv()?;
        let (release, wait) = mpsc::channel();
        let mut wait = Some(wait);
        let created = Arc::new(AtomicUsize::new(0));
        let count = created.clone();
        let mut create = move |value| -> craft_platform::dialogs::DialogFuture<u8> {
            count.fetch_add(1, Ordering::SeqCst);
            let wait = wait.take();
            Box::pin(async move {
                let _ = wait.map(|wait| wait.recv());
                Outcome::Selected(value)
            })
        };
        // The first pump starts the worker; the second, after its wake, opens the picker.
        service.pump(&mut create);
        woken.recv()?;
        service.pump(&mut create);
        assert_eq!(created.load(Ordering::SeqCst), 1, "worker did not settle");
        // Finish after the host's pump, before its app continuation polls the reply.
        release.send(())?;
        woken.recv()?;
        assert_eq!(wakes.load(Ordering::SeqCst), 3, "worker did not publish completion");
        assert!(matches!(first.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(service.pending(), "an unpublished completion still retains the parent");
        service.pump(|value| Box::pin(async move { Outcome::Selected(value) }));
        assert_eq!(first.try_recv()?, Outcome::Selected(1));
        assert!(!service.pending());
        // The app can chain the next dialog immediately upon observing the first answer.
        let second = client.request(2);
        assert!(matches!(second.try_recv(), Err(mpsc::TryRecvError::Empty)));
        // The request's wake, the worker's and the picker's answer each lead to one pump.
        for _ in 0..3 {
            woken.recv()?;
            service.pump(|value| Box::pin(async move { Outcome::Selected(value) }));
        }
        let result = second.try_recv().ok();
        assert!(result.is_some(), "chained picker did not settle");
        assert_eq!(result, Some(Outcome::Selected(2)));
        assert!(!service.pending());
        Ok(())
    })
}
