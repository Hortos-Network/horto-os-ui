# TUI (`horto-os-ui-tui`)

SSH-friendly ratatui shell. Same engine as the CLI; no mouse required.

## Keys

| Key | Action |
| --- | ------ |
| Tab | Setup / Logs / Dashboard |
| Arrows | Select step |
| Enter | Run selected step |
| `a` | Run pipeline |
| `b` / `B` | `/etc` backup / disk probe in Logs |
| `r` | Refresh |
| `d` | Toggle dry-run |
| `q` | Quit |

```bash
make tui
sudo horto-os-ui-tui
```
