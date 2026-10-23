#![cfg(all(feature = "dialogs", not(target_arch = "wasm32")))]
use craft_platform::{
    dialog_outcome::{DialogError, ErrorKind, Outcome},
    dialogs::{NativeDialogs, wait_prepared},
};
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
fn factory_is_eager_on_caller_and_completion_on_worker() -> Result<(), Box<dyn std::error::Error>> {
    let caller = std::thread::current().id();
    let (tx, rx) = mpsc::channel();
    let worker = wait_prepared(
        || {
            assert_eq!(caller, std::thread::current().id());
            async { Outcome::Selected(42) }
        },
        move |outcome| {
            let _ = tx.send((outcome, std::thread::current().id()));
        },
    )?;
    let (outcome, thread) = rx.recv_timeout(Duration::from_secs(3))?;
    assert_eq!(outcome, Outcome::Selected(42));
    assert_ne!(caller, thread);
    assert!(worker.join().is_ok());
    Ok(())
}
#[test]
fn constructor_and_poll_failures_are_not_cancellation() -> Result<(), Box<dyn std::error::Error>> {
    for construct in [false, true] {
        let (tx, rx) = mpsc::channel();
        let worker = wait_prepared(
            || {
                if construct {
                    std::panic::resume_unwind(Box::new("constructor"));
                }
                async {
                    std::panic::resume_unwind(Box::new("poll"));
                    #[allow(unreachable_code)]
                    Outcome::<u8>::Cancelled
                }
            },
            move |outcome| {
                let _ = tx.send(outcome);
            },
        )?;
        assert!(matches!(rx.recv_timeout(Duration::from_secs(3))?, Outcome::Failed(DialogError { kind: ErrorKind::Worker, .. })));
        assert!(worker.join().is_ok());
    }
    Ok(())
}
#[test]
fn queued_and_dropped_replies_keep_live_picker_busy() -> Result<(), Box<dyn std::error::Error>> {
    let (c, mut service) = NativeDialogs::<(), u8>::new(|| {});
    let reply = c.request(());
    assert!(service.pending());
    drop(reply);
    let (release, wait) = mpsc::channel();
    let mut wait = Some(wait);
    let created = Arc::new(AtomicUsize::new(0));
    let count = created.clone();
    pump_until(
        &mut service,
        move |()| {
            count.fetch_add(1, Ordering::SeqCst);
            let wait = wait.take();
            Box::pin(async move {
                let _ = wait.map(|w| w.recv());
                Outcome::Selected(1)
            })
        },
        || created.load(Ordering::SeqCst) == 1,
    );
    assert!(service.pending());
    assert!(matches!(c.request(()).recv_timeout(Duration::from_secs(3))?, Outcome::Failed(_)));
    release.send(())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.pending() {
        assert!(Instant::now() < deadline);
        service.pump(|()| Box::pin(async { Outcome::Indeterminate }));
        std::thread::yield_now();
    }
    Ok(())
}
#[test]
fn proven_absence_skips_native_factory() -> Result<(), Box<dyn std::error::Error>> {
    let (c, mut service) = NativeDialogs::<(), u8>::with_preflight(|| {}, || Err(DialogError::new(ErrorKind::Unavailable, "test backend missing")));
    let reply = c.request(());
    let called = Arc::new(AtomicUsize::new(0));
    let count = called.clone();
    let mut result = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while result.is_none() {
        assert!(Instant::now() < deadline);
        service.pump(|()| {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Outcome::Selected(1) })
        });
        result = reply.try_recv().ok();
        std::thread::yield_now();
    }
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert!(matches!(result, Some(Outcome::Failed(DialogError { kind: ErrorKind::Unavailable, .. }))));
    Ok(())
}
#[test]
fn missing_backend_answer_is_ambiguous() {
    assert_eq!(Outcome::<u8>::from_rfd(None), Outcome::Indeterminate);
}
#[cfg(feature = "egui")]
#[test]
fn held_close_retries_policy_without_granting_discard() {
    let ctx = egui::Context::default();
    let mut close = craft_platform::dialogs::DeferredClose::default();
    let mut input = egui::RawInput::default();
    input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let mut output = ctx.run_ui(input, |ui| assert!(!close.update(ui.ctx(), true)));
    assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::CancelClose));
    output.textures_delta.clear();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| assert!(close.update(ui.ctx(), false)));
    assert!(!output.viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::Close));
    output.textures_delta.clear();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| assert!(!close.update(ui.ctx(), false)));
    output.textures_delta.clear();
}
#[test]
fn notification_panic_cannot_kill_reserved_picker_worker() -> Result<(), Box<dyn std::error::Error>> {
    let (c, mut service) = NativeDialogs::<(), u8>::new(|| std::panic::resume_unwind(Box::new("wake failure")));
    let reply = c.request(());
    let mut result = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while result.is_none() {
        assert!(Instant::now() < deadline);
        service.pump(|()| Box::pin(async { Outcome::Selected(7) }));
        result = reply.try_recv().ok();
        std::thread::yield_now();
    }
    assert_eq!(result, Some(Outcome::Selected(7)));
    Ok(())
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

#[cfg(feature = "egui")]
#[test]
fn pending_dialog_without_close_intent_does_not_schedule_viewport_work() {
    let ctx = egui::Context::default();
    let mut close = craft_platform::dialogs::DeferredClose::default();
    // Let egui settle its own initial font/style passes before testing the idle host.
    for _ in 0..4 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
    }
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(!close.update(ui.ctx(), true));
        });
        output.textures_delta.clear();
        let viewport = output.viewport_output.get(&egui::ViewportId::ROOT).unwrap();
        assert!(viewport.commands.is_empty(), "an idle picker must not enqueue commands: {:?}", viewport.commands);
        assert_ne!(viewport.repaint_delay, std::time::Duration::ZERO, "an idle picker must not request another frame");
    }
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| assert!(!close.update(ui.ctx(), false)));
    output.textures_delta.clear();
}

#[cfg(feature = "egui")]
#[test]
fn programmatic_close_is_held_until_the_pending_dialog_settles() {
    let ctx = egui::Context::default();
    let mut close = craft_platform::dialogs::DeferredClose::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        assert!(!close.update(ui.ctx(), true));
    });
    output.textures_delta.clear();
    let commands = &output.viewport_output.get(&egui::ViewportId::ROOT).unwrap().commands;
    let last_close = commands.iter().rev().find(|command| matches!(command, egui::ViewportCommand::Close | egui::ViewportCommand::CancelClose));
    assert_eq!(last_close, Some(&egui::ViewportCommand::CancelClose));
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| assert!(close.update(ui.ctx(), false)));
    output.textures_delta.clear();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| assert!(!close.update(ui.ctx(), false)));
    output.textures_delta.clear();
}
