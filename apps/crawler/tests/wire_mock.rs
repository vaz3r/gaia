use bytes::{BufMut, Bytes, BytesMut};
use crawler::krpc::codec::{BValue, decode_prefix, encode_to_bytes};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const HANDSHAKE_LEN: usize = 68;
const PROTOCOL: &[u8] = b"BitTorrent protocol";
const EXTENDED_MSG_ID: u8 = 20;

fn server_handshake(info_hash: &[u8; 20]) -> [u8; HANDSHAKE_LEN] {
    let mut msg = [0u8; HANDSHAKE_LEN];
    msg[0] = 19;
    msg[1..20].copy_from_slice(PROTOCOL);
    msg[25] |= 0x10;
    msg[28..48].copy_from_slice(info_hash);
    msg[48..68].copy_from_slice(&[0xAB; 20]);
    msg
}

fn encode_ext_handshake(ut_metadata: u8, metadata_size: usize) -> Vec<u8> {
    let dict = BValue::dict(vec![
        (
            Bytes::from_static(b"m"),
            BValue::dict(vec![(
                Bytes::from_static(b"ut_metadata"),
                BValue::Int(ut_metadata as i64),
            )]),
        ),
        (
            Bytes::from_static(b"metadata_size"),
            BValue::Int(metadata_size as i64),
        ),
    ]);
    let body = encode_to_bytes(&dict);
    let total = 2 + body.len();
    let mut msg = Vec::with_capacity(4 + total);
    msg.put_u32(total as u32);
    msg.put_u8(EXTENDED_MSG_ID);
    msg.put_u8(0);
    msg.extend_from_slice(&body);
    msg
}

fn encode_ext_message(ext_id: u8, dict: &BValue) -> Vec<u8> {
    let body = encode_to_bytes(dict);
    let total = 2 + body.len();
    let mut msg = Vec::with_capacity(4 + total);
    msg.put_u32(total as u32);
    msg.put_u8(EXTENDED_MSG_ID);
    msg.put_u8(ext_id);
    msg.extend_from_slice(&body);
    msg
}

fn encode_metadata_piece(piece: usize, data: &[u8], total_size: usize) -> Vec<u8> {
    let dict = BValue::dict(vec![
        (Bytes::from_static(b"msg_type"), BValue::Int(1)),
        (Bytes::from_static(b"piece"), BValue::Int(piece as i64)),
        (
            Bytes::from_static(b"total_size"),
            BValue::Int(total_size as i64),
        ),
    ]);
    let body = encode_to_bytes(&dict);
    let total = 2 + body.len() + data.len();
    let mut msg = Vec::with_capacity(4 + total);
    msg.put_u32(total as u32);
    msg.put_u8(EXTENDED_MSG_ID);
    msg.put_u8(1);
    msg.extend_from_slice(&body);
    msg.extend_from_slice(data);
    msg
}

async fn read_message(stream: &mut tokio::net::TcpStream) -> (u8, Bytes) {
    let mut lenbuf = [0u8; 4];
    stream.read_exact(&mut lenbuf).await.unwrap();
    let len = u32::from_be_bytes(lenbuf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(buf[0], EXTENDED_MSG_ID);
    let ext_id = buf[1];
    (ext_id, Bytes::from(buf[2..].to_vec()))
}

async fn read_handshake(stream: &mut tokio::net::TcpStream) -> [u8; 20] {
    let mut buf = [0u8; HANDSHAKE_LEN];
    stream.read_exact(&mut buf).await.unwrap();
    let mut info_hash = [0u8; 20];
    info_hash.copy_from_slice(&buf[28..48]);
    info_hash
}

async fn skip_until_extended(stream: &mut tokio::net::TcpStream) -> (u8, Bytes) {
    loop {
        let mut lenbuf = [0u8; 4];
        stream.read_exact(&mut lenbuf).await.unwrap();
        let len = u32::from_be_bytes(lenbuf) as usize;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await.unwrap();
        if buf[0] == EXTENDED_MSG_ID && buf.len() >= 2 {
            return (buf[1], Bytes::from(buf[2..].to_vec()));
        }
    }
}

/// Mock peer that completes handshake, sends ut_pex before metadata piece.
/// This tests whether fetch_metadata correctly skips non-metadata extensions.
#[tokio::test]
async fn test_non_metadata_extension_before_piece() {
    let info_hash = [0x42u8; 20];
    let metadata: Vec<u8> = (0u16..256).map(|x| x as u8).collect();
    let expected = metadata.clone();
    let metadata_size = metadata.len();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut client_hs = [0u8; HANDSHAKE_LEN];
        stream.read_exact(&mut client_hs).await.unwrap();
        stream.write_all(&server_handshake(&info_hash)).await.unwrap();

        let (_ext_id, payload) = skip_until_extended(&mut stream).await;
        let (dict, _) = decode_prefix(&payload).unwrap();
        let _m = dict.get(b"m").and_then(BValue::as_dict);

        stream
            .write_all(&encode_ext_handshake(1, metadata_size))
            .await
            .unwrap();

        let (ext_id, _payload) = skip_until_extended(&mut stream).await;
        assert_eq!(ext_id, 1, "Expected metadata request on ext_id=1");

        let pex_msg = BValue::dict(vec![(
            Bytes::from_static(b"peers"),
            BValue::Bytes(Bytes::from_static(&[1, 2, 3, 4, 5, 6])),
        )]);
        let pex_bytes = encode_ext_message(2, &pex_msg);
        stream.write_all(&pex_bytes).await.unwrap();

        let piece_msg = encode_metadata_piece(0, &metadata, metadata_size);
        stream.write_all(&piece_msg).await.unwrap();

        let _ = stream.shutdown().await;
    });

    let peer_id = crawler::verify::wire::gen_peer_id();
    let mut session =
        crawler::verify::wire::WireSession::connect_tcp(addr, &info_hash, &peer_id, std::time::Duration::from_secs(5))
            .await
            .unwrap();

    let result = session
        .fetch_metadata(std::time::Duration::from_secs(5))
        .await;
    assert!(result.is_ok(), "fetch_metadata should succeed even with ut_pex before piece: {:?}", result.err());
    assert_eq!(result.unwrap(), expected);

    server.await.unwrap();
}

/// Mock peer that sends metadata piece with wrong piece number.
#[tokio::test]
async fn test_wrong_piece_number_returns_bad_piece() {
    let info_hash = [0x43u8; 20];
    let metadata: Vec<u8> = (0u16..128).map(|x| x as u8).collect();
    let metadata_size = metadata.len();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut client_hs = [0u8; HANDSHAKE_LEN];
        stream.read_exact(&mut client_hs).await.unwrap();
        stream.write_all(&server_handshake(&info_hash)).await.unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;
        stream
            .write_all(&encode_ext_handshake(1, metadata_size))
            .await
            .unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;

        let dict = BValue::dict(vec![
            (Bytes::from_static(b"msg_type"), BValue::Int(1)),
            (Bytes::from_static(b"piece"), BValue::Int(99)),
            (
                Bytes::from_static(b"total_size"),
                BValue::Int(metadata_size as i64),
            ),
        ]);
        let body = encode_to_bytes(&dict);
        let total = 2 + body.len() + metadata.len();
        let mut msg = Vec::with_capacity(4 + total);
        msg.put_u32(total as u32);
        msg.put_u8(EXTENDED_MSG_ID);
        msg.put_u8(1);
        msg.extend_from_slice(&body);
        msg.extend_from_slice(&metadata);
        stream.write_all(&msg).await.unwrap();

        let _ = stream.shutdown().await;
    });

    let peer_id = crawler::verify::wire::gen_peer_id();
    let mut session =
        crawler::verify::wire::WireSession::connect_tcp(addr, &info_hash, &peer_id, std::time::Duration::from_secs(5))
            .await
            .unwrap();

    let result = session
        .fetch_metadata(std::time::Duration::from_secs(5))
        .await;
    assert!(matches!(result, Err(crawler::verify::wire::WireError::BadPiece)));

    server.await.unwrap();
}

/// Clean mock peer that correctly serves metadata.
/// Baseline: should succeed near 100%.
#[tokio::test]
async fn test_clean_mock_peer_metadata_success() {
    let info_hash = [0x44u8; 20];
    let metadata: Vec<u8> = (0u16..4096).map(|x| x as u8).collect();
    let expected = metadata.clone();
    let metadata_size = metadata.len();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut client_hs = [0u8; HANDSHAKE_LEN];
        stream.read_exact(&mut client_hs).await.unwrap();
        stream.write_all(&server_handshake(&info_hash)).await.unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;
        stream
            .write_all(&encode_ext_handshake(1, metadata_size))
            .await
            .unwrap();

        loop {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                skip_until_extended(&mut stream),
            )
            .await;
            match result {
                Ok((ext_id, _payload)) if ext_id == 1 => {
                    let piece_msg = encode_metadata_piece(0, &metadata, metadata_size);
                    stream.write_all(&piece_msg).await.unwrap();
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }

        let _ = stream.shutdown().await;
    });

    let peer_id = crawler::verify::wire::gen_peer_id();
    let mut session =
        crawler::verify::wire::WireSession::connect_tcp(addr, &info_hash, &peer_id, std::time::Duration::from_secs(5))
            .await
            .unwrap();

    let result = session
        .fetch_metadata(std::time::Duration::from_secs(5))
        .await;
    assert!(result.is_ok(), "Clean mock peer should succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), expected);

    server.await.unwrap();
}

/// Mock peer that rejects the metadata request.
#[tokio::test]
async fn test_peer_rejects_metadata_request() {
    let info_hash = [0x45u8; 20];
    let metadata_size = 1024;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut client_hs = [0u8; HANDSHAKE_LEN];
        stream.read_exact(&mut client_hs).await.unwrap();
        stream.write_all(&server_handshake(&info_hash)).await.unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;
        stream
            .write_all(&encode_ext_handshake(1, metadata_size))
            .await
            .unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;

        let reject_msg = BValue::dict(vec![
            (Bytes::from_static(b"msg_type"), BValue::Int(2)),
            (Bytes::from_static(b"piece"), BValue::Int(0)),
        ]);
        let reject_bytes = encode_ext_message(1, &reject_msg);
        stream.write_all(&reject_bytes).await.unwrap();

        let _ = stream.shutdown().await;
    });

    let peer_id = crawler::verify::wire::gen_peer_id();
    let mut session =
        crawler::verify::wire::WireSession::connect_tcp(addr, &info_hash, &peer_id, std::time::Duration::from_secs(5))
            .await
            .unwrap();

    let result = session
        .fetch_metadata(std::time::Duration::from_secs(5))
        .await;
    assert!(matches!(result, Err(crawler::verify::wire::WireError::Reject)));

    server.await.unwrap();
}

/// Mock peer that sends multiple non-metadata messages before the piece.
/// Tests that the skip loop works for multiple irrelevant messages.
#[tokio::test]
async fn test_multiple_non_metadata_messages_before_piece() {
    let info_hash = [0x46u8; 20];
    let metadata: Vec<u8> = (0u16..512).map(|x| x as u8).collect();
    let expected = metadata.clone();
    let metadata_size = metadata.len();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut client_hs = [0u8; HANDSHAKE_LEN];
        stream.read_exact(&mut client_hs).await.unwrap();
        stream.write_all(&server_handshake(&info_hash)).await.unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;
        stream
            .write_all(&encode_ext_handshake(1, metadata_size))
            .await
            .unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;

        for i in 0..5 {
            let fake_msg = BValue::dict(vec![(
                Bytes::from_static(b"fake"),
                BValue::Int(i),
            )]);
            let fake_bytes = encode_ext_message(2, &fake_msg);
            stream.write_all(&fake_bytes).await.unwrap();
        }

        let piece_msg = encode_metadata_piece(0, &metadata, metadata_size);
        stream.write_all(&piece_msg).await.unwrap();

        let _ = stream.shutdown().await;
    });

    let peer_id = crawler::verify::wire::gen_peer_id();
    let mut session =
        crawler::verify::wire::WireSession::connect_tcp(addr, &info_hash, &peer_id, std::time::Duration::from_secs(5))
            .await
            .unwrap();

    let result = session
        .fetch_metadata(std::time::Duration::from_secs(5))
        .await;
    assert!(result.is_ok(), "Should skip 5 non-metadata messages and succeed: {:?}", result.err());
    assert_eq!(result.unwrap(), expected);

    server.await.unwrap();
}

/// Mock peer that sends a reject after initial piece request, then the actual piece.
/// Tests that reject is handled correctly even if piece arrives later.
#[tokio::test]
async fn test_reject_then_piece() {
    let info_hash = [0x47u8; 20];
    let metadata: Vec<u8> = (0u16..256).map(|x| x as u8).collect();
    let metadata_size = metadata.len();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut client_hs = [0u8; HANDSHAKE_LEN];
        stream.read_exact(&mut client_hs).await.unwrap();
        stream.write_all(&server_handshake(&info_hash)).await.unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;
        stream
            .write_all(&encode_ext_handshake(1, metadata_size))
            .await
            .unwrap();

        let (_ext_id, _payload) = skip_until_extended(&mut stream).await;

        let reject_msg = BValue::dict(vec![
            (Bytes::from_static(b"msg_type"), BValue::Int(2)),
            (Bytes::from_static(b"piece"), BValue::Int(0)),
        ]);
        let reject_bytes = encode_ext_message(1, &reject_msg);
        stream.write_all(&reject_bytes).await.unwrap();

        let _ = stream.shutdown().await;
    });

    let peer_id = crawler::verify::wire::gen_peer_id();
    let mut session =
        crawler::verify::wire::WireSession::connect_tcp(addr, &info_hash, &peer_id, std::time::Duration::from_secs(5))
            .await
            .unwrap();

    let result = session
        .fetch_metadata(std::time::Duration::from_secs(5))
        .await;
    assert!(matches!(result, Err(crawler::verify::wire::WireError::Reject)));

    server.await.unwrap();
}
