# Syncing with horto-os tip

`horto-os` stays the ops **reference**. This repo owns the Rust behavior and embeds cleaned assets. When the tip moves, absorb here; do not execute horto-os `.sh` from horto-os-ui.

## Reference tip

| Item | Value |
| ---- | ----- |
| Repo | Hortos-Network / horto-os (or your fork remotes) |
| Branch to follow | `dev_multi-platform-os` until `main` catches up |
| Local layout | Sibling checkout next to this tree (operator choice) |

Do **not** hard-code machine-absolute paths in this repo’s docs or code.

## What to diff

| horto-os path | horto-os-ui destination |
| ------------- | ----------------------- |
| `scripts/s1_*.sh` … `s7_*.sh`, `m1_*.sh` | `crates/horto-os-ui-shared/src/steps/sN_*.rs` / `m1_*.rs` |
| `scripts/d1_docker_init.sh` | `crates/horto-os-ui-shared/src/steps/d1_docker.rs` |
| `config/` | `assets/config/` (then rebuild embed) |
| `docker_source/` | `assets/docker_source/` |
| Backup / docker rebuild helpers | `ops/backup.rs`, CLI `backup` / `docker rebuild` |

Step ↔ script table: [README.md](README.md#tracking-horto-os-script-changes).

## Absorb checklist

1. Update the local horto-os checkout onto the tip (`fetch` + fast-forward only).
2. Diff `scripts/`, `config/`, `docker_source/` against `steps/` + `assets/`.
3. Port behavior into the matching step module (and kits/ops if shared).
4. Copy or regenerate embedded assets under `assets/` when templates change.
5. Bump `step_version` on that step if resume semantics change.
6. Run:

```bash
cargo test -p horto-os-ui-shared
make status
make cli ARGS='docker init'   # dry-run by default via Make
make coverage-summary
```

7. Document material deltas in the step rustdoc or this file’s absorb notes (behavior only; no host-absolute lab paths).

## Current absorb snapshot

| Area | Status |
| ---- | ------ |
| Tip commit | `origin/dev_multi-platform-os` @ `d615248` (s1–s3 multi-platform env) |
| `os-configuration.env` | Synced; prompts OS_TYPE / NPU_TYPE / INSTALL_TYP / IOT_LAN |
| `iot-lan_conf.env` | Replaces `my_variables.env`; written when `IOT_LAN=y` |
| `s1` | Cockpit only (`step_version` 2); IoT apt packages moved into s2 IoT path |
| `s2` | OS conf + optional IoT (`s2_init_env_vars_iot.sh`) (`step_version` 2) |
| Docker stacks `common` / `rk3588` / `no_wyoming` | Embedded; `d1` merges common + NPU |
| Privileged full apply on guest/board | Still open (see docs/README QEMU section) |

## Split later

Each surface already has its own crate + [docs/tools/](tools/) page + crate README so CLI / TUI / API / desktop can move to separate repos without rewriting the narrative.
