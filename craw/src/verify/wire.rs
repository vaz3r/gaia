use crate::krpc::codec::{BValue, decode_prefix, encode_to_bytes};
use bytes::{BufMut, Bytes, BytesMut};
use librqbit_utp::{UtpSocketUdp, UtpStream};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

pub const PIECE_SIZE: usize = 16384;
const HANDSHAKE_LEN: usize = 68;
const PROTOCOL: &[u8] = b"BitTorrent protocol";
const EXTENDED_MSG_ID: u8 = 20;
const EXTENDED_HANDSHAKE_ID: u8 = 0;
const MAX_MESSAGE_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum WireError {
    Io(std::io::Error),
    Timeout,
    Handshake,
    NoExtension,
    #[allow(dead_code)]
    NoMetadata,
    Reject,
    BadPiece,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Io(e) => write!(f, "io error: {e}"),
            WireError::Timeout => write!(f, "timeout"),
            WireError::Handshake => write!(f, "bad handshake"),
            WireError::NoExtension => write!(f, "peer lacks ut_metadata extension"),
            WireError::NoMetadata => write!(f, "metadata not available"),
            WireError::Reject => write!(f, "piece rejected"),
            WireError::BadPiece => write!(f, "invalid piece data"),
        }
    }
}

impl std::error::Error for WireError {}

impl From<std::io::Error> for WireError {
    fn from(e: std::io::Error) -> Self {
        WireError::Io(e)
    }
}

pub fn gen_peer_id() -> [u8; 20] {
    let mut id = [0u8; 20];
    id[..8].copy_from_slice(b"-GA0001-");
    id[8..].copy_from_slice(&rand::random::<[u8; 12]>());
    id
}

fn handshake(info_hash: &[u8; 20], peer_id: &[u8; 20]) -> [u8; HANDSHAKE_LEN] {
    let mut msg = [0u8; HANDSHAKE_LEN];
    msg[0] = 19;
    msg[1..20].copy_from_slice(PROTOCOL);
    msg[25] |= 0x10;
    msg[28..48].copy_from_slice(info_hash);
    msg[48..68].copy_from_slice(peer_id);
    msg
}

pub enum Transport {
    Tcp(TcpStream),
    Utp(UtpStream),
}

impl AsyncRead for Transport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Transport::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            Transport::Utp(u) => Pin::new(u).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Transport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            Transport::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            Transport::Utp(u) => Pin::new(u).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Transport::Tcp(s) => Pin::new(s).poll_flush(cx),
            Transport::Utp(u) => Pin::new(u).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Transport::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            Transport::Utp(u) => Pin::new(u).poll_shutdown(cx),
        }
    }
}

pub struct WireSession {
    stream: Transport,
    ut_metadata: u8,
    metadata_size: Option<usize>,
}

impl WireSession {
    pub fn is_tcp(&self) -> bool {
        matches!(self.stream, Transport::Tcp(_))
    }

    pub async fn connect_tcp(
        addr: SocketAddr,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        timeout: Duration,
    ) -> Result<Self, WireError> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| WireError::Timeout)?
            .map_err(WireError::Io)?;
        stream.set_nodelay(true).ok();
        WireSession::init(Transport::Tcp(stream), info_hash, peer_id, timeout).await
    }

    pub async fn connect_utp(
        socket: Arc<UtpSocketUdp>,
        addr: SocketAddr,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        timeout: Duration,
    ) -> Result<Self, WireError> {
        let stream = match tokio::time::timeout(timeout, socket.connect(addr)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(WireError::Io(std::io::Error::other(e.to_string()))),
            Err(_) => return Err(WireError::Timeout),
        };
        WireSession::init(Transport::Utp(stream), info_hash, peer_id, timeout).await
    }

    async fn init(
        stream: Transport,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        timeout: Duration,
    ) -> Result<Self, WireError> {
        let mut session = WireSession {
            stream,
            ut_metadata: 0,
            metadata_size: None,
        };
        session.bep3(info_hash, peer_id, timeout).await?;
        session.extension_handshake(timeout).await?;
        Ok(session)
    }

    async fn bep3(
        &mut self,
        info_hash: &[u8; 20],
        peer_id: &[u8; 20],
        timeout: Duration,
    ) -> Result<(), WireError> {
        let msg = handshake(info_hash, peer_id);
        tokio::time::timeout(timeout, self.stream.write_all(&msg))
            .await
            .map_err(|_| WireError::Timeout)?
            .map_err(WireError::Io)?;
        let mut buf = [0u8; HANDSHAKE_LEN];
        tokio::time::timeout(timeout, self.stream.read_exact(&mut buf))
            .await
            .map_err(|_| WireError::Timeout)?
            .map_err(WireError::Io)?;
        if buf[0] != 19 || &buf[1..20] != PROTOCOL || &buf[28..48] != info_hash {
            return Err(WireError::Handshake);
        }
        if buf[25] & 0x10 == 0 {
            return Err(WireError::NoExtension);
        }
        Ok(())
    }

    async fn extension_handshake(&mut self, timeout: Duration) -> Result<(), WireError> {
        let ours = BValue::dict(vec![
            (
                Bytes::from_static(b"m"),
                BValue::dict(vec![(Bytes::from_static(b"ut_metadata"), BValue::Int(1))]),
            ),
            (
                Bytes::from_static(b"v"),
                BValue::Bytes(Bytes::from_static(b"GA0.1")),
            ),
        ]);
        self.write_message(EXTENDED_HANDSHAKE_ID, &ours, timeout)
            .await?;
        let (_, payload) = self.read_message(timeout).await?;
        let dict = decode_prefix(&payload).map_err(|_| WireError::Handshake)?.0;
        let m = dict
            .get(b"m")
            .and_then(BValue::as_dict)
            .ok_or(WireError::NoExtension)?;
        let ut = m
            .get(b"ut_metadata".as_slice())
            .and_then(BValue::as_int)
            .ok_or(WireError::NoExtension)?;
        self.ut_metadata = ut as u8;
        self.metadata_size = dict.get_int(b"metadata_size").map(|v| v as usize);
        Ok(())
    }

    async fn write_message(
        &mut self,
        ext_id: u8,
        v: &BValue,
        timeout: Duration,
    ) -> Result<(), WireError> {
        let body = encode_to_bytes(v);
        let total = 2 + body.len();
        let mut msg = BytesMut::with_capacity(4 + total);
        msg.put_u32(total as u32);
        msg.put_u8(EXTENDED_MSG_ID);
        msg.put_u8(ext_id);
        msg.extend_from_slice(&body);
        tokio::time::timeout(timeout, self.stream.write_all(&msg))
            .await
            .map_err(|_| WireError::Timeout)?
            .map_err(WireError::Io)?;
        Ok(())
    }

    async fn read_message(&mut self, timeout: Duration) -> Result<(u8, Bytes), WireError> {
        loop {
            let mut lenbuf = [0u8; 4];
            tokio::time::timeout(timeout, self.stream.read_exact(&mut lenbuf))
                .await
                .map_err(|_| WireError::Timeout)?
                .map_err(WireError::Io)?;
            let len = u32::from_be_bytes(lenbuf) as usize;
            if len == 0 {
                continue;
            }
            if len > MAX_MESSAGE_LEN {
                return Err(WireError::BadPiece);
            }
            let mut buf = BytesMut::zeroed(len);
            tokio::time::timeout(timeout, self.stream.read_exact(&mut buf))
                .await
                .map_err(|_| WireError::Timeout)?
                .map_err(WireError::Io)?;
            let buf = buf.freeze();
            if buf[0] != EXTENDED_MSG_ID {
                continue;
            }
            if buf.len() < 2 {
                return Err(WireError::BadPiece);
            }
            let ext_id = buf[1];
            let payload = buf.slice(2..);
            return Ok((ext_id, payload));
        }
    }

    pub async fn fetch_metadata(&mut self, timeout: Duration) -> Result<Vec<u8>, WireError> {
        const MAX_PIECES: usize = 4096;

        let mut known_size = self.metadata_size;
        let mut metadata: Vec<u8> = Vec::new();
        let mut piece = 0usize;

        self.write_message(self.ut_metadata, &request(piece), timeout)
            .await?;

        loop {
            let (_, payload) = self.read_message(timeout).await?;
            let (dict, consumed) = decode_prefix(&payload).map_err(|_| WireError::BadPiece)?;
            let msg_type = dict.get_int(b"msg_type").ok_or(WireError::BadPiece)?;
            match msg_type {
                2 => return Err(WireError::Reject),
                1 => {}
                _ => return Err(WireError::BadPiece),
            }
            let recv_piece = dict.get_int(b"piece").ok_or(WireError::BadPiece)? as usize;
            if recv_piece != piece {
                return Err(WireError::BadPiece);
            }
            if let Some(ts) = dict.get_int(b"total_size").map(|v| v as usize) {
                known_size = Some(ts);
            }
            let data = payload.slice(consumed..);
            metadata.extend_from_slice(&data);

            let done = match known_size {
                Some(total) => metadata.len() >= total,
                None => data.len() < PIECE_SIZE,
            };
            if done || piece + 1 >= MAX_PIECES {
                break;
            }
            piece += 1;
            self.write_message(self.ut_metadata, &request(piece), timeout)
                .await?;
        }

        let total = known_size.unwrap_or(metadata.len());
        if metadata.len() < total {
            return Err(WireError::BadPiece);
        }
        metadata.truncate(total);
        Ok(metadata)
    }
}

fn request(piece: usize) -> BValue {
    BValue::dict(vec![
        (Bytes::from_static(b"msg_type"), BValue::Int(0)),
        (Bytes::from_static(b"piece"), BValue::Int(piece as i64)),
    ])
}
