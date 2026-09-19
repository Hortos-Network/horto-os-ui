# horto-os-ui

<p align="center">
  <img src="assets/hortos-logo.png" alt="Hortos" width="160" />
</p>

<p align="center">
  <strong>Rust installer and ops surfaces for a Horto box</strong><br />
  CLI, TUI, and status API on the box · KPI board and desktop client on a PC
</p>

<p align="center">
  <a href="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-shared.yml"><img src="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-shared.yml/badge.svg?branch=dev" alt="CI shared" /></a>
  <a href="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-cli.yml"><img src="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-cli.yml/badge.svg?branch=dev" alt="CI cli" /></a>
  <a href="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-tui.yml"><img src="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-tui.yml/badge.svg?branch=dev" alt="CI tui" /></a>
  <a href="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-status-api.yml"><img src="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-status-api.yml/badge.svg?branch=dev" alt="CI status-api" /></a>
  <a href="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-kpi.yml"><img src="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-kpi.yml/badge.svg?branch=dev" alt="CI kpi" /></a>
  <a href="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-desktop.yml"><img src="https://github.com/Hortos-Network/horto-os-ui/actions/workflows/ci-desktop.yml/badge.svg?branch=dev" alt="CI desktop" /></a>
  <a href="https://codecov.io/gh/Hortos-Network/horto-os-ui/tree/dev"><img src="https://codecov.io/gh/Hortos-Network/horto-os-ui/branch/dev/graph/badge.svg" alt="Codecov" /></a>
</p>

Templates under `assets/` are embedded at compile time. No separate shell-script checkout is required at runtime.

Integration branch: **`dev`**. Canonical repo: [Hortos-Network/horto-os-ui](https://github.com/Hortos-Network/horto-os-ui).

## Surfaces

| Role | Crate | Binary / artifact | Make run | Docs |
| ---- | ----- | ----------------- | -------- | ---- |
| Engine | `horto-os-ui-shared` | library | (pulled in by others) | [shared](docs/tools/shared.md) |
| CLI | `horto-os-ui-cli` | `horto-os-ui` | `make cli` / `make status` | [cli](docs/tools/cli.md) |
| TUI | `horto-os-ui-tui` | `horto-os-ui-tui` | `make tui` | [tui](docs/tools/tui.md) |
| Status API | `horto-os-ui-status-api` | `horto-os-ui-status-api` | `make api` | [status-api](docs/tools/status-api.md) |
| KPI board | `horto-os-ui-kpi` | `horto-os-ui-kpi` | `make kpi` | [kpi](docs/tools/kpi.md) |
| Web UI | `horto-os-ui-web` | Trunk `dist/` | `make desktop-web` | [web](docs/tools/web.md) |
| Desktop | `horto-os-ui-desktop` | Tauri app | `make desktop` | [desktop](docs/tools/desktop.md) |

Operator hub (longer recipes): [docs/README.md](docs/README.md). Tip sync notes: [docs/TIP_SYNC.md](docs/TIP_SYNC.md).

Full Make catalog: `make help`.

## Prerequisites

| Need | Notes |
| ---- | ----- |
| Rust stable | `rustup` default toolchain |
| Linux + X11 (KPI / desktop) | GPUI and Tauri need a display |
| Trunk (web / desktop) | `cargo install trunk` for `make desktop` / `make desktop-web` |
| Optional root | Apply mode for CLI/TUI setup on a real box (`sudo`) |

Default Cargo members (fast path): shared, CLI, TUI, status-api. KPI (GPUI) and desktop are opt-in via their Make targets.

## Quick start (laptop)

Two terminals is enough to see the stack without touching the box:

```bash
# terminal A: status API (listens on all interfaces by default)
make api
# health: http://127.0.0.1:8787/health
# status: http://127.0.0.1:8787/v1/status

# terminal B: KPI board (demo charts by default)
make kpi

# or TUI (dry-run; Esc / q / Ctrl+C restore the shell)
make tui
```

Point the KPI board at a real box API and turn off demo data:

```bash
make kpi HORTO_BOX_URL=http://192.168.1.10:8787 ARGS='--demo false'
```

Desktop homeowner shell (Trunk release + Tauri window):

```bash
make desktop
# UI talks to HORTO_BOX_URL (default http://localhost:8787)
```

## Build

```bash
make build              # default packages (shared, cli, tui, status-api)
make build-release      # same, release profile
make build-kpi          # GPUI KPI binary
make build-all          # default + kpi
make build-desktop      # Trunk web + release Tauri binary (no open)
make check              # cargo check default packages
make check-all          # cargo check whole workspace
```

## Run each surface

### CLI (`horto-os-ui`)

Dry-run is the default for Make helpers (safe on a laptop). Drop it with `DRY_RUN=0` or `APPLY=1`.

```bash
make status                              # setup status
make doctor
make docker-status
make setup-run                           # full pipeline dry-run
make setup-step STEP=s1
make backup-status
make backup-etc
make backup-disk-status
make cli ARGS='setup status --minimal'
make cli ARGS='net leases'
make cli ARGS='docker rebuild --dir /srv/docker/homepage'
make cli DRY_RUN=0 ARGS='doctor'         # apply mode when you mean it
```

Apply on a real box (root):

```bash
sudo horto-os-ui setup run --full
sudo horto-os-ui setup step s5
sudo horto-os-ui docker init
sudo horto-os-ui backup etc
```

### TUI (`horto-os-ui-tui`)

```bash
make tui                 # dry-run wizard: Setup / Logs / Overview
make tui-release         # release binary, dry-run
make tui DRY_RUN=0       # apply mode (needs privileges for writes)
```

Quit with `q`, Esc, or Ctrl+C (tty is restored).

### Status API (`horto-os-ui-status-api`)

```bash
make api                                 # API_BIND=0.0.0.0:8787
make api API_BIND=127.0.0.1:8787
curl -sS http://127.0.0.1:8787/health
curl -sS http://127.0.0.1:8787/v1/status | head
```

Optional token: `HORTO_API_TOKEN` on the server and clients.

### KPI board (`horto-os-ui-kpi`)

```bash
make kpi                                 # demo series on by default
make kpi ARGS='--demo false'             # live /health + /v1/status (+ EVCC if linked)
make kpi HORTO_BOX_URL=http://box:8787 ARGS='--demo false'
make kpi HORTO_EVCC_URL=http://box:7070 ARGS='--demo false'
# HORTO_KPI_POLL_SECS=1  HORTO_KPI_HISTORY=60
```

### Web + desktop

```bash
make desktop-web              # Trunk release → crates/horto-os-ui-web/dist
make desktop-web-serve        # Trunk serve on :4187 (dev iteration)
make desktop                  # Trunk release + open Tauri window
make build-desktop            # build only, do not open
```

## Quality gates

```bash
make help
make format                 # rustfmt write
make lint                   # fmt --check + clippy (default packages)
make test                   # default packages
make test-all               # whole workspace
make ci                     # lint + test + audit + deny + machete
make coverage-summary       # llvm-cov summary (needs cargo-llvm-cov)
make coverage-shared        # shared crate gate (fail-under)
make coverage-html          # HTML report under target/
make audit
make deny
make machete
make outdated
make doc / make doc-open
```

GitHub Actions runs per-crate workflows on `dev` / PRs (path filters). Coverage for the shared engine uploads to [Codecov](https://codecov.io/gh/Hortos-Network/horto-os-ui).

## Install / package

```bash
make install                # release CLI, TUI, status-api → ~/.local/bin
make install-kpi            # also horto-os-ui-kpi
make uninstall
make release-bins           # dist/*.tar.gz + sha256
make deb                    # needs: cargo install cargo-deb
make bins
make version-show
make clean
```

Override install prefix: `make install PREFIX=/usr/local`.

## Useful overrides

| Variable | Default | Used by |
| -------- | ------- | ------- |
| `API_BIND` | `0.0.0.0:8787` | `make api` |
| `HORTO_BOX_URL` | `http://localhost:8787` | `make kpi`, desktop |
| `HORTO_API_TOKEN` | (unset) | API clients |
| `HORTO_EVCC_URL` | (from status links) | KPI energy tiles |
| `HORTO_KPI_DEMO` | `true` | KPI synthetic charts |
| `DRY_RUN` / `APPLY` | `1` / `0` | CLI / TUI Make helpers |
| `ARGS` | empty | Extra argv for `make cli` / `kpi` / … |
| `STEP` | `s1` | `make setup-step` |
| `PREFIX` | `$HOME/.local` | `make install` |

## License

Apache-2.0. See [LICENSE](LICENSE).
