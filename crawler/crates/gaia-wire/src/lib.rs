#![warn(missing_docs)]
#![forbid(unsafe_code)]
//! `BitTorrent` peer wire protocol: handshake, messages, BEP 6/9/10/21/52 extensions, MSE/PE encryption.
//!
//! Provides message types, handshake, and a tokio codec for framed I/O.

mod codec;
mod error;
mod extended;
mod handshake;
mod message;

pub use codec::MessageCodec;
pub use error::{Error, Result};
pub use extended::{ExtHandshake, ExtMessage, MetadataMessage, MetadataMessageType};
pub use handshake::Handshake;
pub use message::Message;
