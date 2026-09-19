# Security policy

## Supported versions

Security fixes target the latest tip of the `dev` branch and any tagged releases published from this repository.

## Reporting a vulnerability

Do **not** open a public GitHub issue for an unfixed vulnerability.

Prefer a private [GitHub Security Advisory](https://github.com/Hortos-Network/horto-os-ui/security/advisories/new) on this repository when available. Include a clear description, impact, and reproduction steps when possible.

We will acknowledge receipt and follow up. Do not expect a fixed SLA.

## Local supply-chain checks

```bash
make audit
make deny
make machete
```

Those commands are not a vulnerability reporting channel.
