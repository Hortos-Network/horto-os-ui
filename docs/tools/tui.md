# TUI (`horto-os-ui-tui`)

SSH-friendly ratatui shell. Same engine as the CLI. Mouse capture is off so terminal selection / copy-paste works.

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
```
