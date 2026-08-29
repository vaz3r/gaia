pub mod rate_limit;

#[cfg(target_os = "linux")]
pub mod mmsg;

#[cfg(all(test, target_os = "linux"))]
mod mmsg_tests;

use crate::router::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;

pub const MAX_DATAGRAM: usize = 65536;

pub fn bind_reuseport(addr: SocketAddr, n: usize) -> std::io::Result<Vec<UdpSocket>> {
    let mut sockets = Vec::with_capacity(n);
    for _ in 0..n {
        let domain = if addr.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        };
        let sock =
            socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
        sock.set_reuse_address(true)?;
        sock.set_reuse_port(true)?;
        sock.set_nonblocking(true)?;
        sock.bind(&addr.into())?;
        let std_sock: std::net::UdpSocket = sock.into();
        sockets.push(UdpSocket::from_std(std_sock)?);
    }
    Ok(sockets)
}

pub async fn worker(
    sock: Arc<UdpSocket>,
    router: Arc<Router>,
    node_idx: usize,
    worker_idx: usize,
    local_addr: SocketAddr,
) {
    let mut buf = vec![0u8; MAX_DATAGRAM];
    let res: Result<(), std::io::Error> = loop {
        match sock.recv_from(&mut buf).await {
            Ok((n, from)) => {
                router.handle_datagram(&buf[..n], from);
                // Bounded drain: absorb short bursts in the same poll, then always
                // yield via the outer .await so the executor can service other tasks.
                for _ in 0..32 {
                    match sock.try_recv_from(&mut buf) {
                        Ok((n, from)) => router.handle_datagram(&buf[..n], from),
                        Err(_) => break,
                    }
                }
            }
            Err(e) => {
                break Err(e);
            }
        }
    };
    if let Err(e) = res {
        tracing::error!(
            backend = "tokio",
            node = node_idx,
            worker = worker_idx,
            local = %local_addr,
            error = %e,
            "standard worker loop exited with error"
        );
    }
}
