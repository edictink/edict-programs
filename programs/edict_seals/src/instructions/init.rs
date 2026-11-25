//! `init_config` and `init_collection`: the two setup instructions, signed only by the program's upgrade authority (so
//! nobody else can front-run the initialisation and install their own admin or collection).

use anchor_lang::prelude::*;
use anchor_spl::associated_token::get_associated_token_address_with_program_id;
use anchor_spl::token_interface::{Mint, TokenInterface};

use crate::constants::*;
use crate::core_cpi;
use crate::cpi_lite;
use crate::errors::SealError;
use crate::events::{CollectionCreated, ConfigInitialized};
use crate::math::{burn_tiers, tiers_within_bounds};
use crate::edict::validate_edict_mint;
use crate::state::{Config, Pending, State};
use crate::util::{pot_rent_min, valid_rule_set};
use crate::venue::{derive_cpmm, derive_launch, parse_launch_pool};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitConfigArgs {
    /// The admin key.
    pub admin: Pubkey,
    /// The owner wallet, the only signer of `mint_reserve`.
    pub reserve_signer: Pubkey,
    /// The Treasury, the owner of every team Seal.
    pub reserve_recipient: Pubkey,
    /// The Treasury, the payee of 40% of the payment and the 40% royalty creator.
    pub treasury: Pubkey,
    pub buyback_wallet: Pubkey,
    /// Raydium LaunchLab and CPMM of this cluster (they differ between mainnet and devnet).
    pub launchlab_program: Pubkey,
    pub cpmm_program: Pubkey,
    /// The fixed payment of a public mint (G-NFT-PRICE), 60% pot / 40% Treasury.
    pub payment_lamports: u64,
    pub base_lamports: u64,
    pub factor_bps: u32,
    pub min_burn_lamports: u64,
    pub max_burn_lamports: u64,
}

/// Creates `Config`, `State` and `Pending` and funds the pot vault to its rent minimum. Verifies on chain every account it
/// pins: the $EDICT mint and its LaunchLab pool (owner, layout, mint, quote, vaults, global and platform config), and derives
/// the rest of both venues (LaunchLab's authorities and fee vaults, the graduated CPMM pool from the platform's CPMM config).
#[event_cpi]
#[derive(Accounts)]
#[instruction(args: InitConfigArgs)]
pub struct InitConfig<'info> {
    /// The upgrade authority of this program. Pays the rent of the accounts and of the pot vault.
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = 8 + Config::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(init, payer = authority, space = 8 + State::INIT_SPACE, seeds = [STATE_SEED], bump)]
    pub state: Box<Account<'info, State>>,

    #[account(init, payer = authority, space = 8 + Pending::INIT_SPACE, seeds = [PENDING_SEED], bump)]
    pub pending: Box<Account<'info, Pending>>,

    /// The Seal pot vault: a system-owned PDA with no data, funded here to its rent minimum.
    #[account(mut, seeds = [POT_SEED], bump)]
    pub pot: SystemAccount<'info>,

    /// CHECK: the swapper PDA; only its bump and its associated token addresses are recorded.
    #[account(seeds = [SWAPPER_SEED], bump)]
    pub swapper: UncheckedAccount<'info>,

    /// CHECK: the collection authority PDA; only its bump is recorded.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub collection_authority: UncheckedAccount<'info>,

    pub edict_mint: Box<InterfaceAccount<'info, Mint>>,

    /// CHECK: $EDICT's LaunchLab pool, owned by the LaunchLab program, read at its pinned offsets.
    #[account(owner = args.launchlab_program @ SealError::WrongVenueProgram)]
    pub ll_pool: UncheckedAccount<'info>,

    /// CHECK: the pool's global config: owner, discriminator and SOL quote checked in the handler.
    #[account(owner = args.launchlab_program @ SealError::WrongVenueProgram)]
    pub ll_global_config: UncheckedAccount<'info>,

    /// CHECK: the pool's platform config (edict_launch's platform of $EDICT): owner, layout and its CPMM config read in the handler.
    #[account(owner = args.launchlab_program @ SealError::WrongVenueProgram)]
    pub ll_platform_config: UncheckedAccount<'info>,

    /// CHECK: the CPMM config the platform names for the graduated pool: owner and size checked in the handler.
    #[account(owner = args.cpmm_program @ SealError::WrongVenueProgram)]
    pub cpmm_config: UncheckedAccount<'info>,

    /// CHECK: the LaunchLab program (its key is the argument; it must be a program).
    #[account(address = args.launchlab_program @ SealError::WrongVenueProgram, executable)]
    pub launchlab_program: UncheckedAccount<'info>,

    /// CHECK: the CPMM program (its key is the argument; it must be a program).
    #[account(address = args.cpmm_program @ SealError::WrongVenueProgram, executable)]
    pub cpmm_program: UncheckedAccount<'info>,

    /// CHECK: the Treasury (the payee of 40% of the payment and of every team Seal): a system-owned account or one that does not
    /// exist yet, never a program account (review R5). Its key must equal the `treasury` argument.
    #[account(owner = system_program::ID @ SealError::InvalidConfigKey)]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: the buyback wallet: system-owned or not yet existing (review R5). Its key must equal the `buyback_wallet` argument.
    #[account(owner = system_program::ID @ SealError::InvalidConfigKey)]
    pub buyback_wallet: UncheckedAccount<'info>,

    #[account(
        constraint = crate::loader_state::programdata_address(&this_program.to_account_info())? == Some(program_data.key())
            @ SealError::NotUpgradeAuthority
    )]
    pub this_program: Program<'info, crate::program::EdictSeals>,

    #[account(
        constraint = program_data.upgrade_authority_address == Some(authority.key())
            @ SealError::NotUpgradeAuthority
    )]
    pub program_data: Account<'info, crate::loader_state::ProgramDataAuthority>,

    pub edict_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_init_config(ctx: Context<InitConfig>, args: InitConfigArgs) -> Result<()> {
    let a = &mut *ctx.accounts;
    let pot_key = a.pot.key();
    for key in [
        args.admin,
        args.reserve_signer,
        args.reserve_recipient,
        args.treasury,
        args.buyback_wallet,
    ] {
        require!(key != Pubkey::default(), SealError::InvalidConfigKey);
    }
    // the Treasury is both the payee and the owner of the team Seals (design 3.1: equal at init), and the three payee
    // accounts passed are the arguments (review R5): a payee that is a program account would make every mint fail
    require_keys_eq!(
        args.reserve_recipient,
        args.treasury,
        SealError::InvalidConfigKey
    );
    require_keys_eq!(
        a.treasury.key(),
        args.treasury,
        SealError::InvalidConfigKey
    );
    require_keys_eq!(
        a.buyback_wallet.key(),
        args.buyback_wallet,
        SealError::InvalidConfigKey
    );
    // a payee must never be a program PDA of this program
    for key in [
        args.reserve_recipient,
        args.treasury,
        args.buyback_wallet,
    ] {
        require!(
            key != pot_key
                && key != a.swapper.key()
                && key != a.collection_authority.key()
                && key != a.config.key()
                && key != a.state.key(),
            SealError::InvalidConfigKey
        );
    }

    // $EDICT: its token program owns it, no freeze authority, only metadata extensions
    require_keys_eq!(
        *a.edict_mint.to_account_info().owner,
        a.edict_token_program.key(),
        SealError::WrongTokenProgram
    );
    validate_edict_mint(&a.edict_mint)?;

    // the LaunchLab pins, read at their pinned offsets: $EDICT's own pool, SOL-quoted, on its global and platform config
    let pool = parse_launch_pool(&a.ll_pool.try_borrow_data()?)?;
    require_keys_eq!(pool.base_mint, a.edict_mint.key(), SealError::MintNotPinned);
    require_keys_eq!(pool.quote_mint, WSOL_MINT, SealError::MintNotPinned);
    require_keys_eq!(pool.global_config, a.ll_global_config.key(), SealError::PoolNotPinned);
    require_keys_eq!(pool.platform_config, a.ll_platform_config.key(), SealError::PoolNotPinned);
    {
        let g = a.ll_global_config.try_borrow_data()?;
        require!(
            g.len() >= LL_GLOBAL_CONFIG_QUOTE_MINT + 32 && g[..8] == ACC_GLOBAL_CONFIG,
            SealError::PinMismatch
        );
        require!(
            g[LL_GLOBAL_CONFIG_QUOTE_MINT..LL_GLOBAL_CONFIG_QUOTE_MINT + 32] == WSOL_MINT.to_bytes(),
            SealError::PinMismatch
        );
    }
    let cpswap_config = {
        let p = a.ll_platform_config.try_borrow_data()?;
        require!(
            p.len() == LL_PLATFORM_CONFIG_LEN && p[..8] == ACC_PLATFORM_CONFIG,
            SealError::PinMismatch
        );
        let bytes: [u8; 32] = p[LL_PLATFORM_CPSWAP_CONFIG..LL_PLATFORM_CPSWAP_CONFIG + 32]
            .try_into()
            .map_err(|_| error!(SealError::PinMismatch))?;
        Pubkey::new_from_array(bytes)
    };
    require_keys_eq!(cpswap_config, a.cpmm_config.key(), SealError::PoolNotPinned);
    require!(a.cpmm_config.data_len() == CPMM_CONFIG_LEN, SealError::PinMismatch);
    let ll = derive_launch(
        &args.launchlab_program,
        &a.edict_mint.key(),
        &pool.platform_config,
        &pool.creator,
    );
    require_keys_eq!(ll.pool, a.ll_pool.key(), SealError::PoolNotPinned);
    require_keys_eq!(ll.base_vault, pool.base_vault, SealError::VaultNotPinned);
    require_keys_eq!(ll.quote_vault, pool.quote_vault, SealError::VaultNotPinned);
    let cpmm = derive_cpmm(&args.cpmm_program, &cpswap_config, &a.edict_mint.key());

    // the fixed payment (G-NFT-PRICE)
    require!(
        (MIN_PAYMENT_LAMPORTS..=MAX_PAYMENT_LAMPORTS).contains(&args.payment_lamports),
        SealError::InvalidArgument
    );

    // the schedule: every tier inside the bounds, the factor not below 1.0
    require!(args.factor_bps >= 10_000, SealError::InvalidFactor);
    // the bounds stay inside 0.1 to 2.0 SOL (SECURITY, review R5): they cannot change after init
    require!(
        args.min_burn_lamports >= MIN_BURN_FLOOR
            && args.min_burn_lamports <= args.max_burn_lamports
            && args.max_burn_lamports <= MAX_BURN_CEIL,
        SealError::BurnOutOfBounds
    );
    let tiers = burn_tiers(args.base_lamports, args.factor_bps)
        .ok_or_else(|| error!(SealError::InvalidFactor))?;
    require!(
        tiers_within_bounds(&tiers, 0, args.min_burn_lamports, args.max_burn_lamports),
        SealError::BurnOutOfBounds
    );

    // the pot vault holds its rent minimum from the start, so a first, small royalty can never leave it rent-paying
    let rent_min = pot_rent_min()?;
    if a.pot.lamports() < rent_min {
        cpi_lite::system_transfer(
            &a.authority.to_account_info(),
            &a.pot.to_account_info(),
            rent_min - a.pot.lamports(),
            &[],
        )?;
    }

    let swapper_edict_ata = get_associated_token_address_with_program_id(
        &a.swapper.key(),
        &a.edict_mint.key(),
        &a.edict_token_program.key(),
    );
    let swapper_wsol_ata = get_associated_token_address_with_program_id(
        &a.swapper.key(),
        &WSOL_MINT,
        &anchor_spl::token::ID,
    );

    let config = &mut a.config;
    config.admin = args.admin;
    config.reserve_signer = args.reserve_signer;
    config.reserve_recipient = args.reserve_recipient;
    config.treasury = args.treasury;
    config.buyback_wallet = args.buyback_wallet;
    config.edict_mint = a.edict_mint.key();
    config.edict_token_program = a.edict_token_program.key();
    config.swapper_edict_ata = swapper_edict_ata;
    config.swapper_wsol_ata = swapper_wsol_ata;
    config.launchlab_program = args.launchlab_program;
    config.ll_authority = ll.authority;
    config.ll_event_authority = ll.event_authority;
    config.ll_global_config = pool.global_config;
    config.ll_platform_config = pool.platform_config;
    config.ll_pool = ll.pool;
    config.ll_base_vault = ll.base_vault;
    config.ll_quote_vault = ll.quote_vault;
    config.ll_platform_fee_vault = ll.platform_fee_vault;
    config.ll_creator_fee_vault = ll.creator_fee_vault;
    config.cpmm_program = args.cpmm_program;
    config.cpmm_authority = cpmm.authority;
    config.cpmm_config = cpswap_config;
    config.cpmm_pool = cpmm.pool;
    config.cpmm_vault_edict = cpmm.vault_edict;
    config.cpmm_vault_wsol = cpmm.vault_wsol;
    config.cpmm_observation = cpmm.observation;
    config.collection = Pubkey::default();
    config.art_hash = [0u8; 32];
    config.base_uri = String::new();
    config.paused = false;
    config.cancelled = false;
    config.team_set = false;
    config.art_locked = false;
    config.team_mask = [0u8; MASK_BYTES];
    config.payment_lamports = args.payment_lamports;
    config.base_lamports = args.base_lamports;
    config.factor_bps = args.factor_bps;
    config.min_burn_lamports = args.min_burn_lamports;
    config.max_burn_lamports = args.max_burn_lamports;
    config.burn_by_tier = tiers;
    config.config_bump = ctx.bumps.config;
    config.state_bump = ctx.bumps.state;
    config.pending_bump = ctx.bumps.pending;
    config.swapper_bump = ctx.bumps.swapper;
    config.authority_bump = ctx.bumps.collection_authority;
    config.pot_bump = ctx.bumps.pot;

    let state: &mut State = &mut a.state;
    *state = State::new(ctx.bumps.state);

    let pending = &mut a.pending;
    pending.pending_admin = Pubkey::default();
    pending.price_active = false;
    pending.price_base = 0;
    pending.price_factor_bps = 0;
    pending.price_eta = 0;
    pending.rules_active = false;
    pending.rules_eta = 0;
    pending.rules_count = 0;
    pending.rules = [Pubkey::default(); MAX_RULES];
    pending.bump = ctx.bumps.pending;

    let event = ConfigInitialized {
        admin: config.admin,
        reserve_signer: config.reserve_signer,
        reserve_recipient: config.reserve_recipient,
        treasury: config.treasury,
        buyback_wallet: config.buyback_wallet,
        edict_mint: config.edict_mint,
        ll_pool: config.ll_pool,
        cpmm_pool: config.cpmm_pool,
        payment_lamports: config.payment_lamports,
        base_lamports: config.base_lamports,
        factor_bps: config.factor_bps,
        burn_by_tier: config.burn_by_tier,
    };
    emit_cpi!(event);
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitCollectionArgs {
    /// The collection's own metadata URI.
    pub uri: String,
    /// The royalty allow list: the System Program plus the verified marketplaces (design D3), at most 8.
    pub rules: Vec<Pubkey>,
}

/// Creates the Core collection (once) with the Royalties plugin: 500 bps, the pot vault 60 / the Treasury 40, and the
/// allow list; update authority = the `["authority"]` PDA.
#[event_cpi]
#[derive(Accounts)]
pub struct InitCollection<'info> {
    /// The upgrade authority of this program. Pays the collection's rent.
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.config_bump)]
    pub config: Box<Account<'info, Config>>,

    /// The new collection: a fresh keypair that signs the transaction.
    #[account(mut)]
    pub collection: Signer<'info>,

    /// CHECK: the collection authority PDA, the update authority of the collection.
    #[account(seeds = [AUTHORITY_SEED], bump = config.authority_bump)]
    pub collection_authority: UncheckedAccount<'info>,

    /// CHECK: the pot vault PDA, the first royalty creator.
    #[account(seeds = [POT_SEED], bump = config.pot_bump)]
    pub pot: UncheckedAccount<'info>,

    /// CHECK: the Metaplex Core program, a constant address.
    #[account(address = crate::core_cpi::CORE_ID @ SealError::WrongCoreProgram)]
    pub core_program: UncheckedAccount<'info>,

    #[account(
        constraint = crate::loader_state::programdata_address(&this_program.to_account_info())? == Some(program_data.key())
            @ SealError::NotUpgradeAuthority
    )]
    pub this_program: Program<'info, crate::program::EdictSeals>,

    #[account(
        constraint = program_data.upgrade_authority_address == Some(authority.key())
            @ SealError::NotUpgradeAuthority
    )]
    pub program_data: Account<'info, crate::loader_state::ProgramDataAuthority>,

    pub system_program: Program<'info, System>,
}

pub fn handle_init_collection(
    ctx: Context<InitCollection>,
    args: InitCollectionArgs,
) -> Result<()> {
    let a = &mut *ctx.accounts;
    require_keys_eq!(
        a.config.collection,
        Pubkey::default(),
        SealError::CollectionAlreadySet
    );
    require!(valid_rule_set(&args.rules), SealError::InvalidRuleSet);
    require!(args.uri.len() <= 200, SealError::InvalidUri);
    let rules_len =
        u8::try_from(args.rules.len()).map_err(|_| error!(SealError::InvalidRuleSet))?;
    let royalties = core_cpi::royalties(a.pot.key(), a.config.treasury, args.rules);
    core_cpi::create_collection(
        &a.core_program.to_account_info(),
        &a.collection.to_account_info(),
        &a.collection_authority.to_account_info(),
        &a.authority.to_account_info(),
        &a.system_program.to_account_info(),
        args.uri,
        royalties,
    )?;
    a.config.collection = a.collection.key();
    let event = CollectionCreated {
        collection: a.collection.key(),
        royalty_bps: ROYALTY_BPS,
        pot_creator: a.pot.key(),
        treasury_creator: a.config.treasury,
        rule_set_len: rules_len,
    };
    emit_cpi!(event);
    Ok(())
}
