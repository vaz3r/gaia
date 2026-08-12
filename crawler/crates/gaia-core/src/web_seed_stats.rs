//! Per-URL stats for web-seed sources (BEP 17 + BEP 19).
//!
//! Lives in `irontide-core` because [`crate::FastResumeData`] persists a
//! `HashMap<String, WebSeedStats>` so stats survive app restart (M178
//! Tension-1). `irontide-session` re-exports this type to keep its public
//! surface stable.

use serde::{Deserialize, Serialize};

/// Coarse state of a single web-seed source.
///
/// `Idle` — no successful chunk yet observed (cold-start before any progress).
/// `Active` — at least one successful chunk landed; currently downloading.
/// `Errored` — the most recent attempt failed; [`WebSeedStats::last_error`]
/// has the message. Transitions back to `Active` on recovery, but
/// `last_error` PERSISTS so users can see "currently working but flaked
/// recently" in the GUI (M178 Issue 2.2 / D-eng-8).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSeedState {
    /// No successful chunk yet observed.
    #[default]
    Idle,
    /// At least one successful chunk landed; currently downloading.
    Active,
    /// Most recent attempt failed; `last_error` is populated.
    Errored,
}

/// Per-URL stats for a single web-seed source, surfaced to the qBt v2
/// `/api/v2/torrents/webseeds` endpoint and the GUI's HTTP Sources tab.
///
/// `consecutive_failures` resets to zero on a successful chunk; within a
/// failure run it monotonically increments. `last_error` PERSISTS through
/// the Errored→Active transition so the GUI can show "currently active,
/// last failed with X" — intentional per Issue 2.2 / D-eng-8.
///
/// `next_retry_unix_secs` is forward-compatible with M186's BEP 17/19
/// retry-with-backoff logic (D-eng-9 / Issue 2.3); always `None` in M178.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSeedStats {
    /// Web-seed URL (BEP 19 url-list or BEP 17 httpseeds entry).
    pub url: String,
    /// Coarse current state. See [`WebSeedState`].
    #[serde(default)]
    pub state: WebSeedState,
    /// Cumulative bytes downloaded from this URL since first observation.
    #[serde(default)]
    pub downloaded_bytes: u64,
    /// Rate at the most recent emission, computed over the throttle window.
    #[serde(default)]
    pub last_rate_bps: u64,
    /// Most recently observed error message; persists through recovery
    /// (Issue 2.2 / D-eng-8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Consecutive failures since the last success. Resets to zero on
    /// a successful chunk; monotonically increments within a failure run.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Unix timestamp of the most recent emission (success or failure).
    #[serde(default)]
    pub last_attempt_unix_secs: u64,
    /// M186 forward-compat (D-eng-9): when set, the unix timestamp at
    /// which BEP 17/19 retry-with-backoff will next attempt this URL.
    /// Always `None` in M178; populated once the retry logic ships.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_unix_secs: Option<u64>,
}
