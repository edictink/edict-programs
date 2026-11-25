use anchor_lang::prelude::*;

/// `open_platform`: the token's platform exists and is open for its one launch.
#[event]
pub struct TokenOpened {
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub platform_config: Pubkey,
    pub fee_rate: u64,
    pub protocol: bool,
}

/// `seal_platform`: the pool exists, the allow-config is closed (no other pool can ever use this platform).
#[event]
pub struct TokenSealed {
    pub mint: Pubkey,
    pub pool: Pubkey,
    pub created_at: i64,
}

/// A claim forwarded to the vault. `quote` lamports of wSOL reached the vault's wSOL account (and `base` units its base
/// account, Fee Key only). `excess` is the part of `quote` above the 1% base rate (0 except for `$EDICT` before its
/// schedule is final); `rate` is the single platform fee rate the claimed chunk was earned at.
#[event]
pub struct FeeClaimed {
    pub mint: Pubkey,
    pub source: u8,
    pub quote: u64,
    pub base: u64,
    pub rate: u64,
    pub excess: u64,
    pub seq: u64,
}

/// `fee_step`: the platform fee rate went from `from` to `to`, `elapsed` seconds after the pool was created.
#[event]
pub struct FeeStepped {
    pub mint: Pubkey,
    pub from: u64,
    pub to: u64,
    pub elapsed: i64,
    pub at: i64,
}
