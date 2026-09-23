# TUI (`horto-os-ui-tui`)

SSH-friendly ratatui shell. Same engine as the CLI. Mouse capture is off so terminal selection / copy-paste works.

Supports **embedded** (default) and **remote** (`--remote Host`) modes. See [modes.md](modes.md).

Confirm dialogs, Host edit, and sudo password stay in the TUI. Remote reboot sends
the password to the box over SSH (`sudo -S`); the box does not need a desktop.

Host edit loads **LAN** names from `/etc/hosts` and matching OpenSSH aliases (private
`HostName` or hosts-file name); **Tab** cycles them. Internet SSH aliases are omitted.

Tabs: **Setup**, **Overview**, **SSH**, **CLI**, **API**, **MCP**, **Reboot** (remote only), **Logs** (last).
Remote open paints the UI immediately (`box=probing...`), then refreshes surfaces (SSH, CLI,
API, MCP) on a background thread and updates the panels when it finishes. Press `r` to refresh
without blocking the UI. After refresh: `box=<version>`, `missing`, `auth failed`, or
`unreachable`. Auth failure is logged; the UI stays up.

`s0` (Setup or CLI tab Enter) syncs the tip CLI to the box when the version differs.

## Keys

| Key                | Action                                         |
| ------------------ | ---------------------------------------------- |
| `q` / Esc / Ctrl+C | Quit (while typing Host, only Ctrl+C quits)    |
| `?`                | Help overlay                                   |
| Left / Right       | Previous / next tab                            |
| `1`-`8`            | Jump to tab (remote: `7` Reboot, `8` Logs)     |
| j k / Up / Down    | Select step (Setup)                            |
| `p`                | Toggle full / minimal pipeline                 |
| Tab                | Toggle plan / apply                            |
| Enter              | Setup: run · SSH: edit Host · other: action    |
| `e` / `i`          | SSH: edit Host / install key (opt-in flag)     |
| `a`                | Run pipeline                                   |
| `b` / `B`          | `/etc` backup / disk probe in Logs             |
| `r`                | Refresh surfaces (SSH/CLI/API/MCP; background) |
| `c`                | Clear Logs (on Logs tab)                       |
| `y` / `n`          | Confirm / cancel (modals)                      |

```bash
make tui
sudo horto-os-ui-tui
horto-os-ui-tui --remote horto-box
horto-os-ui-tui --remote horto-box --apply
horto-os-ui-tui --remote horto-box --install-ssh-key   # opt-in key install
```
