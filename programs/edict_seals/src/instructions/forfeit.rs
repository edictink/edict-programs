//! `forfeit_burned`: a Seal whose Core asset was burned can never be claimed, so anyone may take it out of the active set and
//! return its unclaimed shares to the pot as income for the Seals that remain (owner decision 2026-11-09, review R6; the
//! keeper runs it automatically, P2-03). It moves no lamport: the reserve shrinks, so the same pot lamports become free income.

use anchor_lang::prelude::*;

use crate::constants::*;
use crate::core_cpi::is_burned_asset;
use crate::errors::SealError;
use crate::events::SealForfeited;
use crate::pot::{check_identity, forfeit, sync_pot_state};
use crate::state::{Seal, Config, State};
use crate::util::pot_rent_min;

#[event_cpi]
#[derive(Accounts)]
pub struct ForfeitBurned<'info> {
    /// CHECK: the tombstone Core leaves after `BurnV1` (an account of the Core program, one byte, `Key::Uninitialized`), checked in
    /// the handler. Its address ties it to the `Seal` record by the seeds below: only an asset the program minted has one.
    pub asset: UncheckedAccount<'info>,
    #[account(mut, seeds = [SEAL_SEED, asset.key().as_ref()], bump = seal.bump)]
    pub seal: Box<Account<'info, Seal>>,
    #[account(seeds = [CONFIG_SEED], bump = config.config_bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [STATE_SEED], bump = config.state_bump)]
    pub state: Box<Account<'info, State>>,
    #[account(seeds = [POT_SEED], bump = config.pot_bump)]
    pub pot: SystemAccount<'info>,
}

pub fn handle_forfeit_burned(ctx: Context<ForfeitBurned>) -> Result<()> {
    let a = &mut *ctx.accounts;
    require!(!a.seal.forfeited, SealError::AlreadyForfeited);
    require!(
        is_burned_asset(&a.asset.to_account_info())?,
        SealError::AssetNotBurned
    );
    require_keys_eq!(a.seal.asset, a.asset.key(), SealError::AssetNotBurned);

    // Order (the burned Seal must not be paid any part of what it releases):
    // 1. sync: income that arrived before this call is credited to the active Seals, the burned one still among them, so
    //    its accumulator share is current. It is released in step 2 together with the rest of its share, never paid.
    // 2. release its unclaimed accumulator and genesis shares and drop it from the active count. The released amount is free
    //    income from here on and only the next sync credits it, with the burned Seal already out of the divisor.
    let rent_min = pot_rent_min()?;
    let pot_lamports = a.pot.to_account_info().lamports();
    let synced = sync_pot_state(
        &mut a.state,
        pot_lamports,
        rent_min,
    )?;
    let released = forfeit(
        &mut a.state,
        a.seal.acc_snapshot,
        a.seal.genesis_eligible,
        a.seal.genesis_claimed,
    )?;
    a.seal.forfeited = true;
    check_identity(pot_lamports, rent_min, &a.state)?;

    let forfeited = SealForfeited {
        number: a.seal.number,
        asset: a.asset.key(),
        released_acc: u64::try_from(released.released_acc_scaled / SCALE).unwrap_or(u64::MAX),
        released_genesis: released.released_genesis,
        active: a.state.active,
    };
    if let Some(e) = synced {
        emit_cpi!(e);
    }
    emit_cpi!(forfeited);
    Ok(())
}
