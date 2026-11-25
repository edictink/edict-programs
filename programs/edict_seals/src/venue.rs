//! The two venues of $EDICT (EDICT_START 3.3, docs/validation/SEALS_VENUE.md): Raydium LaunchLab's curve before the
//! graduation and the CPMM pool the migration creates after it. The swap flow of a mint runs by CPI with the swapper PDA as
//! signer; the typed read of the LaunchLab pool and the derivation of the CPMM accounts serve `init_config`, which pins
//! every venue account in `Config`. Nothing about a venue is ever taken from the caller.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke_signed;
use anchor_spl::token::{Token, TokenAccount as SplTokenAccount};
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::*;
use crate::cpi_lite;
use crate::errors::SealError;

// ---------------------------------------------------------------------------------------------------------------------
// the swap of a mint (P2-00 `run_flow`)
// ---------------------------------------------------------------------------------------------------------------------

/// What the swap did, all measured by balance difference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapOutcome {
    /// wSOL lamports the venue took.
    pub wsol_spent: u64,
    /// $EDICT units received; all of them are burned.
    pub received: u64,
    /// Lamports of the wSOL account that went back to the buyer when it was closed.
    pub refunded_lamports: u64,
}

/// The instruction data of a venue's exact-in swap: LaunchLab `buy_exact_in(amount_in, minimum_amount_out,
/// share_fee_rate = 0)`, CPMM `swap_base_input(amount_in, minimum_amount_out)`.
pub fn swap_data(venue: u8, amount_in: u64, min_out: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(8 + 8 + 8 + 8);
    if venue == VENUE_CURVE {
        data.extend_from_slice(&LL_BUY_EXACT_IN);
    } else {
        data.extend_from_slice(&CPMM_SWAP_BASE_INPUT);
    }
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    if venue == VENUE_CURVE {
        data.extend_from_slice(&0u64.to_le_bytes());
    }
    data
}

/// Invokes the venue with the swapper PDA as the signer.
pub fn swap_cpi<'info>(
    program: &Pubkey,
    metas: Vec<AccountMeta>,
    infos: &[AccountInfo<'info>],
    data: Vec<u8>,
    swapper_bump: u8,
) -> Result<()> {
    let ix = Instruction {
        program_id: *program,
        accounts: metas,
        data,
    };
    let seeds: &[&[u8]] = &[SWAPPER_SEED, &[swapper_bump]];
    invoke_signed(&ix, infos, &[seeds])?;
    Ok(())
}

/// The accounts of the flow that do not depend on the venue.
pub struct FlowAccounts<'a, 'info> {
    pub buyer: &'a Signer<'info>,
    pub swapper: &'a UncheckedAccount<'info>,
    pub swapper_bump: u8,
    pub swapper_edict: &'a mut InterfaceAccount<'info, TokenAccount>,
    pub swapper_wsol: &'a mut Account<'info, SplTokenAccount>,
    pub edict_mint: &'a InterfaceAccount<'info, Mint>,
    pub edict_token_program: &'a Interface<'info, TokenInterface>,
    pub token_program: &'a Program<'info, Token>,
    pub system_program: &'a Program<'info, System>,
}

/// The mint-shaped swap (the P2-00 flow): (1) the buyer pays `burn_lamports` into the swapper's wSOL account and the
/// balance is synced; (2) the swapper PDA signs a CPI to the venue's exact-in swap; (3) the received $EDICT is the
/// difference of the swapper's balance, above zero and at least `min_out`, and the wSOL spent is above zero and at most
/// `burn_lamports`; (4) exactly that difference is burned and the balance is back to what it was; (5) the wSOL account is
/// closed to the buyer (what the venue did not take, e.g. at the end of the curve, and the rent return in the same transaction).
pub fn buy_and_burn<'a, 'info>(
    f: FlowAccounts<'a, 'info>,
    venue_program: &Pubkey,
    venue: u8,
    burn_lamports: u64,
    min_out: u64,
    metas: Vec<AccountMeta>,
    infos: &[AccountInfo<'info>],
) -> Result<SwapOutcome> {
    require!(burn_lamports > 0 && min_out > 0, SealError::ZeroAmount);
    require!(
        venue == VENUE_CURVE || venue == VENUE_CPMM,
        SealError::InvariantBroken
    );

    cpi_lite::system_transfer(
        &f.buyer.to_account_info(),
        &f.swapper_wsol.to_account_info(),
        burn_lamports,
        &[],
    )?;
    cpi_lite::token_sync_native(&f.swapper_wsol.to_account_info())?;
    f.swapper_wsol.reload()?;
    f.swapper_edict.reload()?;
    let wsol_before = f.swapper_wsol.amount;
    let edict_before = f.swapper_edict.amount;

    swap_cpi(
        venue_program,
        metas,
        infos,
        swap_data(venue, burn_lamports, min_out),
        f.swapper_bump,
    )?;

    f.swapper_edict.reload()?;
    f.swapper_wsol.reload()?;
    let received = f
        .swapper_edict
        .amount
        .checked_sub(edict_before)
        .ok_or_else(|| error!(SealError::MathOverflow))?;
    require!(received > 0, SealError::NothingReceived);
    require!(received >= min_out, SealError::SlippageExceeded);
    let spent = wsol_before
        .checked_sub(f.swapper_wsol.amount)
        .ok_or_else(|| error!(SealError::MathOverflow))?;
    require!(spent > 0, SealError::NothingSpent);
    require!(spent <= burn_lamports, SealError::Overspent);

    let seeds: &[&[u8]] = &[SWAPPER_SEED, &[f.swapper_bump]];
    cpi_lite::token_burn(
        &f.edict_token_program.key(),
        &f.swapper_edict.to_account_info(),
        &f.edict_mint.to_account_info(),
        &f.swapper.to_account_info(),
        received,
        &[seeds],
    )?;
    f.swapper_edict.reload()?;
    require_eq!(
        f.swapper_edict.amount,
        edict_before,
        SealError::BurnMismatch
    );

    let refunded_lamports = f.swapper_wsol.to_account_info().lamports();
    cpi_lite::token_close_account(
        &f.swapper_wsol.to_account_info(),
        &f.buyer.to_account_info(),
        &f.swapper.to_account_info(),
        &[seeds],
    )?;
    Ok(SwapOutcome {
        wsol_spent: spent,
        received,
        refunded_lamports,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// the LaunchLab pool and the derived accounts (packages/adapters/raydium-launchlab: decode.ts and pda.ts)
// ---------------------------------------------------------------------------------------------------------------------

fn key_at(data: &[u8], offset: usize) -> Result<Pubkey> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| error!(SealError::PinMismatch))?;
    Ok(Pubkey::new_from_array(bytes))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchPoolView {
    pub status: u8,
    pub global_config: Pubkey,
    pub platform_config: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub creator: Pubkey,
}

/// A LaunchLab `PoolState`: 429 bytes with its discriminator.
pub fn parse_launch_pool(data: &[u8]) -> Result<LaunchPoolView> {
    require!(data.len() == LL_POOL_LEN, SealError::PinMismatch);
    require!(data[..8] == ACC_POOL_STATE, SealError::PinMismatch);
    Ok(LaunchPoolView {
        status: data[LL_POOL_STATUS],
        global_config: key_at(data, LL_POOL_GLOBAL_CONFIG)?,
        platform_config: key_at(data, LL_POOL_PLATFORM_CONFIG)?,
        base_mint: key_at(data, LL_POOL_BASE_MINT)?,
        quote_mint: key_at(data, LL_POOL_QUOTE_MINT)?,
        base_vault: key_at(data, LL_POOL_BASE_VAULT)?,
        quote_vault: key_at(data, LL_POOL_QUOTE_VAULT)?,
        creator: key_at(data, LL_POOL_CREATOR)?,
    })
}

/// The status byte of a LaunchLab pool account (the mint reads only this; the layout was checked at init).
pub fn launch_pool_status(data: &[u8]) -> Result<u8> {
    require!(
        data.len() == LL_POOL_LEN && data[..8] == ACC_POOL_STATE,
        SealError::PinMismatch
    );
    Ok(data[LL_POOL_STATUS])
}

/// The LaunchLab accounts of `buy_exact_in` that are PDAs of the LaunchLab program, derived from the pool's own fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchDerived {
    pub pool: Pubkey,
    pub authority: Pubkey,
    pub event_authority: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    /// `[platform_config, wSOL]`: the platform fee vault (mandatory trailing account of every trade).
    pub platform_fee_vault: Pubkey,
    /// `[creator, wSOL]`: the creator fee vault (mandatory trailing account; empty on Edict platforms).
    pub creator_fee_vault: Pubkey,
}

pub fn derive_launch(
    launchlab: &Pubkey,
    edict_mint: &Pubkey,
    platform_config: &Pubkey,
    creator: &Pubkey,
) -> LaunchDerived {
    let pda = |seeds: &[&[u8]]| Pubkey::find_program_address(seeds, launchlab).0;
    let pool = pda(&[LL_POOL_SEED, edict_mint.as_ref(), WSOL_MINT.as_ref()]);
    LaunchDerived {
        pool,
        authority: pda(&[LL_AUTH_SEED]),
        event_authority: pda(&[EVENT_AUTHORITY_SEED]),
        base_vault: pda(&[LL_POOL_VAULT_SEED, pool.as_ref(), edict_mint.as_ref()]),
        quote_vault: pda(&[LL_POOL_VAULT_SEED, pool.as_ref(), WSOL_MINT.as_ref()]),
        platform_fee_vault: pda(&[platform_config.as_ref(), WSOL_MINT.as_ref()]),
        creator_fee_vault: pda(&[creator.as_ref(), WSOL_MINT.as_ref()]),
    }
}

/// The graduated CPMM pool of $EDICT and its accounts, DERIVED: the pool PDA `["pool", amm_config, token_0, token_1]` with the
/// mints in byte order, its vaults `["pool_vault", pool, mint]`, its observation `["observation", pool]` and CPMM's authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpmmDerived {
    pub authority: Pubkey,
    pub pool: Pubkey,
    pub vault_edict: Pubkey,
    pub vault_wsol: Pubkey,
    pub observation: Pubkey,
}

pub fn derive_cpmm(cpmm: &Pubkey, amm_config: &Pubkey, edict_mint: &Pubkey) -> CpmmDerived {
    let pda = |seeds: &[&[u8]]| Pubkey::find_program_address(seeds, cpmm).0;
    let (t0, t1) = if WSOL_MINT.to_bytes() < edict_mint.to_bytes() {
        (WSOL_MINT, *edict_mint)
    } else {
        (*edict_mint, WSOL_MINT)
    };
    let pool = pda(&[CPMM_POOL_SEED, amm_config.as_ref(), t0.as_ref(), t1.as_ref()]);
    CpmmDerived {
        authority: pda(&[CPMM_AUTH_SEED]),
        pool,
        vault_edict: pda(&[CPMM_POOL_VAULT_SEED, pool.as_ref(), edict_mint.as_ref()]),
        vault_wsol: pda(&[CPMM_POOL_VAULT_SEED, pool.as_ref(), WSOL_MINT.as_ref()]),
        observation: pda(&[CPMM_OBSERVATION_SEED, pool.as_ref()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(data: &mut [u8], offset: usize, key: &Pubkey) {
        data[offset..offset + 32].copy_from_slice(key.as_ref());
    }

    #[test]
    fn the_launch_pool_is_read_at_the_pinned_offsets() {
        let (g, p, m, bv, qv, c) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let mut d = vec![0u8; LL_POOL_LEN];
        d[..8].copy_from_slice(&ACC_POOL_STATE);
        d[LL_POOL_STATUS] = LL_STATUS_MIGRATED;
        put(&mut d, LL_POOL_GLOBAL_CONFIG, &g);
        put(&mut d, LL_POOL_PLATFORM_CONFIG, &p);
        put(&mut d, LL_POOL_BASE_MINT, &m);
        put(&mut d, LL_POOL_QUOTE_MINT, &WSOL_MINT);
        put(&mut d, LL_POOL_BASE_VAULT, &bv);
        put(&mut d, LL_POOL_QUOTE_VAULT, &qv);
        put(&mut d, LL_POOL_CREATOR, &c);
        let v = parse_launch_pool(&d).unwrap();
        assert_eq!(
            v,
            LaunchPoolView {
                status: LL_STATUS_MIGRATED,
                global_config: g,
                platform_config: p,
                base_mint: m,
                quote_mint: WSOL_MINT,
                base_vault: bv,
                quote_vault: qv,
                creator: c,
            }
        );
        assert_eq!(launch_pool_status(&d).unwrap(), LL_STATUS_MIGRATED);
        // a wrong size or discriminator is refused
        assert!(parse_launch_pool(&d[..LL_POOL_LEN - 1]).is_err());
        let mut bad = d.clone();
        bad[0] ^= 1;
        assert!(parse_launch_pool(&bad).is_err());
        assert!(launch_pool_status(&bad).is_err());
    }

    #[test]
    fn swap_data_of_both_venues() {
        let curve = swap_data(VENUE_CURVE, 7, 9);
        assert_eq!(&curve[..8], &LL_BUY_EXACT_IN);
        assert_eq!(curve.len(), 32);
        assert_eq!(&curve[8..16], &7u64.to_le_bytes());
        assert_eq!(&curve[16..24], &9u64.to_le_bytes());
        assert_eq!(&curve[24..], &[0u8; 8]);
        let cpmm = swap_data(VENUE_CPMM, 7, 9);
        assert_eq!(&cpmm[..8], &CPMM_SWAP_BASE_INPUT);
        assert_eq!(cpmm.len(), 24);
    }

    #[test]
    fn the_cpmm_pool_orders_the_mints() {
        let cpmm = Pubkey::new_unique();
        let config = Pubkey::new_unique();
        // a mint below and a mint above wSOL in byte order give the pool of the ordered pair
        for mint in [Pubkey::new_from_array([0u8; 32]), Pubkey::new_from_array([255u8; 32])] {
            let d = derive_cpmm(&cpmm, &config, &mint);
            let (t0, t1) = if WSOL_MINT.to_bytes() < mint.to_bytes() {
                (WSOL_MINT, mint)
            } else {
                (mint, WSOL_MINT)
            };
            let want = Pubkey::find_program_address(
                &[b"pool", config.as_ref(), t0.as_ref(), t1.as_ref()],
                &cpmm,
            )
            .0;
            assert_eq!(d.pool, want);
            assert_ne!(d.vault_edict, d.vault_wsol);
        }
    }
}
