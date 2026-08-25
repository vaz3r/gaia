mod layer;
mod writer;

pub use layer::JsonLayer;

use std::sync::atomic::{AtomicU64};
use std::sync::Arc;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

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

    if config.logging.log_json {
        let log_dir = config.logging.log_dir.clone();
        let file_max = config.logging.log_file_max_bytes;
        let total_max = config.logging.log_total_max_bytes;
        let flush_ms = config.logging.log_flush_interval_ms;
        let buffer_cap = config.logging.log_buffer_capacity.max(1);
        let batch_size = config.logging.log_batch_size.max(1);
        let max_file_age_secs = config.logging.log_max_file_age_secs;

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(buffer_cap);

        let layer = JsonLayer::new(tx, log_dropped);

        let writer = writer::AsyncWriter::new(
            rx,
            log_dir,
            file_max,
            total_max,
            flush_ms,
            batch_size,
            max_file_age_secs,
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
