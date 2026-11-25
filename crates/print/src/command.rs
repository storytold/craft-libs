//! Bounded spooler commands with file-backed output, so neither full pipes nor inherited
//! pipe handles can leave a reader thread behind.

use std::io::{Read as _, Seek as _, SeekFrom};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) const OUTPUT_CAP: u64 = 1 << 20;

struct QueryChild(Child);

impl Drop for QueryChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn output(command: &mut Command, cancelled: &AtomicBool, limit: Duration) -> std::io::Result<std::process::Output> {
    let interrupted = || std::io::Error::new(std::io::ErrorKind::Interrupted, "print command cancelled");
    if cancelled.load(Ordering::Acquire) {
        return Err(interrupted());
    }
    let mut file = tempfile::tempfile()?;
    let mut errors = tempfile::tempfile()?;
    command.stdin(Stdio::null()).stdout(Stdio::from(file.try_clone()?)).stderr(Stdio::from(errors.try_clone()?));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    let mut child = QueryChild(command.spawn()?);
    let started = Instant::now();
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(interrupted());
        }
        if file.metadata()?.len() > OUTPUT_CAP || errors.metadata()?.len() > OUTPUT_CAP {
            return Err(std::io::Error::other("the print command returned too much data"));
        }
        if let Some(status) = child.0.try_wait()? {
            file.seek(SeekFrom::Start(0))?;
            let mut bytes = Vec::new();
            file.take(OUTPUT_CAP + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > OUTPUT_CAP {
                return Err(std::io::Error::other("the print command returned too much data"));
            }
            errors.seek(SeekFrom::Start(0))?;
            let mut stderr = Vec::new();
            errors.take(OUTPUT_CAP + 1).read_to_end(&mut stderr)?;
            if stderr.len() as u64 > OUTPUT_CAP {
                return Err(std::io::Error::other("the print command returned too much data"));
            }
            return Ok(std::process::Output { status, stdout: bytes, stderr });
        }
        if started.elapsed() >= limit {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the print command timed out"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(windows: &str, unix: &str) -> Command {
        #[cfg(windows)]
        {
            let _ = unix;
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command", windows]);
            command
        }
        #[cfg(not(windows))]
        {
            let _ = windows;
            let mut command = Command::new("sh");
            command.args(["-c", unix]);
            command
        }
    }

    #[test]
    fn output_status_and_both_streams_are_preserved() {
        let answer = output(
            &mut script("[Console]::Out.Write('true'); [Console]::Error.Write('problem'); exit 7", "printf true; printf problem >&2; exit 7"),
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(!answer.status.success());
        assert_eq!(answer.stdout, b"true");
        assert_eq!(answer.stderr, b"problem");
        assert_eq!(
            output(&mut Command::new("craft-no-such-helper"), &AtomicBool::new(false), Duration::from_secs(1)).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            output(&mut Command::new("craft-no-such-helper"), &AtomicBool::new(true), Duration::from_secs(1)).unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn cancellation_and_deadlines_kill_and_reap_the_command() {
        let started = Instant::now();
        let timed_out =
            output(&mut script("Start-Sleep -Seconds 30", "exec sleep 30"), &AtomicBool::new(false), Duration::from_millis(100)).unwrap_err();
        assert_eq!(timed_out.kind(), std::io::ErrorKind::TimedOut);
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                cancelled.store(true, Ordering::Release);
            });
            let stopped = output(&mut script("Start-Sleep -Seconds 30", "exec sleep 30"), &cancelled, Duration::from_secs(30)).unwrap_err();
            assert_eq!(stopped.kind(), std::io::ErrorKind::Interrupted);
        });
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn oversized_output_is_refused() {
        let error = output(
            &mut script("[Console]::Out.Write(('x' * 1048577))", "exec head -c 1048577 /dev/zero"),
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
    }
}
