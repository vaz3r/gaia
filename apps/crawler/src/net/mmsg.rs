// New mmsg module for Linux-only recvmmsg receive backend.
#![cfg(target_os = "linux")]

use crate::metrics::{Add1, Metrics};
use crate::router::Router;
use std::io;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;

pub const BATCH_SIZE: usize = 32;
pub const MAX_DATAGRAM: usize = 65536;

#[derive(Debug, Clone)]
pub struct MsgMetadata {
    pub len: u32,
    pub flags: i32,
    pub namelen: u32,
}

pub struct BatchBuffer {
    pub payload: Vec<u8>,
    pub addrs: Vec<libc::sockaddr_storage>,
    pub metadata: Vec<MsgMetadata>,
}

impl BatchBuffer {
    pub fn new() -> Self {
        BatchBuffer {
            payload: vec![0u8; BATCH_SIZE * MAX_DATAGRAM],
            addrs: vec![unsafe { std::mem::zeroed::<libc::sockaddr_storage>() }; BATCH_SIZE],
            metadata: vec![
                MsgMetadata {
                    len: 0,
                    flags: 0,
                    namelen: 0,
                };
                BATCH_SIZE
            ],
        }
    }
}

impl Default for BatchBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum Outcome {
    Batch(usize),
    WouldBlock,
    Interrupted,
    Fatal(io::Error),
}

#[cfg(test)]
thread_local! {
    pub static MOCK_OUTCOME: std::cell::RefCell<Option<std::collections::VecDeque<io::Result<usize>>>> = std::cell::RefCell::new(None);
}

/// Linux-only safe wrapper around recvmmsg syscall.
pub fn recvmmsg_sys(raw_fd: RawFd, buffer: &mut BatchBuffer) -> Outcome {
    #[cfg(test)]
    {
        let injected = MOCK_OUTCOME.with(|m| {
            let mut b = m.borrow_mut();
            if let Some(queue) = b.as_mut() {
                queue.pop_front()
            } else {
                None
            }
        });
        if let Some(res) = injected {
            match res {
                Ok(count) => {
                    for i in 0..count {
                        buffer.metadata[i] = MsgMetadata {
                            len: 10,
                            flags: 0,
                            namelen: 16,
                        };
                    }
                    return Outcome::Batch(count);
                }
                Err(e) if e.raw_os_error() == Some(libc::EINTR) => return Outcome::Interrupted,
                Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => return Outcome::WouldBlock,
                Err(e) => return Outcome::Fatal(e),
            }
        }
    }

    // SAFETY: The stack header arrays `iovecs` and `msgs` are declared on the stack
    // of this function and are passed as mutable pointers to `libc::recvmmsg`.
    // They remain valid and alive for the entire duration of the system call.
    let mut iovecs = [unsafe { std::mem::zeroed::<libc::iovec>() }; BATCH_SIZE];
    let mut msgs = [unsafe { std::mem::zeroed::<libc::mmsghdr>() }; BATCH_SIZE];

    prepare_msg_headers(buffer, &mut msgs, &mut iovecs);

    // SAFETY: The `libc::recvmmsg` system call is passed a valid file descriptor,
    // pointers to the stack-allocated header structures, and a timeout of NULL.
    // The heap allocations inside `buffer` are not reallocated or moved during the call
    // because `buffer` is mutably borrowed for the duration of this call.
    let res = unsafe {
        libc::recvmmsg(
            raw_fd,
            msgs.as_mut_ptr(),
            BATCH_SIZE as libc::c_uint,
            libc::MSG_DONTWAIT,
            std::ptr::null_mut(),
        )
    };

    if res >= 0 {
        let count = res as usize;
        for i in 0..count {
            buffer.metadata[i] = MsgMetadata {
                len: msgs[i].msg_len,
                flags: msgs[i].msg_hdr.msg_flags,
                namelen: msgs[i].msg_hdr.msg_namelen as u32,
            };
        }
        Outcome::Batch(count)
    } else {
        let err = io::Error::last_os_error();
        let raw = err.raw_os_error();
        if raw == Some(libc::EAGAIN) || raw == Some(libc::EWOULDBLOCK) {
            Outcome::WouldBlock
        } else if raw == Some(libc::EINTR) {
            Outcome::Interrupted
        } else {
            Outcome::Fatal(err)
        }
    }
}

/// Preparation function that rebuilds all pointer-bearing structures against stable allocations.
pub fn prepare_msg_headers(
    buffer: &mut BatchBuffer,
    msgs: &mut [libc::mmsghdr],
    iovecs: &mut [libc::iovec],
) {
    // SAFETY: `buffer.payload` and `buffer.addrs` are heap-allocated vectors that are
    // pre-allocated to the maximum capacity at initialization and are never reallocated,
    // resized, or cleared during the crawler's lifetime. Therefore, pointers obtained
    // from `.as_mut_ptr()` remain stable and valid across syscall invocations.
    let payload_ptr = buffer.payload.as_mut_ptr();
    for i in 0..BATCH_SIZE {
        unsafe {
            iovecs[i].iov_base = payload_ptr.add(i * MAX_DATAGRAM) as *mut libc::c_void;
            iovecs[i].iov_len = MAX_DATAGRAM;

            msgs[i].msg_hdr.msg_name =
                &mut buffer.addrs[i] as *mut libc::sockaddr_storage as *mut libc::c_void;
            msgs[i].msg_hdr.msg_namelen =
                std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
            msgs[i].msg_hdr.msg_iov = &mut iovecs[i] as *mut libc::iovec;
            msgs[i].msg_hdr.msg_iovlen = 1;
            msgs[i].msg_hdr.msg_control = std::ptr::null_mut();
            msgs[i].msg_hdr.msg_controllen = 0;
            msgs[i].msg_hdr.msg_flags = 0;
            msgs[i].msg_len = 0;
        }
    }
}

/// Parses a libc sockaddr into std::net::SocketAddr
pub fn parse_sockaddr(
    storage: &libc::sockaddr_storage,
    len: libc::socklen_t,
) -> Option<SocketAddr> {
    if len < 2 {
        return None;
    }
    match storage.ss_family as libc::c_int {
        libc::AF_INET => {
            if len < std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t {
                return None;
            }
            let sin =
                unsafe { &*(storage as *const libc::sockaddr_storage as *const libc::sockaddr_in) };
            let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            let port = u16::from_be(sin.sin_port);
            Some(SocketAddr::new(std::net::IpAddr::V4(ip), port))
        }
        libc::AF_INET6 => {
            if len < std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t {
                return None;
            }
            let sin6 = unsafe {
                &*(storage as *const libc::sockaddr_storage as *const libc::sockaddr_in6)
            };
            let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            let port = u16::from_be(sin6.sin6_port);
            Some(SocketAddr::new(std::net::IpAddr::V6(ip), port))
        }
        _ => None,
    }
}

/// A borrow-bound lending structure containing parsed results from a recvmmsg syscall.
pub struct ReceivedBatch<'a> {
    buffer: &'a BatchBuffer,
    count: usize,
}

impl<'a> std::fmt::Debug for ReceivedBatch<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedBatch")
            .field("count", &self.count)
            .finish()
    }
}

impl<'a> ReceivedBatch<'a> {
    pub fn new(buffer: &'a BatchBuffer, count: usize) -> Self {
        ReceivedBatch { buffer, count }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, idx: usize) -> Result<Option<(&[u8], SocketAddr, bool)>, io::Error> {
        if idx >= self.count {
            return Ok(None);
        }
        let metadata = &self.buffer.metadata[idx];
        let msg_len = metadata.len as usize;
        let is_truncated = (metadata.flags & libc::MSG_TRUNC) != 0;
        let addr_len = metadata.namelen;

        // Verify returned message length bounds
        if msg_len > MAX_DATAGRAM {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Impossible message length: {}", msg_len),
            ));
        }

        // Return None to indicate zero-length packet to skip
        if msg_len == 0 {
            return Ok(None);
        }

        let sender_addr = match parse_sockaddr(&self.buffer.addrs[idx], addr_len as libc::socklen_t)
        {
            Some(addr) => addr,
            None => {
                return Ok(None);
            }
        };

        let payload_slice = &self.buffer.payload[idx * MAX_DATAGRAM..idx * MAX_DATAGRAM + msg_len];
        Ok(Some((payload_slice, sender_addr, is_truncated)))
    }
}

pub struct RecvmmsgReceiver {
    pub buffer: BatchBuffer,
}

#[derive(Debug)]
pub enum ReceiveOutcome<'a> {
    Batch(ReceivedBatch<'a>),
    WouldBlock,
    Interrupted,
    Fatal(io::Error),
}

impl RecvmmsgReceiver {
    pub fn new() -> Self {
        RecvmmsgReceiver {
            buffer: BatchBuffer::new(),
        }
    }

    pub fn recv_batch(&mut self, raw_fd: RawFd) -> ReceiveOutcome<'_> {
        match recvmmsg_sys(raw_fd, &mut self.buffer) {
            Outcome::Batch(count) => {
                if count == 0 || count > BATCH_SIZE {
                    return ReceiveOutcome::Fatal(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Impossible recvmmsg returned count: {}", count),
                    ));
                }
                ReceiveOutcome::Batch(ReceivedBatch {
                    buffer: &self.buffer,
                    count,
                })
            }
            Outcome::WouldBlock => ReceiveOutcome::WouldBlock,
            Outcome::Interrupted => ReceiveOutcome::Interrupted,
            Outcome::Fatal(err) => ReceiveOutcome::Fatal(err),
        }
    }
}

impl Default for RecvmmsgReceiver {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReadinessState {
    pub budget: usize,
}

impl ReadinessState {
    pub fn new(budget: usize) -> Self {
        ReadinessState { budget }
    }

    // Returns (should_clear_ready, should_yield, should_retry)
    pub fn decide(&mut self, outcome: &Outcome) -> (bool, bool, bool) {
        match outcome {
            Outcome::Batch(_) => {
                self.budget -= 1;
                if self.budget == 0 {
                    (false, true, false)
                } else {
                    (false, false, true)
                }
            }
            Outcome::WouldBlock => (true, false, false),
            Outcome::Interrupted => (false, false, true),
            Outcome::Fatal(_) => (false, false, false),
        }
    }
}

/// Receive worker using Linux-only recvmmsg and AsyncFd.
pub async fn run_mmsg_worker(
    fd: Arc<AsyncFd<std::os::fd::OwnedFd>>,
    router: Arc<Router>,
    metrics: Arc<Metrics>,
    node_idx: usize,
    worker_idx: usize,
    local_addr: SocketAddr,
) -> Result<(), io::Error> {
    eprintln!("[DBG] mmsg_worker[{node_idx}:{worker_idx}]: started on {local_addr}");
    let mut receiver = RecvmmsgReceiver::new();
    loop {
        // 1. Await readiness
        let mut guard = fd.readable().await?;

        // 2. Perform batched read in a synchronous scope
        let mut state = ReadinessState::new(10);
        let mut yielded = false;

        loop {
            metrics.udp_recv_syscalls_total.add(1);
            match receiver.recv_batch(fd.as_raw_fd()) {
                ReceiveOutcome::Batch(batch) => {
                    metrics.udp_recv_successful_syscalls_total.add(1);
                    metrics.udp_recv_packets_total.add(batch.len() as u64);

                    let _ = metrics
                        .udp_recv_batch_max_interval
                        .fetch_max(batch.len() as u64, std::sync::atomic::Ordering::Relaxed);

                    for i in 0..batch.len() {
                        match batch.get(i) {
                            Ok(Some((payload, sender_addr, is_truncated))) => {
                                if is_truncated {
                                    metrics.udp_recv_truncated_total.add(1);
                                    continue;
                                }
                                router.handle_datagram(payload, sender_addr);
                            }
                            Ok(None) => {
                                let metadata = &batch.buffer.metadata[i];
                                if metadata.len == 0 {
                                    metrics.udp_recv_zero_length_total.add(1);
                                } else {
                                    metrics.udp_recv_invalid_addr_total.add(1);
                                }
                            }
                            Err(e) => {
                                metrics.udp_recv_errors_total.add(1);
                                metrics.udp_recv_fatal_total.add(1);
                                tracing::error!(
                                    backend = "recvmmsg",
                                    node = node_idx,
                                    worker = worker_idx,
                                    local = %local_addr,
                                    error = %e,
                                    "fatal message length error"
                                );
                                return Err(e);
                            }
                        }
                    }

                    let (_, should_yield, _) = state.decide(&Outcome::Batch(batch.len()));
                    if should_yield {
                        guard.retain_ready();
                        yielded = true;
                        break;
                    }
                }
                ReceiveOutcome::WouldBlock => {
                    metrics.udp_recv_eagain_total.add(1);
                    guard.clear_ready();
                    break;
                }
                ReceiveOutcome::Interrupted => {
                    metrics.udp_recv_eintr_total.add(1);
                    continue;
                }
                ReceiveOutcome::Fatal(err) => {
                    metrics.udp_recv_errors_total.add(1);
                    metrics.udp_recv_fatal_total.add(1);
                    tracing::error!(
                        backend = "recvmmsg",
                        node = node_idx,
                        worker = worker_idx,
                        local = %local_addr,
                        error = %err,
                        "fatal recvmmsg error occurred"
                    );
                    return Err(err);
                }
            }
        }

        drop(guard);

        if yielded {
            tokio::task::yield_now().await;
        }
    }
}
