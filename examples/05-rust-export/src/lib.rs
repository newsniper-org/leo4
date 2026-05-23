//! examples/05-rust-export — end-to-end demo of the Phase 9
//! reverse-direction pipeline.
//!
//! The four `#[leo4::export]` functions below cover the surface
//! Phase 9-1..9-6 wired up:
//!
//! - `is_prime(n: u64) -> bool`     — scalar in / scalar out
//! - `next_prime(n: u64) -> u64`    — same shape, different return
//! - `count_primes_below(n: u64) -> u64`
//!                                  — exercises a loop on the
//!                                    Rust side (stand-in for an
//!                                    SMT solver's main inner loop)
//! - `factor_smallest(n: u64) -> Option<u64>`
//!                                  — exercises `Option<T>` in the
//!                                    return signature
//!
//! Build + run workflow lives in `README.md`. The TL;DR is the
//! 4-step manual sequence documented in `SPEC/reverse-direction.md`
//! §7 — Lake-plugin auto-discovery of the glue shim is a 9-6
//! follow-up, so today the user invokes `leo4-rust-emit` and
//! `leanc -c shim/leo4_rust_bridge_lean.c` by hand.
//!
//! None of the functions are meant to be fast or production-grade;
//! they're picked so the demo's mangle table covers `u64`, `bool`,
//! and `option<u64>` and so the runtime exercise has a believable
//! "let Rust do the heavy compute, hand the answer back to Lean"
//! shape.

/// Trial-division primality test.
#[leo4::export]
pub fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    if n < 4 {
        return true;
    }
    if n % 2 == 0 {
        return false;
    }
    let mut d: u64 = 3;
    while d.saturating_mul(d) <= n {
        if n % d == 0 {
            return false;
        }
        d += 2;
    }
    true
}

/// Smallest prime strictly greater than `n`. Loops via
/// [`is_prime`] — illustrates calling another `#[leo4::export]`
/// function from inside one (no recursion-across-boundary; the
/// inner call stays in Rust).
#[leo4::export]
pub fn next_prime(n: u64) -> u64 {
    let mut candidate = n.saturating_add(1);
    loop {
        if is_prime(candidate) {
            return candidate;
        }
        candidate = match candidate.checked_add(1) {
            Some(v) => v,
            None => return 0, // overflow sentinel
        };
    }
}

/// Count of primes < `n`. Quadratic for clarity; in a real
/// solver this would be a sieve. Demonstrates a long-running
/// inner loop so the dispatcher / worker IPC overhead is
/// dwarfed by the actual compute.
#[leo4::export]
pub fn count_primes_below(n: u64) -> u64 {
    let mut count: u64 = 0;
    let mut k: u64 = 2;
    while k < n {
        if is_prime(k) {
            count += 1;
        }
        k += 1;
    }
    count
}

/// Smallest prime factor of `n`, or `None` for `n < 2`.
/// Exercises `Option<u64>` in the return position.
#[leo4::export]
pub fn factor_smallest(n: u64) -> Option<u64> {
    if n < 2 {
        return None;
    }
    if n % 2 == 0 {
        return Some(2);
    }
    let mut d: u64 = 3;
    while d.saturating_mul(d) <= n {
        if n % d == 0 {
            return Some(d);
        }
        d += 2;
    }
    Some(n) // n itself is prime
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primality_table_matches_handful() {
        assert!(!is_prime(0));
        assert!(!is_prime(1));
        assert!(is_prime(2));
        assert!(is_prime(3));
        assert!(!is_prime(4));
        assert!(is_prime(97));
        assert!(!is_prime(100));
        assert!(is_prime(7919)); // 1000th prime
    }

    #[test]
    fn next_prime_walks_forward() {
        assert_eq!(next_prime(1), 2);
        assert_eq!(next_prime(2), 3);
        assert_eq!(next_prime(10), 11);
        assert_eq!(next_prime(13), 17);
    }

    #[test]
    fn count_primes_below_100_is_25() {
        assert_eq!(count_primes_below(100), 25);
    }

    #[test]
    fn factor_smallest_picks_the_smallest() {
        assert_eq!(factor_smallest(0), None);
        assert_eq!(factor_smallest(1), None);
        assert_eq!(factor_smallest(2), Some(2));
        assert_eq!(factor_smallest(15), Some(3));
        assert_eq!(factor_smallest(49), Some(7));
        assert_eq!(factor_smallest(97), Some(97));
    }
}
