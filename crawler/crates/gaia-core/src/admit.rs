//! The global connection-cap admit decision, shared by the TCP listener and
//! the uTP socket actor (M262 — extracted from duplicated inline logic so the
//! loom model and the two production sites share one source of truth).

/// Whether a new inbound connection may be admitted given the global cap.
///
/// `max` is the qBt-parity cap: `-1` (and the degenerate `0`) means
/// "unlimited" and always admits. For `max >= 1`, admit iff `live < max`.
/// The `max >= 1` guard makes the `as usize` cast safe (no `i32::MIN`
/// wrap-around).
///
/// This is exactly equivalent to the pre-M262 inline logic at both sites,
/// which rejected iff `max >= 1 && live >= max as usize`:
/// `!(max >= 1 && live >= cap)` == `(max < 1) || (live < cap)`.
#[must_use]
#[allow(clippy::cast_sign_loss)] // guarded by `max >= 1`
pub fn admit_permitted(max: i32, live: usize) -> bool {
    if max < 1 {
        return true;
    }
    live < max as usize
}

#[cfg(test)]
mod tests {
    use super::admit_permitted;

    #[test]
    fn unlimited_sentinels_always_admit() {
        // -1 is the qBt "unlimited" sentinel; 0 is the degenerate-but-unlimited case.
        for max in [-1, 0] {
            for live in [0usize, 1, 100, usize::MAX] {
                assert!(admit_permitted(max, live), "max={max} live={live}");
            }
        }
    }

    #[test]
    fn cap_one_admits_only_when_empty() {
        assert!(admit_permitted(1, 0));
        assert!(!admit_permitted(1, 1));
        assert!(!admit_permitted(1, 2));
    }

    #[test]
    fn equivalent_to_legacy_reject_predicate() {
        // Legacy sites rejected iff `max >= 1 && live >= max as usize`.
        for max in [-1i32, 0, 1, 5, 128] {
            for live in [0usize, 1, 4, 5, 6, 127, 128, 129] {
                #[allow(clippy::cast_sign_loss)]
                let legacy_reject = max >= 1 && live >= max as usize;
                assert_eq!(
                    admit_permitted(max, live),
                    !legacy_reject,
                    "divergence at max={max} live={live}"
                );
            }
        }
    }

    #[test]
    fn i32_min_does_not_panic_or_wrap() {
        // The `max < 1` early-return guards the `as usize` cast.
        assert!(admit_permitted(i32::MIN, usize::MAX));
    }
}
