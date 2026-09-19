# horto-os-ui

Rust installer and ops tool for a Horto box. It owns host setup, Docker stack staging, and day-2 checks without requiring a horto-os checkout at runtime.

Templates under `assets/config/` and `assets/docker_source/` are embedded in the binary. The horto-os shell scripts are a reference only: each step module cites the matching script name and is maintained in Rust.

**Repo map:** [../README.md](../README.md) · **Tip sync:** [TIP_SYNC.md](TIP_SYNC.md) · **Per-tool docs:** [tools/](tools/)

## Make (preferred)

```bash
make help
make build / make build-release / make build-all
make lint / make test / make ci
make status          # horto-os-ui --dry-run setup status
make tui             # horto-os-ui-tui --dry-run
make api             # horto-os-ui-status-api on API_BIND (default 0.0.0.0:8787)
make kpi         # GPUI ops KPI viewer → HORTO_BOX_URL
make install         # release bins → ~/.local/bin
```

CLI passthrough: `make cli ARGS='setup status --minimal'`. Drop dry-run with `DRY_RUN=0` or `APPLY=1`.

## Release packaging

```bash
make release-bins   # dist/horto-os-ui-<ver>-<host-triple>.tar.gz + sha256
make deb            # needs `cargo install cargo-deb`; writes .deb under dist/
```

GitHub Release workflow also attaches naked tar.gz (amd64/arm64) and `.deb` when remotes exist.

## Build

Default members build the core library, `horto-os-ui`, `horto-os-ui-tui`, and `horto-os-ui-status-api`. Ops KPI (GPUI) is optional:

```bash
make build-release
make build-kpi   # or: make build-all
make install
```

## CLI

Dry-run works on any Linux without root and never writes privileged paths:

```bash
make status
make cli ARGS='setup status --minimal'
make setup-run
make setup-step STEP=s1
make doctor
make docker-status
make cli ARGS='docker init'
make cli ARGS='docker rebuild --dir /srv/docker/homepage'
make cli DRY_RUN=0 ARGS='net leases'
make cli ARGS='net export-leases'
make cli ARGS='backup status'
make cli ARGS='backup etc'
make cli ARGS='backup disk-status'
make cli ARGS='backup disk'   # guarded; requires SD boot + mounted dest
```

Apply mode needs root:

```bash
sudo horto-os-ui setup run --full
sudo horto-os-ui setup step s5
sudo horto-os-ui docker init
sudo horto-os-ui backup etc
sudo horto-os-ui backup etc --initial
```

### Backup

| Command                                 | What                                                                   |
| --------------------------------------- | ---------------------------------------------------------------------- |
| `horto-os-ui backup etc`                | Timestamped managed `/etc` copy → `/srv/backup/etc/YYYYMMDD-HHMMSS`    |
| `horto-os-ui backup etc --initial`      | Protected initial tree (same as setup step s3)                         |
| `horto-os-ui backup list`               | List timestamped backups                                               |
| `horto-os-ui backup disk-status`        | Probe readiness (safe anytime)                                         |
| `horto-os-ui backup disk`               | partclone eMMC image; **refuses if root is eMMC** (boot from SD first) |
| `horto-os-ui backup shrink --dest PATH` | Optional shrink-backup wrapper if that tool is on PATH                 |
| `horto-os-ui backup status`             | JSON for API / soft client                                             |

Disk backup is destructive if aimed at the wrong device. Defaults: source `/dev/mmcblk0p1`, dest `/mnt/external/horto-os`. Use `--boot-sectors` for the first 4MiB of `/dev/mmcblk0`. `--force` only relaxes “root not clearly SD/USB”; it never overrides eMMC-root detection.

Optional: `HORTO_APPLY_NAT=1` to apply NAT rules in s7 without a prompt. `--skip-piper` skips the piper model download during docker init.

## TUI

```bash
make tui
sudo horto-os-ui-tui          # apply mode on a real box
```

Keys: `?` help; Tab / 1-3 screens; arrows select steps; Enter runs the selected step; `a` pipeline; `b` / `B` backup / disk probe; `r` refresh; `d` dry-run; `q` / Esc / Ctrl+C quit. Mouse capture is off so you can select and copy text.

## Local status API

```bash
make api
# or: make api API_BIND=127.0.0.1:8787
# or: HORTO_API_TOKEN=secret horto-os-ui-status-api --bind 0.0.0.0:8787
```

- `GET /health` always open
- `GET /v1/status` JSON: hostname, setup summary, doctor, **backup**, containers, URLs, leases

If `HORTO_API_TOKEN` is set, protected routes require `Authorization: Bearer <token>`. If unset, the API warns at startup and stays open on the bind address for early LAN use.

## Ops KPI viewer (GPUI)

`horto-os-ui-kpi` is a GPUI control-room dashboard (live charts for fleet, readiness, network, and EVCC energy when linked). It does not offer remote install, apt, netplan, or backup apply.

```bash
make kpi
make kpi HORTO_BOX_URL=http://192.168.1.10:8787
# optional: HORTO_API_TOKEN=secret
```

## Homeowner desktop (Tauri + Leptos + rangular)

`horto-os-ui-desktop` is the PC product shell (Tauri 2). It ships a native File / Edit / View / Help menu and embeds the `horto-os-ui-web` Leptos CSR UI built with **rangular** colocated `.html` / `.scss` panels (Cargo git dep on Interchouette-ITC/rangular `dev`). The SPA talks to `horto-os-ui-status-api` on the box. No privileged remote install from the desktop app.

```bash
make desktop-web          # Trunk release → crates/horto-os-ui-web/dist
make desktop-web-serve    # browser UI on http://localhost:4187
make desktop              # Tauri window (builds web + shell)
make build-desktop        # release Tauri binary
```

Point the UI at the box (`http://<box>:8787`). Set a bearer token when the API requires `HORTO_API_TOKEN`.

Colocated rangular panels cover top bar, connection, box status, services, and containers.

## Testing notes

- `--dry-run` and unit tests cover planning on a normal Linux workstation.
- Docker compose bring-up from staged `/srv/docker` covers stack packaging.
- Full host networking (netplan, hostapd, dnsmasq-as-host, NAT) needs a QEMU/KVM guest or a real board. Docker alone is not a fake Horto box.
- Full eMMC image backup needs a temporary SD/USB boot; dry-run / `backup disk-status` work on a workstation.

### QEMU / board apply (Phase G)

Privileged `setup run` / `setup step s5`–`s7` must be proven on a guest or board, not only with `--dry-run` on a developer laptop.

Suggested path:

1. Boot a Debian or Armbian-like **QEMU/KVM** guest (or use a real RK3588).
2. Install release or locally built `horto-os-ui` / `horto-os-ui-tui` into the guest.
3. Run `horto-os-ui --dry-run setup run --full` first; confirm step plan and paths.
4. Apply with `sudo horto-os-ui setup run --full` (or step-by-step). Expect netplan / hostapd / dnsmasq / NAT side effects.
5. Re-check with `horto-os-ui doctor`, `docker status`, and `GET /v1/status` from the status API.

Workstation dry-run proofs (`make status`, `make setup-run`, `make docker-status`) stay useful for planning, but they do not replace the guest/board gate.

## Tracking horto-os script changes

One Rust module per reference script under `crates/horto-os-ui-shared/src/steps/`:

| Step | Reference script         |
| ---- | ------------------------ |
| s1   | s1_init_horto_os.sh      |
| s2   | s2_init_env_vars.sh      |
| s3   | s3_backup_etc_configs.sh |
| s4   | s4_deploy_configs.sh     |
| s5   | s5_apply_configs.sh      |
| s6   | s6_validate_configs.sh   |
| s7   | s7_activate_services.sh  |
| m1   | m1_minimal_setup_run.sh  |
| d1   | d1_docker_init.sh        |

Related helpers (not setup steps): `timestamped_backup_etc_configs.sh`, `partclone_backup_sda5.sh`, `docker_rebuild.sh` → `horto-os-ui backup …` / `horto-os-ui docker rebuild`.

When a script changes: follow [TIP_SYNC.md](TIP_SYNC.md). Update that module (and embedded assets if needed), bump `step_version` if resume semantics change, then re-run `cargo test -p horto-os-ui-shared`.

### Errors and logging

- Engine (`horto-os-ui-shared`): typed `HortoError` via `thiserror` (not a workspace-wide dep).
- Binaries: `anyhow` at `main`; `?` converts `HortoError`.
- `tracing` events from the engine; CLI/API install `tracing-subscriber` (`RUST_LOG`). TUI keeps step output in its Logs pane.

### Multi-platform absorb

See [TIP_SYNC.md](TIP_SYNC.md) for the live absorb checklist and snapshot table.
