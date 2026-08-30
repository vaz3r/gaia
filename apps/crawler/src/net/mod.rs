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

pub fn resolve_backend(use_mmsg: bool, is_linux: bool) -> (&'static str, &'static str) {
    let requested = if use_mmsg { "recvmmsg" } else { "tokio" };
    if use_mmsg && !is_linux {
        panic!("recvmmsg receive backend is only supported on Linux targets");
    }
    (requested, requested) // It fails closed, so effective always equals requested if it didn't panic
}

#[cfg(test)]
mod backend_tests {
    use super::*;

    #[test]
    fn test_linux_flag_true_selects_recvmmsg() {
        let (req, eff) = resolve_backend(true, true);
        assert_eq!(req, "recvmmsg");
        assert_eq!(eff, "recvmmsg");
    }

    #[test]
    fn test_linux_flag_false_selects_tokio() {
        let (req, eff) = resolve_backend(false, true);
        assert_eq!(req, "tokio");
        assert_eq!(eff, "tokio");
    }

    #[test]
    #[should_panic(expected = "recvmmsg receive backend is only supported on Linux targets")]
    fn test_requested_recvmmsg_fails_closed_on_non_linux() {
        resolve_backend(true, false);
    }

    #[test]
    fn test_startup_backend_log_reports_consistently() {
        // If it doesn't panic, they match.
        let (req, eff) = resolve_backend(true, true);
        assert_eq!(req, eff);
        let (req, eff) = resolve_backend(false, false);
        assert_eq!(req, eff);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_production_module_graph_includes_mmsg() {
        // Just referencing a type from mmsg ensures it's in the module graph
        let _ = std::any::type_name::<crate::net::mmsg::ReceivedBatch>();
    }
}
