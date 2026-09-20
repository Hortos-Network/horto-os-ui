# CLI (`horto-os-ui`)

On-box (**embedded**) or laptop (**remote** via OpenSSH) command line over the shared engine.

## Commands

| Group | Examples |
| ----- | -------- |
| Setup | `setup status\|run\|step` with `--full` / `--minimal` |
| Doctor | `doctor` |
| Docker | `docker status\|init\|rebuild` |
| Net | `net leases`, `net export-leases` |
| Backup | `backup etc\|list\|disk-status\|disk\|shrink\|status` |

Global: `--dry-run`, `--skip-piper`, `--remote Host` / `HORTO_REMOTE_HOST`, `--install-ssh-key` (opt-in, off by default), `--bin-dir` / `HORTO_BIN_DIR`, `--release-tag` / `HORTO_RELEASE_TAG` (default `v{VERSION}`; tip Pre-release: `dev-preview`). Apply needs root on the box (remote uses `sudo` over SSH).

```bash
make status
make cli ARGS='setup step s1'
make doctor
make remote-doctor
make remote-setup
make remote-reinstall INSTALL_SSH_KEY=1
make remote-reinstall INSTALL_SSH_KEY=1 APPLY=1
export HORTO_REMOTE_HOST=horto HORTO_RELEASE_TAG=dev-preview
horto-os-ui --dry-run setup status --full
```

Remote modes and auth: [modes.md](modes.md). Full operator recipes: [../README.md](../README.md).
