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
