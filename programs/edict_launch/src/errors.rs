use anchor_lang::prelude::*;

#[error_code]
pub enum LaunchError {
    #[msg("only the upgrade authority may initialise the config")]
    NotUpgradeAuthority,
    #[msg("only the admin may do this")]
    NotAdmin,
    #[msg("an account is not the one the config pins")]
    WrongAccount,
    #[msg("the foreign account has an unexpected owner, size or discriminator")]
    BadForeignAccount,
    #[msg("the global config is not SOL-quoted")]
    NotSolQuoted,
    #[msg("the protocol token already exists; its mint can no longer change")]
    ProtocolTokenExists,
    #[msg("the token is not in the expected status")]
    WrongStatus,
    #[msg("the pool is not this token's Edict pool")]
    PoolMismatch,
    #[msg("the pool uses a curve setting Edict does not accept")]
    PoolSettings,
    #[msg("the vault account is not the token vault's account for that mint")]
    WrongVaultAccount,
    #[msg("the claim did not move exactly what the fee vault held")]
    ClaimMismatch,
    #[msg("the forward to the vault did not arrive exactly")]
    ForwardMismatch,
    #[msg("a creator fee arrived in the base token")]
    BaseCreatorFee,
    #[msg("this token has no fee schedule, or its schedule is final")]
    NoSchedule,
    #[msg("the schedule does not allow a lower rate yet")]
    StepTooEarly,
    #[msg("arithmetic overflow")]
    Overflow,
}
