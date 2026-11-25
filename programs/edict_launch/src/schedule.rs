//! The `$EDICT` fee schedule and the split of a claimed chunk into its base part and its excess (owner decision
//! 2026-11-23, docs/EDICT_START.md 3.3). Pure functions; the unit tests below pin every boundary.
//!
//! Rates are LaunchLab's unit: millionths (10,000 = 1%).

/// The permanent platform fee of every Edict token on LaunchLab: 1%.
pub const BASE_RATE: u64 = 10_000;
/// The opening rate of `$EDICT`: 5% (LaunchLab's cap; 5.25% with Raydium's 0.25%).
pub const PROTOCOL_OPEN_RATE: u64 = 50_000;

/// The rate the schedule allows `elapsed` seconds after the pool was created (Clock `unix_timestamp`, whole seconds).
///
/// | since pool creation | rate |
/// |---|---|
/// | 0 to 7.5 s | 5% |
/// | 7.5 to 15 s | 4% |
/// | 15 to 22.5 s | 3% |
/// | 22.5 to 30 s | 2% |
/// | from 30 s | 1%, permanent |
///
/// The boundaries are compared in half seconds (`2 * elapsed`), so a step is never earlier than its boundary: with whole
/// seconds the 7.5 s step is allowed from second 8, the 22.5 s step from second 23. A negative `elapsed` (clock skew) is
/// treated as 0.
pub fn scheduled_rate(elapsed: i64) -> u64 {
    let half = (elapsed.max(0) as i128) * 2;
    if half < 15 {
        50_000
    } else if half < 30 {
        40_000
    } else if half < 45 {
        30_000
    } else if half < 60 {
        20_000
    } else {
        BASE_RATE
    }
}

/// The excess of a chunk claimed at `rate` over the 1% base rate: `floor(amount x (rate - 1%) / rate)`. It goes to the
/// Treasury; the rest (`amount - excess`, which carries the rounding dust) is the 1% part split normally. 0 at the base
/// rate. `rate` below the base is impossible on an Edict platform and returns 0.
pub fn excess_of(amount: u64, rate: u64) -> u64 {
    if rate <= BASE_RATE {
        return 0;
    }
    ((amount as u128) * ((rate - BASE_RATE) as u128) / (rate as u128)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_boundaries() {
        for (elapsed, rate) in [
            (-5, 50_000),
            (0, 50_000),
            (7, 50_000),
            (8, 40_000),
            (14, 40_000),
            (15, 30_000),
            (22, 30_000),
            (23, 20_000),
            (29, 20_000),
            (30, 10_000),
            (31, 10_000),
            (i64::MAX, 10_000),
        ] {
            assert_eq!(scheduled_rate(elapsed), rate, "elapsed {elapsed}");
        }
    }

    #[test]
    fn schedule_never_rises() {
        let mut last = u64::MAX;
        for e in -2..120 {
            let r = scheduled_rate(e);
            assert!(r <= last);
            last = r;
        }
    }

    #[test]
    fn excess_parts() {
        assert_eq!(excess_of(1_000_000, 50_000), 800_000);
        assert_eq!(excess_of(1_000_000, 40_000), 750_000);
        assert_eq!(excess_of(1_000_000, 30_000), 666_666);
        assert_eq!(excess_of(1_000_000, 20_000), 500_000);
        assert_eq!(excess_of(1_000_000, 10_000), 0);
        assert_eq!(excess_of(0, 50_000), 0);
        assert_eq!(excess_of(7, 30_000), 4);
        assert_eq!(excess_of(u64::MAX, 50_000), (u64::MAX as u128 * 4 / 5) as u64);
        assert_eq!(excess_of(5, 9_999), 0);
    }
}
