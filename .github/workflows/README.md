# CI layout

Workflows are **split per utility** so a mid-term repo-per-crate split stays cheap, and so PRs only pay for what they touch.

Reusable pieces (called only when needed, so PR checks stay clean):

| File                       | Role                                                          |
| -------------------------- | ------------------------------------------------------------- |
| `rust-crate.yml`           | fmt / clippy / test (+ optional apt packages)                 |
| `rust-crate-doc.yml`       | rustdoc (+ optional gh-pages)                                 |
| `rust-crate-coverage.yml`  | llvm-cov + Codecov                                            |

Thin callers:

| File                  | When it runs                                           | What                                                     |
| --------------------- | ------------------------------------------------------ | -------------------------------------------------------- |
| `ci-shared.yml`       | `crates/horto-os-ui-shared/**`, `assets/**`            | Lint + rustdoc + pages + Codecov                         |
| `ci-cli.yml`          | cli + shared/assets                                    | Lint + rustdoc                                           |
| `ci-tui.yml`          | tui + shared/assets                                    | Lint + rustdoc                                           |
| `ci-status-api.yml`   | status-api + shared/assets                             | Lint + rustdoc                                           |
| `ci-kpi.yml`          | kpi only (+ lock)                                      | GPUI KPI lint/test (X11 apt deps)                        |
| `ci-desktop.yml`      | desktop + web paths                                    | Tauri shell + Trunk wasm UI                              |
| `ci-supply-chain.yml` | `Cargo.lock`, `deny.toml`, `**/Cargo.toml`, `Makefile` | audit + deny + machete                                   |
| `ci-workspace.yml`    | Make / root `Cargo.toml`                               | `make lint && make test` default pkgs                    |
| `release.yml`         | GitHub Release                                         | Multi-arch box binaries                                  |
| `release-github-assets.yml` | workflow_call                                      | Box + KPI + Desktop assets (reusable)                    |
| `release-github-preview.yml` | workflow_dispatch                                 | Overwrite Pre-release `dev-preview`                      |
| `ghcr-status-api.yml` | dispatch / after preview                           | Push `ghcr.io/hortos-network/horto-os-ui-status-api:dev` |

Changing **only** `crates/horto-os-ui-cli/**` runs `ci-cli` (and supply-chain if that crate’s `Cargo.toml` changed). Changing **shared** or **assets** also re-runs cli/tui/status-api because they path-depend on shared.

JavaScript Actions run on **Node 24** (`FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` on workflow `env`, plus current major pins: `actions/checkout@v7`, `actions/upload-artifact@v7`, `codecov/codecov-action@v7`).
