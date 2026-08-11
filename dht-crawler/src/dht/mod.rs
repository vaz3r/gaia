mod sampler;

pub use sampler::{SampledHash, Sampler, SamplerConfig};

use anyhow::Result;
use irontide_core::AddressFamily;
use irontide_dht::{DhtConfig, DhtHandle};
use std::path::PathBuf;

use crate::config::RunArgs;

/// Start the DHT actor bound to the configured UDP port and address family.
pub async fn start_dht(args: &RunArgs, state_dir: PathBuf) -> Result<DhtHandle> {
    let dht = DhtConfig {
        bind_addr: args.bind_addr(),
        bootstrap_nodes: args.bootstrap.clone(),
        state_dir: Some(state_dir),
        address_family: if args.ipv6 {
            AddressFamily::V6
        } else {
            AddressFamily::V4
        },
        queries_per_second: args.qps,
        ..DhtConfig::default()
    };
    let (handle, _ip) = DhtHandle::start(dht).await?;
    Ok(handle)
}
