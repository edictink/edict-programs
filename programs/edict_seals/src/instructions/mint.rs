//! The public mint: `mint_seal_curve` (on $EDICT's LaunchLab curve, before the graduation) and `mint_seal_cpmm` (on the CPMM
//! pool the migration created). One instruction, one transaction: pay, sync the pot, swap SOL for $EDICT by CPI and burn the
//! balance difference (P2-00), create the Core asset and the `Seal` record, count it, share genesis at the 100th public mint.
//! If any part fails nothing happens. Between the end of the curve and the CPMM pool neither venue trades $EDICT: both mints
//! are refused with a clear error and the client waits.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::AccountMeta;
use anchor_spl::token::{Token, TokenAccount as SplTokenAccount};
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::*;
use crate::core_cpi;
use crate::cpi_lite;
use crate::errors::SealError;
use crate::events::{GenesisShared, PotSynced, SealMinted};
use crate::math::{next_public_number, public_left, tier_of};
use crate::pot::{
    check_identity, genesis_window_end, register_mint, require_synced, share_genesis,
    sync_pot_state,
};
use crate::state::{Seal, Config, State};
use crate::util::{seal_name, seal_uri, pot_rent_min};
use crate::venue::{buy_and_burn, launch_pool_status, FlowAccounts};

/// The venue part of a mint: which program, the exact account list of its exact-in swap, and which venue it is.
pub struct VenueCall<'info> {
    pub program: Pubkey,
    pub metas: Vec<AccountMeta>,
    pub infos: Vec<AccountInfo<'info>>,
    /// `VENUE_CURVE` or `VENUE_CPMM` (the swap data and the event).
    pub venue: u8,
}

/// The accounts both mint instructions share (their `Accounts` structs use the same field names).
pub struct MintAccounts<'a, 'info> {
    pub buyer: &'a Signer<'info>,
    pub config: &'a Account<'info, Config>,
    pub state: &'a mut Account<'info, State>,
    pub pot: &'a SystemAccount<'info>,
    pub treasury: &'a SystemAccount<'info>,
    pub swapper: &'a UncheckedAccount<'info>,
    pub collection_authority: &'a UncheckedAccount<'info>,
    pub edict_mint: &'a InterfaceAccount<'info, Mint>,
    pub swapper_edict: &'a mut InterfaceAccount<'info, TokenAccount>,
    pub swapper_wsol: &'a mut Account<'info, SplTokenAccount>,
    pub collection: &'a UncheckedAccount<'info>,
    pub asset: &'a Signer<'info>,
    pub seal: &'a mut Account<'info, Seal>,
    pub core_program: &'a UncheckedAccount<'info>,
    pub edict_token_program: &'a Interface<'info, TokenInterface>,
    pub token_program: &'a Program<'info, Token>,
    pub system_program: &'a Program<'info, System>,
}

/// The events of a mint, in the order they are emitted.
pub struct MintEvents {
    pub synced: Option<PotSynced>,
    pub minted: SealMinted,
    pub genesis: Option<GenesisShared>,
}

/// The checks that decide whether a public mint may start, and what it will cost: `(tier, burn lamports, number)`.
/// Nothing has moved when this returns.
pub fn precheck(config: &Config, state: &State, min_out: u64) -> Result<(usize, u64, u16)> {
    require!(!config.cancelled, SealError::Cancelled);
    require!(!config.paused, SealError::Paused);
    require!(state.opened_at > 0, SealError::NotOpen);
    require!(
        state.reserve_minted == TEAM_COUNT,
        SealError::ReserveIncomplete
    );
    require!(public_left(state.public_minted), SealError::SoldOut);
    require!(min_out > 0, SealError::ZeroAmount);
    let tier = tier_of(state.public_minted);
    let burn = *config
        .burn_by_tier
        .get(tier)
        .ok_or_else(|| error!(SealError::SoldOut))?;
    require!(
        burn >= config.min_burn_lamports && burn <= config.max_burn_lamports,
        SealError::BurnOutOfBounds
    );
    let number = next_public_number(&config.team_mask, &state.minted_mask, state.next_public)
        .ok_or_else(|| error!(SealError::SoldOut))?;
    Ok((tier, burn, number))
}

pub fn run_mint<'a, 'info>(
    m: MintAccounts<'a, 'info>,
    venue: VenueCall<'info>,
    min_out: u64,
    seal_bump: u8,
) -> Result<MintEvents> {
    let MintAccounts {
        buyer,
        config,
        state,
        pot,
        treasury,
        swapper,
        collection_authority,
        edict_mint,
        swapper_edict,
        swapper_wsol,
        collection,
        asset,
        seal,
        core_program,
        edict_token_program,
        token_program,
        system_program,
    } = m;

    let (tier, burn, number) = precheck(config, state, min_out)?;

    // 1. the payment, fixed at init (G-NFT-PRICE): 60% to the pot and 40% to the Treasury (spec 8)
    let (pot_in, treasury_in) = config.payment_split();
    cpi_lite::system_transfer(&buyer.to_account_info(), &pot.to_account_info(), pot_in, &[])?;
    cpi_lite::system_transfer(
        &buyer.to_account_info(),
        &treasury.to_account_info(),
        treasury_in,
        &[],
    )?;

    // 2. the sync, BEFORE the new Seal is counted: the mint's own pot part goes to the Seals that exist
    let rent_min = pot_rent_min()?;
    let pot_info = pot.to_account_info();
    let synced = sync_pot_state(state, pot_info.lamports(), rent_min)?;
    // a Seal is counted only when no income is left to realize later (R1)
    require_synced(pot_info.lamports(), rent_min, state)?;

    // 3. buy $EDICT by CPI with the swapper as signer and burn exactly the balance difference
    let swap = buy_and_burn(
        FlowAccounts {
            buyer,
            swapper,
            swapper_bump: config.swapper_bump,
            swapper_edict,
            swapper_wsol,
            edict_mint,
            edict_token_program,
            token_program,
            system_program,
        },
        &venue.program,
        venue.venue,
        burn,
        min_out,
        venue.metas,
        &venue.infos,
    )?;

    // 4. the Core asset: the program's own name and URI, owned by the buyer
    core_cpi::create_asset(
        &core_program.to_account_info(),
        &asset.to_account_info(),
        &collection.to_account_info(),
        &collection_authority.to_account_info(),
        &buyer.to_account_info(),
        &buyer.to_account_info(),
        &system_program.to_account_info(),
        seal_name(number),
        seal_uri(&config.base_uri, number),
        config.authority_bump,
    )?;

    // 4b. the 24 hour window (review R2): a mint that comes after it closes the genesis set itself, BEFORE the new Seal is
    // counted, so the result does not depend on who calls `finalize_genesis` first and a bundled [mint, finalize] cannot get in
    let mut genesis = None;
    if !state.genesis_shared && Clock::get()?.unix_timestamp >= genesis_window_end(state)? {
        let g = share_genesis(state)?;
        genesis = Some(GenesisShared {
            eligible: g.eligible,
            per_seal: g.per_seal,
            remainder: g.remainder,
            trigger: 2,
        });
    }

    // 5. count it: counters, bitmap, cursor, genesis eligibility, and the record with the accumulator snapshot
    let registered = register_mint(state, number, false)?;
    seal.asset = asset.key();
    seal.number = number;
    seal.seq = registered.seq;
    seal.is_team = false;
    seal.genesis_eligible = registered.genesis_eligible;
    seal.genesis_claimed = false;
    seal.forfeited = false;
    seal.acc_snapshot = state.acc_per_seal;
    seal.sol_claimed = 0;
    seal.bump = seal_bump;

    // 6. the 100th public mint shares genesis, inside this instruction
    if state.public_minted == GENESIS_PUBLIC_MINTS && !state.genesis_shared {
        let g = share_genesis(state)?;
        genesis = Some(GenesisShared {
            eligible: g.eligible,
            per_seal: g.per_seal,
            remainder: g.remainder,
            trigger: 0,
        });
    }

    check_identity(pot_info.lamports(), rent_min, state)?;

    Ok(MintEvents {
        synced,
        minted: SealMinted {
            number,
            seq: registered.seq,
            asset: asset.key(),
            buyer: buyer.key(),
            tier: u8::try_from(tier).unwrap_or(u8::MAX),
            burn_lamports: burn,
            wsol_spent: swap.wsol_spent,
            received: swap.received,
            burned: swap.received,
            refunded_lamports: swap.refunded_lamports,
            pot_in,
            treasury_in,
            venue: venue.venue,
            genesis_eligible: registered.genesis_eligible,
        },
        genesis,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// the LaunchLab curve (before the graduation)
// ---------------------------------------------------------------------------------------------------------------------

#[event_cpi]
#[derive(Accounts)]
pub struct MintSealCurve<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.config_bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [STATE_SEED], bump = config.state_bump)]
    pub state: Box<Account<'info, State>>,
    #[account(mut, seeds = [POT_SEED], bump = config.pot_bump)]
    pub pot: SystemAccount<'info>,
    #[account(mut, address = config.treasury @ SealError::InvalidConfigKey)]
    pub treasury: SystemAccount<'info>,
    /// CHECK: the swapper PDA: system-owned, the owner of the two token accounts and the `payer` of the venue's swap. Writable:
    /// LaunchLab's `buy_exact_in` declares its payer `mut` (Anchor ConstraintMut 2000 otherwise, docs/validation/SEALS_VENUE.md).
    #[account(mut, seeds = [SWAPPER_SEED], bump = config.swapper_bump)]
    pub swapper: UncheckedAccount<'info>,
    /// CHECK: the collection authority PDA, which signs the Core creation.
    #[account(seeds = [AUTHORITY_SEED], bump = config.authority_bump)]
    pub collection_authority: UncheckedAccount<'info>,
    #[account(
        mut,
        address = config.edict_mint @ SealError::MintNotPinned,
        constraint = *edict_mint.to_account_info().owner == edict_token_program.key() @ SealError::WrongTokenProgram
    )]
    pub edict_mint: Box<InterfaceAccount<'info, Mint>>,
    /// The swapper's associated $EDICT account, pinned in Config at init.
    #[account(
        mut,
        address = config.swapper_edict_ata @ SealError::WrongTokenAccount,
        constraint = swapper_edict.mint == edict_mint.key() @ SealError::WrongTokenAccount,
        constraint = swapper_edict.owner == swapper.key() @ SealError::WrongTokenAccount,
        constraint = *swapper_edict.to_account_info().owner == edict_token_program.key() @ SealError::WrongTokenProgram
    )]
    pub swapper_edict: Box<InterfaceAccount<'info, TokenAccount>>,
    /// The swapper's associated wSOL account (created idempotently by the client in the same transaction).
    #[account(
        mut,
        address = config.swapper_wsol_ata @ SealError::WrongTokenAccount,
        constraint = swapper_wsol.mint == WSOL_MINT @ SealError::WrongTokenAccount,
        constraint = swapper_wsol.owner == swapper.key() @ SealError::WrongTokenAccount
    )]
    pub swapper_wsol: Box<Account<'info, SplTokenAccount>>,
    /// CHECK: the wSOL mint, a constant address.
    #[account(address = WSOL_MINT @ SealError::MintNotPinned)]
    pub wsol_mint: UncheckedAccount<'info>,
    /// CHECK: the pinned LaunchLab program; a wrong program would receive the swapper PDA's signature.
    #[account(address = config.launchlab_program @ SealError::WrongVenueProgram, executable)]
    pub launchlab_program: UncheckedAccount<'info>,
    /// CHECK: LaunchLab's `authority` PDA, pinned at init.
    #[account(address = config.ll_authority @ SealError::WrongAuthority)]
    pub ll_authority: UncheckedAccount<'info>,
    /// CHECK: LaunchLab's `event_authority` PDA, pinned at init.
    #[account(address = config.ll_event_authority @ SealError::WrongAuthority)]
    pub ll_event_authority: UncheckedAccount<'info>,
    /// CHECK: the pinned global config.
    #[account(address = config.ll_global_config @ SealError::PoolNotPinned)]
    pub ll_global_config: UncheckedAccount<'info>,
    /// CHECK: the pinned platform config of $EDICT.
    #[account(address = config.ll_platform_config @ SealError::PoolNotPinned)]
    pub ll_platform_config: UncheckedAccount<'info>,
    /// CHECK: the pinned pool; its status must say "trading".
    #[account(mut, address = config.ll_pool @ SealError::PoolNotPinned)]
    pub pool: UncheckedAccount<'info>,
    /// CHECK: the pinned base vault ($EDICT).
    #[account(mut, address = config.ll_base_vault @ SealError::VaultNotPinned)]
    pub base_vault: UncheckedAccount<'info>,
    /// CHECK: the pinned quote vault (wSOL).
    #[account(mut, address = config.ll_quote_vault @ SealError::VaultNotPinned)]
    pub quote_vault: UncheckedAccount<'info>,
    /// CHECK: the pinned platform fee vault (a mandatory trailing account of every LaunchLab trade).
    #[account(mut, address = config.ll_platform_fee_vault @ SealError::VaultNotPinned)]
    pub platform_fee_vault: UncheckedAccount<'info>,
    /// CHECK: the pinned creator fee vault (a mandatory trailing account; empty on Edict platforms).
    #[account(mut, address = config.ll_creator_fee_vault @ SealError::VaultNotPinned)]
    pub creator_fee_vault: UncheckedAccount<'info>,
    /// CHECK: the pinned Core collection.
    #[account(mut, address = config.collection @ SealError::WrongCollection)]
    pub collection: UncheckedAccount<'info>,
    /// The new Seal asset: a fresh keypair that signs the transaction.
    #[account(mut)]
    pub asset: Signer<'info>,
    #[account(
        init,
        payer = buyer,
        space = 8 + Seal::INIT_SPACE,
        seeds = [SEAL_SEED, asset.key().as_ref()],
        bump
    )]
    pub seal: Box<Account<'info, Seal>>,
    /// CHECK: the Metaplex Core program, a constant address.
    #[account(address = crate::core_cpi::CORE_ID @ SealError::WrongCoreProgram)]
    pub core_program: UncheckedAccount<'info>,
    #[account(address = config.edict_token_program @ SealError::WrongTokenProgram)]
    pub edict_token_program: Interface<'info, TokenInterface>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_mint_seal_curve(ctx: Context<MintSealCurve>, min_out: u64) -> Result<()> {
    let seal_bump = ctx.bumps.seal;
    let a = &mut *ctx.accounts;
    require!(
        launch_pool_status(&a.pool.try_borrow_data()?)? == LL_STATUS_TRADING,
        SealError::PoolNotTrading
    );
    // the account list of LaunchLab `buy_exact_in` in IDL order (packages/adapters/raydium-launchlab buyExactInInstruction);
    // `payer` is the swapper PDA, the only signer
    let swapper = a.swapper.key();
    let metas = vec![
        AccountMeta::new(swapper, true), // payer (LaunchLab: mut)
        AccountMeta::new_readonly(a.ll_authority.key(), false),
        AccountMeta::new_readonly(a.ll_global_config.key(), false),
        AccountMeta::new_readonly(a.ll_platform_config.key(), false),
        AccountMeta::new(a.pool.key(), false),
        AccountMeta::new(a.swapper_edict.key(), false), // user_base_token
        AccountMeta::new(a.swapper_wsol.key(), false),  // user_quote_token
        AccountMeta::new(a.base_vault.key(), false),
        AccountMeta::new(a.quote_vault.key(), false),
        AccountMeta::new_readonly(a.edict_mint.key(), false), // base_token_mint
        AccountMeta::new_readonly(WSOL_MINT, false),          // quote_token_mint
        AccountMeta::new_readonly(a.edict_token_program.key(), false), // base_token_program
        AccountMeta::new_readonly(a.token_program.key(), false), // quote_token_program
        AccountMeta::new_readonly(a.ll_event_authority.key(), false),
        AccountMeta::new_readonly(a.launchlab_program.key(), false),
        AccountMeta::new_readonly(a.system_program.key(), false),
        AccountMeta::new(a.platform_fee_vault.key(), false),
        AccountMeta::new(a.creator_fee_vault.key(), false),
    ];
    let infos = vec![
        a.swapper.to_account_info(),
        a.ll_authority.to_account_info(),
        a.ll_global_config.to_account_info(),
        a.ll_platform_config.to_account_info(),
        a.pool.to_account_info(),
        a.swapper_edict.to_account_info(),
        a.swapper_wsol.to_account_info(),
        a.base_vault.to_account_info(),
        a.quote_vault.to_account_info(),
        a.edict_mint.to_account_info(),
        a.wsol_mint.to_account_info(),
        a.edict_token_program.to_account_info(),
        a.token_program.to_account_info(),
        a.ll_event_authority.to_account_info(),
        a.launchlab_program.to_account_info(),
        a.system_program.to_account_info(),
        a.platform_fee_vault.to_account_info(),
        a.creator_fee_vault.to_account_info(),
    ];
    let program = a.launchlab_program.key();
    let events = run_mint(
        MintAccounts {
            buyer: &a.buyer,
            config: &a.config,
            state: &mut a.state,
            pot: &a.pot,
            treasury: &a.treasury,
            swapper: &a.swapper,
            collection_authority: &a.collection_authority,
            edict_mint: &a.edict_mint,
            swapper_edict: &mut a.swapper_edict,
            swapper_wsol: &mut a.swapper_wsol,
            collection: &a.collection,
            asset: &a.asset,
            seal: &mut a.seal,
            core_program: &a.core_program,
            edict_token_program: &a.edict_token_program,
            token_program: &a.token_program,
            system_program: &a.system_program,
        },
        VenueCall {
            program,
            metas,
            infos,
            venue: VENUE_CURVE,
        },
        min_out,
        seal_bump,
    )?;
    if let Some(e) = events.synced {
        emit_cpi!(e);
    }
    emit_cpi!(events.minted);
    if let Some(e) = events.genesis {
        emit_cpi!(e);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// the CPMM pool (after the graduation)
// ---------------------------------------------------------------------------------------------------------------------

#[event_cpi]
#[derive(Accounts)]
pub struct MintSealCpmm<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.config_bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [STATE_SEED], bump = config.state_bump)]
    pub state: Box<Account<'info, State>>,
    #[account(mut, seeds = [POT_SEED], bump = config.pot_bump)]
    pub pot: SystemAccount<'info>,
    #[account(mut, address = config.treasury @ SealError::InvalidConfigKey)]
    pub treasury: SystemAccount<'info>,
    /// CHECK: the swapper PDA: system-owned, the owner of the two token accounts and the `payer` of the venue's swap.
    #[account(seeds = [SWAPPER_SEED], bump = config.swapper_bump)]
    pub swapper: UncheckedAccount<'info>,
    /// CHECK: the collection authority PDA, which signs the Core creation.
    #[account(seeds = [AUTHORITY_SEED], bump = config.authority_bump)]
    pub collection_authority: UncheckedAccount<'info>,
    #[account(
        mut,
        address = config.edict_mint @ SealError::MintNotPinned,
        constraint = *edict_mint.to_account_info().owner == edict_token_program.key() @ SealError::WrongTokenProgram
    )]
    pub edict_mint: Box<InterfaceAccount<'info, Mint>>,
    /// The swapper's associated $EDICT account, pinned in Config at init.
    #[account(
        mut,
        address = config.swapper_edict_ata @ SealError::WrongTokenAccount,
        constraint = swapper_edict.mint == edict_mint.key() @ SealError::WrongTokenAccount,
        constraint = swapper_edict.owner == swapper.key() @ SealError::WrongTokenAccount,
        constraint = *swapper_edict.to_account_info().owner == edict_token_program.key() @ SealError::WrongTokenProgram
    )]
    pub swapper_edict: Box<InterfaceAccount<'info, TokenAccount>>,
    /// The swapper's associated wSOL account (created idempotently by the client in the same transaction).
    #[account(
        mut,
        address = config.swapper_wsol_ata @ SealError::WrongTokenAccount,
        constraint = swapper_wsol.mint == WSOL_MINT @ SealError::WrongTokenAccount,
        constraint = swapper_wsol.owner == swapper.key() @ SealError::WrongTokenAccount
    )]
    pub swapper_wsol: Box<Account<'info, SplTokenAccount>>,
    /// CHECK: the wSOL mint, a constant address.
    #[account(address = WSOL_MINT @ SealError::MintNotPinned)]
    pub wsol_mint: UncheckedAccount<'info>,
    /// CHECK: the pinned LaunchLab pool, read only: its status must say "migrated".
    #[account(address = config.ll_pool @ SealError::PoolNotPinned)]
    pub ll_pool: UncheckedAccount<'info>,
    /// CHECK: the pinned CPMM program (see `MintSealCurve.launchlab_program`).
    #[account(address = config.cpmm_program @ SealError::WrongVenueProgram, executable)]
    pub cpmm_program: UncheckedAccount<'info>,
    /// CHECK: CPMM's authority PDA, pinned at init.
    #[account(address = config.cpmm_authority @ SealError::WrongAuthority)]
    pub cpmm_authority: UncheckedAccount<'info>,
    /// CHECK: the pinned CPMM config.
    #[account(address = config.cpmm_config @ SealError::PoolNotPinned)]
    pub cpmm_config: UncheckedAccount<'info>,
    /// CHECK: the DERIVED CPMM pool of $EDICT; it must exist and belong to the CPMM program.
    #[account(
        mut,
        address = config.cpmm_pool @ SealError::PoolNotPinned,
        owner = config.cpmm_program @ SealError::PoolNotMigrated
    )]
    pub pool: UncheckedAccount<'info>,
    /// CHECK: the derived wSOL vault (the input vault).
    #[account(mut, address = config.cpmm_vault_wsol @ SealError::VaultNotPinned)]
    pub vault_wsol: UncheckedAccount<'info>,
    /// CHECK: the derived $EDICT vault (the output vault).
    #[account(mut, address = config.cpmm_vault_edict @ SealError::VaultNotPinned)]
    pub vault_edict: UncheckedAccount<'info>,
    /// CHECK: the derived observation account of the pool.
    #[account(mut, address = config.cpmm_observation @ SealError::PoolNotPinned)]
    pub observation: UncheckedAccount<'info>,
    /// CHECK: the pinned Core collection.
    #[account(mut, address = config.collection @ SealError::WrongCollection)]
    pub collection: UncheckedAccount<'info>,
    /// The new Seal asset: a fresh keypair that signs the transaction.
    #[account(mut)]
    pub asset: Signer<'info>,
    #[account(
        init,
        payer = buyer,
        space = 8 + Seal::INIT_SPACE,
        seeds = [SEAL_SEED, asset.key().as_ref()],
        bump
    )]
    pub seal: Box<Account<'info, Seal>>,
    /// CHECK: the Metaplex Core program, a constant address.
    #[account(address = crate::core_cpi::CORE_ID @ SealError::WrongCoreProgram)]
    pub core_program: UncheckedAccount<'info>,
    #[account(address = config.edict_token_program @ SealError::WrongTokenProgram)]
    pub edict_token_program: Interface<'info, TokenInterface>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_mint_seal_cpmm(ctx: Context<MintSealCpmm>, min_out: u64) -> Result<()> {
    let seal_bump = ctx.bumps.seal;
    let a = &mut *ctx.accounts;
    // only the pool LaunchLab's migration created: the curve says "migrated" (a pool at the derived address made by anyone
    // before the graduation is never traded)
    require!(
        launch_pool_status(&a.ll_pool.try_borrow_data()?)? == LL_STATUS_MIGRATED,
        SealError::PoolNotMigrated
    );
    // the account list of CPMM `swap_base_input` (raydium-sdk-v2 0.2.85-alpha `makeSwapCpmmBaseInInstruction`): wSOL in,
    // $EDICT out, `payer` is the swapper PDA, the only signer
    let swapper = a.swapper.key();
    let metas = vec![
        AccountMeta::new_readonly(swapper, true), // payer
        AccountMeta::new_readonly(a.cpmm_authority.key(), false),
        AccountMeta::new_readonly(a.cpmm_config.key(), false),
        AccountMeta::new(a.pool.key(), false),
        AccountMeta::new(a.swapper_wsol.key(), false),  // input token account
        AccountMeta::new(a.swapper_edict.key(), false), // output token account
        AccountMeta::new(a.vault_wsol.key(), false),    // input vault
        AccountMeta::new(a.vault_edict.key(), false),   // output vault
        AccountMeta::new_readonly(a.token_program.key(), false), // input token program
        AccountMeta::new_readonly(a.edict_token_program.key(), false), // output token program
        AccountMeta::new_readonly(WSOL_MINT, false),          // input mint
        AccountMeta::new_readonly(a.edict_mint.key(), false), // output mint
        AccountMeta::new(a.observation.key(), false),
    ];
    let infos = vec![
        a.swapper.to_account_info(),
        a.cpmm_authority.to_account_info(),
        a.cpmm_config.to_account_info(),
        a.pool.to_account_info(),
        a.swapper_wsol.to_account_info(),
        a.swapper_edict.to_account_info(),
        a.vault_wsol.to_account_info(),
        a.vault_edict.to_account_info(),
        a.token_program.to_account_info(),
        a.edict_token_program.to_account_info(),
        a.wsol_mint.to_account_info(),
        a.edict_mint.to_account_info(),
        a.observation.to_account_info(),
        a.cpmm_program.to_account_info(),
    ];
    let program = a.cpmm_program.key();
    let events = run_mint(
        MintAccounts {
            buyer: &a.buyer,
            config: &a.config,
            state: &mut a.state,
            pot: &a.pot,
            treasury: &a.treasury,
            swapper: &a.swapper,
            collection_authority: &a.collection_authority,
            edict_mint: &a.edict_mint,
            swapper_edict: &mut a.swapper_edict,
            swapper_wsol: &mut a.swapper_wsol,
            collection: &a.collection,
            asset: &a.asset,
            seal: &mut a.seal,
            core_program: &a.core_program,
            edict_token_program: &a.edict_token_program,
            token_program: &a.token_program,
            system_program: &a.system_program,
        },
        VenueCall {
            program,
            metas,
            infos,
            venue: VENUE_CPMM,
        },
        min_out,
        seal_bump,
    )?;
    if let Some(e) = events.synced {
        emit_cpi!(e);
    }
    emit_cpi!(events.minted);
    if let Some(e) = events.genesis {
        emit_cpi!(e);
    }
    Ok(())
}
