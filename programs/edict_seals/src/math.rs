//! Pure arithmetic of `edict_seals`: the price schedule and the number bitmaps. No account, no syscall, so every
//! function is unit-tested on the host (`cargo test -p edict_seals`).

use crate::constants::{
    FACTOR_DENOMINATOR, MASK_BYTES, MAX_SUPPLY, PUBLIC_MAX, TEAM_COUNT, TIERS, TIER_SIZE,
};

// ---------------------------------------------------------------------------------------------------------------------
// price schedule (spec 8, "Price step")
// ---------------------------------------------------------------------------------------------------------------------

/// The burn amount of tiers 0 to 2: `tiers[0] = base`, `tiers[k] = tiers[k - 1] x factor_bps / 10,000` rounded down at
/// every step (`u128`). A closed form `base x 1.1^k` would overflow `u128`. `None` on overflow of `u64`.
pub fn burn_tiers(base_lamports: u64, factor_bps: u32) -> Option<[u64; TIERS]> {
    let mut out = [0u64; TIERS];
    out[0] = base_lamports;
    for k in 1..TIERS {
        let next = u128::from(out[k - 1])
            .checked_mul(u128::from(factor_bps))?
            .checked_div(FACTOR_DENOMINATOR)?;
        out[k] = u64::try_from(next).ok()?;
    }
    Some(out)
}

/// Whether every amount of the tiers `from_tier..` lies in `[min, max]` (a change only touches tiers above the one being
/// minted; the tiers below keep the amounts they were minted at).
pub fn tiers_within_bounds(tiers: &[u64; TIERS], from_tier: usize, min: u64, max: u64) -> bool {
    tiers
        .iter()
        .skip(from_tier)
        .all(|amount| *amount >= min && *amount <= max)
}

/// The tier of the next public mint: `floor(public mints so far / 100)`, 0-based. Team mints never count.
pub fn tier_of(public_minted: u16) -> usize {
    usize::from(public_minted / TIER_SIZE)
}

/// The schedule after a queued change is applied while `public_minted` public mints exist: the tiers up to and including
/// the current one keep their amounts, the tiers above take the new ones.
pub fn apply_schedule(
    current: &[u64; TIERS],
    new: &[u64; TIERS],
    public_minted: u16,
) -> [u64; TIERS] {
    let current_tier = tier_of(public_minted);
    let mut out = *current;
    let from = (current_tier + 1).min(TIERS);
    out[from..].copy_from_slice(&new[from..]);
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// the number bitmaps: 256 bits for 250 numbers, bit `number - 1`
// ---------------------------------------------------------------------------------------------------------------------

fn position(number: u16) -> Option<(usize, u8)> {
    if number == 0 || number > MAX_SUPPLY {
        return None;
    }
    let bit = usize::from(number - 1);
    Some((bit / 8, 1u8 << (bit % 8)))
}

/// Whether `number` (1 to 250) is set. Out of range is `false`.
pub fn bit_get(mask: &[u8; MASK_BYTES], number: u16) -> bool {
    match position(number) {
        Some((byte, bit)) => mask[byte] & bit != 0,
        None => false,
    }
}

/// Sets `number`. Returns `false` (and changes nothing) when it is out of range.
pub fn bit_set(mask: &mut [u8; MASK_BYTES], number: u16) -> bool {
    match position(number) {
        Some((byte, bit)) => {
            mask[byte] |= bit;
            true
        }
        None => false,
    }
}

/// The number of set bits.
pub fn bit_count(mask: &[u8; MASK_BYTES]) -> u32 {
    mask.iter().map(|b| b.count_ones()).sum()
}

/// The bitmap of the team numbers: exactly 30 distinct numbers from #1 to #250 (the public draw from all 250, owner decision
/// 2026-11-23), else `None`.
pub fn team_mask_from(numbers: &[u16]) -> Option<[u8; MASK_BYTES]> {
    if numbers.len() != usize::from(TEAM_COUNT) {
        return None;
    }
    let mut mask = [0u8; MASK_BYTES];
    for n in numbers {
        if bit_get(&mask, *n) || !bit_set(&mut mask, *n) {
            return None;
        }
    }
    (bit_count(&mask) == u32::from(TEAM_COUNT)).then_some(mask)
}

/// The lowest number from `from` (at least 1) that is neither a team number nor already minted: the number of the next
/// public mint. `None` when none is left up to 250.
pub fn next_public_number(
    team: &[u8; MASK_BYTES],
    minted: &[u8; MASK_BYTES],
    from: u16,
) -> Option<u16> {
    let mut n = from.max(1);
    while n <= MAX_SUPPLY {
        if !bit_get(team, n) && !bit_get(minted, n) {
            return Some(n);
        }
        n = n.checked_add(1)?;
    }
    None
}

/// Whether a public mint is still possible by count (`public_minted < 220`).
pub fn public_left(public_minted: u16) -> bool {
    public_minted < PUBLIC_MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_of_the_spec() {
        // base 0.5 SOL, factor 1.1, floor at every step (design section 5.7): three tiers for 220 public mints
        let t = burn_tiers(500_000_000, 11_000).unwrap();
        assert_eq!(t, [500_000_000, 550_000_000, 605_000_000]);
        assert!(tiers_within_bounds(&t, 0, 100_000_000, 2_000_000_000));
    }

    #[test]
    fn tiers_overflow_is_none() {
        assert!(burn_tiers(u64::MAX, 20_000).is_none());
        assert!(burn_tiers(u64::MAX / 2, 30_000).is_none());
    }

    #[test]
    fn tier_counts_public_mints_only() {
        assert_eq!(tier_of(0), 0);
        assert_eq!(tier_of(99), 0);
        assert_eq!(tier_of(100), 1);
        assert_eq!(tier_of(199), 1);
        assert_eq!(tier_of(200), 2);
        assert_eq!(tier_of(PUBLIC_MAX - 1), TIERS - 1);
    }

    #[test]
    fn schedule_change_never_touches_the_current_tier() {
        let current = burn_tiers(500_000_000, 11_000).unwrap();
        let new = burn_tiers(600_000_000, 12_000).unwrap();
        // 150 public mints: tier 1 is being minted, tier 2 changes
        let out = apply_schedule(&current, &new, 150);
        assert_eq!(out[0], current[0]);
        assert_eq!(out[1], current[1]);
        assert_eq!(out[2], new[2]);
        // at the very start only tier 0 is kept
        let out = apply_schedule(&current, &new, 0);
        assert_eq!(out[0], current[0]);
        assert_eq!(out[1], new[1]);
        // in the last tier nothing changes
        assert_eq!(apply_schedule(&current, &new, PUBLIC_MAX - 1), current);
    }

    #[test]
    fn bounds_apply_from_the_tier() {
        let t = burn_tiers(500_000_000, 11_000).unwrap();
        // a max of 0.6 SOL fails from tier 0 (tier 2 is 0.605) but passes when only tiers 0 and 1 are checked
        assert!(!tiers_within_bounds(&t, 0, 100_000_000, 600_000_000));
        assert!(tiers_within_bounds(&t, 0, 100_000_000, 605_000_000));
        assert!(!tiers_within_bounds(&t, 2, 100_000_000, 600_000_000));
        // nothing above the last tier: empty range passes
        assert!(tiers_within_bounds(&t, TIERS, 0, 0));
    }

    #[test]
    fn bitmaps() {
        let mut m = [0u8; MASK_BYTES];
        assert!(!bit_get(&m, 1));
        assert!(bit_set(&mut m, 1));
        assert!(bit_set(&mut m, 250));
        assert!(bit_get(&m, 1) && bit_get(&m, 250));
        assert_eq!(bit_count(&m), 2);
        assert!(!bit_set(&mut m, 0));
        assert!(!bit_set(&mut m, 251));
        assert!(!bit_set(&mut m, 256));
        assert!(!bit_get(&m, 0) && !bit_get(&m, 251));
        // #1 is the first bit; #250 is bit 1 of byte 31 (bits 250..=255 of the last byte are never used)
        assert_eq!(m[0], 1);
        assert_eq!(m[MASK_BYTES - 1], 0x02);
    }

    /// A team of the draw: 30 numbers spread over 1..=250, #1 and #250 among them.
    fn team() -> [u16; 30] {
        let mut v = [0u16; 30];
        for (i, n) in v.iter_mut().enumerate() {
            *n = 1 + 8 * i as u16;
        }
        v[29] = 250;
        v
    }

    #[test]
    fn team_mask_needs_30_distinct_numbers_from_1_to_250() {
        let t = team();
        let mask = team_mask_from(&t).unwrap();
        assert_eq!(bit_count(&mask), 30);
        assert!(team_mask_from(&t[..29]).is_none());
        let mut dup = t;
        dup[29] = dup[0];
        assert!(team_mask_from(&dup).is_none());
        for bad in [0u16, 251, 1000] {
            let mut v = t;
            v[3] = bad;
            assert!(team_mask_from(&v).is_none(), "#{bad} accepted");
        }
        // any 30 distinct numbers of 1..=250 are fine (the draw decides them), the last 30 included
        let last: Vec<u16> = (221..=250).collect();
        assert!(team_mask_from(&last).is_some());
    }

    #[test]
    fn public_numbers_skip_team_numbers_and_never_repeat() {
        let team_mask = team_mask_from(&team()).unwrap();
        let mut minted = team_mask; // the 30 team Seals exist before the first public mint
        let mut cursor = 1u16;
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..PUBLIC_MAX {
            let n = next_public_number(&team_mask, &minted, cursor).unwrap();
            assert!(
                !bit_get(&team_mask, n),
                "a team number went to a public mint: {n}"
            );
            assert!(seen.insert(n), "number {n} repeated");
            assert!(bit_set(&mut minted, n));
            cursor = n + 1;
        }
        // #1 is a team number here, so the first public number is #2
        assert!(!seen.contains(&1) && seen.contains(&2));
        // the 220 public numbers plus the 30 team numbers are exactly 1..=250
        assert_eq!(seen.len() + 30, 250);
        assert_eq!(bit_count(&minted), 250);
        assert!(next_public_number(&team_mask, &minted, cursor).is_none());
    }

    #[test]
    fn the_cursor_is_only_a_hint() {
        let team_mask = team_mask_from(&team()).unwrap();
        let minted = team_mask;
        // #1, #9, #17, ... are team numbers: from a cursor at or before a free number the lowest free one is found
        assert_eq!(next_public_number(&team_mask, &minted, 0), Some(2));
        assert_eq!(next_public_number(&team_mask, &minted, 1), Some(2));
        assert_eq!(next_public_number(&team_mask, &minted, 9), Some(10));
        assert_eq!(next_public_number(&team_mask, &minted, 249), Some(249));
        assert_eq!(next_public_number(&team_mask, &minted, 250), None);
    }

    #[test]
    fn public_left_counts() {
        assert!(public_left(0));
        assert!(public_left(219));
        assert!(!public_left(220));
    }
}
