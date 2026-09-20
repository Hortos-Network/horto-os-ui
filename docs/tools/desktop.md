# Desktop (`horto-os-ui-desktop`)

Tauri 2 homeowner app. Native File / Edit / View / Help menu; webview loads `horto-os-ui-web`.

Desktop is **always remote**: Connection screen talks HTTP to the status API for day-2, and runs first-time OpenSSH remote setup through the same shared runner as CLI/TUI (system askpass; key install off unless checked). **Dry-run only** is checked by default; uncheck it and confirm to apply (install CLI + TUI + status-api on the box). See [modes.md](modes.md).

Day-2: after a successful remote apply, Desktop offers to save the status-api bearer
into Connection (localStorage). Paste remains a fallback. Overview **Backup /etc**
opens a confirmation modal, then `POST /v1/backup/etc` with bearer and
`X-Horto-Confirm: backup-etc`.

```bash
make desktop
make build-desktop
```

Menu actions refresh status, switch screens, cycle theme, zoom, About.
