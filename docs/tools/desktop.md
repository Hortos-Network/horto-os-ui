# Desktop (`horto-os-ui-desktop`)

Tauri 2 homeowner app. Native File / Edit / View / Help menu; webview loads `horto-os-ui-web`.

Desktop is **always remote**: Connection screen talks HTTP to the status API for day-2, and can run a first-time OpenSSH remote setup (system askpass; key install off unless the user checks the box). See [modes.md](modes.md).

Day-2: paste `HORTO_API_TOKEN` into Connection (persisted in localStorage). Overview
**Backup /etc** opens a confirmation modal, then `POST /v1/backup/etc` with bearer
and `X-Horto-Confirm: backup-etc`. Reinstall and setup stay on SSH / CLI.

```bash
make desktop
make build-desktop
```

Menu actions refresh status, switch screens, cycle theme, zoom, About.
