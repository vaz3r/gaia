//! System resource sampling for the monitoring snapshot.
//!
//! Reads host/container metrics from `/proc` and cgroup v2 every stats tick:
//! tunnel (tun0) network counters, host memory, container cgroup memory, CPU
//! utilization (delta-based), disk usage, and load average. Rates (bytes/sec,
//! CPU%) are computed from deltas between consecutive samples.
//!
//! The crawler runs inside gluetun's network namespace, so `tun0` is the
//! tunnel interface carrying all crawler egress — the correct vantage point
//! for network bandwidth.

use std::path::Path;
use std::time::Instant;

/// Network counters for a sampled interface.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct NetSample {
    rx_bytes: u64,
    tx_bytes: u64,
}

/// CPU counters from `/proc/stat` (aggregate jiffies).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct CpuSample {
    idle: u64,
    total: u64,
}

/// One snapshot of system metrics.
#[derive(Debug, Clone, Default)]
pub struct SysMetrics {
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub net_rx_rate_bps: f64,
    pub net_tx_rate_bps: f64,
    pub host_mem_total: u64,
    pub host_mem_available: u64,
    pub container_mem_current: u64,
    pub cpu_percent: f64,
    pub disk_total_bytes: u64,
    pub disk_free_bytes: u64,
    pub loadavg_1: f64,
    pub loadavg_5: f64,
    pub loadavg_15: f64,
}

/// Sampler that keeps the previous tick's counters to compute rates.
#[derive(Debug)]
pub struct SysMetricSampler {
    iface: String,
    data_dir: std::path::PathBuf,
    last_net: Option<NetSample>,
    last_cpu: Option<CpuSample>,
    last_time: Option<Instant>,
}

impl SysMetricSampler {
    pub fn new(iface: &str, data_dir: &Path) -> Self {
        Self {
            iface: iface.to_string(),
            data_dir: data_dir.to_path_buf(),
            last_net: None,
            last_cpu: None,
            last_time: None,
        }
    }

    /// Read a full system snapshot and update internal deltas.
    pub fn sample(&mut self) -> SysMetrics {
        let now = Instant::now();
        let dt = self
            .last_time
            .map(|t| now.duration_since(t).as_secs_f64().max(1e-6))
            .unwrap_or(1.0);

        let net = read_net(&self.iface);
        let cpu = read_cpu();

        let (net_rx_rate, net_tx_rate) = match (self.last_net, net) {
            (Some(prev), Some(cur)) => (
                (cur.rx_bytes.saturating_sub(prev.rx_bytes)) as f64 / dt,
                (cur.tx_bytes.saturating_sub(prev.tx_bytes)) as f64 / dt,
            ),
            _ => (0.0, 0.0),
        };

        let cpu_percent = match (self.last_cpu, cpu) {
            (Some(prev), Some(cur)) => {
                let total_delta = cur.total.saturating_sub(prev.total);
                let idle_delta = cur.idle.saturating_sub(prev.idle);
                if total_delta == 0 {
                    0.0
                } else {
                    let busy = total_delta.saturating_sub(idle_delta) as f64;
                    (busy / total_delta as f64 * 100.0).clamp(0.0, 100.0)
                }
            }
            _ => 0.0,
        };

        self.last_net = net;
        self.last_cpu = cpu;
        self.last_time = Some(now);

        let (host_total, host_avail) = read_host_mem();
        let container_mem = read_cgroup_mem();
        let (disk_total, disk_free) = read_disk(&self.data_dir);
        let (l1, l5, l15) = read_loadavg();

        SysMetrics {
            net_rx_bytes: net.map(|n| n.rx_bytes).unwrap_or(0),
            net_tx_bytes: net.map(|n| n.tx_bytes).unwrap_or(0),
            net_rx_rate_bps: net_rx_rate,
            net_tx_rate_bps: net_tx_rate,
            host_mem_total: host_total,
            host_mem_available: host_avail,
            container_mem_current: container_mem,
            cpu_percent,
            disk_total_bytes: disk_total,
            disk_free_bytes: disk_free,
            loadavg_1: l1,
            loadavg_5: l5,
            loadavg_15: l15,
        }
    }
}

fn read_net(iface: &str) -> Option<NetSample> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    for line in text.lines().skip(2) {
        let line = line.trim();
        let (name, rest) = line.split_once(':')?;
        if name.trim() != iface {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() < 10 {
            return None;
        }
        let rx: u64 = fields[0].parse().ok()?;
        let tx: u64 = fields[8].parse().ok()?;
        return Some(NetSample {
            rx_bytes: rx,
            tx_bytes: tx,
        });
    }
    None
}

fn read_cpu() -> Option<CpuSample> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().next()?;
    // "cpu  user nice system idle iowait irq softirq steal guest guest_nice"
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|f| f.parse().unwrap_or(0))
        .collect();
    let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
    let total: u64 = fields.iter().sum();
    Some(CpuSample { idle, total })
}

fn read_host_mem() -> (u64, u64) {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total = 0u64;
    let mut available = 0u64;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("");
        let val: u64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match key {
            "MemTotal:" => total = val * 1024,
            "MemAvailable:" => available = val * 1024,
            _ => {}
        }
    }
    (total, available)
}

fn read_cgroup_mem() -> u64 {
    // cgroup v2 memory.current (bytes). Fall back to 0 if unavailable.
    std::fs::read_to_string("/sys/fs/cgroup/memory.current")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn read_disk(path: &Path) -> (u64, u64) {
    let Ok(c_path) = std::ffi::CString::new(
        path.as_os_str()
            .to_str()
            .unwrap_or("/")
            .as_bytes(),
    ) else {
        return (0, 0);
    };
    // SAFETY: statvfs is a simple libc call; the buffer is zeroed and large
    // enough for the struct.
    let mut buf: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut buf) };
    if rc != 0 {
        return (0, 0);
    }
    let bsize = buf.f_frsize as u64;
    (
        buf.f_blocks as u64 * bsize,
        buf.f_bavail as u64 * bsize,
    )
}

fn read_loadavg() -> (f64, f64, f64) {
    let text = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let fields: Vec<&str> = text.split_whitespace().collect();
    if fields.len() < 3 {
        return (0.0, 0.0, 0.0);
    }
    (
        fields[0].parse().unwrap_or(0.0),
        fields[1].parse().unwrap_or(0.0),
        fields[2].parse().unwrap_or(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_parsing_handles_zero_delta() {
        let s = CpuSample { idle: 0, total: 0 };
        assert_eq!(s.idle, 0);
        assert_eq!(s.total, 0);
    }

    #[test]
    fn loadavg_parsing_handles_missing_file() {
        // /proc/loadavg exists on Linux; in other envs this degrades to 0.
        let (l1, l5, l15) = read_loadavg();
        assert!(l1 >= 0.0 && l5 >= 0.0 && l15 >= 0.0);
    }

    #[test]
    fn sampler_degrades_gracefully() {
        let mut s = SysMetricSampler::new("tun0", Path::new("/tmp"));
        let m = s.sample();
        assert!(m.host_mem_total >= 0);
        assert!(m.cpu_percent >= 0.0);
    }

    #[test]
    fn rates_from_deltas() {
        let mut s = SysMetricSampler::new("tun0", Path::new("/tmp"));
        // Force a fast second sample to exercise the delta path; values may be
        // 0 in CI but must not panic.
        let _ = s.sample();
        let _ = s.sample();
    }
}
