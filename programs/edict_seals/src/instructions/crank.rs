//! The permissionless cranks: `sync_pot` and `finalize_genesis`.

use anchor_lang::prelude::*;

use crate::constants::*;
use crate::errors::SealError;
use crate::events::GenesisShared;
use crate::pot::{check_identity, genesis_window_end, share_genesis, sync_pot_state};
use crate::state::{Config, State};
use crate::util::pot_rent_min;

#[event_cpi]
#[derive(Accounts)]
pub struct SyncPot<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.config_bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [STATE_SEED], bump = config.state_bump)]
    pub state: Box<Account<'info, State>>,
    #[account(seeds = [POT_SEED], bump = config.pot_bump)]
    pub pot: SystemAccount<'info>,
}

/// Anyone can call it: realizes the pot's free income for the Seals that exist (design 5.3). A no-op when there is
/// nothing to realize or no Seal yet.
pub fn handle_sync_pot(ctx: Context<SyncPot>) -> Result<()> {
    let a = &mut *ctx.accounts;
    let rent_min = pot_rent_min()?;
    let lamports = a.pot.to_account_info().lamports();
    let synced = sync_pot_state(&mut a.state, lamports, rent_min)?;
    check_identity(lamports, rent_min, &a.state)?;
    if let Some(e) = synced {
        emit_cpi!(e);
    }
    Ok(())
}

#[event_cpi]
#[derive(Accounts)]
pub struct FinalizeGenesis<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.config_bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [STATE_SEED], bump = config.state_bump)]
    pub state: Box<Account<'info, State>>,
    #[account(seeds = [POT_SEED], bump = config.pot_bump)]
    pub pot: SystemAccount<'info>,
}

/// Shares genesis 24 hours after the public mints opened (the 30th team Seal, review R3) when the 100th public mint and a
/// mint after the window have not done it, among the eligible Seals minted at that moment. A public mint after this is NOT
/// eligible, whatever its number. Needs all 30 team Seals (design D15).
pub fn handle_finalize_genesis(ctx: Context<FinalizeGenesis>) -> Result<()> {
    let a = &mut *ctx.accounts;
    require!(a.state.opened_at > 0, SealError::NotOpen);
    require!(!a.state.genesis_shared, SealError::GenesisAlreadyShared);
    require!(
        a.state.reserve_minted == TEAM_COUNT,
        SealError::ReserveIncomplete
    );
    require!(
        Clock::get()?.unix_timestamp >= genesis_window_end(&a.state)?,
        SealError::GenesisWindowOpen
    );
    let g = share_genesis(&mut a.state)?;
    check_identity(
        a.pot.to_account_info().lamports(),
        pot_rent_min()?,
        &a.state,
    )?;
    emit_cpi!(GenesisShared {
        eligible: g.eligible,
        per_seal: g.per_seal,
        remainder: g.remainder,
        trigger: 1,
    });
    Ok(())
}
