mod layer;
mod writer;

pub use layer::JsonLayer;

use std::sync::atomic::{AtomicU64};
use std::sync::Arc;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

const BATCH_SIZE: usize = 1000;

pub struct LoggingGuard {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for LoggingGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

pub fn init(config: &Config, log_dropped: Arc<AtomicU64>) -> LoggingGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    if config.log_json {
        let log_dir = config.log_dir.clone();
        let file_max = config.log_file_max_bytes;
        let total_max = config.log_total_max_bytes;
        let flush_ms = config.log_flush_interval_ms;
        let buffer_cap = config.log_buffer_capacity;

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(buffer_cap);

        let layer = JsonLayer::new(tx, log_dropped);

        let writer = writer::AsyncWriter::new(
            rx,
            log_dir,
            file_max,
            total_max,
            flush_ms,
            shutdown_rx,
        );

        tokio::spawn(writer.run());

        tracing_subscriber::registry()
            .with(filter)
            .with(layer)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .init();
    }

    LoggingGuard {
        shutdown_tx: Some(shutdown_tx),
    }
}
