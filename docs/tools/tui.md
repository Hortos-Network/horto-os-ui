# TUI (`horto-os-ui-tui`)

SSH-friendly ratatui shell. Same engine as the CLI. Mouse capture is off so terminal selection / copy-paste works.

Supports **embedded** (default) and **remote** (`--remote Host`) modes. See [modes.md](modes.md).

On start the TUI clears the alternate screen. Before remote SSH / sudo / interactive
prompts it leaves the TUI, runs on the normal terminal, then restores a clean frame.

With `--remote`, Setup step status and Overview doctor data are loaded from the box
over SSH (`r` or on open), not from the PC's local `/srv`.

## Keys

| Key                | Action                             |
| ------------------ | ---------------------------------- |
| `q` / Esc / Ctrl+C | Quit                               |
| `?`                | Help overlay                       |
| Tab / Shift-Tab    | Next / previous screen             |
| `1` `2` `3`        | Setup / Logs / Overview            |
| j k / arrows       | Select step                        |
| Left / Right       | Full / Minimal kind                |
| Enter              | Run selected step                  |
| `a`                | Run pipeline                       |
| `b` / `B`          | `/etc` backup / disk probe in Logs |
| `r`                | Refresh                            |
| `d`                | Toggle dry-run                     |
| `y` / `n`          | Confirm / cancel destructive step  |

```bash
make tui
sudo horto-os-ui-tui
horto-os-ui-tui --remote horto-box --dry-run
horto-os-ui-tui --remote horto-box --install-ssh-key   # opt-in key install
```
