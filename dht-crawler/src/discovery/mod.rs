mod sampler;

pub use sampler::{SampledHash, Sampler, SamplerConfig};

use std::time::Duration;

use anyhow::Result;
use irontide_core::{AddressFamily, Id20};
use irontide_dht::{DhtConfig, DhtHandle};
use rand::RngCore;
use tokio_util::sync::CancellationToken;
use std::path::PathBuf;

use crate::cli::RunArgs;

/// Load known-live nodes from a persisted routing table (`dht_state.json`)
/// and return them as `host:port` bootstrap seed strings. Used to warm new
/// instances from the primary's already-discovered routing table so they do
/// not start from an empty table.
pub fn seed_nodes_from_state(state_dir: &std::path::Path) -> Vec<String> {
    let path = state_dir.join("dht_state.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(nodes) = state.get("nodes").and_then(|n| n.as_array()) else {
        return Vec::new();
    };
    nodes
        .iter()
        .filter_map(|n| {
            let addr = n.get("addr")?.as_str()?;
            Some(addr.to_string())
        })
        .collect()
}

/// Start the DHT actor for instance `i` (0-based), bound to `port + i` and
/// persisting its routing table to `state_dir/instance-i/`. `extra_seeds` are
/// additional `host:port` bootstrap nodes (e.g. from the primary's warm table).
pub async fn start_dht(
    args: &RunArgs,
    state_dir: PathBuf,
    instance: usize,
    extra_seeds: Vec<String>,
) -> Result<DhtHandle> {
    let port = args.port.saturating_add(instance as u16);
    let bind_addr = if args.ipv6 {
        std::net::SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port))
    } else {
        std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, port))
    };
    let mut bootstrap = args.bootstrap.clone();
    bootstrap.extend(extra_seeds);
    let dht = DhtConfig {
        bind_addr,
        bootstrap_nodes: bootstrap,
        state_dir: Some(state_dir),
        address_family: if args.ipv6 {
            AddressFamily::V6
        } else {
            AddressFamily::V4
        },
        queries_per_second: args.effective_qps(),
        max_routing_nodes: args.effective_max_nodes(),
        query_timeout: Duration::from_secs(args.effective_query_timeout()),
        restrict_routing_ips: !args.no_restrict_ips,
        ..DhtConfig::default()
    };
    let (handle, _ip) = DhtHandle::start(dht).await?;
    Ok(handle)
}

/// Continuously grow the routing table by issuing `get_peers` on random 20-byte
/// targets. Each lookup walks toward the target and injects discovered nodes
/// into the routing table, so the table climbs toward `--max-nodes` throughout
/// the crawl rather than stalling after the startup warmup. Queries are
/// throttled to `interval` and stop on cancellation.
pub async fn grow_routing(
    handle: DhtHandle,
    interval: Duration,
    shutdown: CancellationToken,
) {
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        let mut bytes = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut bytes);
        let target = Id20(bytes);
        // Even with no peers, the DhtLookup injects nodes into the routing
        // table. Drop the receiver immediately.
        let _ = handle.get_peers(target).await;
        tokio::time::sleep(interval).await;
    }
}
