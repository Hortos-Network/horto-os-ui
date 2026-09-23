# Desktop (`horto-os-ui-desktop`)

Tauri 2 homeowner app. Native File / Edit / View / Help menu; webview loads `horto-os-ui-web`.

Desktop is **always remote**: Connection talks HTTP to the status API for day-2, and runs first-time OpenSSH setup through the same shared runner as CLI/TUI (system askpass; key install off unless checked). Apply is off by default (plan only); check Apply and confirm to install CLI + TUI + status-api + MCP on the box. See [modes.md](modes.md).

**SSH host:** Connection lists **LAN** names from `/etc/hosts` plus OpenSSH aliases whose
`HostName` is private or already in hosts (buttons + datalist). Internet SSH aliases are omitted.
Pick a host first, then Status API / Probe / Install.

Day-2: after a successful remote apply, Desktop offers to save the status-api bearer
into Connection. Paste remains a fallback. Overview **Backup /etc**
opens a confirmation modal, then `POST /v1/backup/etc` with bearer and
`X-Horto-Confirm: backup-etc`.

```bash
make desktop
make build-desktop
```

Menu actions refresh status, switch screens, cycle theme, zoom, About.
