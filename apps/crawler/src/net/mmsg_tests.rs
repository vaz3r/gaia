// Linux-only recvmmsg receive backend tests.
#![cfg(target_os = "linux")]

use crate::metrics::Metrics;
use crate::net::mmsg::{
    BATCH_SIZE, BatchBuffer, MAX_DATAGRAM, MOCK_OUTCOME, Outcome, ReadinessState, ReceiveOutcome,
    RecvmmsgReceiver, parse_sockaddr, prepare_msg_headers, recvmmsg_sys,
};
use crate::router::Router;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::unix::AsyncFd;

#[test]
fn test_sockaddr_conversion_ipv4() {
    let mut storage = unsafe { std::mem::zeroed::<libc::sockaddr_storage>() };
    let sin =
        unsafe { &mut *(&mut storage as *mut libc::sockaddr_storage as *mut libc::sockaddr_in) };
    sin.sin_family = libc::AF_INET as libc::sa_family_t;
    sin.sin_port = 8080u16.to_be();
    sin.sin_addr.s_addr = 0x7f000001u32.to_be(); // 127.0.0.1

    let addr = parse_sockaddr(
        &storage,
        std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
    );
    assert_eq!(
        addr,
        Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            8080
        ))
    );
}

#[tokio::test]
async fn test_recvmmsg_eagain_classification() {
    let std_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    std_sock.set_nonblocking(true).unwrap();
    let owned_fd = std::os::fd::OwnedFd::from(std_sock);
    let async_fd = AsyncFd::new(owned_fd).unwrap();
    let mut receiver = RecvmmsgReceiver::new();

    match receiver.recv_batch(async_fd.as_raw_fd()) {
        ReceiveOutcome::WouldBlock => {}
        other => panic!("expected WouldBlock, got {:?}", other),
    }
}

#[test]
fn test_eintr_then_success() {
    MOCK_OUTCOME.with(|m| {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(Err(std::io::Error::from_raw_os_error(libc::EINTR)));
        queue.push_back(Ok(2));
        *m.borrow_mut() = Some(queue);
    });

    let mut receiver = RecvmmsgReceiver::new();
    match receiver.recv_batch(0 /* raw_fd dummy */) {
        ReceiveOutcome::Interrupted => {}
        other => panic!("expected Interrupted, got {:?}", other),
    }

    match receiver.recv_batch(0) {
        ReceiveOutcome::Batch(b) => assert_eq!(b.len(), 2),
        other => panic!("expected Batch(2), got {:?}", other),
    }

    MOCK_OUTCOME.with(|m| *m.borrow_mut() = None);
}

#[test]
fn test_fatal_syscall_error() {
    MOCK_OUTCOME.with(|m| {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(Err(std::io::Error::from_raw_os_error(libc::EBADF)));
        *m.borrow_mut() = Some(queue);
    });

    let mut receiver = RecvmmsgReceiver::new();
    match receiver.recv_batch(0) {
        ReceiveOutcome::Fatal(err) => assert_eq!(err.raw_os_error(), Some(libc::EBADF)),
        other => panic!("expected Fatal, got {:?}", other),
    }

    MOCK_OUTCOME.with(|m| *m.borrow_mut() = None);
}

#[test]
fn test_msg_trunc_small_slot() {
    // Deliberately small receive slot using the real syscall
    let std_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let local = std_sock.local_addr().unwrap();
    std_sock.set_nonblocking(true).unwrap();
    let send_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    send_sock.send_to(b"1234567890", local).unwrap(); // 10 bytes

    let mut buffer = BatchBuffer::new();
    let mut iovecs = [unsafe { std::mem::zeroed::<libc::iovec>() }; BATCH_SIZE];
    let mut msgs = [unsafe { std::mem::zeroed::<libc::mmsghdr>() }; BATCH_SIZE];

    prepare_msg_headers(&mut buffer, &mut msgs, &mut iovecs);
    // Artificially truncate slot
    iovecs[0].iov_len = 5;
    msgs[0].msg_hdr.msg_iov = &mut iovecs[0] as *mut libc::iovec;
    msgs[0].msg_hdr.msg_iovlen = 1;

    let res = unsafe {
        libc::recvmmsg(
            std_sock.as_raw_fd(),
            msgs.as_mut_ptr(),
            1,
            0,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(res, 1);
    assert_eq!(msgs[0].msg_len, 5); // truncated
    assert_ne!(msgs[0].msg_hdr.msg_flags & libc::MSG_TRUNC, 0); // MSG_TRUNC observed
}

#[test]
fn test_readiness_state_decisions() {
    let mut state = ReadinessState::new(3);

    // Batch outcome should decrement budget, do not clear, do not yield, retry
    let res1 = state.decide(&Outcome::Batch(1));
    assert_eq!(res1, (false, false, true));
    assert_eq!(state.budget, 2);

    let res2 = state.decide(&Outcome::Batch(2));
    assert_eq!(res2, (false, false, true));
    assert_eq!(state.budget, 1);

    // Budget exhausted on 3rd batch: do not clear, yield, do not retry
    let res3 = state.decide(&Outcome::Batch(1));
    assert_eq!(res3, (false, true, false));
    assert_eq!(state.budget, 0);

    // WouldBlock: clear ready, do not yield, do not retry
    let mut state2 = ReadinessState::new(3);
    let res_wb = state2.decide(&Outcome::WouldBlock);
    assert_eq!(res_wb, (true, false, false));

    // Interrupted: do not clear, do not yield, retry
    let mut state3 = ReadinessState::new(3);
    let res_intr = state3.decide(&Outcome::Interrupted);
    assert_eq!(res_intr, (false, false, true));
}

#[tokio::test]
async fn test_two_fd_owners_and_source_port() {
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .unwrap();
    socket.bind(&bind_addr.into()).unwrap();
    let local_addr = socket.local_addr().unwrap().as_socket().unwrap();

    let dup = socket.try_clone().unwrap();
    let local_addr_dup = dup.local_addr().unwrap().as_socket().unwrap();

    assert_eq!(local_addr, local_addr_dup);

    let send_sock = UdpSocket::from(dup);
    let target = UdpSocket::bind("127.0.0.1:0").unwrap();
    let target_addr = target.local_addr().unwrap();

    send_sock.send_to(b"test", target_addr).unwrap();
    let mut buf = [0u8; 10];
    let (_, from_addr) = target.recv_from(&mut buf).unwrap();

    assert_eq!(from_addr, local_addr);
}
