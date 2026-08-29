use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};
use tracing::field::Visit;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

pub struct JsonLayer {
    tx: tokio::sync::mpsc::Sender<String>,
    log_dropped: Arc<AtomicU64>,
}

impl JsonLayer {
    pub fn new(tx: tokio::sync::mpsc::Sender<String>, log_dropped: Arc<AtomicU64>) -> Self {
        Self { tx, log_dropped }
    }
}

impl<S> Layer<S> for JsonLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();

        let mut fields = Map::new();

        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);

        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let level = match *meta.level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            tracing::Level::DEBUG => "debug",
            tracing::Level::TRACE => "trace",
        };

        let stream = fields
            .remove("stream")
            .map(|v| v.as_str().unwrap_or("system").to_string())
            .unwrap_or_else(|| "system".to_string());

        let mut obj = Map::new();
        obj.insert("ts".into(), Value::String(ts));
        obj.insert("level".into(), Value::String(level.to_string()));
        obj.insert("service".into(), Value::String("crawler".into()));
        obj.insert("stream".into(), Value::String(stream));
        obj.insert("target".into(), Value::String(meta.target().to_string()));

        for (k, v) in fields {
            obj.insert(k, v);
        }

        let line = match serde_json::to_string(&obj) {
            Ok(s) => s,
            Err(_) => return,
        };

        if self.tx.try_send(line).is_err() {
            self.log_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct FieldVisitor<'a>(&'a mut Map<String, Value>);

impl<'a> Visit for FieldVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{:?}", value);
        let v = if let Some(stripped) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            Value::String(stripped.to_string())
        } else {
            Value::String(s)
        };
        self.0.insert(field.name().to_string(), v);
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0
            .insert(field.name().to_string(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0
            .insert(field.name().to_string(), Value::Number(value.into()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), Value::Bool(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }
}
