//! Bounded, cancellable output capture for the desktop's one-shot settings fallback.

/// Reads at most 8193 bytes from a pipe, without a detached reader or disk spool.
/// Cancellation/timeout drops the reader and kills the owned child; async-process reaps it.
pub(crate) async fn capture(program: &std::path::Path, args: &[&str], timeout: std::time::Duration) -> Option<String> {
    use futures_lite::io::AsyncReadExt;
    use std::process::Stdio;
    let mut child = async_process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .reap_on_drop(true)
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    futures_lite::future::race(
        async {
            let mut bytes = Vec::new();
            stdout.take(8193).read_to_end(&mut bytes).await.ok()?;
            if bytes.len() > 8192 {
                return None;
            }
            if !child.status().await.ok()?.success() {
                return None;
            }
            String::from_utf8(bytes).ok()
        },
        async {
            async_io::Timer::after(timeout).await;
            None
        },
    )
    .await
}
#[cfg(all(test, feature = "appearance", unix, not(target_os = "macos")))]
mod tests {
    use super::capture;
    #[test]
    fn capture_checks_exit_status_size_and_deadline() {
        use std::time::{Duration, Instant};
        futures_lite::future::block_on(async {
            let shell = std::path::Path::new("/bin/sh");
            assert_eq!(capture(shell, &["-c", "printf hello"], Duration::from_secs(1)).await.as_deref(), Some("hello"));
            assert_eq!(capture(shell, &["-c", "printf failure; exit 7"], Duration::from_secs(1)).await, None);
            let before = Instant::now();
            assert_eq!(capture(shell, &["-c", "exec sleep 5"], Duration::from_millis(30)).await, None);
            assert!(before.elapsed() < Duration::from_secs(1));
            assert_eq!(capture(shell, &["-c", "head -c 10000 /dev/zero"], Duration::from_secs(1)).await, None);
        });
    }
}
