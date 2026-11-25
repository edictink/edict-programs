# Security policy

This repository holds the on-chain programs of Edict:

| Program | Address |
|---|---|
| `edict_launch` | `2pvGJ7VtCkmM6AnS2k1Dx3hyoV7H67ucZhixWPKuXAXs` |
| `edict_seals` | `6Eusr5CdS6DMCz9csfvGP24DqLxrPt4sBaeamDijL87f` |

## Reporting a vulnerability

Email **dev@edict.ink** with a description of the issue, the affected program and instruction, and the steps or a transaction that shows it.
Please do not open a public issue and do not exploit the issue on mainnet beyond what is needed to show it.

We acknowledge a report within 72 hours and keep you informed until it is resolved. Once a fix is deployed we credit the reporter if they wish.

## Scope

- The two programs above, at the commit that matches the deployed binary (see README.md, "Verify a deployed program").
- Out of scope: third-party programs Edict calls (Raydium LaunchLab and CPMM, Metaplex Core, the SPL token programs), websites, and issues that
  need a compromised admin or upgrade key.
