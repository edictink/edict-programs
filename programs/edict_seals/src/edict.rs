//! The $EDICT mint check: no freeze authority; if it is a Token-2022 mint, its extensions are only
//! MetadataPointer and TokenMetadata (what pump.fun creates; a transfer fee, a transfer hook or a permanent delegate could
//! block or drain the swapper's token account and break "every $EDICT received is burned"). Extensions cannot be added to an
//! initialised mint, so the check at `init_config` time stays true. A classic SPL mint is accepted (the DBC launch of
//! $EDICT, devnet and test mints).

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{spl_token_2022, Mint};
use spl_token_2022::extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions};

use crate::errors::SealError;

pub fn validate_edict_mint(mint: &InterfaceAccount<Mint>) -> Result<()> {
    require!(mint.freeze_authority.is_none(), SealError::MintNotPinned);
    require!(mint.is_initialized, SealError::MintNotPinned);
    let info = mint.to_account_info();
    if *info.owner == spl_token_2022::ID {
        let data = info.try_borrow_data()?;
        let state = StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&data)
            .map_err(|_| error!(SealError::MintNotPinned))?;
        let extensions = state
            .get_extension_types()
            .map_err(|_| error!(SealError::MintNotPinned))?;
        for extension in extensions {
            require!(
                matches!(
                    extension,
                    ExtensionType::MetadataPointer | ExtensionType::TokenMetadata
                ),
                SealError::MintNotPinned
            );
        }
    }
    Ok(())
}
