# Edict programs

The on-chain programs of [Edict](https://edict.ink), published so that anyone can rebuild them and check that the deployed bytes come from this source.

| Program | Address (mainnet) | What it does |
|---|---|---|
| `edict_launch` | `2pvGJ7VtCkmM6AnS2k1Dx3hyoV7H67ucZhixWPKuXAXs` | The platform authority of Edict's tokens on Raydium LaunchLab: claims their fees into each token's vault and the $EDICT fee schedule. |
| `edict_seals` | `6Eusr5CdS6DMCz9csfvGP24DqLxrPt4sBaeamDijL87f` | The Seals: 250 Metaplex Core NFTs (30 team, 220 public), the shared pot and the per-Seal vaults; a public mint buys $EDICT on its pool and burns it. |

This repository holds only the two programs and what their verifiable build needs. Nothing else of Edict (its API, keeper or website) is here.

## Official addresses

| What | Address |
|---|---|
| $EDICT token (mint) | `FYMef8rs2vANV1vVpt7ifpaA7Qx3ga3rXJkUVUbwmink` |
| Treasury | `69Apt99eSrw785Pnk1bs1zqAGnQDZaeX4in8Pe3iq2Cp` |
| Upgrade authority of both programs, and their admin | `GzPcqCTikFkfgvDuvUJZFuQTWLKzBetrkih2MJjMWCyf` |

Any other token named $EDICT is not Edict's. The same list is on [edict.ink/docs](https://edict.ink/docs) and posted on [@edictink](https://x.com/edictink).

## Verified build

| Program | `.so` size | sha256 of the file | executable hash (`solana-verify get-executable-hash`) |
|---|---|---|---|
| `edict_launch` | 258,208 bytes | `cfc05b0c71bcdd3fdf66013364f06d6cb4d1e5f959d8a1831ee6fa2876cfb059` | `e64f47ca9f26a5a8b48f8b95dde1e4a94ef9261c82e740d583f71b995eb3c206` |
| `edict_seals` | 527,936 bytes | `345a054469d24bd29145793c9af719a8191387b124ccbbd29bf7673ae976e3f0` | `b03ee968749646aa185026dec346f0f6cd5855903af7eeb6f5fcb8399a9ca7aa` |

## Toolchain

- Anchor 1.2.1, Solana (Agave) 4.3.0, SBPF v3 (`--arch v3`)
- The host Rust toolchain is pinned in `rust-toolchain.toml`; the on-chain binary is compiled by the platform-tools that ship with Agave 4.3.0
- `Cargo.lock` is committed and the build uses it as it is

## Verify a deployed program

With [solana-verify](https://github.com/Ellipsis-Labs/solana-verifiable-build) 0.5.2 and Docker:

```bash
# rebuild in the pinned container and print the hash of the binary
solana-verify build --library-name edict_seals --arch v3
solana-verify get-executable-hash target/deploy/edict_seals.so

# the hash of what is deployed
solana-verify get-program-hash -um 6Eusr5CdS6DMCz9csfvGP24DqLxrPt4sBaeamDijL87f

# or both in one step, from this repository at a commit
solana-verify verify-from-repo -um --program-id 6Eusr5CdS6DMCz9csfvGP24DqLxrPt4sBaeamDijL87f \
  --library-name edict_seals --commit-hash <commit> <repository url> -- --arch v3
```

The same commands with `edict_launch` and `2pvGJ7VtCkmM6AnS2k1Dx3hyoV7H67ucZhixWPKuXAXs` check the other program. The two hashes match only for the
exact commit that was deployed; each deployment names its commit.

## Interfaces

The Anchor IDL of each program is in `idl/` (`anchor idl build -p <program>`).

## Security

See SECURITY.md. Reports go to dev@edict.ink.

## Build locally

```bash
cargo build-sbf --arch v3 --manifest-path programs/edict_seals/Cargo.toml
cargo build-sbf --arch v3 --manifest-path programs/edict_launch/Cargo.toml
cargo test -p edict_seals -p edict_launch
```
