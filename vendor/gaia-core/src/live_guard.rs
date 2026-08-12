// M262: aliased through `loom_compat` so `--cfg loom` swaps in loom's
// instrumented `Arc`/`AtomicUsize` and the loom tests exercise this *real*
// RAII guard. Under the normal build these resolve to the identical `std`
// types, so the public API is byte-for-byte unchanged.
use crate::loom_compat::{Arc, AtomicUsize, Ordering};

/// M224 D3: RAII counter increment for the global connection cap. Wraps
/// `Arc<AtomicUsize>` so the live count cannot leak even if the listener/admit
/// loop panics or drops a connection mid-pipeline.
/// dropped automatically when the owning connection is dropped or forwarded.
#[derive(Debug)]
pub struct LiveConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl LiveConnectionGuard {
    /// Create a new guard and increment the backing counter atomically.
    pub fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self { counter }
    }
}

impl Drop for LiveConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}
