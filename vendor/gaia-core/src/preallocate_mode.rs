use serde::{Deserialize, Serialize};

/// Pre-allocation strategy for torrent files.
///
/// Lives in `irontide-core` so configuration crates can reference it
/// without depending on `irontide-storage` (M242); since M257a it is the
/// sole storage-allocation knob (the `StorageMode` enum was deleted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreallocateMode {
    /// No pre-allocation — files created sparse via `set_len`.
    #[default]
    None,
    /// Reserve extents without write amplification (`FALLOC_FL_KEEP_SIZE` on Linux).
    /// Falls back to `None` on unsupported filesystems.
    Sparse,
    /// Full allocation — `fallocate(0)` on Linux, falls back to writing zeros.
    Full,
}

impl From<bool> for PreallocateMode {
    fn from(preallocate: bool) -> Self {
        if preallocate { Self::Full } else { Self::None }
    }
}
