//! Constants of `edict_seals` (docs/validation/P2-01_SEAL_DESIGN.md section 3.1: "Constants in code, not in `Config`").

use anchor_lang::prelude::*;

// ---- PDA seeds ---------------------------------------------------------------------------------------------------
pub const CONFIG_SEED: &[u8] = b"config";
pub const STATE_SEED: &[u8] = b"state";
pub const PENDING_SEED: &[u8] = b"pending";
/// System-owned, no data: the Seal pot lamports and the royalty receiver (60%).
pub const POT_SEED: &[u8] = b"pot";
/// System-owned, no data: owns exactly two token accounts and holds nothing between transactions (P2-00).
pub const SWAPPER_SEED: &[u8] = b"swapper";
/// System-owned, no data: the update authority of the Core collection.
pub const AUTHORITY_SEED: &[u8] = b"authority";
pub const SEAL_SEED: &[u8] = b"seal";
/// System-owned, no data: the authority of a Seal's prize token accounts and Core prizes.
pub const VAULT_SEED: &[u8] = b"vault";

// ---- supply and numbering (EDICT_START 3.4: 250 Seals; owner decision 2026-11-23: numbering as before, the team draw from all 250)
pub const MAX_SUPPLY: u16 = 250;
pub const TEAM_COUNT: u8 = 30;
pub const PUBLIC_MAX: u16 = 220;
/// A price tier is 100 PUBLIC mints; team mints never count (owner decision 2026-11-02). 220 public mints: tiers 0, 1, 2.
pub const TIER_SIZE: u16 = 100;
pub const TIERS: usize = 3;
/// A 256-bit bitmap: bit `number - 1` (numbers 1 to 250).
pub const MASK_BYTES: usize = 32;
/// The most numbers one `mint_reserve` call mints (transaction size, design section 7).
pub const MAX_RESERVE_PER_CALL: usize = 5;
/// Public mints that take part in genesis (with the 30 team Seals).
pub const GENESIS_PUBLIC_MINTS: u16 = 100;

// ---- money ------------------------------------------------------------------------------------------------------
/// The fixed payment of a public mint is split 60% to the pot and 40% to the Treasury (spec 8); the amount itself is set at
/// `init_config` (G-NFT-PRICE) inside these bounds and never changes afterwards.
pub const PAYMENT_POT_PERCENT: u64 = 60;
pub const MIN_PAYMENT_LAMPORTS: u64 = 10_000_000;
pub const MAX_PAYMENT_LAMPORTS: u64 = 10_000_000_000;
/// The accumulator scale: `acc_per_seal` and `owed_scaled` are lamports x 10^12.
pub const SCALE: u128 = 1_000_000_000_000;
/// Basis points of the factor: 11,000 = 1.1.
pub const FACTOR_DENOMINATOR: u128 = 10_000;

/// The burn amount of a tier lies in this range and so do the bounds `init_config` accepts (SECURITY: 0.1 to 2.0 SOL).
pub const MIN_BURN_FLOOR: u64 = 100_000_000;
pub const MAX_BURN_CEIL: u64 = 2_000_000_000;

/// What Metaplex Core leaves of an asset after `BurnV1` (mpl-core 0.12.1 and the cloned binary, `forfeit.test.ts`): an account
/// still owned by Core, resized to ONE byte, whose value is `Key::Uninitialized` (the first variant of `Key`, so 0). It keeps its
/// rent, so it never disappears and its address cannot be created again.
pub const CORE_KEY_UNINITIALIZED: u8 = 0;

// ---- royalty (owner decision 2026-10-28: 5%, 3% pot / 2% Treasury) ------------------------------------------------
pub const ROYALTY_BPS: u16 = 500;
pub const ROYALTY_POT_PERCENT: u8 = 60;
pub const ROYALTY_TREASURY_PERCENT: u8 = 40;
pub const MAX_RULES: usize = 8;

// ---- time -------------------------------------------------------------------------------------------------------
/// The timelock of a queued change (DEFAULT 24 h): the same constant in every build (design D16).
pub const TIMELOCK_SECS: i64 = 86_400;
/// The genesis window after the opening (24 h).
pub const GENESIS_WINDOW_SECS: i64 = 86_400;

// ---- metadata ---------------------------------------------------------------------------------------------------
pub const MAX_URI_BYTES: usize = 96;
pub const MIN_URI_BYTES: usize = 8;
pub const COLLECTION_NAME: &str = "Seals";

// ---- venues of $EDICT (EDICT_START 3.3: Raydium LaunchLab, then CPMM after the graduation) --------------------------
// The program ids differ between mainnet and devnet, so `init_config` takes them and pins them in `Config` with every
// account the swap needs (derived or read from the pool there); a mint compares keys only, nothing is taken from the caller.
/// `SealMinted.venue`: 0 = LaunchLab curve, 1 = CPMM pool.
pub const VENUE_CURVE: u8 = 0;
pub const VENUE_CPMM: u8 = 1;
/// Wrapped SOL, the quote asset of both venues for $EDICT.
pub const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");

/// LaunchLab seeds (docs/validation/LAUNCHLAB.md section 2, packages/adapters/raydium-launchlab/src/pda.ts).
pub const LL_AUTH_SEED: &[u8] = b"vault_auth_seed";
pub const LL_POOL_SEED: &[u8] = b"pool";
pub const LL_POOL_VAULT_SEED: &[u8] = b"pool_vault";
pub const EVENT_AUTHORITY_SEED: &[u8] = b"__event_authority";
/// Anchor discriminators: account `PoolState` (LaunchLab and CPMM share the name, so the hash) and `GlobalConfig`.
pub const ACC_POOL_STATE: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const ACC_GLOBAL_CONFIG: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];
/// `buy_exact_in(amount_in, minimum_amount_out, share_fee_rate)` of LaunchLab: sha256("global:buy_exact_in")[..8].
pub const LL_BUY_EXACT_IN: [u8; 8] = [250, 234, 13, 123, 213, 156, 19, 236];
/// LaunchLab `PoolState`: 429 bytes, borsh (packages/adapters/raydium-launchlab/src/decode.ts).
pub const LL_POOL_LEN: usize = 429;
pub const LL_POOL_STATUS: usize = 17;
pub const LL_POOL_GLOBAL_CONFIG: usize = 141;
pub const LL_POOL_PLATFORM_CONFIG: usize = 173;
pub const LL_POOL_BASE_MINT: usize = 205;
pub const LL_POOL_QUOTE_MINT: usize = 237;
pub const LL_POOL_BASE_VAULT: usize = 269;
pub const LL_POOL_QUOTE_VAULT: usize = 301;
pub const LL_POOL_CREATOR: usize = 333;
/// Pool status: 0 trading on the curve, 1 raise complete (Raydium migrates it), 2 migrated to CPMM.
pub const LL_STATUS_TRADING: u8 = 0;
pub const LL_STATUS_MIGRATED: u8 = 2;
/// LaunchLab `PlatformConfig`: 944 bytes; `cpswap_config` (the CPMM config of the graduated pool) at 688.
pub const ACC_PLATFORM_CONFIG: [u8; 8] = [160, 78, 128, 0, 248, 83, 230, 160];
pub const LL_PLATFORM_CONFIG_LEN: usize = 944;
pub const LL_PLATFORM_CPSWAP_CONFIG: usize = 688;
/// `GlobalConfig.quote_mint` (edict_launch reads the same offset).
pub const LL_GLOBAL_CONFIG_QUOTE_MINT: usize = 83;

/// CPMM seeds (raydium-cp-swap `states/pool.rs`, `states/oracle.rs`, `lib.rs` AUTH_SEED).
pub const CPMM_AUTH_SEED: &[u8] = b"vault_and_lp_mint_auth_seed";
pub const CPMM_POOL_SEED: &[u8] = b"pool";
pub const CPMM_POOL_VAULT_SEED: &[u8] = b"pool_vault";
pub const CPMM_OBSERVATION_SEED: &[u8] = b"observation";
/// `swap_base_input(amount_in, minimum_amount_out)`: sha256("global:swap_base_input")[..8] (the SDK's
/// `makeSwapCpmmBaseInInstruction`, raydium-sdk-v2 0.2.85-alpha: 13 accounts, docs/validation/SEALS_VENUE.md).
pub const CPMM_SWAP_BASE_INPUT: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222];
/// The CPMM `AmmConfig` account: 236 bytes (edict_launch checks the same).
pub const CPMM_CONFIG_LEN: usize = 236;
