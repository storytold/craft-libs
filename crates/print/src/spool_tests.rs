//! The `Spool` entry list and the Windows helper protocol. The protocol trials run a real child:
//! this test binary re-run with only [`stand_in_print_helper`] selected, so pipes, tokens and
//! process cleanup are the production ones, with no printer.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use craft_host::ManualJobs;
use craft_ui::testing::trial;

use crate::spool::isolated::{self, CANCELLED, Request, Submission};
use crate::spool::{Pending, Spool, SpoolBackend};
use crate::{DriverSettings, Duplex, Job, Scaling};

/// A guard against a hang, never a timing the trials rely on.
const LIMIT: Duration = Duration::from_secs(10);
/// Set on the child: the job's private input folder.
const PRINT_DIR: &str = "CRAFT_TEST_PRINT_DIR";
/// Set on the child: how it behaves (`waking`, `held`, `crash`, `sending`, or a plain receipt).
const PRINT_MODE: &str = "CRAFT_TEST_PRINT_MODE";
/// Set on the child: where it marks the stages it reached, apart from the input folder, which
/// the parent removes.
const PRINT_MARKS: &str = "CRAFT_TEST_PRINT_MARKS";
const PDF: &[u8] = b"synthetic PDF";

/// One prepared job of [`Fake`]: its cancel's answer, whether it answers after a cancel, and the
/// channel that lets its run finish.
struct Script {
    cancels: bool,
    waits_after_cancel: bool,
    release: Receiver<()>,
}

/// A backend whose jobs are named by their title, report each cancel asked of them and finish
/// only when released.
struct Fake {
    scripts: Mutex<VecDeque<Script>>,
    asked: Sender<String>,
    finished: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    /// The next prepared job's behaviour; the sender releases its run.
    fn script(&self, cancels: bool, waits_after_cancel: bool) -> Sender<()> {
        let (release, wait) = channel();
        self.scripts.lock().unwrap().push_back(Script { cancels, waits_after_cancel, release: wait });
        release
    }
}

impl SpoolBackend for Fake {
    fn printers(&self) -> Option<Vec<crate::Printer>> {
        Some(Vec::new())
    }

    fn prepare(&self, pdf: Vec<u8>, job: Job) -> Pending {
        assert_eq!(pdf, PDF);

        let Script { cancels, waits_after_cancel, release } = self.scripts.lock().unwrap().pop_front().expect("a script for every job");
        let name = job.title;
        let asked = self.asked.clone();
        let asked_name = name.clone();
        let finished = self.finished.clone();
        Pending {
            cancel: Box::new(move || {
                let _ = asked.send(asked_name.clone());
                cancels
            }),
            run: Box::new(move || {
                let _ = release.recv();
                finished.lock().unwrap().push(name.clone());
                Ok(name)
            }),
            waits_after_cancel,
        }
    }
}

fn titled(title: &str) -> Job {
    Job { title: title.into(), ..Job::default() }
}

/// Shutdown waits for a job already being sent, so the spooler has it before the process exits,
/// and for one whose backend answers after a cancel (a helper process to reap), but not for one
/// still waiting on a driver: that one is cancelled, so it is never sent. Every job is cancelled
/// before any is waited for, so a waiting job can't start sending meanwhile. The Print page's
/// Cancel asks only the latest job.
#[trial]
fn shutdown_waits_for_sending_jobs_and_drops_waiting_ones() {
    let jobs = Arc::new(ManualJobs::new());
    let (asked_tx, asked) = channel();
    let finished = Arc::new(Mutex::new(Vec::new()));
    let fake = Arc::new(Fake { scripts: Mutex::new(VecDeque::new()), asked: asked_tx, finished: finished.clone() });
    let spool = Spool::with_backend(jobs.clone(), fake.clone());
    assert!(spool.cancel_latest(), "no job: nothing will be sent");

    // A is being sent (its cancel refuses); B waits on a driver (its cancel succeeds).
    let release_a = fake.script(false, false);
    let release_b = fake.script(true, false);
    let sent_a = spool.submit(PDF.to_vec(), titled("A"));
    let _sent_b = spool.submit(PDF.to_vec(), titled("B"));
    assert!(spool.cancel_latest(), "the latest job was still waiting: never sent");
    assert_eq!(asked.try_iter().collect::<Vec<_>>(), ["B"], "an earlier job isn't the one being submitted");
    // C is cancelled but still answers afterwards.
    let release_c = fake.script(true, true);
    let sent_c = spool.submit(PDF.to_vec(), titled("C"));

    let runners: Vec<_> = (0..3)
        .map(|_| {
            let jobs = jobs.clone();
            std::thread::spawn(move || jobs.run_next())
        })
        .collect();
    let (returned, shutdown) = channel();
    let stopping = spool.clone();
    let done = finished.clone();
    std::thread::spawn(move || {
        stopping.shutdown();
        let _ = returned.send(done.lock().unwrap().clone());
    });
    let mut cancelled: Vec<String> = (0..3).map(|_| asked.recv_timeout(LIMIT).unwrap()).collect();
    cancelled.sort();
    assert_eq!(cancelled, ["A", "B", "C"], "every job is cancelled before any is waited for");
    assert!(shutdown.try_recv().is_err(), "shutdown waits for the job being sent");
    release_a.send(()).unwrap();
    assert_eq!(sent_a.recv_timeout(LIMIT).unwrap(), Ok("A".to_owned()));
    assert!(shutdown.try_recv().is_err(), "shutdown waits for a job that answers after its cancel");
    release_c.send(()).unwrap();
    let done_at_shutdown = shutdown.recv_timeout(LIMIT).unwrap();
    assert!(done_at_shutdown.contains(&"A".to_owned()) && done_at_shutdown.contains(&"C".to_owned()), "{done_at_shutdown:?}");
    assert!(!done_at_shutdown.contains(&"B".to_owned()), "shutdown didn't wait for the waiting job");
    assert_eq!(sent_c.recv_timeout(LIMIT).unwrap(), Ok("C".to_owned()));
    assert!(spool.cancel_latest(), "shutdown took every job");
    assert_eq!(asked.try_iter().count(), 0, "no job is left to ask");
    // The waiting job was left behind without blocking shutdown; let it end.
    release_b.send(()).unwrap();
    for runner in runners {
        assert!(runner.join().unwrap(), "each runner ran a job");
    }

    // A job that has already been sent answers false, so the Print page keeps its receipt.
    let release_d = fake.script(false, false);
    release_d.send(()).unwrap();
    let sent_d = spool.submit(PDF.to_vec(), titled("D"));
    assert!(jobs.run_next());
    assert_eq!(sent_d.recv_timeout(LIMIT).unwrap(), Ok("D".to_owned()));
    assert!(!spool.cancel_latest(), "a sent job isn't reported as cancelled");
    assert_eq!(asked.try_iter().collect::<Vec<_>>(), ["D"]);
}

/// Not a behaviour trial: the child the protocol trials launch. With [`PRINT_DIR`] unset (a
/// normal run of the suite) it returns at once.
#[trial]
fn stand_in_print_helper() {
    let Some(directory) = std::env::var_os(PRINT_DIR) else { return };
    let directory = PathBuf::from(directory);
    let marks = PathBuf::from(std::env::var_os(PRINT_MARKS).unwrap());
    let mode = std::env::var(PRINT_MODE).unwrap_or_default();
    if mode == "waking" {
        std::fs::write(marks.join("waking"), b"starting").unwrap();
        loop {
            std::thread::park();
        }
    }
    let refuses = mode == "sending";
    let result = isolated::helper(
        &directory,
        move |token| if refuses { false } else { token.cancel() },
        |pdf, job, _token| {
            assert_eq!(pdf, PDF);
            assert_eq!(job.printer.as_deref(), Some("test queue"));
            std::fs::write(marks.join("driver"), b"driver entered").unwrap();
            match mode.as_str() {
                "held" => {
                    let provider = Mutex::new(());
                    let _held = provider.lock();
                    loop {
                        std::thread::park();
                    }
                }
                "crash" => std::process::exit(7),
                "sending" => {
                    std::fs::write(marks.join("sending"), b"sending").unwrap();
                    while !marks.join("release").exists() {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Ok("spooled synthetic job".into())
                }
                _ => Ok("spooled synthetic job".into()),
            }
        },
    );
    std::process::exit(i32::from(result.is_err()));
}

/// Launches [`stand_in_print_helper`] over `directory` in `mode`, marking its stages in `marks`.
fn helper_command(directory: &Path, marks: &Path, mode: &str, may_be_killed: bool) -> std::io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", "spool_tests::stand_in_print_helper", "--nocapture", "--"])
        .env(PRINT_DIR, directory)
        .env(PRINT_MARKS, marks)
        .env(PRINT_MODE, mode);
    // A force-killed child can leave a partial LLVM profile; keep it in this trial's
    // temporary directory while normally exiting children retain their coverage.
    if may_be_killed && std::env::var_os("LLVM_PROFILE_FILE").is_some() {
        command.env("LLVM_PROFILE_FILE", marks.join("killed-%p-%m.profraw"));
    }
    Ok(command)
}

/// A job naming a literal printer, which the child checks to verify the transport.
fn job() -> Job {
    Job { printer: Some("test queue".into()), ..Job::default() }
}

/// Waits until the child has marked `path`, failing after [`LIMIT`].
fn wait_for(path: &Path) {
    let deadline = Instant::now() + LIMIT;
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(path.exists(), "the child never reached {path:?}");
}

/// Runs `submission` on a thread with the child in `mode`; the receiver gets its input folder.
fn submit_in(
    submission: &Submission,
    marks: &Path,
    mode: &'static str,
    may_be_killed: bool,
) -> (std::thread::JoinHandle<Result<String, String>>, Receiver<PathBuf>) {
    let worker = submission.clone();
    let marks = marks.to_path_buf();
    let (folder, folders) = channel();
    let running = std::thread::spawn(move || {
        worker.submit_with(PDF, &job(), |directory| {
            folder.send(directory.to_path_buf()).unwrap();
            helper_command(directory, &marks, mode, may_be_killed)
        })
    });
    (running, folders)
}

/// Cancelling before submission cannot launch a driver process.
#[trial]
fn a_cancelled_start_never_launches_a_helper() {
    let submission = Submission::new();
    assert!(submission.cancel());
    let result = submission.submit_with(PDF, &job(), |_| Err(std::io::Error::other("must not launch")));
    assert_eq!(result, Err(CANCELLED.into()));
}

/// A process without a start grant cannot send anything, so a cancel while it is created or
/// waking stops it at once, with or without its acknowledgement, and removes its inputs.
#[trial]
fn a_cancel_during_startup_never_starts_the_driver() {
    let marks = tempfile::tempdir().unwrap();
    // Cancelled while the process is being created.
    let submission = Submission::new();
    let worker = submission.clone();
    let held = marks.path().to_path_buf();
    let (started, starting) = channel();
    let (release, wait) = channel::<()>();
    let running = std::thread::spawn(move || {
        worker.submit_with(PDF, &job(), |directory| {
            started.send(directory.to_path_buf()).unwrap();
            wait.recv().unwrap();
            helper_command(directory, &held, "held", true)
        })
    });
    let directory = starting.recv_timeout(LIMIT).unwrap();
    assert!(submission.cancel());
    release.send(()).unwrap();
    assert_eq!(running.join().unwrap(), Err(CANCELLED.into()));
    assert!(!directory.exists(), "startup cancellation reaps the child and removes its inputs");

    // Cancelled while the child is waking and has not acknowledged anything.
    let submission = Submission::new();
    let (running, folders) = submit_in(&submission, marks.path(), "waking", true);
    let directory = folders.recv_timeout(LIMIT).unwrap();
    wait_for(&marks.path().join("waking"));
    assert!(submission.cancel(), "a process without a start grant cannot send anything");
    assert_eq!(running.join().unwrap(), Err(CANCELLED.into()));
    assert!(!directory.exists());
    assert!(!marks.path().join("driver").exists(), "neither child reached the driver");
}

/// A refused cancellation keeps the sending process alive until its receipt.
#[trial]
fn a_refusal_while_sending_keeps_the_job() {
    let marks = tempfile::tempdir().unwrap();
    let submission = Submission::new();
    let (running, folders) = submit_in(&submission, marks.path(), "sending", false);
    let directory = folders.recv_timeout(LIMIT).unwrap();
    wait_for(&marks.path().join("sending"));
    assert!(!submission.cancel(), "a sending job refuses cancellation");
    assert!(!submission.cancel(), "and keeps refusing it");
    assert!(!running.is_finished(), "the child is retained until it finishes sending");
    std::fs::write(marks.path().join("release"), b"continue").unwrap();
    assert_eq!(running.join().unwrap(), Ok("spooled synthetic job".into()));
    assert!(!directory.exists());
}

/// A driver cancelled while it holds its lock is killed, and the next job starts at once: the
/// reason for one process per job.
#[trial]
fn a_cancelled_driver_does_not_block_the_next_job() {
    let marks = tempfile::tempdir().unwrap();
    let submission = Submission::new();
    let (running, folders) = submit_in(&submission, marks.path(), "held", true);
    let directory = folders.recv_timeout(LIMIT).unwrap();
    wait_for(&marks.path().join("driver"));
    assert!(!directory.join("input.pdf").exists(), "the stalled driver retains no plaintext PDF");
    assert!(!directory.join("job.json").exists(), "loaded settings were erased before driver work");
    assert!(submission.cancel(), "a stalled driver still acknowledges cancellation");
    let next_marks = tempfile::tempdir().unwrap();
    let next = Submission::new();
    assert_eq!(next.submit_with(PDF, &job(), |dir| helper_command(dir, next_marks.path(), "ready", false)), Ok("spooled synthetic job".into()));
    assert_eq!(running.join().unwrap(), Err(CANCELLED.into()));
    assert!(!directory.exists(), "the reaped helper's PDF and settings were removed");
    assert!(!next.cancel(), "a completed job keeps its receipt");
}

/// A crashed helper is an error and its inputs go; oversized messages, requests and files are
/// refused; a vanished parent ends a waiting child; input cleanup tolerates missing files and
/// reports real failures.
#[trial]
fn helper_crashes_are_errors_and_inputs_are_bounded() {
    let marks = tempfile::tempdir().unwrap();
    let mut inputs = None;
    assert!(
        Submission::new()
            .submit_with(PDF, &job(), |dir| {
                inputs = Some(dir.to_path_buf());
                helper_command(dir, marks.path(), "crash", false)
            })
            .is_err()
    );
    assert!(!inputs.unwrap().exists(), "a helper crash removes the PDF and settings");

    assert!(isolated::bounded(&b"abcd"[..], 3).is_err());
    assert_eq!(isolated::bounded(&b"abc"[..], 3), Ok(b"abc".to_vec()));
    let long = vec![b'x'; 70 * 1024];
    assert!(isolated::line(&mut &long[..]).is_err());
    assert_eq!(isolated::line(&mut &b"start\n"[..]), Ok(Some("start".into())));
    let oversized = Job { options: vec![("x".into(), "y".repeat(3 << 20))], ..job() };
    let launched = std::cell::Cell::new(false);
    let refused = Submission::new().submit_with(PDF, &oversized, |_| {
        launched.set(true);
        Err(std::io::Error::other("must not launch"))
    });
    assert!(refused.is_err());
    assert!(!launched.get(), "an oversized request is refused before any process starts");

    // The parent goes away while the driver waits: the child cancels its token and exits.
    let marks = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("job.json"), serde_json::to_vec(&Request::from_job(&job())).unwrap()).unwrap();
    std::fs::write(folder.path().join("input.pdf"), PDF).unwrap();
    let mut child = helper_command(folder.path(), marks.path(), "held", false)
        .unwrap()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"start\n").and_then(|_| input.flush()).unwrap();
    wait_for(&marks.path().join("driver"));
    drop(input);
    assert!(child.wait().unwrap().success(), "a vanished parent ends the waiting child");
    assert!(!folder.path().join("input.pdf").exists());

    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("job.json"), b"settings").unwrap();
    std::fs::write(directory.path().join("input.pdf"), b"private PDF").unwrap();
    assert_eq!(isolated::remove_inputs(directory.path()), Ok(()));
    assert_eq!(isolated::remove_inputs(directory.path()), Ok(()), "missing inputs are not an error");
    assert!(!directory.path().join("input.pdf").exists());
    std::fs::create_dir(directory.path().join("job.json")).unwrap();
    std::fs::write(directory.path().join("input.pdf"), b"private PDF").unwrap();
    assert!(isolated::remove_inputs(directory.path()).is_err());
    assert!(!directory.path().join("input.pdf").exists(), "a settings cleanup failure does not retain the PDF");
}

/// Job transport preserves every explicit setting, including Unicode, each duplex direction and
/// scaling, and the driver's own settings; damaged driver settings are refused rather than
/// printed with other settings.
#[trial]
fn job_options_survive_the_process_boundary() {
    let across = |job: &Job| -> Result<Job, String> {
        let bytes = serde_json::to_vec(&Request::from_job(job)).unwrap();
        serde_json::from_slice::<Request>(&bytes).unwrap().job()
    };
    // A DEVMODEW header of 156 bytes (`dmSize` at 68) and a 4-byte private part (`dmDriverExtra` at 70).
    let mut dev_mode = vec![0u8; 160];
    dev_mode[68..70].copy_from_slice(&156u16.to_le_bytes());
    dev_mode[70..72].copy_from_slice(&4u16.to_le_bytes());
    dev_mode[156..].copy_from_slice(&[1, 2, 3, 4]);
    let dev_mode = crate::rgb::DevMode::from_bytes(dev_mode).unwrap();
    let expected = Job {
        printer: Some("Office λ".into()),
        copies: 3,
        collate: false,
        duplex: Some(Duplex::ShortEdge),
        grayscale: true,
        grey_pages: true,
        scaling: Scaling::ShrinkToFit,
        title: "A document".into(),
        options: vec![("media".into(), "iso_a4_210x297mm".into())],
        driver_settings: Some(DriverSettings::new("Office λ", dev_mode)),
    };
    assert_eq!(across(&expected), Ok(expected));
    for duplex in [None, Some(Duplex::Off), Some(Duplex::LongEdge), Some(Duplex::ShortEdge)] {
        for scaling in [Scaling::Exact, Scaling::ShrinkToFit] {
            let job = Job { duplex, scaling, ..job() };
            assert_eq!(across(&job), Ok(job.clone()), "{duplex:?} {scaling:?}");
        }
    }
    for hex in ["", "0", "zz", "00"] {
        let mut wire = serde_json::to_value(Request::from_job(&job())).unwrap();
        wire["driver_settings"] = serde_json::json!(["Office", hex]);
        let request: Request = serde_json::from_value(wire).unwrap();
        assert!(request.job().is_err(), "{hex:?}");
    }
}
