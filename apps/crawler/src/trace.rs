use crate::krpc::NodeId;
use std::sync::OnceLock;

pub struct TraceConfig {
    pub sample_rate: f64,
    pub debug_ih: Option<NodeId>,
}

pub static TRACE_CONFIG: OnceLock<TraceConfig> = OnceLock::new();

impl TraceConfig {
    pub fn new(sample_rate: f64, debug_ih: Option<String>) -> Self {
        let debug_ih = debug_ih.and_then(|s| {
            if s.len() == 40 {
                let mut buf = [0u8; 20];
                for i in 0..20 {
                    let byte_str = &s[i*2..i*2+2];
                    if let Ok(byte) = u8::from_str_radix(byte_str, 16) {
                        buf[i] = byte;
                    } else {
                        return None;
                    }
                }
                Some(buf)
            } else {
                None
            }
        });
        Self { sample_rate, debug_ih }
    }
}

pub fn should_trace(ih: &NodeId) -> bool {
    let Some(config) = TRACE_CONFIG.get() else { return false };
    if let Some(debug) = &config.debug_ih {
        if debug == ih {
            return true;
        }
    }
    if config.sample_rate <= 0.0 {
        return false;
    }
    if config.sample_rate >= 1.0 {
        return true;
    }
    let hash_val = (ih[0] as f64 + (ih[1] as f64 / 256.0)) / 256.0;
    hash_val < config.sample_rate
}

pub fn hex_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

#[macro_export]
macro_rules! trace_lifecycle {
    ($ih:expr, $stage:expr, stream = $stream:expr $(, $key:ident = $val:expr)*) => {
        if $crate::trace::should_trace($ih) {
            tracing::info!(
                target: "craw_trace",
                ih = %$crate::trace::hex_encode($ih),
                stage = $stage,
                stream = $stream,
                $($key = $val),*
            );
        }
    };
}
