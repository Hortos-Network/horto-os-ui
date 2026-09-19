# Modes and surfaces

Two modes describe **where you sit**, not two kinds of CLI on the box.

| Mode         | You sit    | You open             | Who applies setup                                                                     |
| ------------ | ---------- | -------------------- | ------------------------------------------------------------------------------------- |
| **Embedded** | On the box | CLI or TUI           | Same process, in-process shared engine                                                |
| **Remote**   | On a PC    | CLI, TUI, or Desktop | Shared remote runner SSHs in and runs the **CLI binary on the box** as an apply agent |

**Desktop is never embedded.** Always on a PC. Always remote (SSH for first install; HTTP day-2 once the status API is up).

Day-2 HTTP: Desktop (and KPI) use `GET /v1/status`. The only mutate route is
`POST /v1/backup/etc` (bearer token + `X-Horto-Confirm: backup-etc`). Reinstall
and setup stay on SSH / embedded CLI/TUI. See [status-api.md](status-api.md).

There is **one** CLI binary. Same `horto-os-ui setup …` commands.

- **Embedded:** you type those commands (or TUI does) while logged into the box.
- **Remote:** the runner uploads that binary once, then `ssh … sudo horto-os-ui setup …`.

## Surfaces

| Surface               | Embedded   | Remote           | Role                        |
| --------------------- | ---------- | ---------------- | --------------------------- |
| CLI                   | yes        | yes (`--remote`) | scripts / CI                |
| TUI                   | yes        | yes (`--remote`) | power users                 |
| Desktop (Tauri + web) | **no**     | **always**       | home users                  |
| Status API            | on the box | HTTP day-2       | LAN clients / Desktop / KPI |

## Auth (OpenSSH only)

Hard rules:

- Horto never shows its own password window.
- Horto never stores box user/root/sudo passwords.
- Remote mode does **not** require installing an SSH key on the box.
- **Default:** never write the box `authorized_keys`.
- **Key install:** only with explicit opt-in (`--install-ssh-key` / TUI flag / Desktop checkbox). Off by default.

| Check     | Who asks          | Typical home box                               |
| --------- | ----------------- | ---------------------------------------------- |
| SSH login | `sshd`            | Password **or** existing public key            |
| sudo      | `sudo` on the box | Same account password again, unless `NOPASSWD` |

CLI / TUI: OpenSSH and sudo prompt in the terminal.
Desktop (no TTY): OpenSSH `SSH_ASKPASS` (system askpass binary).

### CLI remote examples

```bash
# Preferred: Host already in ~/.ssh/config
horto-os-ui --remote horto-box --dry-run setup status --full

# Apply (sudo on the box); leave CLI+TUI+API installed after success
horto-os-ui --remote horto-box setup run --full

# Dev: use local release/debug bins instead of GitHub Release
horto-os-ui --remote horto-box --bin-dir target/debug --dry-run doctor

# Opt-in only: also run ssh-copy-id for this PC's public key
horto-os-ui --remote horto-box --install-ssh-key --dry-run doctor
```

### TUI remote

```bash
horto-os-ui-tui --remote horto-box --dry-run
horto-os-ui-tui --remote horto-box --install-ssh-key
```

Enter / `a` run steps or the full pipeline through the same remote runner.

### Binaries for the box

Remote mode downloads `horto-os-ui-{V}-{target}.tar.gz` from GitHub Releases for the box arch (`uname -m`), or uses `--bin-dir` / `HORTO_BIN_DIR`.

Default download tag is `v{VERSION}` (matches a stable Release). For tip Pre-release assets use `--release-tag dev-preview` or `HORTO_RELEASE_TAG=dev-preview` (asset filenames still use the Cargo workspace version).

### Practice tests

Unit tests mock OpenSSH. Live Docker SSH fixture (ignored by default):

```bash
make test-remote-docker
```
