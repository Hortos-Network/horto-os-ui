# horto-os-ui

Rust installer and ops surfaces for a Horto box (CLI, TUI, status API) plus PC clients (KPI, desktop).

| Surface | Crate / binary | Docs |
| ------- | -------------- | ---- |
| Engine | `horto-os-ui-shared` | [docs/tools/shared.md](docs/tools/shared.md) · [crate README](crates/horto-os-ui-shared/README.md) |
| CLI | `horto-os-ui` | [docs/tools/cli.md](docs/tools/cli.md) · [crate README](crates/horto-os-ui-cli/README.md) |
| TUI | `horto-os-ui-tui` | [docs/tools/tui.md](docs/tools/tui.md) · [crate README](crates/horto-os-ui-tui/README.md) |
| Status API | `horto-os-ui-status-api` | [docs/tools/status-api.md](docs/tools/status-api.md) · [crate README](crates/horto-os-ui-status-api/README.md) |
| Ops KPI | `horto-os-ui-kpi` | [docs/tools/kpi.md](docs/tools/kpi.md) · [crate README](crates/horto-os-ui-kpi/README.md) |
| Web UI | `horto-os-ui-web` | [docs/tools/web.md](docs/tools/web.md) · [crate README](crates/horto-os-ui-web/README.md) |
| Desktop | `horto-os-ui-desktop` | [docs/tools/desktop.md](docs/tools/desktop.md) · [crate README](crates/horto-os-ui-desktop/README.md) |

**Operator hub:** [docs/README.md](docs/README.md)  
**Absorb horto-os tip:** [docs/TIP_SYNC.md](docs/TIP_SYNC.md)  
**Make targets:** `make help`

```bash
make lint
make test
make coverage-summary
make ci
```

Templates under `assets/` are embedded at compile time. Sibling `horto-os` shell scripts are reference only; they are not executed at runtime.
