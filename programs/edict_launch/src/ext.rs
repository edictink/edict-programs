//! The foreign programs: Raydium LaunchLab, CPMM, the LP lock (Fee Key) program, the Token program. Instruction encodings
//! and account offsets are written by hand from the on-chain IDLs and the CPMM source (docs/validation/LAUNCHLAB.md
//! sections 1 to 3, 9), and checked by the unit tests below and by the LiteSVM suite against the deployed binaries.
//! Program ids are never taken from a caller: they come from `Config`, set once by the upgrade authority.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};

/// Wrapped SOL, the only quote asset of an Edict LaunchLab token.
pub const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
pub const TOKEN_PROGRAM_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM_ID: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const ATA_PROGRAM_ID: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// ---------------------------------------------------------------------------------------------------------------------
// LaunchLab
// ---------------------------------------------------------------------------------------------------------------------

pub const LL_PLATFORM_CONFIG_SEED: &[u8] = b"platform_config";
pub const LL_ALLOW_CONFIG_SEED: &[u8] = b"platform_allow_config";
pub const LL_FEE_VAULT_AUTH_SEED: &[u8] = b"platform_fee_vault_auth_seed";
pub const LL_POOL_SEED: &[u8] = b"pool";

/// `PlatformConfig` and `PlatformAllowConfig` data sizes (IDL; rent 5,445,760 and 1,346,200 lamports on mainnet).
pub const LL_PLATFORM_CONFIG_LEN: usize = 944;
pub const LL_ALLOW_CONFIG_LEN: usize = 137;

const IX_CREATE_PLATFORM_CONFIG: [u8; 8] = [176, 90, 196, 175, 253, 113, 220, 20];
const IX_UPDATE_PLATFORM_CONFIG: [u8; 8] = [195, 60, 76, 129, 146, 45, 67, 143];
const IX_CREATE_ALLOW_CONFIG: [u8; 8] = [69, 71, 168, 7, 214, 250, 107, 102];
const IX_CLOSE_ALLOW_CONFIG: [u8; 8] = [82, 205, 36, 216, 16, 6, 240, 215];
const IX_CLAIM_PLATFORM_FEE_FROM_VAULT: [u8; 8] = [117, 241, 198, 168, 248, 218, 80, 29];

pub const ACC_POOL_STATE: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const ACC_GLOBAL_CONFIG: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];

/// `PlatformConfigParam` variant indexes (IDL enum order).
const PARAM_FEE_RATE: u8 = 3;
const PARAM_CP_CREATOR: u8 = 11;
const PARAM_RESTRICT_GLOBAL_CONFIG: u8 = 12;

/// `PoolState` (borsh, 429 bytes; offsets cross-checked with the SDK layout `LaunchpadPool`).
pub mod pool {
    pub const LEN: usize = 429;
    pub const STATUS: usize = 17;
    pub const MIGRATE_TYPE: usize = 20;
    pub const VESTING_TOTAL_LOCKED: usize = 101;
    pub const GLOBAL_CONFIG: usize = 141;
    pub const PLATFORM_CONFIG: usize = 173;
    pub const BASE_MINT: usize = 205;
    pub const QUOTE_MINT: usize = 237;
    pub const CREATOR: usize = 333;
    pub const AMM_CREATOR_FEE_ON: usize = 366;
    /// `migrate_type` 1 = CPMM; `amm_creator_fee_on` 0 = quote token only.
    pub const MIGRATE_CPMM: u8 = 1;
    pub const FEE_ON_QUOTE: u8 = 0;
}

/// `GlobalConfig`: the quote mint (offset 83 = 8 + 8 + 1 + 2 + 8 x 8).
pub const GLOBAL_CONFIG_QUOTE_MINT: usize = 83;

/// `PlatformParams` of an Edict token platform: all LP to the Fee Key (platform scale 1,000,000), no creator fee, no
/// platform vesting.
#[allow(clippy::too_many_arguments)]
pub fn create_platform_config_ix(
    program: Pubkey,
    auth: Pubkey,
    platform_config: Pubkey,
    cpswap_config: Pubkey,
    fee_rate: u64,
    name: &str,
    web: &str,
    img: &str,
) -> Instruction {
    let mut data = Vec::with_capacity(8 + 24 + 8 + 12 + name.len() + web.len() + img.len() + 16);
    data.extend_from_slice(&IX_CREATE_PLATFORM_CONFIG);
    data.extend_from_slice(&1_000_000u64.to_le_bytes()); // migrate_nft_info.platform_scale
    data.extend_from_slice(&0u64.to_le_bytes()); // creator_scale
    data.extend_from_slice(&0u64.to_le_bytes()); // burn_scale
    data.extend_from_slice(&fee_rate.to_le_bytes());
    for s in [name, web, img] {
        data.extend_from_slice(&(s.len() as u32).to_le_bytes());
        data.extend_from_slice(s.as_bytes());
    }
    data.extend_from_slice(&0u64.to_le_bytes()); // creator_fee_rate
    data.extend_from_slice(&0u64.to_le_bytes()); // platform_vesting_scale
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(auth, true),                     // platform_admin
            AccountMeta::new_readonly(auth, false),           // platform_fee_wallet
            AccountMeta::new_readonly(auth, false),           // platform_nft_wallet
            AccountMeta::new(platform_config, false),         // platform_config
            AccountMeta::new_readonly(cpswap_config, false),  // cpswap_config
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(auth, false),           // transfer_fee_extension_authority
            AccountMeta::new_readonly(auth, false),           // platform_vesting_wallet
        ],
        data,
    }
}

fn update_platform_config_ix(program: Pubkey, auth: Pubkey, platform_config: Pubkey, param: &[u8]) -> Instruction {
    let mut data = Vec::with_capacity(8 + param.len());
    data.extend_from_slice(&IX_UPDATE_PLATFORM_CONFIG);
    data.extend_from_slice(param);
    Instruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(auth, true), AccountMeta::new(platform_config, false)],
        data,
    }
}

pub fn set_fee_rate_ix(program: Pubkey, auth: Pubkey, platform_config: Pubkey, rate: u64) -> Instruction {
    let mut p = [0u8; 9];
    p[0] = PARAM_FEE_RATE;
    p[1..].copy_from_slice(&rate.to_le_bytes());
    update_platform_config_ix(program, auth, platform_config, &p)
}

pub fn set_cp_creator_ix(program: Pubkey, auth: Pubkey, platform_config: Pubkey, creator: Pubkey) -> Instruction {
    let mut p = [0u8; 33];
    p[0] = PARAM_CP_CREATOR;
    p[1..].copy_from_slice(creator.as_ref());
    update_platform_config_ix(program, auth, platform_config, &p)
}

pub fn restrict_global_config_ix(program: Pubkey, auth: Pubkey, platform_config: Pubkey) -> Instruction {
    let mut p = [0u8; 9];
    p[0] = PARAM_RESTRICT_GLOBAL_CONFIG;
    p[1..].copy_from_slice(&1u64.to_le_bytes());
    update_platform_config_ix(program, auth, platform_config, &p)
}

pub fn create_allow_config_ix(
    program: Pubkey,
    auth: Pubkey,
    platform_config: Pubkey,
    global_config: Pubkey,
    allow_config: Pubkey,
) -> Instruction {
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(auth, true),
            AccountMeta::new_readonly(platform_config, false),
            AccountMeta::new_readonly(global_config, false),
            AccountMeta::new(allow_config, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: IX_CREATE_ALLOW_CONFIG.to_vec(),
    }
}

pub fn close_allow_config_ix(
    program: Pubkey,
    auth: Pubkey,
    platform_config: Pubkey,
    global_config: Pubkey,
    allow_config: Pubkey,
) -> Instruction {
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(auth, true),
            AccountMeta::new_readonly(platform_config, false),
            AccountMeta::new_readonly(global_config, false),
            AccountMeta::new(allow_config, false),
        ],
        data: IX_CLOSE_ALLOW_CONFIG.to_vec(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn claim_platform_fee_from_vault_ix(
    program: Pubkey,
    auth: Pubkey,
    fee_vault_authority: Pubkey,
    platform_config: Pubkey,
    platform_fee_vault: Pubkey,
    recipient: Pubkey,
) -> Instruction {
    Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(auth, true),
            AccountMeta::new_readonly(fee_vault_authority, false),
            AccountMeta::new_readonly(platform_config, false),
            AccountMeta::new(platform_fee_vault, false),
            AccountMeta::new(recipient, false),
            AccountMeta::new_readonly(WSOL_MINT, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(ATA_PROGRAM_ID, false),
        ],
        data: IX_CLAIM_PLATFORM_FEE_FROM_VAULT.to_vec(),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// CPMM (raydium-cp-swap, PoolState is `repr(C, packed)`, 637 bytes)
// ---------------------------------------------------------------------------------------------------------------------

pub const CPMM_AUTH_SEED: &[u8] = b"vault_and_lp_mint_auth_seed";
pub const ACC_CPMM_POOL: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
const IX_COLLECT_CREATOR_FEE_PERMISSIONLESS: [u8; 8] = [202, 202, 34, 83, 226, 122, 145, 229];

pub mod cpmm_pool {
    pub const LEN: usize = 637;
    pub const AMM_CONFIG: usize = 8;
    pub const POOL_CREATOR: usize = 40;
    pub const TOKEN_0_VAULT: usize = 72;
    pub const TOKEN_1_VAULT: usize = 104;
    pub const TOKEN_0_MINT: usize = 168;
    pub const TOKEN_1_MINT: usize = 200;
    pub const CREATOR_FEES_0: usize = 397;
    pub const CREATOR_FEES_1: usize = 405;
}

/// `collect_creator_fee_permissionless`: pays the pool creator's ATAs (created if missing, by `payer`).
pub fn collect_creator_fee_ix(program: Pubkey, payer: Pubkey, metas_after_payer: Vec<AccountMeta>) -> Instruction {
    let mut accounts = Vec::with_capacity(16);
    accounts.push(AccountMeta::new(payer, true));
    accounts.extend(metas_after_payer);
    Instruction { program_id: program, accounts, data: IX_COLLECT_CREATOR_FEE_PERMISSIONLESS.to_vec() }
}

// ---------------------------------------------------------------------------------------------------------------------
// LP lock (Fee Key): `collectCpFees(fee_lp_amount)`
// ---------------------------------------------------------------------------------------------------------------------

const IX_COLLECT_CP_FEES: [u8; 8] = [8, 30, 51, 199, 209, 184, 247, 133];

pub fn collect_cp_fees_ix(program: Pubkey, accounts: Vec<AccountMeta>, fee_lp_amount: u64) -> Instruction {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&IX_COLLECT_CP_FEES);
    data.extend_from_slice(&fee_lp_amount.to_le_bytes());
    Instruction { program_id: program, accounts, data }
}

// ---------------------------------------------------------------------------------------------------------------------
// Token accounts (classic and Token-2022 share the first 165 bytes)
// ---------------------------------------------------------------------------------------------------------------------

pub struct TokenAcc {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
}

pub fn read_token_account(ai: &AccountInfo) -> Option<TokenAcc> {
    if *ai.owner != TOKEN_PROGRAM_ID && *ai.owner != TOKEN_2022_PROGRAM_ID {
        return None;
    }
    let d = ai.try_borrow_data().ok()?;
    if d.len() < 165 || d[108] != 1 {
        // state byte 108: 1 = initialized (0 uninitialized, 2 frozen: a frozen account cannot receive)
        return None;
    }
    Some(TokenAcc { mint: read_pubkey(&d, 0), owner: read_pubkey(&d, 32), amount: read_u64(&d, 64) })
}

pub fn read_pubkey(d: &[u8], at: usize) -> Pubkey {
    let mut b = [0u8; 32];
    b.copy_from_slice(&d[at..at + 32]);
    Pubkey::new_from_array(b)
}

pub fn read_u64(d: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[at..at + 8]);
    u64::from_le_bytes(b)
}

pub fn ata_of(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[owner.as_ref(), token_program.as_ref(), mint.as_ref()], &ATA_PROGRAM_ID).0
}

/// Classic Token `Transfer { amount }` (tag 3): `source` -> `destination`, signed by `authority`.
pub fn token_transfer_ix(source: Pubkey, destination: Pubkey, authority: Pubkey, amount: u64) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(3);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(source, false),
            AccountMeta::new(destination, false),
            AccountMeta::new_readonly(authority, true),
        ],
        data,
    }
}

/// `SystemInstruction::Transfer { lamports }` (tag 2, u32 little endian).
pub fn system_transfer_ix(from: Pubkey, to: Pubkey, lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: system_program::ID,
        accounts: vec![AccountMeta::new(from, true), AccountMeta::new(to, false)],
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn sha256(data: &[u8]) -> [u8; 32] {
        Sha256::digest(data).into()
    }

    /// The discriminators equal sha256("global:<name>")[..8] / sha256("account:<Name>")[..8].
    #[test]
    fn discriminators() {
        for (name, d) in [
            ("global:create_platform_config", IX_CREATE_PLATFORM_CONFIG),
            ("global:update_platform_config", IX_UPDATE_PLATFORM_CONFIG),
            ("global:create_platform_allow_config", IX_CREATE_ALLOW_CONFIG),
            ("global:close_platform_allow_config", IX_CLOSE_ALLOW_CONFIG),
            ("global:claim_platform_fee_from_vault", IX_CLAIM_PLATFORM_FEE_FROM_VAULT),
            ("global:collect_creator_fee_permissionless", IX_COLLECT_CREATOR_FEE_PERMISSIONLESS),
            ("global:collect_cp_fees", IX_COLLECT_CP_FEES),
            ("account:PoolState", ACC_POOL_STATE),
            ("account:GlobalConfig", ACC_GLOBAL_CONFIG),
        ] {
            assert_eq!(&sha256(name.as_bytes())[..8], &d, "{name}");
        }
    }

    #[test]
    fn transfer_encodings() {
        let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
        let t = system_transfer_ix(a, b, 7);
        assert_eq!(t.data, [2, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0]);
        let k = token_transfer_ix(a, b, a, 9);
        assert_eq!(k.data, [3, 9, 0, 0, 0, 0, 0, 0, 0]);
        assert!(k.accounts[2].is_signer && !k.accounts[2].is_writable);
    }

    #[test]
    fn update_params() {
        let (p, a, c) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
        let f = set_fee_rate_ix(p, a, c, 40_000);
        assert_eq!(&f.data[8..], &[3, 0x40, 0x9c, 0, 0, 0, 0, 0, 0]);
        let r = restrict_global_config_ix(p, a, c);
        assert_eq!(&r.data[8..], &[12, 1, 0, 0, 0, 0, 0, 0, 0]);
        let k = set_cp_creator_ix(p, a, c, a);
        assert_eq!(k.data[8], 11);
        assert_eq!(&k.data[9..], a.as_ref());
    }

    #[test]
    fn platform_params_layout() {
        let (p, a, c, s) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
        let ix = create_platform_config_ix(p, a, c, s, 10_000, "Edict", "https://edict.ink", "i");
        let d = &ix.data[8..];
        assert_eq!(read_u64(d, 0), 1_000_000);
        assert_eq!(read_u64(d, 8), 0);
        assert_eq!(read_u64(d, 16), 0);
        assert_eq!(read_u64(d, 24), 10_000);
        assert_eq!(&d[32..36], &5u32.to_le_bytes());
        assert_eq!(&d[36..41], b"Edict");
        assert_eq!(d.len(), 32 + 4 + 5 + 4 + 17 + 4 + 1 + 16);
        assert_eq!(ix.accounts.len(), 8);
        assert!(ix.accounts[0].is_signer && ix.accounts[0].is_writable);
    }
}
