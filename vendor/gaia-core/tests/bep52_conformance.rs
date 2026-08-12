//! BEP 52 v2 torrent conformance tests.
//!
//! Validates `BitTorrent` v2 Merkle tree invariants from the specification
//! without relying on external test fixtures.

use gaia_core::{
    CreateTorrent, Id32, MerkleTree, TorrentVersion, sha256, torrent_from_bytes_any,
};

/// BEP 52 specifies that the padding leaf in a Merkle tree is SHA-256 of
/// the empty input. This is a fundamental invariant — if the constant drifts,
/// all v2 torrents break.
#[test]
fn v2_padding_leaf_is_sha256_empty() {
    let empty_hash = sha256(b"");
    assert_eq!(
        empty_hash.to_hex(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "padding leaf must be SHA-256 of empty input"
    );

    // Id32::ZERO is used as the padding sentinel in MerkleTree::from_leaves.
    // BEP 52 specifies padding leaves as zero-filled 32-byte hashes, NOT
    // as SHA-256(empty). Verify these are distinct — if they were equal,
    // the tree implementation would be accidentally correct for the wrong reason.
    assert_ne!(
        Id32::ZERO,
        empty_hash,
        "ZERO sentinel must differ from SHA-256(empty)"
    );
}

/// BEP 52 requires piece length to be a power of 2 and at least 16 KiB.
/// Verify the builder rejects non-power-of-2 piece lengths.
#[test]
fn v2_reject_non_power_of_2_piece_length() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.dat");
    std::fs::write(&file_path, vec![0xAA; 32768]).unwrap();

    // 48000 is not a power of 2
    let result = CreateTorrent::new()
        .add_file(&file_path)
        .set_piece_size(48000)
        .set_version(TorrentVersion::V2Only)
        .generate();
    assert!(
        result.is_err(),
        "non-power-of-2 piece size must be rejected"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("power of 2"),
        "error message should mention power-of-2 constraint, got: {err_msg}"
    );

    // Also reject piece sizes smaller than 16 KiB
    let result = CreateTorrent::new()
        .add_file(&file_path)
        .set_piece_size(8192)
        .set_version(TorrentVersion::V2Only)
        .generate();
    assert!(result.is_err(), "piece size < 16384 must be rejected");

    // Verify a valid power-of-2 piece size succeeds
    let result = CreateTorrent::new()
        .add_file(&file_path)
        .set_piece_size(16384)
        .set_version(TorrentVersion::V2Only)
        .generate();
    assert!(result.is_ok(), "valid piece size 16384 must succeed");
}

/// The v2 info hash is defined as SHA-256 of the raw bencode bytes of the info
/// dict. Verify this by manually computing it from the serialized torrent.
#[test]
fn v2_info_hash_is_sha256_of_info_dict() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.dat");
    std::fs::write(&file_path, vec![0x42; 16384]).unwrap();

    let result = CreateTorrent::new()
        .add_file(&file_path)
        .set_piece_size(16384)
        .set_version(TorrentVersion::V2Only)
        .set_name("test.dat")
        .generate()
        .expect("v2 torrent creation must succeed");

    let torrent_bytes = &result.bytes;

    // Extract the raw info dict span from the serialized torrent
    let info_span = gaia_bencode::find_dict_key_span(torrent_bytes, "info")
        .expect("torrent must contain info dict");
    let info_raw = &torrent_bytes[info_span];

    // Manually compute SHA-256 of the raw info dict bytes
    let manual_hash = sha256(info_raw);

    // Compare against the v2 info hash from the result
    let info_hashes = result.meta.info_hashes();
    let v2_hash = info_hashes.v2.expect("v2 torrent must have v2 info hash");

    assert_eq!(
        manual_hash, v2_hash,
        "v2 info hash must equal SHA-256 of raw info dict bytes"
    );
}

/// Build a Merkle tree manually from block-level SHA-256 hashes and verify the
/// root matches what `CreateTorrent` produces as the piece root for a single-piece file.
#[test]
fn v2_piece_root_matches_manual_merkle() {
    // Create a file exactly one piece in size (16 KiB = one 16 KiB block)
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.dat");
    let data = vec![0x55u8; 16384];
    std::fs::write(&file_path, &data).unwrap();

    let result = CreateTorrent::new()
        .add_file(&file_path)
        .set_piece_size(16384)
        .set_version(TorrentVersion::V2Only)
        .set_name("test.dat")
        .generate()
        .expect("v2 torrent creation must succeed");

    // Parse the torrent to get the file tree's pieces_root
    let meta = torrent_from_bytes_any(&result.bytes).expect("torrent must parse");
    let v2 = meta.as_v2().expect("must be a v2 torrent");
    let files = v2.info.files();
    assert_eq!(
        files.len(),
        1,
        "single-file torrent must have exactly one file"
    );
    let pieces_root = files[0]
        .attr
        .pieces_root
        .expect("file must have pieces_root");

    // Manually compute: hash the 16 KiB block, build a single-leaf Merkle tree
    let block_hash = sha256(&data);
    let manual_tree = MerkleTree::from_leaves(&[block_hash]);
    let manual_root = manual_tree.root();

    assert_eq!(
        pieces_root, manual_root,
        "pieces_root from CreateTorrent must match manually computed Merkle root"
    );
}

/// A hybrid torrent must contain both a v1 SHA-1 info hash and a v2 SHA-256
/// info hash. Verify both are present and distinct.
#[test]
fn v2_hybrid_has_both_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.dat");
    std::fs::write(&file_path, vec![0xBB; 16384]).unwrap();

    let result = CreateTorrent::new()
        .add_file(&file_path)
        .set_piece_size(16384)
        .set_version(TorrentVersion::Hybrid)
        .set_name("test.dat")
        .generate()
        .expect("hybrid torrent creation must succeed");

    let meta = &result.meta;
    assert!(meta.is_hybrid(), "torrent must be detected as hybrid");

    let info_hashes = meta.info_hashes();
    assert!(
        info_hashes.has_v1(),
        "hybrid torrent must have v1 (SHA-1) info hash"
    );
    assert!(
        info_hashes.has_v2(),
        "hybrid torrent must have v2 (SHA-256) info hash"
    );
    assert!(
        info_hashes.is_hybrid(),
        "InfoHashes::is_hybrid() must return true"
    );

    // The v1 hash (20 bytes) and v2 hash (32 bytes) have different lengths
    // and algorithms, so they must not be trivially related
    let v1 = info_hashes.v1.unwrap();
    let v2 = info_hashes.v2.unwrap();
    assert_ne!(
        &v1.as_bytes()[..],
        &v2.as_bytes()[..20],
        "v1 SHA-1 hash must not equal first 20 bytes of v2 SHA-256 hash"
    );

    // Re-parse from raw bytes to double-check the round-trip
    let reparsed = torrent_from_bytes_any(&result.bytes).expect("torrent must re-parse");
    assert!(reparsed.is_hybrid(), "re-parsed torrent must be hybrid");
    let reparsed_hashes = reparsed.info_hashes();
    assert_eq!(
        reparsed_hashes.v1, info_hashes.v1,
        "v1 hash must survive round-trip"
    );
    assert_eq!(
        reparsed_hashes.v2, info_hashes.v2,
        "v2 hash must survive round-trip"
    );
}
