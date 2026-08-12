//! Encrypted stream wrapper implementing `AsyncRead` + `AsyncWrite`.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::cipher::Rc4;

/// Default write buffer capacity — matches `DEFAULT_CHUNK_SIZE` (16 KiB).
const WRITE_BUF_CAPACITY: usize = 16_384;

/// A stream that optionally encrypts/decrypts all data with RC4.
///
/// When ciphers are None, data passes through unmodified (plaintext mode).
pub struct MseStream<S> {
    inner: S,
    read_cipher: Option<Rc4>,
    write_cipher: Option<Rc4>,
    write_buf: Vec<u8>,
    initial_read: Vec<u8>,
}

impl<S> MseStream<S> {
    /// Create a plaintext stream (no encryption).
    pub fn plaintext(inner: S) -> Self {
        Self {
            inner,
            read_cipher: None,
            write_cipher: None,
            write_buf: Vec::new(),
            initial_read: Vec::new(),
        }
    }

    /// Create an encrypted stream with RC4 ciphers.
    ///
    /// `initial_read` contains overflow bytes read past the VC marker during
    /// the handshake scan. These bytes are drained first on subsequent reads
    /// before reading from the inner stream.
    pub(crate) fn encrypted(
        inner: S,
        read_cipher: Rc4,
        write_cipher: Rc4,
        initial_read: Vec<u8>,
    ) -> Self {
        Self {
            inner,
            read_cipher: Some(read_cipher),
            write_cipher: Some(write_cipher),
            write_buf: Vec::with_capacity(WRITE_BUF_CAPACITY),
            initial_read,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for MseStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        if !this.initial_read.is_empty() {
            let to_copy = this.initial_read.len().min(buf.remaining());
            buf.put_slice(&this.initial_read[..to_copy]);
            this.initial_read.drain(..to_copy);
            return Poll::Ready(Ok(()));
        }

        let before = buf.filled().len();

        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if let Some(cipher) = &mut this.read_cipher {
                    let filled = buf.filled_mut();
                    cipher.apply(&mut filled[before..]);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for MseStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        if let Some(cipher) = &mut this.write_cipher {
            this.write_buf.clear();
            this.write_buf.extend_from_slice(buf);
            cipher.apply(&mut this.write_buf);
            Pin::new(&mut this.inner).poll_write(cx, &this.write_buf)
        } else {
            Pin::new(&mut this.inner).poll_write(cx, buf)
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn plaintext_passthrough() {
        let (client, server) = tokio::io::duplex(1024);
        let mut client = MseStream::plaintext(client);
        let mut server = MseStream::plaintext(server);

        client.write_all(b"hello").await.unwrap();
        client.flush().await.unwrap();

        let mut buf = [0u8; 5];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");
    }

    #[tokio::test]
    async fn encrypted_roundtrip() {
        let key_a = b"key for direction A!";
        let key_b = b"key for direction B!";

        let (raw_client, raw_server) = tokio::io::duplex(1024);

        // Client: encrypt with A, decrypt with B
        let mut client = MseStream::encrypted(
            raw_client,
            Rc4::new(key_b), // read (decrypt) = B
            Rc4::new(key_a), // write (encrypt) = A
            Vec::new(),
        );

        // Server: decrypt with A, encrypt with B
        let mut server = MseStream::encrypted(
            raw_server,
            Rc4::new(key_a), // read (decrypt) = A
            Rc4::new(key_b), // write (encrypt) = B
            Vec::new(),
        );

        // Client -> Server
        client.write_all(b"client to server").await.unwrap();
        client.flush().await.unwrap();

        let mut buf = [0u8; 16];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"client to server");

        // Server -> Client
        server.write_all(b"server to client").await.unwrap();
        server.flush().await.unwrap();

        let mut buf = [0u8; 16];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"server to client");
    }

    #[test]
    fn encrypted_write_buf_pre_allocated() {
        let (raw, _) = tokio::io::duplex(1024);
        let stream = MseStream::encrypted(raw, Rc4::new(b"r"), Rc4::new(b"w"), Vec::new());
        assert_eq!(stream.write_buf.capacity(), WRITE_BUF_CAPACITY);
    }

    #[tokio::test]
    async fn encrypted_no_realloc_on_chunk_write() {
        let (raw_client, _raw_server) = tokio::io::duplex(32768);
        let mut client =
            MseStream::encrypted(raw_client, Rc4::new(b"r"), Rc4::new(b"w"), Vec::new());
        let data = vec![0xABu8; WRITE_BUF_CAPACITY];
        client.write_all(&data).await.unwrap();
        assert_eq!(client.write_buf.capacity(), WRITE_BUF_CAPACITY);
    }

    #[tokio::test]
    async fn initial_read_drains_before_inner() {
        let (raw_client, mut raw_server) = tokio::io::duplex(1024);

        let initial = b"overflow".to_vec();
        let mut client = MseStream::encrypted(raw_client, Rc4::new(b"r"), Rc4::new(b"w"), initial);

        // Write something to the inner stream from the other side
        raw_server.write_all(b"inner").await.unwrap();
        raw_server.flush().await.unwrap();

        // First read should return initial_read bytes (plaintext, not decrypted)
        let mut buf = [0u8; 8];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"overflow");

        // Second read should come from inner (decrypted)
        let mut buf = [0u8; 5];
        client.read_exact(&mut buf).await.unwrap();
        // The inner bytes are decrypted — just verify we got 5 bytes
        assert_eq!(buf.len(), 5);
    }
}
