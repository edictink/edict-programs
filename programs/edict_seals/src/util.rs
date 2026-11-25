//! Small helpers shared by the instructions: the pot vault, the Core program check, ASCII and bounds checks.

use anchor_lang::prelude::*;

use crate::constants::{MAX_RULES, MAX_URI_BYTES, MIN_URI_BYTES, POT_SEED};
use crate::cpi_lite;
use crate::errors::SealError;

/// The rent minimum of the data-less pot vault, read from the Rent sysvar (never a constant: the cluster's rate differs
/// between mainnet, the local validator and LiteSVM).
pub fn pot_rent_min() -> Result<u64> {
    Ok(Rent::get()?.minimum_balance(0))
}

/// Sends lamports out of the pot vault (a system-owned PDA) by `invoke_signed`. The pot keeps at least its rent minimum:
/// anything that would leave it lower is `InsufficientPot`. Zero is a no-op.
pub fn pot_transfer<'info>(
    // the system program account stays a parameter: the transfer below needs it among the accounts of the instruction
    _system_program: &Program<'info, System>,
    pot: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    amount: u64,
    pot_bump: u8,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let rent_min = pot_rent_min()?;
    let left = pot
        .lamports()
        .checked_sub(amount)
        .ok_or_else(|| error!(SealError::InsufficientPot))?;
    require!(left >= rent_min, SealError::InsufficientPot);
    let seeds: &[&[u8]] = &[POT_SEED, &[pot_bump]];
    cpi_lite::system_transfer(pot, to, amount, &[seeds])
}

/// A royalty allow list: 1 to 8 distinct programs, the System Program among them (a plain wallet is owned by it, so
/// without it wallet-to-wallet transfers of a Seal would stop).
pub fn valid_rule_set(list: &[Pubkey]) -> bool {
    if list.is_empty() || list.len() > MAX_RULES {
        return false;
    }
    let mut seen = std::collections::BTreeSet::new();
    list.iter().all(|k| seen.insert(*k)) && list.contains(&system_program::ID)
}

/// A metadata folder URI: 8 to 96 ASCII bytes, printable, ending with a slash.
pub fn valid_base_uri(uri: &str) -> bool {
    let b = uri.as_bytes();
    b.len() >= MIN_URI_BYTES
        && b.len() <= MAX_URI_BYTES
        && b.iter().all(|c| (0x21..=0x7e).contains(c))
        && b[b.len() - 1] == b'/'
}

/// `base_uri + NNN + ".json"`: three digits, as the owner's metadata files are named (`001.json` to `250.json`).
pub fn seal_uri(base_uri: &str, number: u16) -> String {
    format!("{base_uri}{number:03}.json")
}

/// `Seal #NNN`, the name in the owner's metadata (`Seal #001` to `Seal #250`).
pub fn seal_name(number: u16) -> String {
    format!("Seal #{number:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_rules() {
        assert!(valid_base_uri("https://arweave.net/abc/"));
        assert!(!valid_base_uri("https://arweave.net/abc")); // no slash
        assert!(!valid_base_uri("a/")); // too short
        assert!(!valid_base_uri(&format!("{}/", "a".repeat(96)))); // 97 bytes
        assert!(valid_base_uri(&format!("{}/", "a".repeat(95)))); // 96 bytes
        assert!(!valid_base_uri("https://x.y/\u{e9}/")); // not ASCII
        assert!(!valid_base_uri("https://x.y/a b/")); // a space
    }

    #[test]
    fn rule_set_rules() {
        let a = Pubkey::new_unique();
        let sys = system_program::ID;
        assert!(valid_rule_set(&[sys]));
        assert!(valid_rule_set(&[sys, a]));
        assert!(!valid_rule_set(&[]));
        assert!(!valid_rule_set(&[a])); // no System Program
        assert!(!valid_rule_set(&[sys, a, a])); // a repeat
        let nine: Vec<Pubkey> = std::iter::once(sys)
            .chain((0..8).map(|_| Pubkey::new_unique()))
            .collect();
        assert_eq!(nine.len(), 9);
        assert!(!valid_rule_set(&nine));
        assert!(valid_rule_set(&nine[..8]));
    }

    #[test]
    fn names_and_uris() {
        assert_eq!(seal_name(7), "Seal #007");
        assert_eq!(seal_name(42), "Seal #042");
        assert_eq!(seal_name(250), "Seal #250");
        assert_eq!(seal_uri("https://a.b/c/", 15), "https://a.b/c/015.json");
        assert_eq!(seal_uri("https://a.b/c/", 250), "https://a.b/c/250.json");
    }
}
