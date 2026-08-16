use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::storage::Storage;

/// Minimal HTTP health server so the container healthcheck (and external
/// monitoring) can probe a real endpoint instead of pid-1 comm. The crawler is
/// primarily a UDP daemon, so this speaks just enough HTTP/1.1 to answer
/// `GET /health` (and `/`). Returns a brief JSON status.
///
/// `storage` is used for a liveness probe (a trivial query proves Postgres is
/// reachable); `process_start_ts` is echoed so callers can derive uptime.
pub async fn serve(
    port: u16,
    process_start_ts: u64,
    storage: Storage,
    shutdown: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("bind health listener on {port}"))?;
    info!(port = port, "health http endpoint listening");

    loop {
        let (mut socket, _) = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            accepted = listener.accept() => accepted?,
        };
        let storage = storage.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            match socket.read(&mut buf).await {
                Ok(n) if n > 0 => {
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();
                    let db_ok = storage.ping().await.is_ok();
                    let status = if path == "/health" || path == "/" {
                        "ok"
                    } else {
                        "not_found"
                    };
                    let code = if status == "ok" {
                        "200 OK"
                    } else {
                        "404 Not Found"
                    };
                    let body = format!(
                        "{{\"status\":\"{status}\",\"port\":{port},\"process_start_ts\":{process_start_ts},\"postgres\":{db_ok}}}\n"
                    );
                    let resp = format!(
                        "HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    if socket.write_all(resp.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = socket.flush().await;
                }
                _ => {}
            }
        });
    }
}