//! The Windows print helper process and its protocol. The module compiles under
//! `cfg(any(windows, test))` so its protocol trials run on every platform.
//!
//! One driver submission per process: a cancelled driver cannot retain another job's provider
//! lock. The child acknowledges its [`Cancel`](crate::Cancel) token before it exits.

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};

#[cfg(windows)]
const FLAG: &str = "--craft-print-helper";
const PREFIX: &str = "CRAFT_PRINT:";
const MAX_MESSAGE: u64 = 64 * 1024;
/// The job's options, and a driver's settings of up to 1 MiB written as hex.
const MAX_REQUEST: u64 = MAX_MESSAGE + 2 * 1024 * 1024;
const MAX_PDF: u64 = 512 * 1024 * 1024;
pub(crate) const CANCELLED: &str = "the print job was cancelled before anything was sent to the printer";

/// The wire form of [`crate::Duplex`], kept apart so the public type carries no serde.
#[derive(Serialize, Deserialize)]
enum Duplex {
    Off,
    LongEdge,
    ShortEdge,
}

/// The wire form of a [`crate::Job`].
#[derive(Serialize, Deserialize)]
pub(crate) struct Request {
    printer: Option<String>,
    copies: u32,
    collate: bool,
    duplex: Option<Duplex>,
    grayscale: bool,
    grey_pages: bool,
    shrink: bool,
    title: String,
    options: Vec<(String, String)>,
    /// The printer the driver's settings were chosen for, and their DEVMODE as hex.
    driver_settings: Option<(String, String)>,
}

impl Request {
    /// Preserve all spooler options in a platform-independent wire representation.
    pub(crate) fn from_job(job: &crate::Job) -> Self {
        Self {
            printer: job.printer.clone(),
            copies: job.copies,
            collate: job.collate,
            duplex: job.duplex.map(|d| match d {
                crate::Duplex::Off => Duplex::Off,
                crate::Duplex::LongEdge => Duplex::LongEdge,
                crate::Duplex::ShortEdge => Duplex::ShortEdge,
            }),
            grayscale: job.grayscale,
            grey_pages: job.grey_pages,
            shrink: job.scaling == crate::Scaling::ShrinkToFit,
            title: job.title.clone(),
            options: job.options.clone(),
            driver_settings: job.driver_settings.as_ref().map(|settings| (settings.printer().to_owned(), settings.dev_mode().to_hex())),
        }
    }

    /// Restore the job without substituting printer defaults or changing copy semantics.
    pub(crate) fn job(self) -> Result<crate::Job, String> {
        let driver_settings = match self.driver_settings {
            Some((printer, hex)) => Some(crate::DriverSettings::new(printer, crate::rgb::DevMode::from_hex(&hex).map_err(error)?)),
            None => None,
        };
        Ok(crate::Job {
            printer: self.printer,
            copies: self.copies,
            collate: self.collate,
            duplex: self.duplex.map(|d| match d {
                Duplex::Off => crate::Duplex::Off,
                Duplex::LongEdge => crate::Duplex::LongEdge,
                Duplex::ShortEdge => crate::Duplex::ShortEdge,
            }),
            grayscale: self.grayscale,
            grey_pages: self.grey_pages,
            scaling: if self.shrink { crate::Scaling::ShrinkToFit } else { crate::Scaling::Exact },
            title: self.title,
            options: self.options,
            driver_settings,
        })
    }
}

#[derive(Serialize, Deserialize)]
enum Event {
    Ready,
    Cancelled,
    Sending,
    Done(Result<String, String>),
}

enum State {
    Starting,
    Waking(Arc<Mutex<Child>>),
    Running(ChildStdin),
    Sending(ChildStdin),
    Cancelled,
    Finished,
}

/// The parent side of one helper process: its lifecycle, shared between the job that runs it
/// and whoever cancels it.
#[derive(Clone)]
pub(crate) struct Submission(Arc<(Mutex<State>, Condvar)>);

impl Submission {
    /// Create one submission whose driver cannot run until the parent grants startup.
    pub(crate) fn new() -> Self {
        Self(Arc::new((Mutex::new(State::Starting), Condvar::new())))
    }

    /// Recover the lifecycle state after poisoning without invoking another driver call.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Only an acknowledgement of the child's token can cancel a running helper. A timeout
    /// is a refusal, never permission to kill a process that may already be sending.
    pub(crate) fn cancel(&self) -> bool {
        let mut state = self.state();
        match &mut *state {
            State::Starting => {
                *state = State::Cancelled;
                return true;
            }
            State::Waking(child) => {
                let _ = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner).kill();
                *state = State::Cancelled;
                return true;
            }
            State::Cancelled => return true,
            State::Finished | State::Sending(_) => return false,
            State::Running(input) => {
                if input.write_all(b"cancel\n").and_then(|_| input.flush()).is_err() {
                    return false;
                }
            }
        }
        let (state, _) = self
            .0
            .1
            .wait_timeout_while(state, Duration::from_secs(1), |s| matches!(s, State::Running(_)))
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        matches!(*state, State::Cancelled)
    }

    /// Submit through this executable's private helper mode and await its final receipt.
    #[cfg(windows)]
    pub(crate) fn submit(&self, pdf: &[u8], job: &crate::Job) -> Result<String, String> {
        self.submit_with(pdf, job, |directory| {
            let mut command = Command::new(std::env::current_exe()?);
            command.arg(FLAG).arg(directory);
            Ok(command)
        })
    }

    /// Publish a terminal state on every result, waking a cancellation caller awaiting an ack.
    /// `command` makes the helper process for the private input folder.
    pub(crate) fn submit_with(
        &self,
        pdf: &[u8],
        job: &crate::Job,
        command: impl FnOnce(&Path) -> std::io::Result<Command>,
    ) -> Result<String, String> {
        let result = self.run(pdf, job, command);
        let mut state = self.state();
        if !matches!(*state, State::Cancelled) {
            *state = State::Finished;
        }
        self.0.1.notify_all();
        if matches!(*state, State::Cancelled) { Err(CANCELLED.into()) } else { result }
    }

    /// Own the private inputs and child until receipt, acknowledged cancellation or failure.
    fn run(&self, pdf: &[u8], job: &crate::Job, command: impl FnOnce(&Path) -> std::io::Result<Command>) -> Result<String, String> {
        if matches!(*self.state(), State::Cancelled) {
            return Err(CANCELLED.into());
        }
        if pdf.len() as u64 > MAX_PDF {
            return Err("the print PDF exceeds the 512 MiB helper limit".into());
        }
        let request = serde_json::to_vec(&Request::from_job(job)).map_err(error)?;
        if request.len() as u64 > MAX_REQUEST {
            return Err("the print settings exceed the helper message limit".into());
        }
        let directory = tempfile::tempdir().map_err(error)?;
        std::fs::write(directory.path().join("job.json"), request).map_err(error)?;
        std::fs::write(directory.path().join("input.pdf"), pdf).map_err(error)?;
        let child =
            command(directory.path()).map_err(error)?.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(error)?;
        let child = Arc::new(Mutex::new(child));
        let outcome = (|| {
            let (input, output) = {
                let mut process = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                (process.stdin.take().ok_or("print helper has no input pipe")?, process.stdout.take().ok_or("print helper has no output pipe")?)
            };
            {
                let mut state = self.state();
                if matches!(*state, State::Cancelled) {
                    return Err(CANCELLED.into());
                }
                *state = State::Waking(child.clone());
            }
            let mut output = BufReader::new(output);
            loop {
                let message = line(&mut output)?.ok_or("the print helper stopped before it was ready")?;
                let Some((_, json)) = message.split_once(PREFIX) else { continue };
                if !matches!(serde_json::from_str::<Event>(json).map_err(error)?, Event::Ready) {
                    return Err("the print helper did not acknowledge startup".into());
                }
                break;
            }
            {
                let mut state = self.state();
                if matches!(*state, State::Cancelled) {
                    return Err(CANCELLED.into());
                }
                *state = State::Running(input);
                if let State::Running(input) = &mut *state {
                    input.write_all(b"start\n").and_then(|_| input.flush()).map_err(error)?;
                }
            }
            while let Some(line) = line(&mut output)? {
                let Some((_, json)) = line.split_once(PREFIX) else { continue };
                match serde_json::from_str::<Event>(json).map_err(error)? {
                    Event::Ready => return Err("the print helper repeated its startup acknowledgement".into()),
                    Event::Cancelled => {
                        *self.state() = State::Cancelled;
                        self.0.1.notify_all();
                        return Err(CANCELLED.into());
                    }
                    Event::Sending => {
                        let mut state = self.state();
                        let previous = std::mem::replace(&mut *state, State::Finished);
                        *state = match previous {
                            State::Running(input) | State::Sending(input) => State::Sending(input),
                            other => other,
                        };
                        self.0.1.notify_all();
                    }
                    Event::Done(result) => return result,
                }
            }
            Err("the print helper stopped without a receipt".into())
        })();
        // A cancelled child has acknowledged that it cannot send. Other outcomes are terminal
        // receipts or a broken protocol/pipe; reap the process and its private input files.
        if outcome.is_err() {
            let _ = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner).kill();
        }
        let status = child.lock().unwrap_or_else(std::sync::PoisonError::into_inner).wait().map_err(error)?;
        if outcome.is_ok() && !status.success() {
            return Err("the print helper failed after its receipt".into());
        }
        outcome
    }
}

/// Keep helper transport failures distinct from a printer's own error message.
fn error(error: impl std::fmt::Display) -> String {
    format!("print helper: {error}")
}

/// Read at most one byte beyond the cap so oversized files fail without unbounded allocation.
pub(crate) fn bounded(mut input: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    input.by_ref().take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 > limit { Err("print helper input exceeds its limit".into()) } else { Ok(bytes) }
}

/// Read one bounded UTF-8 protocol record, distinguishing EOF from an oversized message.
pub(crate) fn line(input: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    input.take(MAX_MESSAGE + 1).read_until(b'\n', &mut bytes).map_err(error)?;
    if bytes.len() as u64 > MAX_MESSAGE {
        return Err("print helper message exceeds its limit".into());
    }
    if bytes.is_empty() { Ok(None) } else { String::from_utf8(bytes).map(|s| Some(s.trim_end().to_owned())).map_err(error) }
}

/// Serialize complete events under stdout's lock so the driver and listener cannot interleave.
fn emit(event: &Event) -> Result<(), String> {
    let mut output = std::io::stdout().lock();
    write!(output, "{PREFIX}").map_err(error)?;
    serde_json::to_writer(&mut output, event).map_err(error)?;
    writeln!(output).and_then(|_| output.flush()).map_err(error)
}

/// Remove both plaintext inputs even when one fails; already-removed inputs count as success.
pub(crate) fn remove_inputs(directory: &Path) -> Result<(), String> {
    let mut first_error = None;
    for name in ["job.json", "input.pdf"] {
        if let Err(failure) = std::fs::remove_file(directory.join(name))
            && failure.kind() != std::io::ErrorKind::NotFound
        {
            first_error.get_or_insert_with(|| error(format!("could not remove {name}: {failure}")));
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// The child side: load and erase the private inputs before driver work, while an independent
/// listener can cancel. `cancel_request` cancels the token and answers whether it was still
/// cancellable; `submit` is the driver.
pub(crate) fn helper(
    directory: &Path,
    cancel_request: impl Fn(&crate::Cancel) -> bool + Send + 'static,
    submit: impl FnOnce(&[u8], &crate::Job, &crate::Cancel) -> Result<String, String>,
) -> Result<(), String> {
    let mut input = BufReader::new(std::io::stdin());
    emit(&Event::Ready)?;
    if line(&mut input)?.as_deref() != Some("start") {
        return Err("print helper requires a start acknowledgement".into());
    }
    let cancel = crate::Cancel::new();
    let listener = cancel.clone();
    std::thread::Builder::new()
        .name("print-cancel".into())
        .spawn(move || {
            loop {
                match line(&mut input) {
                    Ok(Some(command)) if command == "cancel" => {
                        if cancel_request(&listener) {
                            let _ = emit(&Event::Cancelled);
                            std::process::exit(0);
                        }
                        if emit(&Event::Sending).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            // If the parent disappears, never leave a waiting driver process behind. Sending
            // jobs finish normally so the OS gets their pages even if the UI has gone away.
            if cancel_request(&listener) {
                std::process::exit(0);
            }
        })
        .map_err(error)?;
    let request: Request =
        serde_json::from_slice(&bounded(std::fs::File::open(directory.join("job.json")).map_err(error)?, MAX_REQUEST)?).map_err(error)?;
    let pdf = bounded(std::fs::File::open(directory.join("input.pdf")).map_err(error)?, MAX_PDF)?;
    remove_inputs(directory)?;
    let result = submit(&pdf, &request.job()?, &cancel);
    emit(&Event::Done(result))
}

/// Runs the print helper when the process was started with `--craft-print-helper`: the app's
/// `main` calls it first and exits with the code when it answers `Some`. The helper's reply lines
/// start `CRAFT_PRINT:`. It reads only its first argument, so it runs before logging or GUI
/// initialization.
#[cfg(windows)]
pub fn helper_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(FLAG)) {
        return None;
    }
    let result = args.next().ok_or_else(|| "print helper directory missing".to_owned()).and_then(|dir| {
        helper(Path::new(&dir), crate::Cancel::cancel, |pdf, job, cancel| crate::submit_cancellable(pdf, job, &mut |_, _| {}, cancel).map_err(error))
    });
    if let Err(message) = &result {
        let _ = emit(&Event::Done(Err(message.clone())));
    }
    Some(i32::from(result.is_err()))
}
