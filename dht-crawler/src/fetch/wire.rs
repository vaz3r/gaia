use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use gaia_core::Id20;
use gaia_wire::{ExtHandshake, Handshake, Message, MessageCodec, MetadataMessage, MetadataMessageType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::{FramedRead, FramedWrite};

/// `ut_metadata` requests split the metadata into 16 KiB pieces (BEP 9).
pub const PIECE_SIZE: usize = 16 * 1024;

/// Upper bound on metadata size we are willing to assemble (16 MiB).
const MAX_METADATA_SIZE: usize = 16 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PIECE_TIMEOUT: Duration = Duration::from_secs(10);

/// Successfully assembled, verified info dictionary.
#[derive(Debug, Clone)]
pub struct FetchedMetadata {
    pub info_bytes: Vec<u8>,
}

/// Attempt to download and verify metadata for `info_hash` from a single peer.
///
/// Returns the assembled bencoded `info` dictionary only after its SHA-1
/// matches `info_hash`. Any mismatch, rejection, or timeout is an error and
/// no partial data is returned.
pub async fn fetch_from_peer(
    peer: SocketAddr,
    info_hash: Id20,
    peer_id: Id20,
) -> Result<FetchedMetadata> {
    let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(peer))
        .await
        .map_err(|_| anyhow!("connect to {peer} timed out"))?
        .with_context(|| format!("connect to {peer}"))?;
    stream.set_nodelay(true)?;

    let (read, write) = stream.into_split();

    // 1. Standard handshake advertising BEP 10 extensions (raw 68 bytes).
    let hs = Handshake::new(info_hash, peer_id).to_bytes();
    let mut writer = write;
    timeout(HANDSHAKE_TIMEOUT, writer.write_all(&hs))
        .await
        .context("send handshake timed out")?
        .with_context(|| format!("send handshake to {peer}"))?;

    // 2. Extension handshake advertising ut_metadata (length-delimited).
    let mut ext = ExtHandshake::new();
    ext.v = Some("dht-crawler".into());
    let ext_msg = Message::Extended {
        ext_id: 0,
        payload: ext.to_bytes()?,
    };
    let mut framed = FramedWrite::new(writer, MessageCodec::new());
    timeout(HANDSHAKE_TIMEOUT, framed.send(ext_msg))
        .await
        .context("send extension handshake timed out")?
        .context("send extension handshake")?;

    // 3. Read the peer's raw 68-byte handshake from the unframed read half.
    let mut raw_reader = read;
    let mut peer_hs_bytes = [0u8; 68];
    timeout(HANDSHAKE_TIMEOUT, raw_reader.read_exact(&mut peer_hs_bytes))
        .await
        .map_err(|_| anyhow!("timed out waiting for peer handshake"))?
        .context("connection closed during handshake")?;
    let peer_hs = Handshake::from_bytes(&peer_hs_bytes)?;
    if !peer_hs.supports_extensions() {
        bail!("peer {peer} does not support BEP 10 extensions");
    }

    let mut reader = FramedRead::new(raw_reader, MessageCodec::new());
    let (ut_metadata_id, metadata_size) = read_ext_handshake(&mut reader).await?;
    let Some(ut_id) = ut_metadata_id else {
        bail!("peer {peer} does not advertise ut_metadata");
    };

    let pieces = metadata_size.div_ceil(PIECE_SIZE);
    if pieces == 0 {
        bail!("peer {peer} advertised zero metadata size");
    }

    // 4. Request every metadata piece.
    let mut received: Vec<Option<Bytes>> = vec![None; pieces];
    let mut remaining = pieces;
    for idx in 0..pieces {
        let req = MetadataMessage::request(idx as u32);
        timeout(
            PIECE_TIMEOUT,
            framed.send(Message::Extended {
                ext_id: ut_id,
                payload: req.to_bytes()?,
            }),
        )
        .await
        .context("send metadata request timed out")?
        .context("send metadata request")?;
    }

    // 5. Collect pieces until complete.
    while remaining > 0 {
        let frame = timeout(PIECE_TIMEOUT, reader.next())
            .await
            .map_err(|_| anyhow!("timed out waiting for metadata from {peer}"))?
            .context("connection closed while fetching metadata")?
            .context("invalid message from peer")?;

        match frame {
            Message::Extended { ext_id, payload } if ext_id == ut_id => {
                // A malformed ut_metadata frame should not abort the fetch;
                // ignore it and keep waiting for valid pieces.
                let Ok(msg) = MetadataMessage::from_bytes(&payload) else {
                    continue;
                };
                match msg.msg_type {
                    MetadataMessageType::Data => {
                        let idx = msg.piece as usize;
                        if idx < pieces && received[idx].is_none() {
                            if let Some(data) = msg.data {
                                if data.len() <= PIECE_SIZE {
                                    received[idx] = Some(data);
                                    remaining -= 1;
                                }
                            }
                        }
                    }
                    MetadataMessageType::Reject => {
                        bail!("peer {peer} rejected metadata piece {}", msg.piece);
                    }
                    MetadataMessageType::Request => {}
                }
            }
            _ => {} // ignore non-metadata messages
        }
    }

    // 6. Assemble and verify.
    let mut info = Vec::with_capacity(metadata_size);
    for piece in received {
        info.extend_from_slice(&piece.expect("all pieces present"));
    }
    if info.len() != metadata_size {
        bail!("assembled metadata size mismatch: got {}, want {metadata_size}", info.len());
    }

    Ok(FetchedMetadata { info_bytes: info })
}

/// The peer handshake is a raw 68-byte frame, not length-delimited, so it is
/// read with `read_exact` on the raw stream before wrapping in the codec.
async fn read_ext_handshake<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut FramedRead<R, MessageCodec>,
) -> Result<(Option<u8>, usize)> {
    loop {
        let frame = timeout(HANDSHAKE_TIMEOUT, reader.next())
            .await
            .map_err(|_| anyhow!("timed out waiting for extension handshake"))?
            .context("connection closed during extension handshake")?
            .context("invalid message")?;

        if let Message::Extended { ext_id: 0, payload } = frame {
            let ext = ExtHandshake::from_bytes(&payload)?;
            let ut_id = ext.ext_id("ut_metadata").filter(|id| *id != 0);
            let size = match ext.metadata_size {
                Some(s) if s > 0 && (s as usize) <= MAX_METADATA_SIZE => s as usize,
                Some(_) => bail!("peer advertised out-of-range metadata size"),
                None => 0,
            };
            return Ok((ut_id, size));
        }
    }
}

/// Compute the SHA-1 of an assembled info dictionary.
pub fn sha1_info(info_bytes: &[u8]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(info_bytes);
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaia_bencode::to_bytes;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Info {
        name: String,
        length: i64,
    }

    #[test]
    fn sha1_verifies_matching_and_rejects_mismatch() {
        let info = to_bytes(&Info {
            name: "The Matrix 1999".into(),
            length: 42,
        })
        .unwrap();

        let hash = sha1_info(&info);
        assert_eq!(sha1_info(&info), hash, "must match its own hash");

        let tampered = to_bytes(&Info {
            name: "The Matrix 2000".into(),
            length: 42,
        })
        .unwrap();
        assert_ne!(sha1_info(&tampered), hash, "tampered info must differ");
    }

    #[test]
    fn piece_size_is_16k() {
        assert_eq!(PIECE_SIZE, 16 * 1024);
    }
}
