#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "M175: piece arithmetic — narrowing casts bounded by `num_pieces: u32` invariant established in `Lengths::new`"
)]

/// Piece and chunk arithmetic for `BitTorrent` downloads.
///
/// Manages the mapping between:
/// - **Pieces**: fixed-size blocks verified by SHA1 (except possibly the last piece)
/// - **Chunks**: sub-piece blocks requested from peers (typically 16 KiB)
/// - **Files**: the actual files on disk that pieces map across
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lengths {
    /// Total size of all files in bytes.
    total_length: u64,
    /// Size of each piece in bytes (last piece may be smaller).
    piece_length: u64,
    /// Size of each chunk/block in bytes (typically 16384).
    chunk_size: u32,
    /// Pre-computed number of pieces.
    num_pieces: u32,
    /// Pre-computed size of the last piece.
    last_piece_size: u32,
}

/// Default chunk size (16 KiB) — standard in `BitTorrent`.
pub const DEFAULT_CHUNK_SIZE: u32 = 16384;

impl Lengths {
    /// Create a new Lengths calculator.
    ///
    /// # Panics
    /// Panics if `piece_length` or `chunk_size` is 0.
    #[must_use]
    pub fn new(total_length: u64, piece_length: u64, chunk_size: u32) -> Self {
        assert!(piece_length > 0, "piece_length must be > 0");
        assert!(chunk_size > 0, "chunk_size must be > 0");

        let num_pieces = if total_length == 0 {
            0
        } else {
            total_length.div_ceil(piece_length) as u32
        };

        let last_piece_size = if num_pieces == 0 {
            0
        } else {
            let remainder = total_length % piece_length;
            if remainder == 0 {
                piece_length as u32
            } else {
                remainder as u32
            }
        };

        Self {
            total_length,
            piece_length,
            chunk_size,
            num_pieces,
            last_piece_size,
        }
    }

    /// Total size of all content.
    #[must_use]
    pub fn total_length(&self) -> u64 {
        self.total_length
    }

    /// Standard piece length.
    #[must_use]
    pub fn piece_length(&self) -> u64 {
        self.piece_length
    }

    /// Chunk/block size.
    #[must_use]
    pub fn chunk_size(&self) -> u32 {
        self.chunk_size
    }

    /// Total number of pieces.
    #[must_use]
    pub fn num_pieces(&self) -> u32 {
        self.num_pieces
    }

    /// Actual length of a specific piece (last piece may be shorter).
    #[inline]
    #[must_use]
    pub fn piece_size(&self, piece_index: u32) -> u32 {
        if piece_index >= self.num_pieces {
            0
        } else if piece_index == self.num_pieces - 1 {
            self.last_piece_size
        } else {
            self.piece_length as u32
        }
    }

    /// Number of chunks in a specific piece.
    #[inline]
    #[must_use]
    pub fn chunks_in_piece(&self, piece_index: u32) -> u32 {
        let piece_size = u64::from(self.piece_size(piece_index));
        if piece_size == 0 {
            return 0;
        }
        piece_size.div_ceil(u64::from(self.chunk_size)) as u32
    }

    /// Offset and length of a specific chunk within a piece.
    ///
    /// Returns `(offset_within_piece, chunk_length)`.
    #[inline]
    #[must_use]
    pub fn chunk_info(&self, piece_index: u32, chunk_index: u32) -> Option<(u32, u32)> {
        let piece_size = self.piece_size(piece_index);
        if piece_size == 0 {
            return None;
        }

        let offset = chunk_index * self.chunk_size;
        if offset >= piece_size {
            return None;
        }

        let remaining = piece_size - offset;
        let len = remaining.min(self.chunk_size);
        Some((offset, len))
    }

    /// Absolute byte offset for the start of a piece.
    #[must_use]
    pub fn piece_offset(&self, piece_index: u32) -> u64 {
        u64::from(piece_index) * self.piece_length
    }

    /// Map an absolute byte offset to a piece index.
    ///
    /// Returns `None` when `byte_offset >= total_length`.
    ///
    /// Prefer this over the ad-hoc `(byte / piece_length) as u32` form: M132's
    /// in-flight underflow is the cautionary precedent for narrowing casts on
    /// hot paths. Routing all byte→piece conversions through one bounded
    /// function keeps that bug class in one place.
    #[inline]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "byte_offset < total_length, total_length / piece_length ≤ num_pieces (u32)"
    )]
    #[must_use]
    pub fn piece_index_for_byte(&self, byte_offset: u64) -> Option<u32> {
        if byte_offset >= self.total_length {
            return None;
        }
        Some((byte_offset / self.piece_length) as u32)
    }

    /// Map an absolute byte offset to `(piece_index, offset_within_piece)`.
    ///
    /// Sibling of [`Self::piece_index_for_byte`] for callers that also need
    /// the offset within the piece (e.g. disk reads at a sub-piece position).
    /// Returns `None` when `byte_offset >= total_length`.
    #[inline]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "byte_offset < total_length: piece_index bounded by num_pieces (u32); offset_in_piece bounded by piece_length (u32 by construction in `Lengths::new`)"
    )]
    #[must_use]
    pub fn byte_to_piece_with_offset(&self, byte_offset: u64) -> Option<(u32, u32)> {
        if byte_offset >= self.total_length {
            return None;
        }
        let piece_index = (byte_offset / self.piece_length) as u32;
        let offset_in_piece = (byte_offset % self.piece_length) as u32;
        Some((piece_index, offset_in_piece))
    }

    /// Given file boundaries, determine which pieces a file spans.
    /// Returns `(first_piece, last_piece)` inclusive.
    #[must_use]
    pub fn file_pieces(&self, file_offset: u64, file_length: u64) -> Option<(u32, u32)> {
        if file_length == 0 || file_offset >= self.total_length {
            return None;
        }
        let first = (file_offset / self.piece_length) as u32;
        let last_byte = file_offset + file_length - 1;
        let last = (last_byte.min(self.total_length - 1) / self.piece_length) as u32;
        Some((first, last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lengths() -> Lengths {
        // 1 MiB total, 256 KiB pieces, 16 KiB chunks
        Lengths::new(1_048_576, 262_144, 16_384)
    }

    #[test]
    fn num_pieces_exact_division() {
        let l = make_lengths();
        assert_eq!(l.num_pieces(), 4); // 1 MiB / 256 KiB = 4
    }

    #[test]
    fn num_pieces_with_remainder() {
        let l = Lengths::new(1_000_000, 262_144, 16_384);
        assert_eq!(l.num_pieces(), 4); // ceil(1000000 / 262144) = 4
    }

    #[test]
    fn piece_size_regular() {
        let l = make_lengths();
        assert_eq!(l.piece_size(0), 262_144);
        assert_eq!(l.piece_size(1), 262_144);
        assert_eq!(l.piece_size(3), 262_144); // last piece, exact division
    }

    #[test]
    fn piece_size_last_piece_shorter() {
        let l = Lengths::new(1_000_000, 262_144, 16_384);
        assert_eq!(l.piece_size(0), 262_144);
        assert_eq!(l.piece_size(3), 1_000_000 - 3 * 262_144); // 213568
    }

    #[test]
    fn piece_size_out_of_bounds() {
        let l = make_lengths();
        assert_eq!(l.piece_size(4), 0);
        assert_eq!(l.piece_size(100), 0);
    }

    #[test]
    fn chunks_in_piece() {
        let l = make_lengths();
        assert_eq!(l.chunks_in_piece(0), 16); // 262144 / 16384 = 16
    }

    #[test]
    fn chunks_in_last_piece() {
        let l = Lengths::new(1_000_000, 262_144, 16_384);
        let last_piece_size = 1_000_000 - 3 * 262_144; // 213568
        let expected_chunks = (last_piece_size + 16383) / 16384; // 14
        assert_eq!(l.chunks_in_piece(3), expected_chunks as u32);
    }

    #[test]
    fn chunk_info_regular() {
        let l = make_lengths();
        assert_eq!(l.chunk_info(0, 0), Some((0, 16384)));
        assert_eq!(l.chunk_info(0, 1), Some((16384, 16384)));
        assert_eq!(l.chunk_info(0, 15), Some((15 * 16384, 16384)));
    }

    #[test]
    fn chunk_info_last_chunk_shorter() {
        // 100000 byte total, 50000 byte pieces, 16384 chunks
        let l = Lengths::new(100_000, 50_000, 16_384);
        // Piece 0: 50000 bytes, chunks: 0..16384, 16384..32768, 32768..49152 (16384), 49152..50000 (848)
        assert_eq!(l.chunk_info(0, 3), Some((49152, 848)));
    }

    #[test]
    fn chunk_info_out_of_bounds() {
        let l = make_lengths();
        assert_eq!(l.chunk_info(0, 16), None); // only 16 chunks (0..15)
        assert_eq!(l.chunk_info(4, 0), None); // piece doesn't exist
    }

    #[test]
    fn piece_offset() {
        let l = make_lengths();
        assert_eq!(l.piece_offset(0), 0);
        assert_eq!(l.piece_offset(1), 262_144);
        assert_eq!(l.piece_offset(3), 786_432);
    }

    #[test]
    fn byte_to_piece_with_offset_basic() {
        let l = make_lengths();
        assert_eq!(l.byte_to_piece_with_offset(0), Some((0, 0)));
        assert_eq!(l.byte_to_piece_with_offset(262_143), Some((0, 262_143)));
        assert_eq!(l.byte_to_piece_with_offset(262_144), Some((1, 0)));
        assert_eq!(l.byte_to_piece_with_offset(1_048_575), Some((3, 262_143)));
        assert_eq!(l.byte_to_piece_with_offset(1_048_576), None); // past end
    }

    #[test]
    fn piece_index_for_byte_at_zero() {
        let l = make_lengths();
        assert_eq!(l.piece_index_for_byte(0), Some(0));
    }

    #[test]
    fn piece_index_for_byte_at_piece_boundary() {
        let l = make_lengths();
        // First byte of piece 1 — exactly at piece_length.
        assert_eq!(l.piece_index_for_byte(262_144), Some(1));
        // Last byte of piece 0 — one before piece_length.
        assert_eq!(l.piece_index_for_byte(262_143), Some(0));
    }

    #[test]
    fn piece_index_for_byte_at_total_length_returns_none() {
        // Boundary regression: total_length is exclusive — exactly at it must
        // return None, not Some(num_pieces) which would index past the bitmap.
        let l = make_lengths();
        assert_eq!(l.piece_index_for_byte(1_048_576), None);
        assert_eq!(l.piece_index_for_byte(u64::MAX), None);
    }

    #[test]
    fn piece_index_for_byte_matches_byte_to_piece_with_offset() {
        // Sibling consistency: indices must agree across the full range.
        let l = make_lengths();
        for byte in [0, 1, 262_143, 262_144, 524_287, 524_288, 1_048_575] {
            let single = l.piece_index_for_byte(byte);
            let pair = l.byte_to_piece_with_offset(byte).map(|(p, _)| p);
            assert_eq!(single, pair, "disagreement at byte {byte}");
        }
        // Past-end agreement.
        assert_eq!(l.piece_index_for_byte(1_048_576), None);
        assert_eq!(l.byte_to_piece_with_offset(1_048_576), None);
    }

    #[test]
    fn piece_index_for_byte_uneven_last_piece() {
        // 1 MB total - 1 byte, 256 KiB pieces — last piece is short.
        let l = Lengths::new(1_048_575, 262_144, 16_384);
        assert_eq!(l.piece_index_for_byte(1_048_574), Some(3));
        assert_eq!(l.piece_index_for_byte(1_048_575), None);
    }

    #[test]
    fn file_pieces_spanning() {
        let l = make_lengths();
        // File starting at 100000, length 500000 — spans pieces 0..2
        assert_eq!(l.file_pieces(100_000, 500_000), Some((0, 2)));
    }

    #[test]
    fn file_pieces_single_piece() {
        let l = make_lengths();
        // File entirely within piece 1
        assert_eq!(l.file_pieces(262_144, 100), Some((1, 1)));
    }

    #[test]
    fn file_pieces_entire_torrent() {
        let l = make_lengths();
        assert_eq!(l.file_pieces(0, 1_048_576), Some((0, 3)));
    }

    #[test]
    fn zero_length_torrent() {
        let l = Lengths::new(0, 262_144, 16_384);
        assert_eq!(l.num_pieces(), 0);
    }

    #[test]
    fn tiny_torrent() {
        let l = Lengths::new(1, 262_144, 16_384);
        assert_eq!(l.num_pieces(), 1);
        assert_eq!(l.piece_size(0), 1);
        assert_eq!(l.chunks_in_piece(0), 1);
        assert_eq!(l.chunk_info(0, 0), Some((0, 1)));
    }
}
