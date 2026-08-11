mod sampler;

pub use sampler::{SampledHash, Sampler, SamplerConfig};

use std::time::Duration;

use anyhow::Result;
use irontide_core::AddressFamily;
use irontide_dht::{DhtConfig, DhtHandle};
use std::path::PathBuf;

use crate::cli::RunArgs;

/// Start the DHT actor for instance `i` (0-based), bound to `port + i` and
/// persisting its routing table to `state_dir/instance-i/`.
pub async fn start_dht(args: &RunArgs, state_dir: PathBuf, instance: usize) -> Result<DhtHandle> {
    let port = args.port.saturating_add(instance as u16);
    let bind_addr = if args.ipv6 {
        std::net::SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port))
    } else {
        std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, port))
    };
    let dht = DhtConfig {
        bind_addr,
        bootstrap_nodes: args.bootstrap.clone(),
        state_dir: Some(state_dir),
        address_family: if args.ipv6 {
            AddressFamily::V6
        } else {
            AddressFamily::V4
        },
        queries_per_second: args.effective_qps(),
        max_routing_nodes: args.effective_max_nodes(),
        query_timeout: Duration::from_secs(args.effective_query_timeout()),
        ..DhtConfig::default()
    };
    let (handle, _ip) = DhtHandle::start(dht).await?;
    Ok(handle)
}

/// Grow the routing table quickly at startup by issuing `get_peers` on random
/// targets. This forces find_node/get_peers cascades that populate the table
/// faster than passive BEP 51 sampling alone, so more BEP 51-capable nodes are
/// discovered sooner.
pub async fn warmup_routing(handle: &DhtHandle, targets: usize) {
    use irontide_core::Id20;
    use rand::RngCore;

    for _ in 0..targets {
        let mut bytes = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut bytes);
        let target = Id20(bytes);
        // get_peers on a random hash: even with no peers, the lookup injects
        // nodes into the routing table. Drop the receiver immediately.
        let _ = handle.get_peers(target).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
