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
    pub async fn finish(self) {
        self.reader.abort();
        let _ = self.writer.await;
    }
}
/// Spawn a line adapter. Invalid JSON gets a JSON-RPC parse error; valid values pass through.
/// The bounded error queue applies backpressure if the client stops reading.
pub fn spawn(input: impl AsyncRead + Unpin + Send + 'static, output: impl AsyncWrite + Unpin + Send + 'static) -> Adapter {
    const PIPE_BYTES: usize = 1 << 20;
    const MAX_QUEUED_PARSE_ERRORS: usize = 64;
    let (mut to_service, service_in) = tokio::io::duplex(PIPE_BYTES);
    let (service_out, from_service) = tokio::io::duplex(PIPE_BYTES);
    let (err_tx, mut err_rx) = tokio::sync::mpsc::channel::<String>(MAX_QUEUED_PARSE_ERRORS);
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(input).lines();
        loop {
            let line = tokio::select! {
                l = lines.next_line() => match l { Ok(Some(l)) => l, _ => break },
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
        let mut replies = BufReader::new(from_service).lines();
        loop {
            let line = tokio::select! {
                l = replies.next_line() => match l { Ok(Some(l)) => l, _ => break },
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
        let Adapter { mut service_in, mut service_out, tasks } = spawn(input, output);
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
}
