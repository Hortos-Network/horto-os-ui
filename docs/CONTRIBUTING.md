# Contributing to horto-os-ui

Thanks for improving Horto box installer and ops surfaces. This repo ships the shared engine plus CLI, TUI, status API, KPI board, and desktop clients.

## Before you open a PR

1. Skim [README.md](README.md) and the relevant [tools/](tools/) page.
2. Run the local gate:

```bash
make ci
```

Or at least:

```bash
make lint
make test
```

3. One concern per PR. Finish the concern locally, then open a **ready** PR (draft only when the branch must be visible before that concern is done).
4. Add or extend tests for behavior you introduce. Keep public API documented.

## Toolchain

- Rust stable (`rustup` default).
- Integration branch: `dev` on [Hortos-Network/horto-os-ui](https://github.com/Hortos-Network/horto-os-ui).
- Feature branches land via PR from a personal fork.

## Quality bar

| Gate | Command |
| ---- | ------- |
| Format + Clippy | `make lint` |
| Tests (default packages) | `make test` |
| Whole workspace tests | `make test-all` |
| Coverage summary | `make coverage-summary` (needs `cargo-llvm-cov`) |
| Shared fail-under | `make coverage-shared` |
| Coverage HTML | `make coverage-html` |
| Audit / deny / machete | `make audit` / `make deny` / `make machete` |
| Outdated deps | `make outdated` (needs `cargo outdated`) |
| Rustdoc | `make doc` |
| Full local CI slice | `make ci` |

Clippy for default packages uses `-D warnings -D clippy::all`. Do not paper over issues with `#[allow(clippy::too_many_arguments)]`, `too_many_lines`, or `dead_code`.

## Make habits

```bash
make help
make api
make tui
make kpi
make desktop
make cli ARGS='setup status --minimal'
```

Default members: shared, CLI, TUI, status-api. KPI and desktop are opt-in (`make build-kpi`, `make desktop`).

## Documentation

- Product hub: [README.md](README.md) (this `docs/` tree).
- Per-surface notes: [tools/](tools/).
- Tip absorb: [TIP_SYNC.md](TIP_SYNC.md).
- English only in code, docs, commits, and PR text.
- No plan jargon or host-absolute paths in shipped text.
- Code of Conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security: [SECURITY.md](SECURITY.md)

## Commits and PRs

Conventional commits (`feat:`, `fix:`, `docs:`, `ci:`, …). PR body follows
[pull_request_template.md](pull_request_template.md) (**Summary** + **Test plan** only).
