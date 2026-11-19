//! JSON-lines adapter that forwards valid JSON to a service and emits JSON-RPC parse errors.
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream};
use tokio::task::JoinHandle;

/// The service-facing streams and the tasks forwarding the client streams.
pub struct Adapter {
    /// Input for the JSON-RPC service.
    pub service_in: DuplexStream,
    /// Output from the JSON-RPC service.
    pub service_out: DuplexStream,
    /// Forwarding tasks, to finish after the service exits.
    pub tasks: Tasks,
}
/// Background forwarding tasks.
pub struct Tasks {
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}
impl Tasks {
    /// Stop the reader and allow queued output to finish, as after a service exits.
    /// This waits for a blocked output sink; callers needing cancellation can call `abort` instead.
    pub async fn finish(self) {
        self.reader.abort();
        let _ = self.writer.await;
    }
    /// Cancel both forwarding tasks, including a writer blocked on output.
    pub fn abort(self) {
        self.reader.abort();
        self.writer.abort();
    }
}

async fn bounded_line(reader: &mut BufReader<impl AsyncRead + Unpin>, bytes: &mut Vec<u8>, limit: usize) -> std::io::Result<Option<String>> {
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            if bytes.len() > limit {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "JSON-RPC line exceeds limit"));
            }
            break;
        }
        let newline = chunk.iter().position(|b| *b == b'\n');
        let count = newline.unwrap_or(chunk.len());
        let size = bytes.len().saturating_add(count);
        let last = if count == 0 { bytes.last().copied() } else { Some(chunk[count - 1]) };
        if size > limit && !(size == limit.saturating_add(1) && last == Some(b'\r')) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "JSON-RPC line exceeds limit"));
        }
        bytes.try_reserve(count).map_err(|_| std::io::Error::new(std::io::ErrorKind::OutOfMemory, "JSON-RPC line allocation failed"))?;
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count + usize::from(newline.is_some()));
        if newline.is_some() {
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            break;
        }
    }
    String::from_utf8(std::mem::take(bytes)).map(Some).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
/// Spawn a line adapter. Invalid JSON gets a JSON-RPC parse error; valid values pass through.
/// The bounded error queue applies backpressure if the client stops reading. Input and output lines
/// above their respective limits close their forwarding direction, including unterminated lines.
/// Buffers grow incrementally; forwarding and JSON parsing can temporarily hold additional copies.
pub fn spawn(
    input: impl AsyncRead + Unpin + Send + 'static,
    output: impl AsyncWrite + Unpin + Send + 'static,
    input_limit: usize,
    output_limit: usize,
) -> Adapter {
    const PIPE_BYTES: usize = 1 << 20;
    const MAX_QUEUED_PARSE_ERRORS: usize = 64;
    let (mut to_service, service_in) = tokio::io::duplex(PIPE_BYTES);
    let (service_out, from_service) = tokio::io::duplex(PIPE_BYTES);
    let (err_tx, mut err_rx) = tokio::sync::mpsc::channel::<String>(MAX_QUEUED_PARSE_ERRORS);
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(input);
        let mut partial = Vec::new();
        loop {
            let line = tokio::select! {
                l = bounded_line(&mut lines, &mut partial, input_limit) => match l { Ok(Some(l)) => l, _ => break },
                () = err_tx.closed() => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(_) => {
                    if to_service.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let reply = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}});
                    if err_tx.send(reply.to_string()).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = to_service.shutdown().await;
    });
    let writer = tokio::spawn(async move {
        let mut output = output;
        let mut replies = BufReader::new(from_service);
        let mut partial = Vec::new();
        loop {
            let line = tokio::select! {
                l = bounded_line(&mut replies, &mut partial, output_limit) => match l { Ok(Some(l)) => l, _ => break },
                Some(e) = err_rx.recv() => e,
            };
            if output.write_all(format!("{line}\n").as_bytes()).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });
    Adapter { service_in, service_out, tasks: Tasks { reader, writer } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    #[tokio::test(flavor = "current_thread")]
    async fn forwards_valid_lines_and_returns_parse_errors() {
        let (mut client_in, input) = tokio::io::duplex(1024);
        let (output, client_out) = tokio::io::duplex(1024);
        let Adapter { mut service_in, mut service_out, tasks } = spawn(input, output, 1024, 1024);
        client_in.write_all(b"\n{bad}\n{\"id\":1}\n").await.unwrap();
        client_in.shutdown().await.unwrap();
        let mut forwarded = String::new();
        service_in.read_to_string(&mut forwarded).await.unwrap();
        assert_eq!(forwarded, "{\"id\":1}\n");
        let mut lines = BufReader::new(client_out).lines();
        let error: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(error["error"]["code"], -32700);
        assert!(error["id"].is_null());
        service_out.write_all(b"{\"id\":1,\"result\":true}\n").await.unwrap();
        let result: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(result["result"], true);
        assert_eq!(result["id"], 1);
        service_out.shutdown().await.unwrap();
        tasks.finish().await;
    }
    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_line_read_retains_consumed_prefix() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;
        let (mut writer, stream) = tokio::io::duplex(32);
        let mut reader = BufReader::new(stream);
        let mut partial = Vec::new();
        writer.write_all(b"prefix").await.unwrap();
        {
            let mut read = Box::pin(bounded_line(&mut reader, &mut partial, 32));
            poll_fn(|cx| {
                assert!(matches!(read.as_mut().poll(cx), Poll::Pending));
                Poll::Ready(())
            })
            .await;
        }
        assert_eq!(partial, b"prefix");
        writer.write_all(b" suffix\n").await.unwrap();
        assert_eq!(bounded_line(&mut reader, &mut partial, 32).await.unwrap().as_deref(), Some("prefix suffix"));
    }
    #[tokio::test(flavor = "current_thread")]
    async fn parse_error_does_not_discard_partial_service_reply() {
        let (mut client, input) = tokio::io::duplex(128);
        let (output, client_out) = tokio::io::duplex(128);
        let Adapter { service_in: _, mut service_out, tasks } = spawn(input, output, 128, 128);
        service_out.write_all(b"{\"id\":1,").await.unwrap();
        tokio::task::yield_now().await;
        client.write_all(b"{bad}\n").await.unwrap();
        let mut lines = BufReader::new(client_out).lines();
        let error: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(error["error"]["code"], -32700);
        service_out.write_all(b"\"result\":true}\n").await.unwrap();
        let reply: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(reply, json!({"id": 1, "result": true}));
        tasks.abort();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn crlf_at_exact_limit_does_not_count_the_terminator() {
        let mut reader = BufReader::new(&b"1234\r\n"[..]);
        assert_eq!(bounded_line(&mut reader, &mut Vec::new(), 4).await.unwrap().as_deref(), Some("1234"));
        let mut reader = BufReader::new(&b"1234\r"[..]);
        assert!(bounded_line(&mut reader, &mut Vec::new(), 4).await.is_err());
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oversized_unterminated_lines_stop_both_directions() {
        let (mut client, input) = tokio::io::duplex(128);
        let (output, _client_out) = tokio::io::duplex(128);
        let Adapter { mut service_in, service_out, tasks } = spawn(input, output, 8, 8);
        client.write_all(b"123456789").await.unwrap();
        let mut forwarded = Vec::new();
        service_in.read_to_end(&mut forwarded).await.unwrap();
        assert!(forwarded.is_empty());
        drop(service_out);
        tasks.finish().await;

        let (_client, input) = tokio::io::duplex(128);
        let (output, mut client_out) = tokio::io::duplex(128);
        let Adapter { service_in: _, mut service_out, tasks } = spawn(input, output, 8, 8);
        service_out.write_all(b"123456789").await.unwrap();
        let mut received = Vec::new();
        client_out.read_to_end(&mut received).await.unwrap();
        assert!(received.is_empty());
        tasks.abort();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn blocked_output_tasks_can_be_cancelled() {
        let (_client, input) = tokio::io::duplex(16);
        let (output, _blocked) = tokio::io::duplex(1);
        let Adapter { service_in: _, mut service_out, tasks } = spawn(input, output, 64, 64);
        service_out.write_all(b"123456789\n").await.unwrap();
        tokio::task::yield_now().await;
        tasks.abort();
    }
}
