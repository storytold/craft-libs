//! One bounded run of a child process, shared by the modules that start programs.
//!
//! [`run`] starts a [`Command`] with standard input connected to nothing and reads standard output
//! and standard error at the same time, each on its own thread, under [`RunLimits`]. When a stream
//! passes `max_output_bytes`, when `timeout` passes, or when reading fails, the child is killed and
//! reaped before the error returns, and `run` waits up to 2 seconds for the pipe readers to finish.
//!
//! On Unix the child leads its own process group, and the whole group is killed, so a process it
//! started that still holds the output pipes ends too. A successful run waits for the child alone
//! and leaves the other members of its group running. On Windows the child and its descendants run
//! in a Job Object: a failed run terminates the job, and every run closes it when `run` returns,
//! which ends any process still in it. A successful run waits for the child alone, not for every
//! process in the job.

use std::fmt;
use std::io::{self, Read};
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

#[cfg(not(windows))]
type Child = std::process::Child;
#[cfg(windows)]
type Child = Box<dyn process_wrap::std::ChildWrapper>;

/// Limits for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RunLimits {
    /// The most bytes the child may write to standard output, and separately to standard error.
    pub(crate) max_output_bytes: usize,
    /// How long the child may run, counted from its start. `None` waits for it without a limit.
    pub(crate) timeout: Option<Duration>,
}

/// One of a child's output streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// The child's standard output.
    Stdout,
    /// The child's standard error.
    Stderr,
}

impl fmt::Display for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        })
    }
}

/// Why a run failed. For every variant after `Start`, the child has been killed and reaped.
#[derive(Debug)]
pub(crate) enum RunError {
    /// The timeout is too long to add to the current time. Nothing was started.
    InvalidTimeout(Duration),
    /// The program could not be started.
    Start(io::Error),
    /// The child ran past the timeout, or its exit was first seen after the deadline.
    Timeout(Duration),
    /// `stream` passed `limit` bytes.
    OutputLimit { stream: Stream, limit: usize },
    /// Reading a stream, starting a reader thread or waiting for the child failed.
    Read(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTimeout(timeout) => write!(f, "a timeout of {timeout:?} is too long to set a deadline"),
            Self::Start(source) => write!(f, "could not start the program: {source}"),
            Self::Timeout(timeout) => write!(f, "the child did not finish within {timeout:?} and was killed"),
            Self::OutputLimit { stream, limit } => write!(f, "the child wrote more than {limit} bytes to {stream} and was killed"),
            Self::Read(source) => write!(f, "could not read the child's output or exit status: {source}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Start(source) | Self::Read(source) => Some(source),
            _ => None,
        }
    }
}

/// What a test can observe about one run.
#[derive(Debug, Default)]
pub(crate) struct Observed {
    /// The child's process id.
    pub(crate) pid: Option<u32>,
    /// Whether every pending pipe reader finished before a failed run returned.
    pub(crate) pipes_closed: bool,
}

/// How long a failed run waits for its pipe readers to finish after the child is killed.
const PIPE_GRACE: Duration = Duration::from_secs(2);

enum ReadFail {
    Limit,
    Io(io::Error),
}

type StreamResult = (Stream, Result<Vec<u8>, ReadFail>);

/// Runs `cmd` under `limits` and returns its exit status and output. The exit status is not
/// checked. The runner sets standard input, standard output and standard error, and on Unix the
/// process group; every other setting of `cmd` is kept. A timeout too long to set a deadline
/// returns [`RunError::InvalidTimeout`] without starting the program.
pub(crate) fn run(mut cmd: Command, limits: RunLimits, seen: &mut Observed) -> Result<Output, RunError> {
    if let Some(timeout) = limits.timeout
        && Instant::now().checked_add(timeout).is_none()
    {
        return Err(RunError::InvalidTimeout(timeout));
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = spawn(cmd).map_err(RunError::Start)?;
    seen.pid = Some(child.id());
    // The timeout counts from the start of the child, not from the start of `spawn`'s retries.
    let deadline = match limits.timeout.map(|timeout| Instant::now().checked_add(timeout).ok_or(timeout)) {
        Some(Err(timeout)) => {
            kill_and_reap(&mut child);
            return Err(RunError::InvalidTimeout(timeout));
        }
        Some(Ok(deadline)) => Some(deadline),
        None => None,
    };
    let (tx, rx) = mpsc::channel();
    let (stdout_pipe, stderr_pipe) = take_pipes(&mut child);
    if let Err(source) = spawn_reader(stdout_pipe, Stream::Stdout, limits.max_output_bytes, tx.clone()) {
        return abort(&mut child, &rx, 0, seen, RunError::Read(source));
    }
    if let Err(source) = spawn_reader(stderr_pipe, Stream::Stderr, limits.max_output_bytes, tx) {
        return abort(&mut child, &rx, 1, seen, RunError::Read(source));
    }
    let timeout = || RunError::Timeout(limits.timeout.unwrap_or_default());
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let mut pending = 2;
    while pending > 0 {
        let received = receive(&rx, deadline);
        if received.is_ok() {
            pending -= 1;
        }
        match received {
            Ok((Stream::Stdout, Ok(bytes))) => stdout = bytes,
            Ok((Stream::Stderr, Ok(bytes))) => stderr = bytes,
            Ok((stream, Err(ReadFail::Limit))) => {
                return abort(&mut child, &rx, pending, seen, RunError::OutputLimit { stream, limit: limits.max_output_bytes });
            }
            Ok((_, Err(ReadFail::Io(source)))) => return abort(&mut child, &rx, pending, seen, RunError::Read(source)),
            Err(RecvTimeoutError::Timeout) => return abort(&mut child, &rx, pending, seen, timeout()),
            Err(RecvTimeoutError::Disconnected) => {
                let source = io::Error::other("a pipe reader stopped without a result");
                return abort(&mut child, &rx, 0, seen, RunError::Read(source));
            }
        }
    }
    let status = wait_for_exit(&mut child, deadline).map_err(|e| e.map_or_else(timeout, RunError::Read))?;
    Ok(Output { status, stdout, stderr })
}

/// The next reader result, waiting until `deadline` or, without one, until a reader sends.
fn receive(rx: &Receiver<StreamResult>, deadline: Option<Instant>) -> Result<StreamResult, RecvTimeoutError> {
    match deadline {
        Some(deadline) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
    }
}

/// Read `r` to its end, or stop with [`ReadFail::Limit`] once it holds more than `cap` bytes.
fn read_capped(mut r: impl Read, cap: usize) -> Result<Vec<u8>, ReadFail> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = match r.read(&mut chunk) {
            Ok(0) => return Ok(buf),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(ReadFail::Io(e)),
        };
        if buf.len().saturating_add(n) > cap {
            return Err(ReadFail::Limit);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Read one pipe on its own thread and send the result with its stream.
fn spawn_reader(pipe: Option<impl Read + Send + 'static>, stream: Stream, cap: usize, tx: Sender<StreamResult>) -> io::Result<()> {
    let Some(pipe) = pipe else {
        let _ = tx.send((stream, Ok(Vec::new())));
        return Ok(());
    };
    std::thread::Builder::new().name(format!("craft-testkit-process-{stream}")).spawn(move || {
        let _ = tx.send((stream, read_capped(pipe, cap)));
    })?;
    Ok(())
}

/// Kill the child and reap it. On Unix the child leads its own process group, and the whole group
/// is signaled. A process the child started that still holds the output pipes ends too. On Windows
/// the child's job object is terminated, including descendants. The child must not have been
/// reaped yet: on Unix its process id, and with it the group id, can be reused after that.
fn kill_and_reap(child: &mut Child) {
    #[cfg(unix)]
    let _ = rustix::process::kill_process_group(rustix::process::Pid::from_child(child), rustix::process::Signal::KILL);
    let _ = child.kill();
    let _ = child.wait();
}

/// Kill and reap the child, then wait up to [`PIPE_GRACE`] for the `pending` pipe readers.
fn abort(child: &mut Child, rx: &Receiver<StreamResult>, mut pending: usize, seen: &mut Observed, e: RunError) -> Result<Output, RunError> {
    kill_and_reap(child);
    let until = Instant::now().checked_add(PIPE_GRACE).unwrap_or_else(Instant::now);
    while pending > 0 {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(_) => pending -= 1,
            Err(RecvTimeoutError::Disconnected) => pending = 0,
            Err(RecvTimeoutError::Timeout) => break,
        }
    }
    seen.pipes_closed = pending == 0;
    Err(e)
}

/// Start `cmd`, trying again up to 8 times while the program file is busy (`ETXTBSY`). A file is
/// busy for a moment after it was written while another thread starts a process, which happens
/// when a test writes a stand-in program and runs it.
#[cfg(not(windows))]
fn spawn(mut cmd: Command) -> io::Result<Child> {
    let mut pause = Duration::from_millis(1);
    for _ in 0..8 {
        match cmd.spawn() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy => std::thread::sleep(pause),
            started => return started,
        }
        pause *= 2;
    }
    cmd.spawn()
}

#[cfg(windows)]
fn spawn(cmd: Command) -> io::Result<Child> {
    use process_wrap::std::{CommandWrap, JobObject, KillOnDrop};
    let mut wrapped = CommandWrap::from(cmd);
    wrapped.wrap(KillOnDrop).wrap(JobObject);
    wrapped.spawn()
}

fn take_pipes(child: &mut Child) -> (Option<ChildStdout>, Option<ChildStderr>) {
    #[cfg(not(windows))]
    {
        (child.stdout.take(), child.stderr.take())
    }
    #[cfg(windows)]
    {
        (child.stdout().take(), child.stderr().take())
    }
}

/// The child's exit status, if it has exited, without reaping any other process.
#[cfg(not(windows))]
fn child_exit(child: &mut Child) -> io::Result<Option<ExitStatus>> {
    child.try_wait()
}

/// The child's exit status, if it has exited. `JobObjectChild::try_wait` in process-wrap 11 reports
/// an exit only once every process in the job has ended, so this asks the wrapper below it, which
/// passes the call to the child's own `try_wait`. `spawn` applies `KillOnDrop` first and `JobObject`
/// over it, so the outer wrapper is the job.
#[cfg(windows)]
fn child_exit(child: &mut Child) -> io::Result<Option<ExitStatus>> {
    child.inner_mut().try_wait()
}

/// Wait for a child whose pipes have closed. Past `deadline` it is killed and reaped, and the
/// error is `None`; a failed wait kills and reaps it too and returns the error.
fn wait_for_exit(child: &mut Child, deadline: Option<Instant>) -> Result<ExitStatus, Option<io::Error>> {
    wait_for_exit_with_clock(child, deadline, Instant::now)
}

/// The deadline is checked before each look at the child, so the child is never signaled after it
/// was reaped, and an exit first seen after the deadline is a timeout.
fn wait_for_exit_with_clock(child: &mut Child, deadline: Option<Instant>, mut now: impl FnMut() -> Instant) -> Result<ExitStatus, Option<io::Error>> {
    let mut pause = Duration::from_millis(1);
    loop {
        let left = deadline.map(|deadline| deadline.checked_duration_since(now()));
        if left == Some(None) {
            kill_and_reap(child);
            return Err(None);
        }
        match child_exit(child) {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => {
                kill_and_reap(child);
                return Err(Some(e));
            }
        }
        std::thread::sleep(left.flatten().map_or(pause, |left| left.min(pause)));
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: RunLimits = RunLimits { max_output_bytes: 1024, timeout: Some(Duration::from_secs(60)) };

    /// `/bin/sh -c script`.
    #[cfg(unix)]
    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    /// Whether a process with this id still exists, reaped or not. An unreaped child stays visible.
    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        Command::new("sh").arg("-c").arg(format!("kill -0 {pid} 2>/dev/null")).status().unwrap().success()
    }

    /// A child of `sh -c 'exit 0'` that has exited and is not reaped yet.
    #[cfg(unix)]
    fn exited_child() -> Child {
        use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
        let child = spawn(sh("exit 0")).unwrap();
        waitid(WaitId::Pid(Pid::from_child(&child)), WaitIdOptions::EXITED | WaitIdOptions::NOWAIT).unwrap();
        child
    }

    #[cfg(unix)]
    #[test]
    fn an_exit_observed_after_the_deadline_is_a_timeout() {
        let deadline = Instant::now();
        let observed = deadline + Duration::from_secs(1);
        let result = wait_for_exit_with_clock(&mut exited_child(), Some(deadline), || observed);
        assert!(matches!(result, Err(None)), "{result:?}");
        assert!(wait_for_exit_with_clock(&mut exited_child(), Some(deadline), || deadline).is_ok());
    }

    #[test]
    fn output_is_read_up_to_the_limit() {
        assert_eq!(read_capped(&[7u8; 100][..], 100).ok(), Some(vec![7u8; 100]));
        assert!(matches!(read_capped(&[7u8; 101][..], 100), Err(ReadFail::Limit)));
        assert_eq!(read_capped(&[][..], 0).ok(), Some(Vec::new()));
    }

    #[cfg(unix)]
    #[test]
    fn version_child_is_reaped_after_timeout() {
        let mut seen = Observed::default();
        // The child writes nothing and a zero timeout has passed when the wait starts, so the result
        // does not depend on timing.
        let limits = RunLimits { max_output_bytes: 1024, timeout: Some(Duration::ZERO) };
        let got = run(sh("exec sleep 1000"), limits, &mut seen);
        assert!(matches!(got, Err(RunError::Timeout(_))), "the child must time out: {got:?}");
        let pid = seen.pid.unwrap();
        assert!(!process_exists(pid), "child {pid} was left running or unreaped");
    }

    #[test]
    fn timeout_without_a_deadline_is_an_error() {
        let mut seen = Observed::default();
        let limits = RunLimits { max_output_bytes: 1024, timeout: Some(Duration::MAX) };
        let got = run(Command::new("craft-testkit-no-such-program"), limits, &mut seen);
        assert!(matches!(got, Err(RunError::InvalidTimeout(Duration::MAX))), "{got:?}");
        assert_eq!(seen.pid, None, "the program must not start");
    }

    #[cfg(unix)]
    #[test]
    fn version_child_is_reaped_after_output_limit() {
        let mut seen = Observed::default();
        let got = run(sh("exec yes"), LIMITS, &mut seen);
        assert!(matches!(got, Err(RunError::OutputLimit { stream: Stream::Stdout, limit: 1024 })), "{got:?}");
        let pid = seen.pid.unwrap();
        assert!(!process_exists(pid), "child {pid} was left running or unreaped");
    }

    #[cfg(unix)]
    #[test]
    fn the_output_limit_names_the_stream() {
        let got = run(sh("exec yes >&2"), LIMITS, &mut Observed::default());
        assert!(matches!(got, Err(RunError::OutputLimit { stream: Stream::Stderr, limit: 1024 })), "{got:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_run_without_a_timeout_returns_the_exit_status() {
        let limits = RunLimits { max_output_bytes: 1024, timeout: None };
        let out = run(sh("printf x; printf y >&2; exit 3"), limits, &mut Observed::default()).unwrap();
        assert_eq!(out.status.code(), Some(3));
        assert_eq!((out.stdout.as_slice(), out.stderr.as_slice()), (&b"x"[..], &b"y"[..]));
    }

    /// A descendant that holds neither output pipe does not hold up a successful run: the run ends
    /// when the child has exited and both pipes have closed.
    #[cfg(unix)]
    #[test]
    fn a_descendant_without_the_pipes_does_not_hold_up_a_successful_run() {
        let mut seen = Observed::default();
        let limits = RunLimits { max_output_bytes: 1024, timeout: None };
        let got = run(sh("sleep 1000 </dev/null >/dev/null 2>&1 & echo started"), limits, &mut seen);
        let pid = seen.pid.unwrap();
        // A successful run leaves the rest of the child's group running; end it here.
        let _ = Command::new("/bin/sh").args(["-c", &format!("kill -s KILL -- -{pid}")]).status();
        let out = got.unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"started\n");
    }

    /// On Windows, a successful run waits for the child alone. A descendant that holds neither
    /// output pipe stays in the job until `run` returns and closes it.
    #[cfg(windows)]
    #[test]
    fn a_descendant_without_the_pipes_does_not_hold_up_a_successful_run() {
        let powershell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        // `Start-Process -WindowStyle Hidden` starts the descendant through the shell, which does
        // not pass it the child's handles, so it holds neither output pipe.
        let script = "$null = Start-Process -FilePath $env:CRAFT_TEST_PROCESS_POWERSHELL -ArgumentList '-NoLogo','-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 1000' -WindowStyle Hidden -PassThru; 'started'";
        let mut command = Command::new(&powershell);
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script]);
        command.env("CRAFT_TEST_PROCESS_POWERSHELL", &powershell);
        let got = run(command, LIMITS, &mut Observed::default());
        let out = got.unwrap();
        assert!(out.status.success(), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "started");
    }

    #[cfg(windows)]
    #[test]
    fn windows_descendants_release_the_output_pipes_after_abort() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::path::PathBuf;

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let directory = std::env::temp_dir().join(format!("craft-testkit-process-windows-descendant-{}-{nanos:x}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let ready = directory.join("ready");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut started = [0];
            stream.read_exact(&mut started).unwrap();
            tx.send(stream.try_clone().unwrap()).unwrap();
            stream.write_all(&[1]).unwrap();
        });
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let script = r#"
$child = @'
$client = [Net.Sockets.TcpClient]::new('127.0.0.1', [int]$env:CRAFT_TEST_ORACLE_PORT)
$stream = $client.GetStream()
$stream.WriteByte(1)
$null = $stream.ReadByte()
[IO.File]::WriteAllText($env:CRAFT_TEST_ORACLE_READY, 'ready')
$null = $stream.ReadByte()
'@
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($child))
$info = [Diagnostics.ProcessStartInfo]::new()
$info.FileName = $env:CRAFT_TEST_ORACLE_POWERSHELL
$info.Arguments = '-NoLogo -NoProfile -NonInteractive -EncodedCommand ' + $encoded
$info.UseShellExecute = $false
$null = [Diagnostics.Process]::Start($info)
while (-not [IO.File]::Exists($env:CRAFT_TEST_ORACLE_READY)) { [Threading.Thread]::Yield() | Out-Null }
$output = [Console]::OpenStandardOutput()
$bytes = [byte[]]::new(8192)
$output.Write($bytes, 0, $bytes.Length)
$output.Flush()
"#;
        let mut command = Command::new(&powershell);
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script]);
        command.env("CRAFT_TEST_ORACLE_PORT", port.to_string());
        command.env("CRAFT_TEST_ORACLE_READY", &ready);
        command.env("CRAFT_TEST_ORACLE_POWERSHELL", &powershell);
        let mut seen = Observed::default();
        let limits = RunLimits { max_output_bytes: 1024, timeout: Some(Duration::from_secs(5)) };
        let got = run(command, limits, &mut seen);
        let mut held = rx.try_recv().expect("the descendant connected before the output limit");
        let _ = held.write_all(&[1]);
        drop(held);
        let _ = std::fs::remove_dir_all(&directory);
        assert!(matches!(got, Err(RunError::OutputLimit { .. })), "{got:?}");
        assert!(seen.pipes_closed, "a descendant kept the output pipe readers blocked after abort");
    }

    /// A process the child started in the background keeps standard error open. Killing the child's
    /// process group ends it, so both pipe readers finish.
    #[cfg(unix)]
    #[test]
    fn descendants_holding_the_pipes_are_killed_with_the_child() {
        let mut seen = Observed::default();
        let got = run(sh("sleep 1000 &\nexec yes"), LIMITS, &mut seen);
        assert!(matches!(got, Err(RunError::OutputLimit { .. })), "{got:?}");
        assert!(seen.pipes_closed, "a pipe reader was left blocked on a pipe a descendant holds");
    }
}
