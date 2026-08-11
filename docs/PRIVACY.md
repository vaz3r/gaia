# Privacy & footprint on the BitTorrent DHT

This document covers how `dht-crawler` is visible on the public DHT, what it
can and cannot do about it, and the operational choices that actually reduce
your exposure. Read it before running a long-lived instance.

## The honest baseline

There is **no invisible mode on a public DHT**. Every KRPC/UDP packet you send
carries your source IP, and the node you query sees it — in real time, in its
connection table, and often in its routing-table candidates. The same applies
(strongly) to the TCP metadata fetches: each BEP 9 scrape opens a TCP
connection to a live peer in the swarm, handing your IP to that peer.

What you *can* control is:

1. how long you stay in other nodes' routing tables (the "appearing in DHT"
   sense), and
2. whether the IP that participates is actually your host's IP.

## 1. Reduce routing-table presence (in-code, one line)

`irontide-dht` supports **BEP 43 read-only mode**. When enabled:

- every outgoing query carries `ro: 1`, which compliant clients honor by *not*
  adding you to their routing tables, and
- outbound `announce_peer` is suppressed.

Enable it in `src/dht/mod.rs`:

```rust
let dht = DhtConfig {
    bind_addr: args.bind_addr(),
    bootstrap_nodes: args.bootstrap.clone(),
    state_dir: Some(state_dir),
    address_family: if args.ipv6 { AddressFamily::V6 } else { AddressFamily::V4 },
    queries_per_second: args.qps,
    read_only_mode: true,   // BEP 43: ro:1 on queries, no announce_peer
    ..DhtConfig::default()
};
```

**Caveat (verified in the `irontide-dht` source, `actor.rs handle_query`):**
the actor still *answers* inbound `ping` / `find_node` / `get_peers` /
`sample_infohashes` queries regardless of `read_only_mode`. So `ro:1` shrinks
your footprint but does not make you silent.

## 2. Query-only mode (firewall)

Make your node a pure query source: it sends KRPC queries but never replies.
Because a node that never answers a `ping` is quickly dropped from everyone's
routing tables, and combined with `ro:1` you essentially stop "existing" in the
DHT beyond the individual packets you send.

The trick is to drop **unsolicited inbound** UDP on the DHT port while allowing
replies to your own queries (they are `related` to your outbound traffic).

nftables example (replace `6881` with your `--port`):

```text
table inet dht_filter {
    chain input {
        type filter hook input priority filter; policy drop;
        # our replies to queries we sent are established/related
        ct state established,related accept
        # loopback so local tooling still works
        iif lo accept
        # drop unsolicited DHT UDP
        udp dport 6881 drop
        # everything else on your host, as you normally allow
        tcp dport 22 accept
        ct state new accept
    }
}
```

iptables equivalent:

```sh
iptables -A INPUT -p udp --dport 6881 -m conntrack --ctstate NEW -j DROP
```

Test: after bootstrapping, run a crawl; if your routing table still fills up,
the node is being discovered via your own outbound packets (normal) — the point
is that nobody *else* can reach you.

## 3. Actually hiding your IP (network layer)

`ro:1` + a silent port change the *observed presence*, but the packets still
originate from your IP. If your threat model requires that a DHT participant,
a swarm peer, or an observer on the wire cannot tie the crawl to you, you must
move the crawl off your host's IP:

- **VPN / WireGuard tunnel** — route the crawler's traffic through the tunnel
  (bind the UDP socket to the tunnel interface). The DHT only ever sees the
  VPN exit IP. This also covers the TCP metadata scrapes, which are otherwise
  your strongest attribution vector (a persistent TCP session to a live peer).
- **Rotating egress** — pool several egress IPs and rotate the identity
  (node ID + IP) over time to break long-term correlation.
- **Tor is not suitable** — the DHT is UDP; Tor transports TCP. A Tor SOCKS
  proxy cannot carry DHT traffic without pluggable-transport hacks.

## 4. Opsec framing

- Passive DHT crawling (sending queries) is routine, low-risk network activity.
- BEP 9 metadata scraping is a *swarm interaction*: you connect to peers and
  exchange BitTorrent protocol messages. This is the activity most likely to
  be logged, observed, or considered "engaging" with a swarm. If that is a
  concern, prefer the VPN/egress setup above before running the fetcher.
- Persisting raw `info` dictionaries (`scanned.info_bytes`) stores third-party
  metadata, not media content, but be aware of what your database contains.

## Summary

| Goal | Do |
|------|----|
| Don't accumulate in routing tables | `read_only_mode: true` |
| Never respond / be unreachable | Drop unsolicited inbound UDP on the DHT port |
| Hide your host IP entirely | VPN / rotating egress for UDP **and** TCP |
| Stay un-announced | Read-only mode suppresses `announce_peer` (we never announce anyway) |

There is no fully invisible crawler; there are only degrees of footprint
reduction and layers that move the exposure off your real host.
