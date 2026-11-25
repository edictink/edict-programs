//! `edict_seals`: the Edict Seal (docs/PROTOCOL_SPEC.md section 8, docs/validation/P2-01_SEAL_DESIGN.md).
//!
//! A Metaplex Core collection of 250 membership NFTs (30 team Seals minted free by the owner wallet, 220 public mints). A
//! public mint pays a fixed amount (G-NFT-PRICE) 60% to the pot and 40% to the Treasury and spends a burn amount of SOL, by
//! CPI with a PDA signer, on $EDICT's active venue (Raydium LaunchLab's curve before the graduation, the CPMM pool after it),
//! then burns every $EDICT it received (balance difference, P2-00), all in one transaction. The pot is shared equally among the Seals that exist,
//! from the mint onward, through a reward-per-Seal accumulator; the pot before the opening is locked as the genesis
//! amount and shared once. The 5% resale royalty (3% pot / 2% Treasury) is a collection-level Core Royalties plugin.
//!
//! Read `docs/validation/P2-01_SEAL_DESIGN.md` first: every account, instruction, check and invariant is there.

use anchor_lang::prelude::*;

pub mod constants;
pub mod core_cpi;
pub mod cpi_lite;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod loader_state;
pub mod math;
pub mod edict;
pub mod pot;
pub mod state;
pub mod util;
pub mod venue;

use instructions::*;

// The program id of the LOCAL / DEV deployment (its secret key was never kept: the program is loaded at genesis of the test
// validator and of LiteSVM). The devnet and mainnet ids are created at deploy time (gates G-KEYS and G-FUNDS):
// declare_id!, Anchor.toml and the clients are changed together.
declare_id!("6Eusr5CdS6DMCz9csfvGP24DqLxrPt4sBaeamDijL87f");

#[program]
pub mod edict_seals {
    use super::*;

    /// Upgrade authority: creates `Config`, `State`, `Pending` and funds the pot vault; verifies every pinned account.
    pub fn init_config(ctx: Context<InitConfig>, args: InitConfigArgs) -> Result<()> {
        instructions::init::handle_init_config(ctx, args)
    }

    /// Upgrade authority, once: creates the Core collection with the Royalties plugin (500 bps, pot 60 / Treasury 40).
    pub fn init_collection(ctx: Context<InitCollection>, args: InitCollectionArgs) -> Result<()> {
        instructions::init::handle_init_collection(ctx, args)
    }

    /// Admin, before the opening: the metadata folder and the art manifest hash.
    pub fn set_art(ctx: Context<SetArt>, base_uri: String, art_hash: [u8; 32]) -> Result<()> {
        instructions::admin::handle_set_art(ctx, base_uri, art_hash)
    }

    /// Admin, once, before the opening: the 30 team numbers.
    pub fn set_team_numbers(ctx: Context<SetTeamNumbers>, numbers: [u16; 30]) -> Result<()> {
        instructions::admin::handle_set_team_numbers(ctx, numbers)
    }

    /// Admin: queues a new base and factor behind the 24 hour timelock.
    pub fn queue_price_change(
        ctx: Context<QueuePriceChange>,
        base_lamports: u64,
        factor_bps: u32,
    ) -> Result<()> {
        instructions::admin::handle_queue_price_change(ctx, base_lamports, factor_bps)
    }

    pub fn cancel_price_change(ctx: Context<CancelPriceChange>) -> Result<()> {
        instructions::admin::handle_cancel_price_change(ctx)
    }

    /// Anyone, after the eta: applies the queued price change to the tiers above the one being minted.
    pub fn apply_price_change(ctx: Context<ApplyPriceChange>) -> Result<()> {
        instructions::admin::handle_apply_price_change(ctx)
    }

    /// Admin: queues a new royalty allow list (the System Program required) behind the timelock.
    pub fn queue_rule_set(ctx: Context<QueueRuleSet>, rules: Vec<Pubkey>) -> Result<()> {
        instructions::admin::handle_queue_rule_set(ctx, rules)
    }

    pub fn cancel_rule_set(ctx: Context<CancelRuleSet>) -> Result<()> {
        instructions::admin::handle_cancel_rule_set(ctx)
    }

    /// Anyone, after the eta: writes the queued allow list to the collection's Royalties plugin.
    pub fn apply_rule_set(ctx: Context<ApplyRuleSet>) -> Result<()> {
        instructions::admin::handle_apply_rule_set(ctx)
    }

    /// Admin: stops or resumes PUBLIC mints (claims and cranks keep working).
    pub fn set_paused(ctx: Context<SetPaused>, paused: bool) -> Result<()> {
        instructions::admin::handle_set_paused(ctx, paused)
    }

    pub fn propose_admin(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
        instructions::admin::handle_propose_admin(ctx, new_admin)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        instructions::admin::handle_accept_admin(ctx)
    }

    /// Admin, only before the opening: cancels the Seal and sends the free pot to the fixed buyback wallet.
    pub fn cancel_seal(ctx: Context<CancelSeal>) -> Result<()> {
        instructions::admin::handle_cancel_seal(ctx)
    }

    /// Anyone, after a cancel: sends what the pot holds above its rent minimum to the buyback wallet.
    pub fn sweep_to_buyback(ctx: Context<SweepToBuyback>) -> Result<()> {
        instructions::admin::handle_sweep_to_buyback(ctx)
    }

    /// The owner wallet: mints 1 to 5 team Seals to the Treasury, free. The first call opens the mint.
    pub fn mint_reserve<'info>(
        ctx: Context<'info, MintReserve<'info>>,
        numbers: Vec<u16>,
    ) -> Result<()> {
        instructions::reserve::handle_mint_reserve(ctx, numbers)
    }

    /// Anyone: the public mint on $EDICT's LaunchLab curve (before the graduation).
    pub fn mint_seal_curve(ctx: Context<MintSealCurve>, min_out: u64) -> Result<()> {
        instructions::mint::handle_mint_seal_curve(ctx, min_out)
    }

    /// Anyone: the public mint on $EDICT's CPMM pool (after the migration).
    pub fn mint_seal_cpmm(ctx: Context<MintSealCpmm>, min_out: u64) -> Result<()> {
        instructions::mint::handle_mint_seal_cpmm(ctx, min_out)
    }

    /// Anyone: realizes the pot's free income for the Seals that exist.
    pub fn sync_pot(ctx: Context<SyncPot>) -> Result<()> {
        instructions::crank::handle_sync_pot(ctx)
    }

    /// Anyone, 24 hours after the opening: shares genesis among the eligible Seals minted so far.
    pub fn finalize_genesis(ctx: Context<FinalizeGenesis>) -> Result<()> {
        instructions::crank::handle_finalize_genesis(ctx)
    }

    /// The current owner of a Seal: claims its accumulator share and, once, its genesis share.
    pub fn claim_vault(ctx: Context<ClaimVault>) -> Result<()> {
        instructions::vault::handle_claim_vault(ctx)
    }

    /// Anyone, for a Seal whose Core asset was burned: returns its unclaimed shares to the pot as income and removes it from
    /// the active set (owner decision 2026-11-09).
    pub fn forfeit_burned(ctx: Context<ForfeitBurned>) -> Result<()> {
        instructions::forfeit::handle_forfeit_burned(ctx)
    }

    /// The current owner: moves tokens out of the Seal's vault.
    pub fn vault_withdraw_token(ctx: Context<VaultWithdrawToken>, amount: u64) -> Result<()> {
        instructions::vault::handle_vault_withdraw_token(ctx, amount)
    }

    /// The current owner: moves a Core prize out of the Seal's vault.
    pub fn vault_withdraw_core(ctx: Context<VaultWithdrawCore>) -> Result<()> {
        instructions::vault::handle_vault_withdraw_core(ctx)
    }
}
