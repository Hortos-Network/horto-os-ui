# CI layout

Workflows are **split per utility** so a mid-term repo-per-crate split stays cheap, and so PRs only pay for what they touch.

| File                  | When it runs                                           | What                                                     |
| --------------------- | ------------------------------------------------------ | -------------------------------------------------------- |
| `rust-crate.yml`     | `workflow_call` only                                   | Reusable fmt / clippy / test (+ optional doc / coverage) |
| `ci-shared.yml`       | `crates/horto-os-ui-shared/**`, `assets/**`            | Library + embed + Codecov + rustdoc pages                |
| `ci-cli.yml`          | cli + shared/assets                                    | `horto-os-ui` binary                                     |
| `ci-tui.yml`          | tui + shared/assets                                    | TUI                                                      |
| `ci-status-api.yml`   | status-api + shared/assets                             | Status API                                               |
| `ci-kpi.yml`          | kpi only (+ lock)                                      | GPUI KPI (heavier; isolated)                             |
| `ci-desktop.yml`      | desktop + web paths                                | Tauri shell + Trunk wasm UI                          |
| `ci-supply-chain.yml` | `Cargo.lock`, `deny.toml`, `**/Cargo.toml`, `Makefile` | audit + deny + machete                                   |
| `ci-workspace.yml`    | Make / root `Cargo.toml`                               | `make lint && make test` default pkgs                    |
| `release.yml`         | GitHub Release                                         | Multi-arch box binaries                                  |

Changing **only** `crates/horto-os-ui-cli/**` runs `ci-cli` (and supply-chain if that crate’s `Cargo.toml` changed). Changing **shared** or **assets** also re-runs cli/tui/status-api because they path-depend on shared.
