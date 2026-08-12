//! Atomic + `Arc` aliases that swap to loom's instrumented primitives under
//! `--cfg loom` (M262).
//!
//! Under the normal build these are byte-identical to the `std` types, so
//! production callers (`LiveConnectionGuard`) compile completely unchanged.
//! Under `RUSTFLAGS="--cfg loom" cargo test -p irontide-core` they become
//! loom's model-checked equivalents, which lets the loom tests in
//! `tests/loom_admit.rs` drive the *real* `LiveConnectionGuard` RAII path
//! rather than a hand-rolled copy. `cargo test -p irontide-core` under
//! `--cfg loom` only compiles `irontide-core` and its dependencies — never the
//! dependents (`irontide-utp` / `irontide-session-core`) that construct the
//! guard with `std::sync` types — so there is no cross-crate type mismatch.

#[cfg(loom)]
pub(crate) use loom::sync::Arc;
#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicUsize, Ordering};

#[cfg(not(loom))]
pub(crate) use std::sync::Arc;
#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicUsize, Ordering};
