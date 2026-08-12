//! Loom model of the connection-admission concurrency (M262).
//!
//! The whole file is gated on `#![cfg(loom)]`, so under the normal build it
//! compiles to an empty test crate (0 tests) and `loom` is not even a
//! dependency. Run the model checker with:
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test -p irontide-core --test loom_admit
//! ```
//!
//! Production wires two `SeqCst` atomics into the admit gate
//! (`max_connections_global: AtomicI32`, `live_count: AtomicUsize`) plus the
//! `LiveConnectionGuard` RAII counter. Per the M262 plan's Phase-3 review
//! (gemini D1), the highest-risk primitive — the RAII guard — is exercised as
//! *real* production code here: under `--cfg loom`, `LiveConnectionGuard`
//! aliases its `Arc`/`AtomicUsize` through `gaia_core::loom_compat` to
//! loom's instrumented types, so tests 1 and 2 drive the genuine
//! `LiveConnectionGuard::new`/`Drop` and the genuine `admit_permitted`
//! decision. Only the cross-crate cap store/load sequence (test 3) is a
//! faithful model, since `max_connections_global` lives in higher crates.
#![cfg(loom)]

use gaia_core::{LiveConnectionGuard, admit_permitted};
use loom::sync::Arc;
use loom::sync::atomic::{AtomicI32, AtomicUsize, Ordering::SeqCst};

/// Edge 1 — the RAII live-count always balances back to 0 across every
/// interleaving of two concurrent admit/drop pairs. A lost decrement or a
/// `fetch_sub` underflow inside the real `LiveConnectionGuard` would fail here.
#[test]
fn live_count_raii_balances_to_zero() {
    loom::model(|| {
        let live = Arc::new(AtomicUsize::new(0));

        let l_thread = Arc::clone(&live);
        let handle = loom::thread::spawn(move || {
            let _g = LiveConnectionGuard::new(l_thread);
            // guard drops at scope end -> fetch_sub
        });

        {
            let _g = LiveConnectionGuard::new(Arc::clone(&live));
            // guard drops at block end -> fetch_sub
        }

        handle.join().unwrap();
        assert_eq!(live.load(SeqCst), 0, "every guard must decrement on drop");
    });
}

/// Edge 2 — soft-cap TOCTOU. Two admitters race a cap of 1 through the *real*
/// `admit_permitted` + `LiveConnectionGuard`. The check-then-admit gap means
/// both can observe `live == 0` and admit (the documented, accepted soft-cap
/// overshoot), but the overshoot is bounded by the admitter count and the
/// counter must still drain to 0 — no lost decrement, no underflow.
#[test]
fn soft_cap_overshoot_bounded_and_drains() {
    loom::model(|| {
        let max = Arc::new(AtomicI32::new(1)); // cap = 1
        let live = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let max = Arc::clone(&max);
            let live = Arc::clone(&live);
            handles.push(loom::thread::spawn(move || {
                let m = max.load(SeqCst);
                let l = live.load(SeqCst);
                if admit_permitted(m, l) {
                    let _g = LiveConnectionGuard::new(Arc::clone(&live));
                    // Even with both admitters racing, no more than 2 guards
                    // (the admitter count) can ever coexist.
                    assert!(
                        live.load(SeqCst) <= 2,
                        "overshoot must be bounded by the admitter count",
                    );
                    // guard drops here
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(live.load(SeqCst), 0, "all admitted guards must drain to 0");
    });
}

/// Edge 3 — a cap reconfigure (`setPreferences`) concurrent with an admit read
/// is SeqCst-linearizable: the reader observes either the old cap or the new
/// cap, never a torn/intermediate value, and its decision matches
/// `admit_permitted` evaluated on whichever coherent cap it saw.
#[test]
fn reconfigure_during_admit_is_linearizable() {
    loom::model(|| {
        let max = Arc::new(AtomicI32::new(1));
        let live = Arc::new(AtomicUsize::new(1)); // one connection already live

        let m_writer = Arc::clone(&max);
        let writer = loom::thread::spawn(move || {
            // operator raises the cap 1 -> 2
            m_writer.store(2, SeqCst);
        });

        let m_reader = Arc::clone(&max);
        let l_reader = Arc::clone(&live);
        let reader = loom::thread::spawn(move || {
            let m = m_reader.load(SeqCst);
            let l = l_reader.load(SeqCst);
            let decision = admit_permitted(m, l);
            // With live == 1: cap 1 rejects, cap 2 admits. The observed cap is
            // exactly one of the two committed values (never torn), and the
            // decision is the pure function of it.
            assert!(m == 1 || m == 2, "cap read was torn: {m}");
            assert_eq!(decision, admit_permitted(m, 1));
        });

        writer.join().unwrap();
        reader.join().unwrap();
        assert_eq!(max.load(SeqCst), 2);
    });
}
