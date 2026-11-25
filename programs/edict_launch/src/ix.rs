//! Accounts and handlers. Foreign accounts are `UncheckedAccount`s checked by hand (owner, size, discriminator, PDA,
//! fields) against what `Config` and `TokenState` pin; nothing about a foreign program is taken from the caller.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::AccountMeta;
use anchor_lang::solana_program::program::{invoke, invoke_signed};

use crate::errors::LaunchError;
use crate::events::*;
use crate::ext::{self, *};
use crate::schedule::{excess_of, scheduled_rate, BASE_RATE, PROTOCOL_OPEN_RATE};
use crate::state::*;

// ---------------------------------------------------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitConfigArgs {
    pub admin: Pubkey,
    pub launchlab: Pubkey,
    pub cpmm: Pubkey,
    pub lock: Pubkey,
}

#[derive(Accounts)]
pub struct InitConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = 8 + Config::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Box<Account<'info, Config>>,

    /// CHECK: the LaunchLab global config: owner, discriminator and SOL quote checked in the handler.
    pub global_config: UncheckedAccount<'info>,
    /// CHECK: the CPMM config of graduated pools: owner and size checked in the handler.
    pub cpswap_config: UncheckedAccount<'info>,

    #[account(
        constraint = crate::loader_state::programdata_address(&this_program.to_account_info())? == Some(program_data.key())
            @ LaunchError::NotUpgradeAuthority
    )]
    pub this_program: Program<'info, crate::program::EdictLaunch>,

    #[account(
        constraint = program_data.upgrade_authority_address == Some(authority.key()) @ LaunchError::NotUpgradeAuthority
    )]
    pub program_data: Account<'info, crate::loader_state::ProgramDataAuthority>,

    pub system_program: Program<'info, System>,
}

pub fn handle_init_config(ctx: Context<InitConfig>, args: InitConfigArgs) -> Result<()> {
    let g = &ctx.accounts.global_config;
    require_keys_eq!(*g.owner, args.launchlab, LaunchError::BadForeignAccount);
    {
        let d = g.try_borrow_data()?;
        require!(d.len() > GLOBAL_CONFIG_QUOTE_MINT + 32 && d[..8] == ACC_GLOBAL_CONFIG, LaunchError::BadForeignAccount);
        require_keys_eq!(read_pubkey(&d, GLOBAL_CONFIG_QUOTE_MINT), WSOL_MINT, LaunchError::NotSolQuoted);
    }
    let c = &ctx.accounts.cpswap_config;
    require_keys_eq!(*c.owner, args.cpmm, LaunchError::BadForeignAccount);
    require!(c.data_len() == 236, LaunchError::BadForeignAccount);

    let config = &mut ctx.accounts.config;
    config.admin = args.admin;
    config.launchlab = args.launchlab;
    config.cpmm = args.cpmm;
    config.lock = args.lock;
    config.global_config = g.key();
    config.cpswap_config = c.key();
    config.protocol_mint = Pubkey::default();
    config.bump = ctx.bumps.config;
    Ok(())
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ LaunchError::NotAdmin)]
    pub config: Box<Account<'info, Config>>,
}

#[derive(Accounts)]
pub struct SetProtocolMint<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ LaunchError::NotAdmin)]
    pub config: Box<Account<'info, Config>>,
    /// CHECK: the token state PDA of the mint pinned now; it must not exist.
    #[account(seeds = [TOKEN_SEED, config.protocol_mint.as_ref()], bump)]
    pub current_state: UncheckedAccount<'info>,
}

pub fn handle_set_protocol_mint(ctx: Context<SetProtocolMint>, mint: Pubkey) -> Result<()> {
    if ctx.accounts.config.protocol_mint != Pubkey::default() {
        require!(ctx.accounts.current_state.data_is_empty(), LaunchError::ProtocolTokenExists);
    }
    ctx.accounts.config.protocol_mint = mint;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// launch: open_platform, (LaunchLab initialize, dev buy), seal_platform
// ---------------------------------------------------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct OpenArgs {
    pub vault: Pubkey,
}

/// The platform's public strings, the same for every Edict token platform (cosmetic; constants so the launch
/// transaction does not carry them).
pub const PLATFORM_NAME: &str = "Edict";
pub const PLATFORM_WEB: &str = "https://edict.ink";
pub const PLATFORM_IMG: &str = "https://cdn.edict.ink/brand/edict-mark.png";

#[event_cpi]
#[derive(Accounts)]
pub struct OpenPlatform<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// The new token's mint keypair: only the holder of the launch transaction can open its platform.
    pub mint: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(init, payer = payer, space = 8 + TokenState::INIT_SPACE, seeds = [TOKEN_SEED, mint.key().as_ref()], bump)]
    pub state: Box<Account<'info, TokenState>>,
    /// CHECK: `auth_t`, system-owned without data (checked).
    #[account(mut, seeds = [AUTH_SEED, mint.key().as_ref()], bump)]
    pub auth: UncheckedAccount<'info>,
    /// CHECK: `["platform_config", auth_t]` of LaunchLab, created by the CPI.
    #[account(mut)]
    pub platform_config: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.cpswap_config @ LaunchError::WrongAccount)]
    pub cpswap_config: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.global_config @ LaunchError::WrongAccount)]
    pub global_config: UncheckedAccount<'info>,
    /// CHECK: `["platform_allow_config", platform_config, global_config]` of LaunchLab, created by the CPI.
    #[account(mut)]
    pub allow_config: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.launchlab @ LaunchError::WrongAccount)]
    pub launchlab_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_open_platform(ctx: Context<OpenPlatform>, args: OpenArgs) -> Result<()> {
    let a = &ctx.accounts;
    let ll = a.config.launchlab;
    let auth = a.auth.key();
    require!(a.auth.data_is_empty() && *a.auth.owner == system_program::ID, LaunchError::WrongAccount);
    let (platform, _) = Pubkey::find_program_address(&[LL_PLATFORM_CONFIG_SEED, auth.as_ref()], &ll);
    require_keys_eq!(a.platform_config.key(), platform, LaunchError::WrongAccount);
    let (allow, _) =
        Pubkey::find_program_address(&[LL_ALLOW_CONFIG_SEED, platform.as_ref(), a.global_config.key.as_ref()], &ll);
    require_keys_eq!(a.allow_config.key(), allow, LaunchError::WrongAccount);

    let mint = a.mint.key();
    let protocol = a.config.protocol_mint != Pubkey::default() && mint == a.config.protocol_mint;
    let fee_rate = if protocol { PROTOCOL_OPEN_RATE } else { BASE_RATE };

    // auth_t pays the two LaunchLab accounts (LaunchLab makes the platform admin the payer); the allow-config rent comes
    // back to auth_t when `seal_platform` closes it, and `seal_platform` returns everything auth_t holds to its payer.
    let rent = Rent::get()?;
    let need = rent.minimum_balance(LL_PLATFORM_CONFIG_LEN) + rent.minimum_balance(LL_ALLOW_CONFIG_LEN);
    let top_up = need.saturating_sub(a.auth.lamports());
    if top_up > 0 {
        invoke(
            &ext::system_transfer_ix(a.payer.key(), auth, top_up),
            &[a.payer.to_account_info(), a.auth.to_account_info(), a.system_program.to_account_info()],
        )?;
    }

    let bump = ctx.bumps.auth;
    let seeds: &[&[u8]] = &[AUTH_SEED, mint.as_ref(), &[bump]];
    let infos = [
        a.auth.to_account_info(),
        a.platform_config.to_account_info(),
        a.cpswap_config.to_account_info(),
        a.global_config.to_account_info(),
        a.allow_config.to_account_info(),
        a.system_program.to_account_info(),
        a.launchlab_program.to_account_info(),
    ];
    let cpswap = a.cpswap_config.key();
    let global = a.global_config.key();
    invoke_signed(
        &create_platform_config_ix(ll, auth, platform, cpswap, fee_rate, PLATFORM_NAME, PLATFORM_WEB, PLATFORM_IMG),
        &infos,
        &[seeds],
    )?;
    invoke_signed(&set_cp_creator_ix(ll, auth, platform, auth), &infos, &[seeds])?;
    invoke_signed(&restrict_global_config_ix(ll, auth, platform), &infos, &[seeds])?;
    invoke_signed(&create_allow_config_ix(ll, auth, platform, global, allow), &infos, &[seeds])?;

    let state = &mut ctx.accounts.state;
    state.mint = mint;
    state.vault = args.vault;
    state.platform_config = platform;
    state.pool = Pubkey::default();
    state.status = STATUS_OPEN;
    state.protocol = protocol as u8;
    state.auth_bump = bump;
    state.bump = ctx.bumps.state;
    state.fee_rate = fee_rate;
    state.created_at = 0;
    emit_cpi!(TokenOpened { mint, vault: args.vault, platform_config: platform, fee_rate, protocol });
    Ok(())
}

#[event_cpi]
#[derive(Accounts)]
pub struct SealPlatform<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [TOKEN_SEED, state.mint.as_ref()],
        bump = state.bump,
        constraint = state.status == STATUS_OPEN @ LaunchError::WrongStatus
    )]
    pub state: Box<Account<'info, TokenState>>,
    /// CHECK: `auth_t`.
    #[account(mut, seeds = [AUTH_SEED, state.mint.as_ref()], bump = state.auth_bump)]
    pub auth: UncheckedAccount<'info>,
    /// CHECK: the token's platform.
    #[account(address = state.platform_config @ LaunchError::WrongAccount)]
    pub platform_config: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.global_config @ LaunchError::WrongAccount)]
    pub global_config: UncheckedAccount<'info>,
    /// CHECK: PDA checked in the handler; closed by the CPI.
    #[account(mut)]
    pub allow_config: UncheckedAccount<'info>,
    /// CHECK: the LaunchLab pool of the mint: checked field by field in the handler.
    pub pool_state: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.launchlab @ LaunchError::WrongAccount)]
    pub launchlab_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_seal_platform(ctx: Context<SealPlatform>) -> Result<()> {
    let a = &ctx.accounts;
    let ll = a.config.launchlab;
    let mint = a.state.mint;
    let auth = a.auth.key();
    let platform = a.state.platform_config;
    let (allow, _) =
        Pubkey::find_program_address(&[LL_ALLOW_CONFIG_SEED, platform.as_ref(), a.global_config.key.as_ref()], &ll);
    require_keys_eq!(a.allow_config.key(), allow, LaunchError::WrongAccount);

    // the pool: LaunchLab's, at its PDA, on this platform and global config, creator auth_t, SOL quote, migrating to
    // CPMM with the creator fee on the quote token only, no locked vesting
    let (pool, _) = Pubkey::find_program_address(&[LL_POOL_SEED, mint.as_ref(), WSOL_MINT.as_ref()], &ll);
    let p = &a.pool_state;
    require_keys_eq!(p.key(), pool, LaunchError::PoolMismatch);
    require_keys_eq!(*p.owner, ll, LaunchError::PoolMismatch);
    {
        let d = p.try_borrow_data()?;
        require!(d.len() == ext::pool::LEN && d[..8] == ACC_POOL_STATE, LaunchError::PoolMismatch);
        require!(
            read_pubkey(&d, ext::pool::PLATFORM_CONFIG) == platform
                && read_pubkey(&d, ext::pool::GLOBAL_CONFIG) == a.config.global_config
                && read_pubkey(&d, ext::pool::BASE_MINT) == mint
                && read_pubkey(&d, ext::pool::QUOTE_MINT) == WSOL_MINT
                && read_pubkey(&d, ext::pool::CREATOR) == auth,
            LaunchError::PoolMismatch
        );
        require!(
            d[ext::pool::MIGRATE_TYPE] == ext::pool::MIGRATE_CPMM
                && d[ext::pool::AMM_CREATOR_FEE_ON] == ext::pool::FEE_ON_QUOTE
                && read_u64(&d, ext::pool::VESTING_TOTAL_LOCKED) == 0,
            LaunchError::PoolSettings
        );
    }

    let seeds: &[&[u8]] = &[AUTH_SEED, mint.as_ref(), &[a.state.auth_bump]];
    invoke_signed(
        &close_allow_config_ix(ll, auth, platform, a.global_config.key(), allow),
        &[
            a.auth.to_account_info(),
            a.platform_config.to_account_info(),
            a.global_config.to_account_info(),
            a.allow_config.to_account_info(),
            a.launchlab_program.to_account_info(),
        ],
        &[seeds],
    )?;
    // auth_t keeps nothing: the allow-config refund and any top-up left go back to the payer
    let left = a.auth.lamports();
    if left > 0 {
        invoke_signed(
            &ext::system_transfer_ix(auth, a.payer.key(), left),
            &[a.auth.to_account_info(), a.payer.to_account_info(), a.system_program.to_account_info()],
            &[seeds],
        )?;
    }

    let now = Clock::get()?.unix_timestamp;
    let state = &mut ctx.accounts.state;
    state.pool = pool;
    state.status = STATUS_LIVE;
    state.created_at = now;
    emit_cpi!(TokenSealed { mint, pool, created_at: now });
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// claims
// ---------------------------------------------------------------------------------------------------------------------

/// The token vault's account for `mint`: an initialised token account of that mint owned by the vault.
fn check_vault_account(ai: &AccountInfo, vault: &Pubkey, mint: &Pubkey) -> Result<u64> {
    let t = read_token_account(ai).ok_or(LaunchError::WrongVaultAccount)?;
    require!(t.owner == *vault && t.mint == *mint, LaunchError::WrongVaultAccount);
    Ok(t.amount)
}

/// The balance of a token account that may not exist yet (0 then).
fn balance_or_zero(ai: &AccountInfo) -> Result<u64> {
    if ai.data_is_empty() {
        return Ok(0);
    }
    Ok(read_token_account(ai).ok_or(LaunchError::BadForeignAccount)?.amount)
}

/// Moves exactly `amount` of wSOL from auth_t's account to the vault's and checks it arrived.
fn forward<'info>(
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    auth: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    seeds: &[&[u8]],
    amount: u64,
) -> Result<()> {
    let before = read_token_account(to).ok_or(LaunchError::WrongVaultAccount)?.amount;
    invoke_signed(
        &token_transfer_ix(from.key(), to.key(), auth.key(), amount),
        &[from.clone(), to.clone(), auth.clone(), token_program.clone()],
        &[seeds],
    )?;
    let after = read_token_account(to).ok_or(LaunchError::WrongVaultAccount)?.amount;
    require!(after.checked_sub(before) == Some(amount), LaunchError::ForwardMismatch);
    Ok(())
}

#[event_cpi]
#[derive(Accounts)]
pub struct ClaimPlatformFee<'info> {
    /// Pays the transaction only; never holds funds.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [TOKEN_SEED, state.mint.as_ref()],
        bump = state.bump,
        constraint = state.status == STATUS_LIVE @ LaunchError::WrongStatus
    )]
    pub state: Box<Account<'info, TokenState>>,
    /// CHECK: `auth_t`, LaunchLab's `platform_fee_wallet`.
    #[account(mut, seeds = [AUTH_SEED, state.mint.as_ref()], bump = state.auth_bump)]
    pub auth: UncheckedAccount<'info>,
    /// CHECK: LaunchLab's platform fee vault authority PDA (checked).
    pub fee_vault_authority: UncheckedAccount<'info>,
    /// CHECK: the token's platform.
    #[account(mut, address = state.platform_config @ LaunchError::WrongAccount)]
    pub platform_config: UncheckedAccount<'info>,
    /// CHECK: `[platform_config, wSOL]` of LaunchLab (checked).
    #[account(mut)]
    pub platform_fee_vault: UncheckedAccount<'info>,
    /// CHECK: the wSOL ATA of auth_t (checked); it must exist.
    #[account(mut)]
    pub auth_quote: UncheckedAccount<'info>,
    /// CHECK: the vault's wSOL account (checked).
    #[account(mut)]
    pub vault_quote: UncheckedAccount<'info>,
    /// CHECK: wSOL.
    #[account(address = WSOL_MINT @ LaunchError::WrongAccount)]
    pub quote_mint: UncheckedAccount<'info>,
    /// CHECK: classic Token.
    #[account(address = TOKEN_PROGRAM_ID @ LaunchError::WrongAccount)]
    pub token_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    /// CHECK: the ATA program.
    #[account(address = ATA_PROGRAM_ID @ LaunchError::WrongAccount)]
    pub associated_token_program: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.launchlab @ LaunchError::WrongAccount)]
    pub launchlab_program: UncheckedAccount<'info>,
}

/// Claim + forward. Returns (quote forwarded, its excess, the rate it was earned at); (0, 0, rate) when the fee vault is
/// empty or does not exist yet.
fn platform_claim(a: &mut ClaimPlatformFee) -> Result<(u64, u64, u64)> {
    let ll = a.config.launchlab;
    let mint = a.state.mint;
    let auth = a.auth.key();
    let rate = a.state.fee_rate;
    let (fva, _) = Pubkey::find_program_address(&[LL_FEE_VAULT_AUTH_SEED], &ll);
    require_keys_eq!(a.fee_vault_authority.key(), fva, LaunchError::WrongAccount);
    let (pfv, _) = Pubkey::find_program_address(&[a.state.platform_config.as_ref(), WSOL_MINT.as_ref()], &ll);
    require_keys_eq!(a.platform_fee_vault.key(), pfv, LaunchError::WrongAccount);
    require_keys_eq!(a.auth_quote.key(), ata_of(&auth, &WSOL_MINT, &TOKEN_PROGRAM_ID), LaunchError::WrongAccount);
    check_vault_account(&a.vault_quote, &a.state.vault, &WSOL_MINT)?;

    let held = balance_or_zero(&a.platform_fee_vault)?;
    if held == 0 {
        return Ok((0, 0, rate));
    }
    let before = read_token_account(&a.auth_quote).ok_or(LaunchError::WrongAccount)?.amount;
    let seeds: &[&[u8]] = &[AUTH_SEED, mint.as_ref(), &[a.state.auth_bump]];
    invoke_signed(
        &claim_platform_fee_from_vault_ix(ll, auth, fva, a.state.platform_config, pfv, a.auth_quote.key()),
        &[
            a.auth.to_account_info(),
            a.fee_vault_authority.to_account_info(),
            a.platform_config.to_account_info(),
            a.platform_fee_vault.to_account_info(),
            a.auth_quote.to_account_info(),
            a.quote_mint.to_account_info(),
            a.token_program.to_account_info(),
            a.system_program.to_account_info(),
            a.associated_token_program.to_account_info(),
            a.launchlab_program.to_account_info(),
        ],
        &[seeds],
    )?;
    let after = read_token_account(&a.auth_quote).ok_or(LaunchError::WrongAccount)?.amount;
    let delta = after.checked_sub(before).ok_or(LaunchError::ClaimMismatch)?;
    require!(delta == held && balance_or_zero(&a.platform_fee_vault)? == 0, LaunchError::ClaimMismatch);

    forward(
        &a.auth_quote.to_account_info(),
        &a.vault_quote.to_account_info(),
        &a.auth.to_account_info(),
        &a.token_program.to_account_info(),
        seeds,
        delta,
    )?;
    let excess = excess_of(delta, rate);
    let s = &mut a.state;
    s.platform_quote = s.platform_quote.checked_add(delta).ok_or(LaunchError::Overflow)?;
    s.excess_quote = s.excess_quote.checked_add(excess).ok_or(LaunchError::Overflow)?;
    s.seq = s.seq.checked_add(1).ok_or(LaunchError::Overflow)?;
    Ok((delta, excess, rate))
}

pub fn handle_claim_platform_fee(ctx: Context<ClaimPlatformFee>) -> Result<()> {
    let (quote, excess, rate) = platform_claim(&mut *ctx.accounts)?;
    if quote > 0 {
        let (mint, seq) = (ctx.accounts.state.mint, ctx.accounts.state.seq);
        emit_cpi!(FeeClaimed { mint, source: SOURCE_PLATFORM, quote, base: 0, rate, excess, seq });
    }
    Ok(())
}

pub fn handle_fee_step(ctx: Context<ClaimPlatformFee>) -> Result<()> {
    let from = ctx.accounts.state.fee_rate;
    require!(ctx.accounts.state.protocol == 1 && from > BASE_RATE, LaunchError::NoSchedule);
    let now = Clock::get()?.unix_timestamp;
    let elapsed = now.saturating_sub(ctx.accounts.state.created_at);
    let to = scheduled_rate(elapsed);
    require!(to < from, LaunchError::StepTooEarly);

    // claim first: everything in the fee vault was earned at `from`
    let (quote, excess, rate) = platform_claim(&mut *ctx.accounts)?;
    let mint = ctx.accounts.state.mint;
    if quote > 0 {
        let seq = ctx.accounts.state.seq;
        emit_cpi!(FeeClaimed { mint, source: SOURCE_PLATFORM, quote, base: 0, rate, excess, seq });
    }

    let a = &ctx.accounts;
    let seeds: &[&[u8]] = &[AUTH_SEED, mint.as_ref(), &[a.state.auth_bump]];
    invoke_signed(
        &set_fee_rate_ix(a.config.launchlab, a.auth.key(), a.state.platform_config, to),
        &[a.auth.to_account_info(), a.platform_config.to_account_info(), a.launchlab_program.to_account_info()],
        &[seeds],
    )?;
    ctx.accounts.state.fee_rate = to;
    emit_cpi!(FeeStepped { mint, from, to, elapsed, at: now });
    Ok(())
}

/// The graduated CPMM pool of the token: CPMM's, creator auth_t, one side wSOL and the other the token, on the pinned
/// config. Returns (quote index 0 or 1, the two creator fee counters).
fn check_cpmm_pool(pool: &AccountInfo, cpmm: &Pubkey, auth: &Pubkey, mint: &Pubkey, amm_config: &Pubkey) -> Result<(usize, u64, u64)> {
    require_keys_eq!(*pool.owner, *cpmm, LaunchError::PoolMismatch);
    let d = pool.try_borrow_data()?;
    require!(d.len() == cpmm_pool::LEN && d[..8] == ACC_CPMM_POOL, LaunchError::PoolMismatch);
    require!(
        read_pubkey(&d, cpmm_pool::POOL_CREATOR) == *auth && read_pubkey(&d, cpmm_pool::AMM_CONFIG) == *amm_config,
        LaunchError::PoolMismatch
    );
    let (m0, m1) = (read_pubkey(&d, cpmm_pool::TOKEN_0_MINT), read_pubkey(&d, cpmm_pool::TOKEN_1_MINT));
    let q = if m0 == WSOL_MINT && m1 == *mint {
        0
    } else if m1 == WSOL_MINT && m0 == *mint {
        1
    } else {
        return err!(LaunchError::PoolMismatch);
    };
    Ok((q, read_u64(&d, cpmm_pool::CREATOR_FEES_0), read_u64(&d, cpmm_pool::CREATOR_FEES_1)))
}

#[event_cpi]
#[derive(Accounts)]
pub struct ClaimCpmmCreatorFee<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [TOKEN_SEED, state.mint.as_ref()],
        bump = state.bump,
        constraint = state.status == STATUS_LIVE @ LaunchError::WrongStatus
    )]
    pub state: Box<Account<'info, TokenState>>,
    /// CHECK: `auth_t`, the pool creator.
    #[account(seeds = [AUTH_SEED, state.mint.as_ref()], bump = state.auth_bump)]
    pub auth: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.cpmm @ LaunchError::WrongAccount)]
    pub cpmm_program: UncheckedAccount<'info>,
    /// CHECK: CPMM's vault authority (CPMM checks the seeds).
    pub cpmm_authority: UncheckedAccount<'info>,
    /// CHECK: checked in the handler.
    #[account(mut)]
    pub pool: UncheckedAccount<'info>,
    /// CHECK: CPMM checks them against the pool.
    #[account(mut)]
    pub token_0_vault: UncheckedAccount<'info>,
    /// CHECK: as above.
    #[account(mut)]
    pub token_1_vault: UncheckedAccount<'info>,
    /// CHECK: as above.
    pub vault_0_mint: UncheckedAccount<'info>,
    /// CHECK: as above.
    pub vault_1_mint: UncheckedAccount<'info>,
    /// CHECK: auth_t's ATA of token 0 (CPMM checks it and creates it if missing).
    #[account(mut)]
    pub creator_token_0: UncheckedAccount<'info>,
    /// CHECK: auth_t's ATA of token 1.
    #[account(mut)]
    pub creator_token_1: UncheckedAccount<'info>,
    /// CHECK: CPMM checks it.
    pub token_0_program: UncheckedAccount<'info>,
    /// CHECK: CPMM checks it.
    pub token_1_program: UncheckedAccount<'info>,
    /// CHECK: the ATA program.
    #[account(address = ATA_PROGRAM_ID @ LaunchError::WrongAccount)]
    pub associated_token_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    /// CHECK: pinned.
    #[account(address = config.cpswap_config @ LaunchError::WrongAccount)]
    pub amm_config: UncheckedAccount<'info>,
    /// CHECK: CPMM checks the seeds (it need not exist).
    pub creator_fee_share: UncheckedAccount<'info>,
    /// CHECK: the vault's wSOL account (checked).
    #[account(mut)]
    pub vault_quote: UncheckedAccount<'info>,
    /// CHECK: classic Token, for the forward.
    #[account(address = TOKEN_PROGRAM_ID @ LaunchError::WrongAccount)]
    pub token_program: UncheckedAccount<'info>,
}

pub fn handle_claim_cpmm_creator_fee(ctx: Context<ClaimCpmmCreatorFee>) -> Result<()> {
    let a = &ctx.accounts;
    let mint = a.state.mint;
    let auth = a.auth.key();
    let (q, f0, f1) = check_cpmm_pool(&a.pool, &a.config.cpmm, &auth, &mint, &a.config.cpswap_config)?;
    if f0 == 0 && f1 == 0 {
        return Ok(());
    }
    check_vault_account(&a.vault_quote, &a.state.vault, &WSOL_MINT)?;
    let (cq, cb) = if q == 0 { (&a.creator_token_0, &a.creator_token_1) } else { (&a.creator_token_1, &a.creator_token_0) };
    require_keys_eq!(cq.key(), ata_of(&auth, &WSOL_MINT, &TOKEN_PROGRAM_ID), LaunchError::WrongAccount);
    let (q0, b0) = (balance_or_zero(cq)?, balance_or_zero(cb)?);

    let metas = vec![
        AccountMeta::new_readonly(auth, false),
        AccountMeta::new_readonly(a.cpmm_authority.key(), false),
        AccountMeta::new(a.pool.key(), false),
        AccountMeta::new(a.token_0_vault.key(), false),
        AccountMeta::new(a.token_1_vault.key(), false),
        AccountMeta::new_readonly(a.vault_0_mint.key(), false),
        AccountMeta::new_readonly(a.vault_1_mint.key(), false),
        AccountMeta::new(a.creator_token_0.key(), false),
        AccountMeta::new(a.creator_token_1.key(), false),
        AccountMeta::new_readonly(a.token_0_program.key(), false),
        AccountMeta::new_readonly(a.token_1_program.key(), false),
        AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(a.amm_config.key(), false),
        AccountMeta::new_readonly(a.creator_fee_share.key(), false),
    ];
    invoke(
        &collect_creator_fee_ix(a.config.cpmm, a.payer.key(), metas),
        &[
            a.payer.to_account_info(),
            a.auth.to_account_info(),
            a.cpmm_authority.to_account_info(),
            a.pool.to_account_info(),
            a.token_0_vault.to_account_info(),
            a.token_1_vault.to_account_info(),
            a.vault_0_mint.to_account_info(),
            a.vault_1_mint.to_account_info(),
            a.creator_token_0.to_account_info(),
            a.creator_token_1.to_account_info(),
            a.token_0_program.to_account_info(),
            a.token_1_program.to_account_info(),
            a.associated_token_program.to_account_info(),
            a.system_program.to_account_info(),
            a.amm_config.to_account_info(),
            a.creator_fee_share.to_account_info(),
            a.cpmm_program.to_account_info(),
        ],
    )?;
    let dq = balance_or_zero(cq)?.checked_sub(q0).ok_or(LaunchError::ClaimMismatch)?;
    let db = balance_or_zero(cb)?.checked_sub(b0).ok_or(LaunchError::ClaimMismatch)?;
    require!(db == 0, LaunchError::BaseCreatorFee);
    if dq == 0 {
        return Ok(());
    }
    let seeds: &[&[u8]] = &[AUTH_SEED, mint.as_ref(), &[a.state.auth_bump]];
    forward(&cq.to_account_info(), &a.vault_quote.to_account_info(), &a.auth.to_account_info(), &a.token_program.to_account_info(), seeds, dq)?;

    let s = &mut ctx.accounts.state;
    s.cpmm_creator_quote = s.cpmm_creator_quote.checked_add(dq).ok_or(LaunchError::Overflow)?;
    s.seq = s.seq.checked_add(1).ok_or(LaunchError::Overflow)?;
    let seq = s.seq;
    emit_cpi!(FeeClaimed { mint, source: SOURCE_CPMM_CREATOR, quote: dq, base: 0, rate: 0, excess: 0, seq });
    Ok(())
}

#[event_cpi]
#[derive(Accounts)]
pub struct ClaimFeeKey<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [TOKEN_SEED, state.mint.as_ref()],
        bump = state.bump,
        constraint = state.status == STATUS_LIVE @ LaunchError::WrongStatus
    )]
    pub state: Box<Account<'info, TokenState>>,
    /// CHECK: `auth_t`, the Fee Key holder (signs the collection).
    #[account(seeds = [AUTH_SEED, state.mint.as_ref()], bump = state.auth_bump)]
    pub auth: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.lock @ LaunchError::WrongAccount)]
    pub lock_program: UncheckedAccount<'info>,
    /// CHECK: the lock program checks the rest.
    pub lock_authority: UncheckedAccount<'info>,
    /// CHECK: auth_t's Fee Key NFT account (checked).
    pub fee_nft_account: UncheckedAccount<'info>,
    /// CHECK: lock program.
    #[account(mut)]
    pub locked_liquidity: UncheckedAccount<'info>,
    /// CHECK: pinned.
    #[account(address = config.cpmm @ LaunchError::WrongAccount)]
    pub cpmm_program: UncheckedAccount<'info>,
    /// CHECK: lock program / CPMM.
    pub cpmm_authority: UncheckedAccount<'info>,
    /// CHECK: checked in the handler.
    #[account(mut)]
    pub pool: UncheckedAccount<'info>,
    /// CHECK: lock program / CPMM.
    #[account(mut)]
    pub lp_mint: UncheckedAccount<'info>,
    /// CHECK: the vault's account of token 0 (checked).
    #[account(mut)]
    pub recipient_0: UncheckedAccount<'info>,
    /// CHECK: the vault's account of token 1 (checked).
    #[account(mut)]
    pub recipient_1: UncheckedAccount<'info>,
    /// CHECK: lock program / CPMM.
    #[account(mut)]
    pub token_0_vault: UncheckedAccount<'info>,
    /// CHECK: as above.
    #[account(mut)]
    pub token_1_vault: UncheckedAccount<'info>,
    /// CHECK: as above.
    pub vault_0_mint: UncheckedAccount<'info>,
    /// CHECK: as above.
    pub vault_1_mint: UncheckedAccount<'info>,
    /// CHECK: lock program.
    #[account(mut)]
    pub locked_lp_vault: UncheckedAccount<'info>,
    /// CHECK: lock program.
    pub token_program: UncheckedAccount<'info>,
    /// CHECK: lock program.
    pub token_program_2022: UncheckedAccount<'info>,
    /// CHECK: lock program.
    pub memo_program: UncheckedAccount<'info>,
}

pub fn handle_claim_fee_key(ctx: Context<ClaimFeeKey>, fee_lp_amount: u64) -> Result<()> {
    let a = &ctx.accounts;
    let mint = a.state.mint;
    let auth = a.auth.key();
    let (q, _, _) = check_cpmm_pool(&a.pool, &a.config.cpmm, &auth, &mint, &a.config.cpswap_config)?;
    let nft = read_token_account(&a.fee_nft_account).ok_or(LaunchError::WrongAccount)?;
    require!(nft.owner == auth && nft.amount == 1, LaunchError::WrongAccount);
    let (m0, m1) = if q == 0 { (WSOL_MINT, mint) } else { (mint, WSOL_MINT) };
    let r0 = check_vault_account(&a.recipient_0, &a.state.vault, &m0)?;
    let r1 = check_vault_account(&a.recipient_1, &a.state.vault, &m1)?;

    let metas = vec![
        AccountMeta::new_readonly(a.lock_authority.key(), false),
        AccountMeta::new_readonly(auth, true),
        AccountMeta::new_readonly(a.fee_nft_account.key(), false),
        AccountMeta::new(a.locked_liquidity.key(), false),
        AccountMeta::new_readonly(a.cpmm_program.key(), false),
        AccountMeta::new_readonly(a.cpmm_authority.key(), false),
        AccountMeta::new(a.pool.key(), false),
        AccountMeta::new(a.lp_mint.key(), false),
        AccountMeta::new(a.recipient_0.key(), false),
        AccountMeta::new(a.recipient_1.key(), false),
        AccountMeta::new(a.token_0_vault.key(), false),
        AccountMeta::new(a.token_1_vault.key(), false),
        AccountMeta::new_readonly(a.vault_0_mint.key(), false),
        AccountMeta::new_readonly(a.vault_1_mint.key(), false),
        AccountMeta::new(a.locked_lp_vault.key(), false),
        AccountMeta::new_readonly(a.token_program.key(), false),
        AccountMeta::new_readonly(a.token_program_2022.key(), false),
        AccountMeta::new_readonly(a.memo_program.key(), false),
    ];
    let seeds: &[&[u8]] = &[AUTH_SEED, mint.as_ref(), &[a.state.auth_bump]];
    invoke_signed(
        &collect_cp_fees_ix(a.config.lock, metas, fee_lp_amount),
        &[
            a.lock_authority.to_account_info(),
            a.auth.to_account_info(),
            a.fee_nft_account.to_account_info(),
            a.locked_liquidity.to_account_info(),
            a.cpmm_program.to_account_info(),
            a.cpmm_authority.to_account_info(),
            a.pool.to_account_info(),
            a.lp_mint.to_account_info(),
            a.recipient_0.to_account_info(),
            a.recipient_1.to_account_info(),
            a.token_0_vault.to_account_info(),
            a.token_1_vault.to_account_info(),
            a.vault_0_mint.to_account_info(),
            a.vault_1_mint.to_account_info(),
            a.locked_lp_vault.to_account_info(),
            a.token_program.to_account_info(),
            a.token_program_2022.to_account_info(),
            a.memo_program.to_account_info(),
            a.lock_program.to_account_info(),
        ],
        &[seeds],
    )?;
    let d0 = check_vault_account(&a.recipient_0, &a.state.vault, &m0)?.checked_sub(r0).ok_or(LaunchError::ClaimMismatch)?;
    let d1 = check_vault_account(&a.recipient_1, &a.state.vault, &m1)?.checked_sub(r1).ok_or(LaunchError::ClaimMismatch)?;
    let (quote, base) = if q == 0 { (d0, d1) } else { (d1, d0) };
    if quote == 0 && base == 0 {
        return Ok(());
    }
    let s = &mut ctx.accounts.state;
    s.fee_key_quote = s.fee_key_quote.checked_add(quote).ok_or(LaunchError::Overflow)?;
    s.fee_key_base = s.fee_key_base.checked_add(base).ok_or(LaunchError::Overflow)?;
    s.seq = s.seq.checked_add(1).ok_or(LaunchError::Overflow)?;
    let seq = s.seq;
    emit_cpi!(FeeClaimed { mint, source: SOURCE_FEE_KEY, quote, base, rate: 0, excess: 0, seq });
    Ok(())
}
