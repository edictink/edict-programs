use anchor_lang::prelude::*;

pub const CONFIG_SEED: &[u8] = b"config";
pub const TOKEN_SEED: &[u8] = b"token";
/// `auth_t`: the system-owned PDA without data that plays every LaunchLab role of one token (platform admin, fee wallet,
/// Fee Key holder, CPMM creator). Seeds `["auth", mint]`.
pub const AUTH_SEED: &[u8] = b"auth";

pub const STATUS_OPEN: u8 = 1;
pub const STATUS_LIVE: u8 = 2;

/// The fee sources of `FeeClaimed`.
pub const SOURCE_PLATFORM: u8 = 0;
pub const SOURCE_CPMM_CREATOR: u8 = 1;
pub const SOURCE_FEE_KEY: u8 = 2;

/// Set once by the upgrade authority. Every foreign program and account the instructions use is pinned here.
#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    pub launchlab: Pubkey,
    pub cpmm: Pubkey,
    pub lock: Pubkey,
    /// The LaunchLab global config (SOL quote) every Edict token launches on.
    pub global_config: Pubkey,
    /// The CPMM config of the graduated pool (mainnet index 22).
    pub cpswap_config: Pubkey,
    /// `$EDICT`'s mint (a fresh keypair made before the launch), or the default key while it is not set. A token with
    /// this mint opens at 5% and follows the fee schedule.
    pub protocol_mint: Pubkey,
    pub bump: u8,
}

/// One per token, seeds `["token", mint]`.
#[account]
#[derive(InitSpace)]
pub struct TokenState {
    pub mint: Pubkey,
    /// The token's Edict vault (the keeper's per-coin vault). Every claimed quote amount is forwarded to its wSOL
    /// account, every Fee Key amount is paid into its token accounts.
    pub vault: Pubkey,
    pub platform_config: Pubkey,
    /// The LaunchLab pool, set by `seal_platform`.
    pub pool: Pubkey,
    pub status: u8,
    /// 1 for `$EDICT` (the fee schedule applies), 0 otherwise.
    pub protocol: u8,
    pub auth_bump: u8,
    pub bump: u8,
    /// The platform fee rate in force (LaunchLab unit, 10,000 = 1%). Changed only by `fee_step`, after a claim.
    pub fee_rate: u64,
    /// `unix_timestamp` of the transaction that created the pool (`seal_platform` runs in it).
    pub created_at: i64,
    /// Totals forwarded to the vault, per source (quote lamports; the Fee Key base side in base units).
    pub platform_quote: u64,
    pub excess_quote: u64,
    pub cpmm_creator_quote: u64,
    pub fee_key_quote: u64,
    pub fee_key_base: u64,
    /// Sequence number of the last `FeeClaimed` event of this token.
    pub seq: u64,
}
