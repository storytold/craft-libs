//! Integration tests of craft-platform: dialog lifecycle.

#![cfg(not(target_arch = "wasm32"))]
use craft_platform::{dialog_outcome::Outcome, dialogs::NativeDialogs};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
fn pump_until<R, T: Send + 'static>(
    service: &mut NativeDialogs<R, T>,
    mut create: impl FnMut(R) -> craft_platform::dialogs::DialogFuture<T>,
    until: impl Fn() -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !until() {
        assert!(Instant::now() < deadline, "worker did not settle");
        service.pump(&mut create);
        std::thread::yield_now();
    }
}
#[test]
fn completion_between_pumps_cannot_reject_a_chained_request() -> Result<(), Box<dyn std::error::Error>> {
    let wakes = Arc::new(AtomicUsize::new(0));
    let count = wakes.clone();
    let (client, mut service) = NativeDialogs::<u8, u8>::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    });
    let first = client.request(1);
    let (release, wait) = mpsc::channel();
    let mut wait = Some(wait);
    let created = Arc::new(AtomicUsize::new(0));
    let count = created.clone();
    pump_until(
        &mut service,
        move |value| {
            count.fetch_add(1, Ordering::SeqCst);
            let wait = wait.take();
            Box::pin(async move {
                let _ = wait.map(|wait| wait.recv());
                Outcome::Selected(value)
            })
        },
        || created.load(Ordering::SeqCst) == 1,
    );
    // Finish after the host's pump, before its app continuation polls the reply.
    release.send(())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while wakes.load(Ordering::SeqCst) < 3 {
        assert!(Instant::now() < deadline, "worker did not publish completion");
        std::thread::yield_now();
    }
    assert!(matches!(first.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(service.pending(), "an unpublished completion still retains the parent");
    service.pump(|value| Box::pin(async move { Outcome::Selected(value) }));
    assert_eq!(first.try_recv()?, Outcome::Selected(1));
    assert!(!service.pending());
    // The app can chain the next dialog immediately upon observing the first answer.
    let second = client.request(2);
    assert!(matches!(second.try_recv(), Err(mpsc::TryRecvError::Empty)));
    let mut result = None;
    while result.is_none() {
        assert!(Instant::now() < deadline, "chained picker did not settle");
        service.pump(|value| Box::pin(async move { Outcome::Selected(value) }));
        result = second.try_recv().ok();
        std::thread::yield_now();
    }
    assert_eq!(result, Some(Outcome::Selected(2)));
    assert!(!service.pending());
    Ok(())
}
