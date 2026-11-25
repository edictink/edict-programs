//! Accounts of `edict_seals` (docs/validation/P2-01_SEAL_DESIGN.md section 3).
//!
//! `Config` is cold (pins and parameters, read by every mint), `State` is hot (counters, bitmap, accumulator, genesis), `Pending` holds the queued changes so the mint does not deserialize them, `Seal` is one record per
//! Seal. The pot (`["pot"]`), the swapper, the collection authority and every asset vault are system-owned PDAs with
//! no data: they appear in the seeds only.

use anchor_lang::prelude::*;

use crate::constants::{MASK_BYTES, MAX_RULES, MAX_URI_BYTES, PAYMENT_POT_PERCENT, TIERS};

/// `Config`: the singleton `["config"]`. Written by `init_config`, `init_collection` and the admin instructions.
#[account]
#[derive(InitSpace)]
pub struct Config {
    /// Squads vault on mainnet. Admin instructions and nothing else; it can never move a holder's money.
    pub admin: Pubkey,
    /// The owner wallet: the only signer of `mint_reserve`. Fixed at init.
    pub reserve_signer: Pubkey,
    /// The Treasury: the owner of every team Seal. Fixed at init.
    pub reserve_recipient: Pubkey,
    /// Receives 40% of a mint's payment and 40% of every royalty. Fixed at init.
    pub treasury: Pubkey,
    /// The only destination of a cancel. Fixed at init.
    pub buyback_wallet: Pubkey,
    pub edict_mint: Pubkey,
    pub edict_token_program: Pubkey,
    /// The swapper PDA's associated $EDICT and wSOL accounts, computed once at init (P2-00: a key compare per mint).
    pub swapper_edict_ata: Pubkey,
    pub swapper_wsol_ata: Pubkey,
    /// Raydium LaunchLab: the program and every account of `buy_exact_in` on $EDICT's pool, read from the pool or derived
    /// at init.
    pub launchlab_program: Pubkey,
    pub ll_authority: Pubkey,
    pub ll_event_authority: Pubkey,
    pub ll_global_config: Pubkey,
    pub ll_platform_config: Pubkey,
    pub ll_pool: Pubkey,
    pub ll_base_vault: Pubkey,
    pub ll_quote_vault: Pubkey,
    pub ll_platform_fee_vault: Pubkey,
    pub ll_creator_fee_vault: Pubkey,
    /// Raydium CPMM: the program and the graduated pool of $EDICT, DERIVED at init from the CPMM config the platform names
    /// (the pool exists only after the migration; a mint on it also requires the LaunchLab pool to say "migrated").
    pub cpmm_program: Pubkey,
    pub cpmm_authority: Pubkey,
    pub cpmm_config: Pubkey,
    pub cpmm_pool: Pubkey,
    pub cpmm_vault_edict: Pubkey,
    pub cpmm_vault_wsol: Pubkey,
    pub cpmm_observation: Pubkey,
    /// The Core collection (the default key until `init_collection`).
    pub collection: Pubkey,
    /// sha256 of the art manifest, published with the metadata. Not checkable on chain.
    pub art_hash: [u8; 32],
    /// The metadata folder URI; the program builds `base_uri + NNN + ".json"` (three digits, as the files are named).
    #[max_len(MAX_URI_BYTES)]
    pub base_uri: String,
    pub paused: bool,
    pub cancelled: bool,
    pub team_set: bool,
    pub art_locked: bool,
    /// The 30 team numbers: bit `number - 1`.
    pub team_mask: [u8; MASK_BYTES],
    /// The fixed payment of a public mint (G-NFT-PRICE): 60% to the pot, 40% to the Treasury.
    pub payment_lamports: u64,
    pub base_lamports: u64,
    pub factor_bps: u32,
    pub min_burn_lamports: u64,
    pub max_burn_lamports: u64,
    /// The burn amount of tiers 0 to 2 (derived from the base and the factor, floor at every step).
    pub burn_by_tier: [u64; TIERS],
    pub config_bump: u8,
    pub state_bump: u8,
    pub pending_bump: u8,
    pub swapper_bump: u8,
    pub authority_bump: u8,
    pub pot_bump: u8,
}

impl Config {
    /// The pot's and the Treasury's part of a mint's payment (the rounding to the Treasury).
    pub fn payment_split(&self) -> (u64, u64) {
        let pot = self.payment_lamports / 100 * PAYMENT_POT_PERCENT
            + self.payment_lamports % 100 * PAYMENT_POT_PERCENT / 100;
        (pot, self.payment_lamports - pot)
    }
}

/// `State`: the singleton `["state"]`. Written by every mint, claim and crank.
#[account]
#[derive(InitSpace)]
pub struct State {
    /// The supply and sequence counter: every Seal ever minted, burned or not (at most 250). The number of a burned Seal
    /// stays set in `minted_mask`, so it can never be minted again.
    pub minted: u16,
    /// The Seals that share income: `minted` minus the forfeited (burned) ones. The divisor `n` of the accumulator.
    pub active: u16,
    pub reserve_minted: u8,
    pub public_minted: u16,
    /// The lowest candidate for the next public number.
    pub next_public: u16,
    /// 0 until the first `mint_reserve`: the art is locked and the genesis amount is locked at this moment.
    pub opened_at: i64,
    /// 0 until the 30th team Seal exists: public mints are possible from this moment and the 24 hour genesis window
    /// starts here (review R3: a team batch that takes longer than 24 hours must not let a finalize precede every public mint).
    pub public_open_at: i64,
    /// The numbers that exist: bit `number - 1`.
    pub minted_mask: [u8; MASK_BYTES],
    /// The reward-per-Seal accumulator, lamports x 10^12.
    pub acc_per_seal: u128,
    /// Accumulator shares credited and not yet paid, lamports x 10^12 (exact; the reserve is its ceiling).
    pub owed_scaled: u128,
    /// The amount locked at the opening, apart from the regular pot, until it is shared.
    pub genesis_locked: u64,
    /// The shares created at the sharing and not yet paid.
    pub genesis_owed: u64,
    pub genesis_shared: bool,
    /// Seals eligible for genesis: 30 team plus public mints 1 to 100 minted before the sharing.
    pub genesis_eligible_count: u16,
    pub genesis_per_seal: u64,
    pub income_realized_total: u64,
    pub claimed_total: u64,
    pub bump: u8,
}

impl State {
    /// A fresh state (arrays of 32 bytes have no `Default`).
    pub fn new(bump: u8) -> Self {
        Self {
            minted: 0,
            active: 0,
            reserve_minted: 0,
            public_minted: 0,
            next_public: 1,
            opened_at: 0,
            public_open_at: 0,
            minted_mask: [0u8; MASK_BYTES],
            acc_per_seal: 0,
            owed_scaled: 0,
            genesis_locked: 0,
            genesis_owed: 0,
            genesis_shared: false,
            genesis_eligible_count: 0,
            genesis_per_seal: 0,
            income_realized_total: 0,
            claimed_total: 0,
            bump,
        }
    }
}

/// `Pending`: the singleton `["pending"]`: the queued price change, the queued rule set, a proposed admin.
#[account]
#[derive(InitSpace)]
pub struct Pending {
    /// The default key when no rotation is proposed.
    pub pending_admin: Pubkey,
    pub price_active: bool,
    pub price_base: u64,
    pub price_factor_bps: u32,
    pub price_eta: i64,
    pub rules_active: bool,
    pub rules_eta: i64,
    pub rules_count: u8,
    pub rules: [Pubkey; MAX_RULES],
    pub bump: u8,
}

/// `Seal`: one per Seal, `["seal", asset]`.
#[account]
#[derive(InitSpace)]
pub struct Seal {
    pub asset: Pubkey,
    /// 1 to 250: its art.
    pub number: u16,
    /// The mint sequence, 1 to 250 in the order minted (team first).
    pub seq: u16,
    pub is_team: bool,
    pub genesis_eligible: bool,
    pub genesis_claimed: bool,
    /// Set by `forfeit_burned`: the Core asset was burned and the Seal left the active set.
    pub forfeited: bool,
    /// The accumulator value at the mint, advanced by every claim (the fraction is kept).
    pub acc_snapshot: u128,
    pub sol_claimed: u64,
    pub bump: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The data lengths the clients decode (`packages/sdk/src/edict-seals/accounts.ts` ACCOUNT_SIZES), discriminator included.
    #[test]
    fn account_sizes_are_the_clients() {
        assert_eq!(8 + Config::INIT_SPACE, 1_106);
        assert_eq!(8 + State::INIT_SPACE, 141);
        assert_eq!(8 + Pending::INIT_SPACE, 328);
        assert_eq!(8 + Seal::INIT_SPACE, 73);
    }

    #[test]
    fn the_payment_splits_60_40_with_the_rounding_to_the_treasury() {
        let mut c = Config {
            admin: Pubkey::default(),
            reserve_signer: Pubkey::default(),
            reserve_recipient: Pubkey::default(),
            treasury: Pubkey::default(),
            buyback_wallet: Pubkey::default(),
            edict_mint: Pubkey::default(),
            edict_token_program: Pubkey::default(),
            swapper_edict_ata: Pubkey::default(),
            swapper_wsol_ata: Pubkey::default(),
            launchlab_program: Pubkey::default(),
            ll_authority: Pubkey::default(),
            ll_event_authority: Pubkey::default(),
            ll_global_config: Pubkey::default(),
            ll_platform_config: Pubkey::default(),
            ll_pool: Pubkey::default(),
            ll_base_vault: Pubkey::default(),
            ll_quote_vault: Pubkey::default(),
            ll_platform_fee_vault: Pubkey::default(),
            ll_creator_fee_vault: Pubkey::default(),
            cpmm_program: Pubkey::default(),
            cpmm_authority: Pubkey::default(),
            cpmm_config: Pubkey::default(),
            cpmm_pool: Pubkey::default(),
            cpmm_vault_edict: Pubkey::default(),
            cpmm_vault_wsol: Pubkey::default(),
            cpmm_observation: Pubkey::default(),
            collection: Pubkey::default(),
            art_hash: [0; 32],
            base_uri: String::new(),
            paused: false,
            cancelled: false,
            team_set: false,
            art_locked: false,
            team_mask: [0; MASK_BYTES],
            payment_lamports: 500_000_000,
            base_lamports: 0,
            factor_bps: 0,
            min_burn_lamports: 0,
            max_burn_lamports: 0,
            burn_by_tier: [0; TIERS],
            config_bump: 0,
            state_bump: 0,
            pending_bump: 0,
            swapper_bump: 0,
            authority_bump: 0,
            pot_bump: 0,
        };
        assert_eq!(c.payment_split(), (300_000_000, 200_000_000));
        c.payment_lamports = 333;
        assert_eq!(c.payment_split(), (199, 134));
        c.payment_lamports = u64::MAX;
        let (pot, treasury) = c.payment_split();
        assert_eq!(pot + treasury, u64::MAX);
        assert_eq!(u128::from(pot), u128::from(u64::MAX) * 60 / 100);
    }
}
