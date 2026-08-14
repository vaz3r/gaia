#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "M175: BitTorrent wire format — message field widths fixed by BEP 3/6/10 spec (u32 piece indices, u32 chunk lengths)"
)]

use bytes::{BufMut, Bytes, BytesMut};

use crate::error::{Error, Result};

/// Standard `BitTorrent` peer wire messages (BEP 3).
///
/// Generic over buffer type `B` to support both owned (`Bytes`) and borrowed
/// (`&[u8]`) payloads.  Data-carrying variants (`Piece`, `Bitfield`,
/// `Extended`) use `B`; fixed-field variants are buffer-agnostic.
///
/// The `Piece` variant carries two buffer fields (`data_0`, `data_1`) to
/// support ring-buffer wrap-around: when a block spans the ring boundary the
/// two non-contiguous slices are stored separately. In the common (no-wrap)
/// case `data_1` is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message<B = Bytes> {
    /// Keep connection alive (no payload, length=0).
    KeepAlive,
    /// Peer is choking us.
    Choke,
    /// Peer is unchoking us.
    Unchoke,
    /// We're interested in the peer's data.
    Interested,
    /// We're not interested.
    NotInterested,
    /// Peer has piece `index`.
    Have {
        /// Piece index.
        index: u32,
    },
    /// Peer's complete bitfield.
    Bitfield(B),
    /// Request a block: piece index, byte offset within piece, length.
    Request {
        /// Piece index.
        index: u32,
        /// Byte offset within the piece.
        begin: u32,
        /// Requested block length in bytes.
        length: u32,
    },
    /// A data block: piece index, byte offset, data.
    ///
    /// Two buffer fields support ring-buffer wrap-around.  When the block does
    /// not straddle the ring boundary, `data_1` is empty.
    Piece {
        /// Piece index.
        index: u32,
        /// Byte offset within the piece.
        begin: u32,
        /// First (or only) contiguous slice of block payload.
        data_0: B,
        /// Second contiguous slice when the ring buffer wraps; empty otherwise.
        data_1: B,
    },
    /// Cancel a previously sent request.
    Cancel {
        /// Piece index.
        index: u32,
        /// Byte offset within the piece.
        begin: u32,
        /// Block length in bytes.
        length: u32,
    },
    /// DHT port (BEP 5).
    Port(u16),
    /// Extension message (BEP 10). `ext_id=0` is handshake.
    Extended {
        /// Extension message ID (0 = handshake).
        ext_id: u8,
        /// Bencoded extension payload.
        payload: B,
    },
    /// BEP 6: Suggest a piece for the peer to download.
    SuggestPiece(u32),
    /// BEP 6: We have all pieces.
    HaveAll,
    /// BEP 6: We have no pieces.
    HaveNone,
    /// BEP 6: Reject a request from the peer.
    RejectRequest {
        /// Piece index.
        index: u32,
        /// Byte offset within the piece.
        begin: u32,
        /// Block length in bytes.
        length: u32,
    },
    /// BEP 6: Piece index the peer is allowed to request while choked.
    AllowedFast(u32),
    /// BEP 52: Request hashes from a file's Merkle tree.
    HashRequest {
        /// File root hash identifying the Merkle tree.
        pieces_root: gaia_core::Id32,
        /// Tree layer (0 = leaf/block layer).
        base: u32,
        /// Starting node index within the layer.
        index: u32,
        /// Number of consecutive hashes requested.
        count: u32,
        /// Number of uncle proof layers to include.
        proof_layers: u32,
    },
    /// BEP 52: Response with hashes and uncle proof.
    Hashes {
        /// File root hash identifying the Merkle tree.
        pieces_root: gaia_core::Id32,
        /// Tree layer (0 = leaf/block layer).
        base: u32,
        /// Starting node index within the layer.
        index: u32,
        /// Number of consecutive hashes in the response.
        count: u32,
        /// Number of uncle proof layers included.
        proof_layers: u32,
        /// Hash values followed by uncle proof hashes.
        hashes: Vec<gaia_core::Id32>,
    },
    /// BEP 52: Reject a hash request.
    HashReject {
        /// File root hash identifying the Merkle tree.
        pieces_root: gaia_core::Id32,
        /// Tree layer that was requested.
        base: u32,
        /// Starting node index that was requested.
        index: u32,
        /// Number of hashes that was requested.
        count: u32,
        /// Number of proof layers that was requested.
        proof_layers: u32,
    },
}

// Message IDs per BEP 3
const ID_CHOKE: u8 = 0;
const ID_UNCHOKE: u8 = 1;
const ID_INTERESTED: u8 = 2;
const ID_NOT_INTERESTED: u8 = 3;
const ID_HAVE: u8 = 4;
const ID_BITFIELD: u8 = 5;
const ID_REQUEST: u8 = 6;
const ID_PIECE: u8 = 7;
const ID_CANCEL: u8 = 8;
const ID_PORT: u8 = 9;
const ID_EXTENDED: u8 = 20;

// BEP 6 Fast Extension
const ID_SUGGEST_PIECE: u8 = 0x0D;
const ID_HAVE_ALL: u8 = 0x0E;
const ID_HAVE_NONE: u8 = 0x0F;
const ID_REJECT_REQUEST: u8 = 0x10;
const ID_ALLOWED_FAST: u8 = 0x11;

// BEP 52 Hash Messages
const ID_HASH_REQUEST: u8 = 21;
const ID_HASHES: u8 = 22;
const ID_HASH_REJECT: u8 = 23;

impl<B: AsRef<[u8]>> Message<B> {
    /// Serialize a message to bytes (length-prefix + id + payload).
    ///
    /// The returned bytes include the 4-byte length prefix.
    pub fn to_bytes(&self) -> Bytes {
        match self {
            Self::KeepAlive => {
                let mut buf = BytesMut::with_capacity(4);
                buf.put_u32(0);
                buf.freeze()
            }
            Self::Choke => fixed_msg(ID_CHOKE),
            Self::Unchoke => fixed_msg(ID_UNCHOKE),
            Self::Interested => fixed_msg(ID_INTERESTED),
            Self::NotInterested => fixed_msg(ID_NOT_INTERESTED),
            Self::Have { index } => {
                let mut buf = BytesMut::with_capacity(9);
                buf.put_u32(5);
                buf.put_u8(ID_HAVE);
                buf.put_u32(*index);
                buf.freeze()
            }
            Self::Bitfield(bits) => {
                let bits = bits.as_ref();
                let mut buf = BytesMut::with_capacity(5 + bits.len());
                buf.put_u32(1 + bits.len() as u32);
                buf.put_u8(ID_BITFIELD);
                buf.put_slice(bits);
                buf.freeze()
            }
            Self::Request {
                index,
                begin,
                length,
            } => triple_msg(ID_REQUEST, *index, *begin, *length),
            Self::Piece {
                index,
                begin,
                data_0,
                data_1,
            } => {
                let d0 = data_0.as_ref();
                let d1 = data_1.as_ref();
                let data_len = d0.len() + d1.len();
                let mut buf = BytesMut::with_capacity(13 + data_len);
                buf.put_u32(9 + data_len as u32);
                buf.put_u8(ID_PIECE);
                buf.put_u32(*index);
                buf.put_u32(*begin);
                buf.put_slice(d0);
                buf.put_slice(d1);
                buf.freeze()
            }
            Self::Cancel {
                index,
                begin,
                length,
            } => triple_msg(ID_CANCEL, *index, *begin, *length),
            Self::Port(port) => {
                let mut buf = BytesMut::with_capacity(7);
                buf.put_u32(3);
                buf.put_u8(ID_PORT);
                buf.put_u16(*port);
                buf.freeze()
            }
            Self::Extended { ext_id, payload } => {
                let payload = payload.as_ref();
                let mut buf = BytesMut::with_capacity(6 + payload.len());
                buf.put_u32(2 + payload.len() as u32);
                buf.put_u8(ID_EXTENDED);
                buf.put_u8(*ext_id);
                buf.put_slice(payload);
                buf.freeze()
            }
            Self::SuggestPiece(index) => {
                let mut buf = BytesMut::with_capacity(9);
                buf.put_u32(5);
                buf.put_u8(ID_SUGGEST_PIECE);
                buf.put_u32(*index);
                buf.freeze()
            }
            Self::HaveAll => fixed_msg(ID_HAVE_ALL),
            Self::HaveNone => fixed_msg(ID_HAVE_NONE),
            Self::RejectRequest {
                index,
                begin,
                length,
            } => triple_msg(ID_REJECT_REQUEST, *index, *begin, *length),
            Self::AllowedFast(index) => {
                let mut buf = BytesMut::with_capacity(9);
                buf.put_u32(5);
                buf.put_u8(ID_ALLOWED_FAST);
                buf.put_u32(*index);
                buf.freeze()
            }
            Self::HashRequest {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            }
            | Self::HashReject {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            } => {
                let id = match self {
                    Self::HashRequest { .. } => ID_HASH_REQUEST,
                    _ => ID_HASH_REJECT,
                };
                let mut buf = BytesMut::with_capacity(53);
                buf.put_u32(49); // 1 + 32 + 4*4
                buf.put_u8(id);
                buf.put_slice(&pieces_root.0);
                buf.put_u32(*base);
                buf.put_u32(*index);
                buf.put_u32(*count);
                buf.put_u32(*proof_layers);
                buf.freeze()
            }
            Self::Hashes {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
                hashes,
            } => {
                let hash_bytes = hashes.len() * 32;
                let payload_len = 1 + 32 + 16 + hash_bytes;
                let mut buf = BytesMut::with_capacity(4 + payload_len);
                buf.put_u32(payload_len as u32);
                buf.put_u8(ID_HASHES);
                buf.put_slice(&pieces_root.0);
                buf.put_u32(*base);
                buf.put_u32(*index);
                buf.put_u32(*count);
                buf.put_u32(*proof_layers);
                for h in hashes {
                    buf.put_slice(&h.0);
                }
                buf.freeze()
            }
        }
    }

    /// Encode this message (with length prefix) directly into a buffer.
    ///
    /// Unlike [`to_bytes`](Self::to_bytes), this writes directly into `dst`
    /// without allocating an intermediate `Bytes`, avoiding a double-copy
    /// when used with `tokio_util::codec::Encoder`.
    pub fn encode_into(&self, dst: &mut BytesMut) {
        match self {
            Self::KeepAlive => {
                dst.put_u32(0);
            }
            Self::Choke => encode_fixed_into(dst, ID_CHOKE),
            Self::Unchoke => encode_fixed_into(dst, ID_UNCHOKE),
            Self::Interested => encode_fixed_into(dst, ID_INTERESTED),
            Self::NotInterested => encode_fixed_into(dst, ID_NOT_INTERESTED),
            Self::Have { index } => {
                dst.put_u32(5);
                dst.put_u8(ID_HAVE);
                dst.put_u32(*index);
            }
            Self::Bitfield(bits) => {
                let bits = bits.as_ref();
                dst.reserve(5 + bits.len());
                dst.put_u32(1 + bits.len() as u32);
                dst.put_u8(ID_BITFIELD);
                dst.put_slice(bits);
            }
            Self::Request {
                index,
                begin,
                length,
            } => encode_triple_into(dst, ID_REQUEST, *index, *begin, *length),
            Self::Piece {
                index,
                begin,
                data_0,
                data_1,
            } => {
                let d0 = data_0.as_ref();
                let d1 = data_1.as_ref();
                let data_len = d0.len() + d1.len();
                dst.reserve(13 + data_len);
                dst.put_u32(9 + data_len as u32);
                dst.put_u8(ID_PIECE);
                dst.put_u32(*index);
                dst.put_u32(*begin);
                dst.put_slice(d0);
                dst.put_slice(d1);
            }
            Self::Cancel {
                index,
                begin,
                length,
            } => encode_triple_into(dst, ID_CANCEL, *index, *begin, *length),
            Self::Port(port) => {
                dst.put_u32(3);
                dst.put_u8(ID_PORT);
                dst.put_u16(*port);
            }
            Self::Extended { ext_id, payload } => {
                let payload = payload.as_ref();
                dst.reserve(6 + payload.len());
                dst.put_u32(2 + payload.len() as u32);
                dst.put_u8(ID_EXTENDED);
                dst.put_u8(*ext_id);
                dst.put_slice(payload);
            }
            Self::SuggestPiece(index) => {
                dst.put_u32(5);
                dst.put_u8(ID_SUGGEST_PIECE);
                dst.put_u32(*index);
            }
            Self::HaveAll => encode_fixed_into(dst, ID_HAVE_ALL),
            Self::HaveNone => encode_fixed_into(dst, ID_HAVE_NONE),
            Self::RejectRequest {
                index,
                begin,
                length,
            } => encode_triple_into(dst, ID_REJECT_REQUEST, *index, *begin, *length),
            Self::AllowedFast(index) => {
                dst.put_u32(5);
                dst.put_u8(ID_ALLOWED_FAST);
                dst.put_u32(*index);
            }
            Self::HashRequest {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            }
            | Self::HashReject {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            } => {
                let id = match self {
                    Self::HashRequest { .. } => ID_HASH_REQUEST,
                    _ => ID_HASH_REJECT,
                };
                dst.put_u32(49); // 1 + 32 + 4*4
                dst.put_u8(id);
                dst.put_slice(&pieces_root.0);
                dst.put_u32(*base);
                dst.put_u32(*index);
                dst.put_u32(*count);
                dst.put_u32(*proof_layers);
            }
            Self::Hashes {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
                hashes,
            } => {
                let hash_bytes = hashes.len() * 32;
                let payload_len = 1 + 32 + 16 + hash_bytes;
                dst.reserve(4 + payload_len);
                dst.put_u32(payload_len as u32);
                dst.put_u8(ID_HASHES);
                dst.put_slice(&pieces_root.0);
                dst.put_u32(*base);
                dst.put_u32(*index);
                dst.put_u32(*count);
                dst.put_u32(*proof_layers);
                for h in hashes {
                    dst.put_slice(&h.0);
                }
            }
        }
    }

    /// Return the exact encoded wire length in bytes, including the 4-byte
    /// length prefix.
    ///
    /// This is a pure computation — no allocation, no encoding.  Use it to
    /// check whether a message fits in a fixed-size buffer before calling
    /// [`encode_to_slice`](Self::encode_to_slice).
    #[must_use]
    pub fn wire_len(&self) -> usize {
        match self {
            Self::KeepAlive => 4,
            Self::Choke
            | Self::Unchoke
            | Self::Interested
            | Self::NotInterested
            | Self::HaveAll
            | Self::HaveNone => 5,
            Self::Have { .. } | Self::SuggestPiece(_) | Self::AllowedFast(_) => 9,
            Self::Port(_) => 7,
            Self::Request { .. } | Self::Cancel { .. } | Self::RejectRequest { .. } => 17,
            Self::Bitfield(bits) => 5 + bits.as_ref().len(),
            Self::Piece { data_0, data_1, .. } => {
                13 + data_0.as_ref().len() + data_1.as_ref().len()
            }
            Self::Extended { payload, .. } => 6 + payload.as_ref().len(),
            Self::HashRequest { .. } | Self::HashReject { .. } => 53,
            Self::Hashes { hashes, .. } => 53 + hashes.len() * 32,
        }
    }

    /// Encode this message (with length prefix) into a raw byte slice.
    ///
    /// Returns the number of bytes written. The caller must ensure `dst` is
    /// large enough to hold the encoded message. For peer wire messages,
    /// `MAX_MSG_LEN` (16397) is always sufficient.
    ///
    /// Uses `std::io::Cursor<&mut [u8]>` + `std::io::Write` instead of
    /// `BufMut`, producing identical bytes to [`encode_into`](Self::encode_into).
    #[must_use]
    pub fn encode_to_slice(&self, dst: &mut [u8]) -> usize {
        use std::io::{Cursor, Write};

        let mut cursor = Cursor::new(dst);

        match self {
            Self::KeepAlive => {
                cursor.write_all(&0u32.to_be_bytes()).unwrap();
            }
            Self::Choke => {
                cursor.write_all(&1u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_CHOKE]).unwrap();
            }
            Self::Unchoke => {
                cursor.write_all(&1u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_UNCHOKE]).unwrap();
            }
            Self::Interested => {
                cursor.write_all(&1u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_INTERESTED]).unwrap();
            }
            Self::NotInterested => {
                cursor.write_all(&1u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_NOT_INTERESTED]).unwrap();
            }
            Self::Have { index } => {
                cursor.write_all(&5u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_HAVE]).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
            }
            Self::Bitfield(bits) => {
                let bits = bits.as_ref();
                cursor
                    .write_all(&(1 + bits.len() as u32).to_be_bytes())
                    .unwrap();
                cursor.write_all(&[ID_BITFIELD]).unwrap();
                cursor.write_all(bits).unwrap();
            }
            Self::Request {
                index,
                begin,
                length,
            } => {
                cursor.write_all(&13u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_REQUEST]).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
                cursor.write_all(&begin.to_be_bytes()).unwrap();
                cursor.write_all(&length.to_be_bytes()).unwrap();
            }
            Self::Piece {
                index,
                begin,
                data_0,
                data_1,
            } => {
                let d0 = data_0.as_ref();
                let d1 = data_1.as_ref();
                let data_len = d0.len() + d1.len();
                cursor
                    .write_all(&(9 + data_len as u32).to_be_bytes())
                    .unwrap();
                cursor.write_all(&[ID_PIECE]).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
                cursor.write_all(&begin.to_be_bytes()).unwrap();
                cursor.write_all(d0).unwrap();
                cursor.write_all(d1).unwrap();
            }
            Self::Cancel {
                index,
                begin,
                length,
            } => {
                cursor.write_all(&13u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_CANCEL]).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
                cursor.write_all(&begin.to_be_bytes()).unwrap();
                cursor.write_all(&length.to_be_bytes()).unwrap();
            }
            Self::Port(port) => {
                cursor.write_all(&3u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_PORT]).unwrap();
                cursor.write_all(&port.to_be_bytes()).unwrap();
            }
            Self::Extended { ext_id, payload } => {
                let payload = payload.as_ref();
                cursor
                    .write_all(&(2 + payload.len() as u32).to_be_bytes())
                    .unwrap();
                cursor.write_all(&[ID_EXTENDED]).unwrap();
                cursor.write_all(&[*ext_id]).unwrap();
                cursor.write_all(payload).unwrap();
            }
            Self::SuggestPiece(index) => {
                cursor.write_all(&5u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_SUGGEST_PIECE]).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
            }
            Self::HaveAll => {
                cursor.write_all(&1u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_HAVE_ALL]).unwrap();
            }
            Self::HaveNone => {
                cursor.write_all(&1u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_HAVE_NONE]).unwrap();
            }
            Self::RejectRequest {
                index,
                begin,
                length,
            } => {
                cursor.write_all(&13u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_REJECT_REQUEST]).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
                cursor.write_all(&begin.to_be_bytes()).unwrap();
                cursor.write_all(&length.to_be_bytes()).unwrap();
            }
            Self::AllowedFast(index) => {
                cursor.write_all(&5u32.to_be_bytes()).unwrap();
                cursor.write_all(&[ID_ALLOWED_FAST]).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
            }
            Self::HashRequest {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            }
            | Self::HashReject {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            } => {
                let id = match self {
                    Self::HashRequest { .. } => ID_HASH_REQUEST,
                    _ => ID_HASH_REJECT,
                };
                cursor.write_all(&49u32.to_be_bytes()).unwrap();
                cursor.write_all(&[id]).unwrap();
                cursor.write_all(&pieces_root.0).unwrap();
                cursor.write_all(&base.to_be_bytes()).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
                cursor.write_all(&count.to_be_bytes()).unwrap();
                cursor.write_all(&proof_layers.to_be_bytes()).unwrap();
            }
            Self::Hashes {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
                hashes,
            } => {
                let hash_bytes = hashes.len() * 32;
                let payload_len = 1 + 32 + 16 + hash_bytes;
                cursor
                    .write_all(&(payload_len as u32).to_be_bytes())
                    .unwrap();
                cursor.write_all(&[ID_HASHES]).unwrap();
                cursor.write_all(&pieces_root.0).unwrap();
                cursor.write_all(&base.to_be_bytes()).unwrap();
                cursor.write_all(&index.to_be_bytes()).unwrap();
                cursor.write_all(&count.to_be_bytes()).unwrap();
                cursor.write_all(&proof_layers.to_be_bytes()).unwrap();
                for h in hashes {
                    cursor.write_all(&h.0).unwrap();
                }
            }
        }

        cursor.position() as usize
    }
}

impl Message<&[u8]> {
    /// Convert a borrowed message to an owned `Message<Bytes>`.
    ///
    /// Fixed-field variants are zero-cost (no data to copy).
    /// Data-carrying variants (`Piece`, `Bitfield`, `Extended`) copy their
    /// slices into fresh `Bytes` allocations.
    #[must_use]
    pub fn to_owned_bytes(&self) -> Message<Bytes> {
        match *self {
            Message::KeepAlive => Message::KeepAlive,
            Message::Choke => Message::Choke,
            Message::Unchoke => Message::Unchoke,
            Message::Interested => Message::Interested,
            Message::NotInterested => Message::NotInterested,
            Message::Have { index } => Message::Have { index },
            Message::Bitfield(data) => Message::Bitfield(Bytes::copy_from_slice(data)),
            Message::Request {
                index,
                begin,
                length,
            } => Message::Request {
                index,
                begin,
                length,
            },
            Message::Piece {
                index,
                begin,
                data_0,
                data_1,
            } => Message::Piece {
                index,
                begin,
                data_0: Bytes::copy_from_slice(data_0),
                data_1: Bytes::copy_from_slice(data_1),
            },
            Message::Cancel {
                index,
                begin,
                length,
            } => Message::Cancel {
                index,
                begin,
                length,
            },
            Message::Port(port) => Message::Port(port),
            Message::Extended { ext_id, payload } => Message::Extended {
                ext_id,
                payload: Bytes::copy_from_slice(payload),
            },
            Message::SuggestPiece(index) => Message::SuggestPiece(index),
            Message::HaveAll => Message::HaveAll,
            Message::HaveNone => Message::HaveNone,
            Message::RejectRequest {
                index,
                begin,
                length,
            } => Message::RejectRequest {
                index,
                begin,
                length,
            },
            Message::AllowedFast(index) => Message::AllowedFast(index),
            Message::HashRequest {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            } => Message::HashRequest {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            },
            Message::Hashes {
                ref pieces_root,
                base,
                index,
                count,
                proof_layers,
                ref hashes,
            } => Message::Hashes {
                pieces_root: *pieces_root,
                base,
                index,
                count,
                proof_layers,
                hashes: hashes.clone(),
            },
            Message::HashReject {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            } => Message::HashReject {
                pieces_root,
                base,
                index,
                count,
                proof_layers,
            },
        }
    }
}

impl Message<Bytes> {
    /// Parse a message from its payload (after the 4-byte length prefix has
    /// been consumed). `payload` is everything after the length prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if the message ID is unknown or the payload is malformed.
    #[allow(clippy::needless_pass_by_value, reason = "pub API stability")]
    pub fn from_payload(payload: Bytes) -> Result<Self> {
        if payload.is_empty() {
            return Ok(Self::KeepAlive);
        }

        let id = payload[0];
        let body = &payload[1..];

        match id {
            ID_CHOKE => Ok(Self::Choke),
            ID_UNCHOKE => Ok(Self::Unchoke),
            ID_INTERESTED => Ok(Self::Interested),
            ID_NOT_INTERESTED => Ok(Self::NotInterested),
            ID_HAVE => {
                ensure_len(body, 4, "Have")?;
                Ok(Self::Have {
                    index: read_u32(body),
                })
            }
            ID_BITFIELD => Ok(Self::Bitfield(payload.slice(1..))),
            ID_REQUEST => {
                ensure_len(body, 12, "Request")?;
                Ok(Self::Request {
                    index: read_u32(body),
                    begin: read_u32(&body[4..]),
                    length: read_u32(&body[8..]),
                })
            }
            ID_PIECE => {
                ensure_len(body, 8, "Piece")?;
                let index = read_u32(body);
                let begin = read_u32(&body[4..]);
                Ok(Self::Piece {
                    index,
                    begin,
                    data_0: payload.slice(9..),
                    data_1: Bytes::new(),
                })
            }
            ID_CANCEL => {
                ensure_len(body, 12, "Cancel")?;
                Ok(Self::Cancel {
                    index: read_u32(body),
                    begin: read_u32(&body[4..]),
                    length: read_u32(&body[8..]),
                })
            }
            ID_PORT => {
                ensure_len(body, 2, "Port")?;
                Ok(Self::Port(u16::from_be_bytes([body[0], body[1]])))
            }
            ID_EXTENDED => {
                ensure_len(body, 1, "Extended")?;
                let ext_id = body[0];
                Ok(Self::Extended {
                    ext_id,
                    payload: payload.slice(2..),
                })
            }
            ID_SUGGEST_PIECE => {
                ensure_len(body, 4, "SuggestPiece")?;
                Ok(Self::SuggestPiece(read_u32(body)))
            }
            ID_HAVE_ALL => Ok(Self::HaveAll),
            ID_HAVE_NONE => Ok(Self::HaveNone),
            ID_REJECT_REQUEST => {
                ensure_len(body, 12, "RejectRequest")?;
                Ok(Self::RejectRequest {
                    index: read_u32(body),
                    begin: read_u32(&body[4..]),
                    length: read_u32(&body[8..]),
                })
            }
            ID_ALLOWED_FAST => {
                ensure_len(body, 4, "AllowedFast")?;
                Ok(Self::AllowedFast(read_u32(body)))
            }
            ID_HASH_REQUEST | ID_HASH_REJECT => {
                ensure_len(body, 48, "HashRequest/Reject")?;
                let mut root = [0u8; 32];
                root.copy_from_slice(&body[..32]);
                let pieces_root = gaia_core::Id32(root);
                let base = read_u32(&body[32..]);
                let index = read_u32(&body[36..]);
                let count = read_u32(&body[40..]);
                let proof_layers = read_u32(&body[44..]);
                if id == ID_HASH_REQUEST {
                    Ok(Self::HashRequest {
                        pieces_root,
                        base,
                        index,
                        count,
                        proof_layers,
                    })
                } else {
                    Ok(Self::HashReject {
                        pieces_root,
                        base,
                        index,
                        count,
                        proof_layers,
                    })
                }
            }
            ID_HASHES => {
                ensure_len(body, 48, "Hashes")?;
                let mut root = [0u8; 32];
                root.copy_from_slice(&body[..32]);
                let pieces_root = gaia_core::Id32(root);
                let base = read_u32(&body[32..]);
                let index = read_u32(&body[36..]);
                let count = read_u32(&body[40..]);
                let proof_layers = read_u32(&body[44..]);
                let hash_data = &body[48..];
                if !hash_data.len().is_multiple_of(32) {
                    return Err(Error::MessageTooShort {
                        expected: 48 + 32,
                        got: body.len(),
                    });
                }
                let hashes = hash_data
                    .chunks_exact(32)
                    .map(|chunk| {
                        let mut h = [0u8; 32];
                        h.copy_from_slice(chunk);
                        gaia_core::Id32(h)
                    })
                    .collect();
                Ok(Self::Hashes {
                    pieces_root,
                    base,
                    index,
                    count,
                    proof_layers,
                    hashes,
                })
            }
            _ => Err(Error::InvalidMessageId(id)),
        }
    }
}

fn encode_fixed_into(dst: &mut BytesMut, id: u8) {
    dst.put_u32(1);
    dst.put_u8(id);
}

fn encode_triple_into(dst: &mut BytesMut, id: u8, a: u32, b: u32, c: u32) {
    dst.put_u32(13);
    dst.put_u8(id);
    dst.put_u32(a);
    dst.put_u32(b);
    dst.put_u32(c);
}

fn fixed_msg(id: u8) -> Bytes {
    let mut buf = BytesMut::with_capacity(5);
    buf.put_u32(1);
    buf.put_u8(id);
    buf.freeze()
}

fn triple_msg(id: u8, a: u32, b: u32, c: u32) -> Bytes {
    let mut buf = BytesMut::with_capacity(17);
    buf.put_u32(13);
    buf.put_u8(id);
    buf.put_u32(a);
    buf.put_u32(b);
    buf.put_u32(c);
    buf.freeze()
}

fn read_u32(buf: &[u8]) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&buf[..4]);
    u32::from_be_bytes(b)
}

fn ensure_len(body: &[u8], min: usize, _name: &str) -> Result<()> {
    if body.len() < min {
        Err(Error::MessageTooShort {
            expected: min,
            got: body.len(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::needless_pass_by_value, reason = "test helper convenience")]
    fn round_trip(msg: Message) {
        let bytes = msg.to_bytes();
        // Skip the 4-byte length prefix for parsing
        let parsed = Message::from_payload(Bytes::copy_from_slice(&bytes[4..])).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn keepalive() {
        round_trip(Message::KeepAlive);
    }

    #[test]
    fn choke_unchoke() {
        round_trip(Message::Choke);
        round_trip(Message::Unchoke);
    }

    #[test]
    fn interested() {
        round_trip(Message::Interested);
        round_trip(Message::NotInterested);
    }

    #[test]
    fn have() {
        round_trip(Message::Have { index: 42 });
    }

    #[test]
    fn bitfield() {
        round_trip(Message::Bitfield(Bytes::from_static(&[0xFF, 0x80])));
    }

    #[test]
    fn request() {
        round_trip(Message::Request {
            index: 1,
            begin: 0,
            length: 16384,
        });
    }

    #[test]
    fn piece() {
        round_trip(Message::Piece {
            index: 1,
            begin: 0,
            data_0: Bytes::from_static(b"hello world"),
            data_1: Bytes::new(),
        });
    }

    #[test]
    fn cancel() {
        round_trip(Message::Cancel {
            index: 1,
            begin: 0,
            length: 16384,
        });
    }

    #[test]
    fn port() {
        round_trip(Message::Port(6881));
    }

    #[test]
    fn extended() {
        round_trip(Message::Extended {
            ext_id: 1,
            payload: Bytes::from_static(b"test payload"),
        });
    }

    #[test]
    fn invalid_message_id() {
        assert!(Message::from_payload(Bytes::from_static(&[99u8])).is_err());
    }

    #[test]
    fn suggest_piece() {
        round_trip(Message::SuggestPiece(42));
    }

    #[test]
    fn have_all() {
        round_trip(Message::HaveAll);
    }

    #[test]
    fn have_none() {
        round_trip(Message::HaveNone);
    }

    #[test]
    fn reject_request() {
        round_trip(Message::RejectRequest {
            index: 1,
            begin: 0,
            length: 16384,
        });
    }

    #[test]
    fn allowed_fast() {
        round_trip(Message::AllowedFast(7));
    }

    #[test]
    fn hash_request_round_trip() {
        let msg = Message::HashRequest {
            pieces_root: gaia_core::Id32::ZERO,
            base: 7,
            index: 0,
            count: 512,
            proof_layers: 3,
        };
        round_trip(msg);
    }

    #[test]
    fn hash_reject_round_trip() {
        let msg = Message::HashReject {
            pieces_root: gaia_core::Id32::ZERO,
            base: 7,
            index: 0,
            count: 512,
            proof_layers: 3,
        };
        round_trip(msg);
    }

    #[test]
    fn hashes_round_trip() {
        let h1 = gaia_core::sha256(b"block1");
        let h2 = gaia_core::sha256(b"block2");
        let uncle = gaia_core::sha256(b"uncle");
        let msg = Message::Hashes {
            pieces_root: gaia_core::Id32::ZERO,
            base: 0,
            index: 0,
            count: 2,
            proof_layers: 1,
            hashes: vec![h1, h2, uncle],
        };
        round_trip(msg);
    }

    #[test]
    fn hash_request_exact_wire_size() {
        let msg: Message = Message::HashRequest {
            pieces_root: gaia_core::Id32::ZERO,
            base: 0,
            index: 0,
            count: 1,
            proof_layers: 0,
        };
        let bytes = msg.to_bytes();
        // 4 (length prefix) + 1 (msg id) + 32 (root) + 4*4 (fields) = 53
        assert_eq!(bytes.len(), 53);
    }

    #[test]
    fn hashes_variable_length() {
        let h = gaia_core::sha256(b"test");
        let msg: Message = Message::Hashes {
            pieces_root: gaia_core::Id32::ZERO,
            base: 0,
            index: 0,
            count: 1,
            proof_layers: 0,
            hashes: vec![h],
        };
        let bytes = msg.to_bytes();
        // 4 + 1 + 32 + 4*4 + 1*32 = 85
        assert_eq!(bytes.len(), 85);
    }

    #[test]
    fn hash_request_too_short() {
        // msg id 21, but only 10 bytes of body (need 48)
        let mut payload = vec![21u8];
        payload.extend_from_slice(&[0u8; 10]);
        assert!(Message::from_payload(Bytes::from(payload)).is_err());
    }

    #[test]
    fn encode_into_matches_to_bytes() {
        let messages = vec![
            Message::KeepAlive,
            Message::Choke,
            Message::Unchoke,
            Message::Interested,
            Message::NotInterested,
            Message::Have { index: 42 },
            Message::Bitfield(Bytes::from_static(b"\xff\x00")),
            Message::Request {
                index: 1,
                begin: 0,
                length: 16384,
            },
            Message::Piece {
                index: 0,
                begin: 0,
                data_0: Bytes::from_static(b"block data here"),
                data_1: Bytes::new(),
            },
            Message::Cancel {
                index: 1,
                begin: 0,
                length: 16384,
            },
            Message::Port(6881),
            Message::Extended {
                ext_id: 0,
                payload: Bytes::from_static(b"ext payload"),
            },
            Message::SuggestPiece(7),
            Message::HaveAll,
            Message::HaveNone,
            Message::RejectRequest {
                index: 1,
                begin: 0,
                length: 16384,
            },
            Message::AllowedFast(5),
            Message::HashRequest {
                pieces_root: gaia_core::Id32::ZERO,
                base: 7,
                index: 0,
                count: 512,
                proof_layers: 3,
            },
            Message::HashReject {
                pieces_root: gaia_core::Id32::ZERO,
                base: 7,
                index: 0,
                count: 512,
                proof_layers: 3,
            },
            Message::Hashes {
                pieces_root: gaia_core::Id32::ZERO,
                base: 0,
                index: 0,
                count: 2,
                proof_layers: 1,
                hashes: vec![
                    gaia_core::sha256(b"block1"),
                    gaia_core::sha256(b"block2"),
                    gaia_core::sha256(b"uncle"),
                ],
            },
        ];
        for msg in messages {
            let expected = msg.to_bytes();
            let mut buf = BytesMut::new();
            msg.encode_into(&mut buf);
            assert_eq!(&expected[..], &buf[..], "mismatch for {msg:?}");
        }
    }

    // ── M110: Generic Message<B> tests ──

    #[test]
    fn message_piece_two_fields_round_trip() {
        // Encode a Piece with data_0 only (data_1 empty) — common case.
        let msg = Message::Piece {
            index: 5,
            begin: 16384,
            data_0: Bytes::from_static(b"block payload here"),
            data_1: Bytes::new(),
        };
        let bytes = msg.to_bytes();
        let parsed = Message::from_payload(Bytes::copy_from_slice(&bytes[4..])).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn message_piece_split_data_round_trip() {
        // Encode a Piece with both data_0 and data_1 populated (ring-wrap case).
        // The wire format concatenates them, so decoding puts everything into data_0.
        let msg = Message::Piece {
            index: 3,
            begin: 0,
            data_0: Bytes::from_static(b"first half"),
            data_1: Bytes::from_static(b" second half"),
        };
        let bytes = msg.to_bytes();
        let parsed = Message::from_payload(Bytes::copy_from_slice(&bytes[4..])).unwrap();
        // After round-trip through the wire, the data is concatenated into data_0
        // with data_1 empty (the receiver doesn't know about ring-wrap).
        assert_eq!(
            parsed,
            Message::Piece {
                index: 3,
                begin: 0,
                data_0: Bytes::from_static(b"first half second half"),
                data_1: Bytes::new(),
            }
        );
    }

    #[test]
    fn message_generic_encode_borrowed() {
        // Verify that Message<&[u8]> can encode_into just like Message<Bytes>.
        let borrowed: Message<&[u8]> = Message::Piece {
            index: 1,
            begin: 0,
            data_0: b"borrowed data",
            data_1: b"",
        };
        let owned: Message<Bytes> = Message::Piece {
            index: 1,
            begin: 0,
            data_0: Bytes::from_static(b"borrowed data"),
            data_1: Bytes::new(),
        };
        let mut buf_borrowed = BytesMut::new();
        borrowed.encode_into(&mut buf_borrowed);
        let mut buf_owned = BytesMut::new();
        owned.encode_into(&mut buf_owned);
        assert_eq!(
            buf_borrowed, buf_owned,
            "borrowed and owned encode identically"
        );

        // Also test to_bytes
        assert_eq!(borrowed.to_bytes(), owned.to_bytes());

        // Test borrowed Bitfield
        let bf_borrowed: Message<&[u8]> = Message::Bitfield(b"\xff\x80");
        let bf_owned: Message<Bytes> = Message::Bitfield(Bytes::from_static(b"\xff\x80"));
        assert_eq!(bf_borrowed.to_bytes(), bf_owned.to_bytes());

        // Test borrowed Extended
        let ext_borrowed: Message<&[u8]> = Message::Extended {
            ext_id: 1,
            payload: b"payload",
        };
        let ext_owned: Message<Bytes> = Message::Extended {
            ext_id: 1,
            payload: Bytes::from_static(b"payload"),
        };
        assert_eq!(ext_borrowed.to_bytes(), ext_owned.to_bytes());
    }

    // ── M115: encode_to_slice tests ──

    /// Build the complete set of message variants used for `encode_to_slice` tests.
    fn all_message_variants() -> Vec<Message> {
        vec![
            Message::KeepAlive,
            Message::Choke,
            Message::Unchoke,
            Message::Interested,
            Message::NotInterested,
            Message::Have { index: 42 },
            Message::Bitfield(Bytes::from_static(b"\xff\x00")),
            Message::Request {
                index: 1,
                begin: 0,
                length: 16384,
            },
            Message::Piece {
                index: 0,
                begin: 0,
                data_0: Bytes::from_static(b"block data here"),
                data_1: Bytes::new(),
            },
            Message::Piece {
                index: 3,
                begin: 0,
                data_0: Bytes::from_static(b"first half"),
                data_1: Bytes::from_static(b" second half"),
            },
            Message::Cancel {
                index: 1,
                begin: 0,
                length: 16384,
            },
            Message::Port(6881),
            Message::Extended {
                ext_id: 0,
                payload: Bytes::from_static(b"ext payload"),
            },
            Message::SuggestPiece(7),
            Message::HaveAll,
            Message::HaveNone,
            Message::RejectRequest {
                index: 1,
                begin: 0,
                length: 16384,
            },
            Message::AllowedFast(5),
            Message::HashRequest {
                pieces_root: gaia_core::Id32::ZERO,
                base: 7,
                index: 0,
                count: 512,
                proof_layers: 3,
            },
            Message::HashReject {
                pieces_root: gaia_core::Id32::ZERO,
                base: 7,
                index: 0,
                count: 512,
                proof_layers: 3,
            },
            Message::Hashes {
                pieces_root: gaia_core::Id32::ZERO,
                base: 0,
                index: 0,
                count: 2,
                proof_layers: 1,
                hashes: vec![
                    gaia_core::sha256(b"block1"),
                    gaia_core::sha256(b"block2"),
                    gaia_core::sha256(b"uncle"),
                ],
            },
        ]
    }

    #[test]
    fn encode_to_slice_roundtrip() {
        for msg in all_message_variants() {
            let mut buf = [0u8; 4096];
            let n = msg.encode_to_slice(&mut buf);
            // Skip the 4-byte length prefix for parsing
            let parsed = Message::from_payload(Bytes::copy_from_slice(&buf[4..n])).unwrap();
            // For Piece with split data, wire format concatenates into data_0
            match &msg {
                Message::Piece {
                    index,
                    begin,
                    data_0,
                    data_1,
                } if !data_1.is_empty() => {
                    let mut combined = Vec::from(data_0.as_ref());
                    combined.extend_from_slice(data_1.as_ref());
                    let expected = Message::Piece {
                        index: *index,
                        begin: *begin,
                        data_0: Bytes::from(combined),
                        data_1: Bytes::new(),
                    };
                    assert_eq!(parsed, expected, "roundtrip mismatch for split Piece");
                }
                _ => {
                    assert_eq!(msg, parsed, "roundtrip mismatch for {msg:?}");
                }
            }
        }
    }

    #[test]
    fn encode_to_slice_matches_encode_into() {
        for msg in all_message_variants() {
            let mut slice_buf = [0u8; 4096];
            let n = msg.encode_to_slice(&mut slice_buf);

            let mut bytes_buf = BytesMut::new();
            msg.encode_into(&mut bytes_buf);

            assert_eq!(
                &slice_buf[..n],
                &bytes_buf[..],
                "encode_to_slice vs encode_into mismatch for {msg:?}"
            );
        }
    }

    #[test]
    fn wire_len_matches_encoded_size() {
        for msg in all_message_variants() {
            let expected = msg.to_bytes().len();
            assert_eq!(msg.wire_len(), expected, "wire_len mismatch for {msg:?}");
        }
    }

    #[test]
    fn wire_len_large_bitfield() {
        // Bitfield for a torrent with >131K pieces (16,384 bytes of bitfield data).
        let bits = vec![0xFFu8; 20_000];
        let msg = Message::Bitfield(Bytes::from(bits.clone()));
        assert_eq!(msg.wire_len(), 5 + bits.len());
        assert_eq!(msg.wire_len(), msg.to_bytes().len());
    }
}
