use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// fsync policy for verified piece data (M260).
///
/// Lives in `irontide-core` so the settings and storage crates can reference it
/// without a cross-dependency (mirrors [`crate::PreallocateMode`]). The default
/// preserves pre-M260 behaviour: the engine never issued explicit `fsync`,
/// relying on the OS to flush — fast, but a crash can lose unflushed pieces
/// and force a re-download on restart. `PerPiece` / `OnComplete` are explicit
/// opt-ins that trade throughput for crash durability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DurabilityMode {
    /// No explicit fsync — the OS flushes on its own schedule (default, pre-M260
    /// behaviour). Highest throughput; a crash may lose recently-verified pieces.
    #[default]
    Never,
    /// fsync each piece's backing file handles as the piece passes verification.
    /// Bounds crash loss to in-flight pieces at the cost of per-piece flush stalls.
    PerPiece,
    /// fsync all of a torrent's file handles once, when the torrent completes.
    /// A middle ground: durable at completion without per-piece flush overhead.
    OnComplete,
}

impl fmt::Display for DurabilityMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_slug())
    }
}

impl DurabilityMode {
    /// The canonical lowercase slug used in config files and qBt-style wire forms.
    #[must_use]
    pub const fn as_slug(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::PerPiece => "per_piece",
            Self::OnComplete => "on_complete",
        }
    }
}

impl FromStr for DurabilityMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "never" => Ok(Self::Never),
            "per_piece" => Ok(Self::PerPiece),
            "on_complete" => Ok(Self::OnComplete),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_never() {
        assert_eq!(DurabilityMode::default(), DurabilityMode::Never);
    }

    #[test]
    fn slug_round_trip() {
        for m in [
            DurabilityMode::Never,
            DurabilityMode::PerPiece,
            DurabilityMode::OnComplete,
        ] {
            assert_eq!(m.as_slug().parse::<DurabilityMode>(), Ok(m));
            assert_eq!(m.to_string(), m.as_slug());
        }
    }

    #[test]
    fn unknown_slug_rejected() {
        assert_eq!("fsync_always".parse::<DurabilityMode>(), Err(()));
        assert_eq!("".parse::<DurabilityMode>(), Err(()));
    }

    #[test]
    fn serde_round_trip() {
        let json = serde_json::to_string(&DurabilityMode::PerPiece).unwrap();
        assert_eq!(json, "\"PerPiece\"");
        let back: DurabilityMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, DurabilityMode::PerPiece);
    }
}
