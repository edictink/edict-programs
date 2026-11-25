//! Events of `edict_seals`: one per state change, for the indexer and the Transparency page. Emitted with `emit_cpi!`
//! (an inner instruction of the program itself), so a long log of chatty swap CPIs cannot truncate them (design D17).
//! Amounts are lamports unless a field name says otherwise.

use anchor_lang::prelude::*;

use crate::constants::TIERS;

#[event]
pub struct ConfigInitialized {
    pub admin: Pubkey,
    pub reserve_signer: Pubkey,
    pub reserve_recipient: Pubkey,
    pub treasury: Pubkey,
    pub buyback_wallet: Pubkey,
    pub edict_mint: Pubkey,
    pub ll_pool: Pubkey,
    pub cpmm_pool: Pubkey,
    pub payment_lamports: u64,
    pub base_lamports: u64,
    pub factor_bps: u32,
    pub burn_by_tier: [u64; TIERS],
}

#[event]
pub struct CollectionCreated {
    pub collection: Pubkey,
    pub royalty_bps: u16,
    pub pot_creator: Pubkey,
    pub treasury_creator: Pubkey,
    pub rule_set_len: u8,
}

#[event]
pub struct ArtSet {
    pub base_uri: String,
    pub art_hash: [u8; 32],
}

#[event]
pub struct TeamNumbersSet {
    pub count: u8,
}

#[event]
pub struct MintOpened {
    pub opened_at: i64,
    pub genesis_locked: u64,
}

/// The 30th team Seal exists: public mints are possible and the 24 hour genesis window starts (review R3).
#[event]
pub struct PublicMintsOpened {
    pub public_open_at: i64,
    pub genesis_window_ends: i64,
}

#[event]
pub struct ReserveMinted {
    pub number: u16,
    pub seq: u16,
    pub asset: Pubkey,
    pub owner: Pubkey,
}

#[event]
pub struct SealMinted {
    pub number: u16,
    pub seq: u16,
    pub asset: Pubkey,
    pub buyer: Pubkey,
    pub tier: u8,
    /// The burn amount of the tier, in SOL lamports, the buyer offered to the swap.
    pub burn_lamports: u64,
    /// What the venue actually took (a partial fill takes less).
    pub wsol_spent: u64,
    /// $EDICT units received (balance difference), all burned.
    pub received: u64,
    pub burned: u64,
    /// Lamports of the wSOL account closed to the buyer (the unspent part of a partial fill plus the account rent).
    pub refunded_lamports: u64,
    pub pot_in: u64,
    pub treasury_in: u64,
    /// 0 = LaunchLab curve, 1 = CPMM pool.
    pub venue: u8,
    pub genesis_eligible: bool,
}

#[event]
pub struct PotSynced {
    /// Income realized now, all credited through the accumulator.
    pub realized: u64,
    pub minted: u16,
    /// The divisor of this sync: the Seals that share income (minted minus forfeited).
    pub active: u16,
    pub acc_per_seal: u128,
}

#[event]
pub struct GenesisShared {
    pub eligible: u16,
    pub per_seal: u64,
    pub remainder: u64,
    /// 0 = the 100th public mint, 1 = `finalize_genesis` after 24 hours, 2 = the first public mint after the 24 hours.
    pub trigger: u8,
}

/// A burned Seal left the active set (`forfeit_burned`).
#[event]
pub struct SealForfeited {
    pub number: u16,
    pub asset: Pubkey,
    /// The unclaimed accumulator share returned to the pot as income, in lamports (rounded down; the exact amount is scaled).
    pub released_acc: u64,
    /// The unclaimed genesis share returned to the pot as income, in lamports.
    pub released_genesis: u64,
    /// The Seals that share income from now on.
    pub active: u16,
}

#[event]
pub struct VaultClaimed {
    pub asset: Pubkey,
    pub owner: Pubkey,
    pub acc_share: u64,
    pub genesis_share: u64,
}

#[event]
pub struct VaultWithdrawn {
    pub asset: Pubkey,
    pub owner: Pubkey,
    /// 0 = token, 1 = Core asset.
    pub kind: u8,
    /// The mint (token) or the prize asset (Core).
    pub item: Pubkey,
    pub amount: u64,
}

#[event]
pub struct PriceChangeQueued {
    pub base_lamports: u64,
    pub factor_bps: u32,
    pub eta: i64,
}

#[event]
pub struct PriceChangeCancelled {
    pub base_lamports: u64,
    pub factor_bps: u32,
}

#[event]
pub struct PriceChangeApplied {
    pub from_tier: u8,
    pub base_lamports: u64,
    pub factor_bps: u32,
    pub burn_by_tier: [u64; TIERS],
}

#[event]
pub struct RuleSetQueued {
    pub count: u8,
    pub eta: i64,
}

#[event]
pub struct RuleSetCancelled {
    pub count: u8,
}

#[event]
pub struct RuleSetApplied {
    pub count: u8,
}

#[event]
pub struct PausedSet {
    pub paused: bool,
}

#[event]
pub struct SealCancelled {
    pub swept: u64,
}

#[event]
pub struct SweptToBuyback {
    pub amount: u64,
}

#[event]
pub struct AdminProposed {
    pub new_admin: Pubkey,
}

#[event]
pub struct AdminAccepted {
    pub admin: Pubkey,
}
