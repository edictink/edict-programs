//! `edict_launch`: Edict on Raydium LaunchLab (docs/validation/EDICT_LAUNCH.md, docs/validation/LAUNCHLAB.md 9.2,
//! owner decisions 2026-11-23 in docs/EDICT_START.md 3.2 and 3.3).
//!
//! Every Edict token on LaunchLab gets its own LaunchLab platform whose every role (platform admin, fee wallet, Fee Key
//! holder, CPMM creator) is one PDA of this program, `auth_t = ["auth", mint]`. The launch transaction is
//! `open_platform` (creates and opens the platform), LaunchLab `initialize_v2` / `initialize_with_token_2022` (the pool,
//! creator = `auth_t`), an optional dev buy, then `seal_platform` (closes the allow-config: no other pool can ever use
//! the platform; checks the pool). From then on the fees of that one pool are claimed by this program and forwarded,
//! inside the same instruction, to the token's Edict vault, with the invariant "claimed == received" checked on chain.
//!
//! `$EDICT` (the mint pinned as `protocol_mint`) opens at 5% and steps down with `fee_step` (5, 4, 3, 2% in 7.5 s steps,
//! then 1% for good); each step claims first, so every claimed chunk has one rate, and the event carries the excess
//! over 1% (which the keeper books to the Treasury).

use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod ext;
pub mod ix;
pub mod loader_state;
pub mod schedule;
pub mod state;

use ix::*;

declare_id!("2pvGJ7VtCkmM6AnS2k1Dx3hyoV7H67ucZhixWPKuXAXs");

#[program]
pub mod edict_launch {
    use super::*;

    /// Upgrade authority, once: pins the foreign programs and accounts.
    pub fn init_config(ctx: Context<InitConfig>, args: InitConfigArgs) -> Result<()> {
        ix::handle_init_config(ctx, args)
    }

    /// Admin: hands the admin role to another key (Squads at G-TREASURY).
    pub fn set_admin(ctx: Context<AdminOnly>, admin: Pubkey) -> Result<()> {
        ctx.accounts.config.admin = admin;
        Ok(())
    }

    /// Admin: pins `$EDICT`'s mint, allowed only while no token state exists for the mint pinned before.
    pub fn set_protocol_mint(ctx: Context<SetProtocolMint>, mint: Pubkey) -> Result<()> {
        ix::handle_set_protocol_mint(ctx, mint)
    }

    /// Anyone holding the new mint's keypair: creates the token's platform and opens it for exactly one launch.
    pub fn open_platform(ctx: Context<OpenPlatform>, args: OpenArgs) -> Result<()> {
        ix::handle_open_platform(ctx, args)
    }

    /// Same transaction as the launch, after it: closes the allow-config and checks the pool.
    pub fn seal_platform(ctx: Context<SealPlatform>) -> Result<()> {
        ix::handle_seal_platform(ctx)
    }

    /// Anyone: claims the token platform's fee vault and forwards it to the token's vault.
    pub fn claim_platform_fee(ctx: Context<ClaimPlatformFee>) -> Result<()> {
        ix::handle_claim_platform_fee(ctx)
    }

    /// Anyone (the keeper on time): `$EDICT` only, claim first, then lower the rate as the schedule allows.
    pub fn fee_step(ctx: Context<ClaimPlatformFee>) -> Result<()> {
        ix::handle_fee_step(ctx)
    }

    /// Anyone: after graduation, collects the CPMM creator fee (paid to `auth_t`) and forwards it to the vault.
    pub fn claim_cpmm_creator_fee(ctx: Context<ClaimCpmmCreatorFee>) -> Result<()> {
        ix::handle_claim_cpmm_creator_fee(ctx)
    }

    /// Anyone: after graduation, collects the Fee Key's LP fees straight into the vault's token accounts.
    pub fn claim_fee_key(ctx: Context<ClaimFeeKey>, fee_lp_amount: u64) -> Result<()> {
        ix::handle_claim_fee_key(ctx, fee_lp_amount)
    }
}
