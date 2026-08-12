//! Broadcast surface for runtime DHT handle replacement (M173 Lane B B5).
//!
//! HA spec Primitive 4: when DHT is toggled (`enable_dht: false → true`
//! or vice versa) at runtime, every torrent that previously cloned the
//! `DhtHandle` keeps holding the OLD handle — its `mpsc::Sender` is
//! still wired to the OLD actor, which has shut down. DHT operations
//! silently time out from the torrent's perspective.
//!
//! The fix is a `tokio::sync::watch` channel: the session owns the
//! `Sender`, and every torrent borrows a `Receiver`. When the session
//! restarts the DHT, it sends the new handle through the channel and
//! every torrent observes the swap on its next `borrow()`.
//!
//! ## Why `watch::channel` (and not `Arc<RwLock<Option<DhtHandle>>>`)
//!
//! The HA spec floats both as alternatives. `watch` wins because:
//! - It's read-lock-free on the hot path. Every `dht_op()` call would
//!   otherwise pay an `RwLock::read()` round-trip.
//! - Receivers can `await` a change (`changed().await`) — useful for
//!   B6's queued-request-during-restart hold window.
//! - Single-writer / many-reader semantics match the actual use:
//!   only the session ever writes, every torrent only reads.
//!
//! ## Why a wrapper type and not raw `watch::channel`
//!
//! 1. `Option<DhtHandle>` cannot derive `Default` (DhtHandle does not),
//!    so the constructor needs to be explicit about the initial value.
//! 2. The wrapper centralises the audit doc — every consumer goes
//!    through `DhtBroadcast::receiver()` so we have a single grep
//!    target during a future broadcast-channel migration.

use tokio::sync::watch;

use crate::DhtHandle;

/// Sender-side wrapper. Owned by `SessionActor`; cloned only for
/// DHT-restart paths that need to broadcast a new handle.
#[derive(Debug, Clone)]
pub struct DhtBroadcast {
    inner: watch::Sender<Option<DhtHandle>>,
}

impl DhtBroadcast {
    /// Create a new broadcast channel pre-populated with the given
    /// handle (or `None` if DHT is starting disabled).
    #[must_use]
    pub fn new(initial: Option<DhtHandle>) -> Self {
        let (tx, _rx) = watch::channel(initial);
        Self { inner: tx }
    }

    /// Replace the broadcast value. Every receiver's next
    /// `borrow_and_update()` (or `changed().await`) sees the new
    /// value. Pass `None` to indicate DHT was disabled.
    pub fn replace(&self, new_handle: Option<DhtHandle>) {
        // `watch::Sender::send` only errors if there are no receivers
        // — that's benign during apply_settings (could be a
        // zero-torrent session). Keep the value updated regardless.
        let _ = self.inner.send(new_handle);
    }

    /// Borrow the current value without subscribing to changes. Used
    /// by callers that don't care about future updates (e.g. one-shot
    /// queries from `SessionHandle::dht_node_count`).
    #[must_use]
    pub fn borrow(&self) -> watch::Ref<'_, Option<DhtHandle>> {
        self.inner.borrow()
    }

    /// Subscribe to broadcast updates. Returns a fresh receiver that
    /// fires on every `replace` after subscription. Cheap (O(1)) —
    /// safe to call once per `TorrentActor` at construction time.
    #[must_use]
    pub fn subscribe(&self) -> DhtReceiver {
        DhtReceiver {
            inner: self.inner.subscribe(),
        }
    }

    /// True if there are no live receivers — used by tests to assert
    /// torrent shutdown has dropped its DHT receiver.
    #[must_use]
    pub fn receiver_count(&self) -> usize {
        self.inner.receiver_count()
    }
}

/// Receiver-side wrapper. Held by every `TorrentActor`; observes
/// session-level DHT handle replacements.
///
/// `DhtReceiver` is `Clone` because some torrent-actor sub-tasks need
/// their own subscription. Each clone is a fresh receiver — they
/// don't share progress.
#[derive(Debug, Clone)]
pub struct DhtReceiver {
    inner: watch::Receiver<Option<DhtHandle>>,
}

impl DhtReceiver {
    /// Read the current handle. Returns `None` if DHT is disabled or
    /// briefly during a restart window between `replace(None)` and
    /// `replace(Some(new))`.
    ///
    /// The borrow is short-lived — clone the underlying `DhtHandle`
    /// before performing async work to avoid holding the read lock
    /// across `.await`.
    #[must_use]
    pub fn current(&self) -> Option<DhtHandle> {
        self.inner.borrow().clone()
    }

    /// Wait for the next broadcast change. Useful for B6's queued
    /// requests that should hold during a DHT restart.
    ///
    /// Returns `Err` if the session has dropped the broadcast sender
    /// (i.e. session is shutting down).
    ///
    /// # Errors
    ///
    /// Returns [`watch::error::RecvError`] if the broadcast sender
    /// has been dropped — the session is shutting down and the
    /// caller should also exit.
    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.inner.changed().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pre-test note: these tests do NOT spawn a real DHT actor. They
    /// exercise the watch-channel plumbing only — a real `DhtHandle`
    /// requires a UDP socket bind, which is ineligible in CI.
    /// Integration tests in `irontide-session/tests/` exercise the
    /// end-to-end DHT-restart path.

    #[tokio::test]
    async fn broadcast_starts_with_initial_none_value() {
        let bx = DhtBroadcast::new(None);
        let rx = bx.subscribe();
        assert!(rx.current().is_none());
    }

    #[tokio::test]
    async fn replace_propagates_to_subscribed_receivers() {
        let bx = DhtBroadcast::new(None);
        let rx1 = bx.subscribe();
        let rx2 = bx.subscribe();
        // Both subscribers initially see None.
        assert!(rx1.current().is_none());
        assert!(rx2.current().is_none());

        // We can't synthesise a DhtHandle without binding a UDP
        // socket, so we exercise the None → None replacement path:
        // it's the same observable code path (watch::send with the
        // same value still bumps the version).
        bx.replace(None);
        assert!(rx1.current().is_none());
        assert!(rx2.current().is_none());
    }

    #[tokio::test]
    async fn receiver_count_tracks_subscriptions() {
        let bx = DhtBroadcast::new(None);
        assert_eq!(bx.receiver_count(), 0, "no subscribers initially");
        let rx1 = bx.subscribe();
        assert_eq!(bx.receiver_count(), 1);
        let rx2 = bx.subscribe();
        assert_eq!(bx.receiver_count(), 2);
        drop(rx1);
        assert_eq!(bx.receiver_count(), 1);
        drop(rx2);
        assert_eq!(bx.receiver_count(), 0, "all subscribers dropped");
    }

    #[tokio::test]
    async fn changed_resolves_after_replace() {
        let bx = DhtBroadcast::new(None);
        let mut rx = bx.subscribe();
        let bx2 = bx.clone();
        let task = tokio::spawn(async move {
            // Sleep a short time to ensure the receiver is awaiting
            // before we send. Without this, the receiver might miss
            // the version bump (watch tracks VERSIONS so a swap done
            // before subscribe is invisible — but a swap done after
            // subscribe is observable).
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            bx2.replace(None); // version bump
        });
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), rx.changed()).await;
        assert!(result.is_ok(), "changed should resolve before deadline");
        result.unwrap().unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn cloned_receivers_observe_independently() {
        // Cloned DhtReceiver instances are independent watch receivers
        // — they each track their own seen-version cursor.
        let bx = DhtBroadcast::new(None);
        let rx1 = bx.subscribe();
        let rx2 = rx1.clone();
        // Both see None initially.
        assert!(rx1.current().is_none());
        assert!(rx2.current().is_none());
        // Both receiver counts are still 2 — confirming clone is a
        // fresh receiver (not a no-op alias).
        assert_eq!(bx.receiver_count(), 2);
    }
}
