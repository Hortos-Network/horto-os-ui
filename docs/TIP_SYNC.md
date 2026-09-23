# Syncing with horto-os tip

`horto-os` stays the ops **reference**. This repo owns the Rust behavior and embeds cleaned assets. When the tip moves, absorb here; do not execute horto-os `.sh` from horto-os-ui.

## Reference tip

| Item             | Value                                                |
| ---------------- | ---------------------------------------------------- |
| Repo             | Hortos-Network / horto-os (or your fork remotes)     |
| Branch to follow | `dev_multi-platform-os` until `main` catches up      |
| Local layout     | Sibling checkout next to this tree (operator choice) |

Do **not** hard-code machine-absolute paths in this repo’s docs or code.

## What to diff

| horto-os path                            | horto-os-ui destination                                                               |
| ---------------------------------------- | ------------------------------------------------------------------------------------- |
| `scripts/s1_*.sh` … `s7_*.sh`, `m1_*.sh` | `crates/horto-os-ui-shared/src/steps/sN_*.rs` / `m1_*.rs`                             |
| `scripts/d1_docker_init.sh`              | `crates/horto-os-ui-shared/src/steps/d1_docker.rs`                                    |
| (UI) `d0` / `d2`                         | `d0_docker_engine.rs` / `d2_start_stacks.rs` (Engine install + start Dockge/Homepage) |
| `config/`                                | `assets/config/` (then rebuild embed)                                                 |
| `docker_source/`                         | `assets/docker_source/`                                                               |
| Backup / docker rebuild helpers          | `ops/backup.rs`, CLI `backup` / `docker rebuild`                                      |

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
make cli ARGS='docker init'   # plan by default via Make; APPLY=1 for --apply
make coverage-summary
```

7. Document material deltas in the step rustdoc or this file’s absorb notes (behavior only; no host-absolute lab paths).

## Current absorb snapshot

| Area                                             | Status                                                                                                         |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| Tip commit                                       | `origin/dev_multi-platform-os` @ `85185ef`                                                                     |
| `assets/config/`                                 | Absorbed from tip `config/` (UI keeps `service_links.env` when tip lacks it)                                   |
| `os-configuration.env`                           | Synced; `INSTALL_TYPE` (migrates legacy `INSTALL_TYP`); NPU default `rk3588`; prompts OS_TYPE / NPU_TYPE / IOT_LAN |
| `iot-lan_conf.env`                               | Replaces `my_variables.env`; written when `IOT_LAN=y`                                                          |
| `s1`                                             | Cockpit only + listen 9890; IoT apt packages moved into s2 IoT path                                            |
| `s2`                                             | OS conf + optional IoT; WiFi optional via `WIFI_INTERFACE=none` (`step_version` 4)                             |
| `s4`                                             | Full IoT stages hostapd when WiFi AP enabled; host/minimal also stages `resolv.conf` (`step_version` 3)       |
| `s5`                                             | IoT: full staging tree + stop systemd-resolved; host-only: `hosts`+`hostname` (`step_version` 2)              |
| `s6` / `s7`                                      | Keep WIFI=none hostapd skips + WAN NAT (`ETH_LAN` / default route); tip always-on hostapd not absorbed         |
| `d1`                                             | Homepage `env_file: os-configuration.env`; render vars fall back to OS conf; stages homepage env (`step_version` 4) |
| Docker stacks `common` / `rk3588` / `no_wyoming` | Absorbed from tip; `d1` merges common + NPU                                                                    |
| Tip service stacks (`evcc`, `homepage`, …)       | Present on tip as separate stack dirs; UI keeps the merge layout only                                          |
| Privileged full apply on guest/board             | Still open (see docs/README QEMU section)                                                                      |

### Absorb notes (`d615248` → `85185ef`)

- Ported tip `INSTALL_TYPE`, non-IoT host apply (`s5_apply_host_configs`), homepage compose env file, and `d1` OS-conf render fallback.
- Did **not** absorb tip networking always-on hostapd / required WiFi (UI keeps Ethernet-only).
- horto-os PR #4 (shell WIFI=none on tip) was still open at absorb time; shell tip may lag UI on that gate.

## Split later

Each surface already has its own crate + [docs/tools/](tools/) page + crate README so CLI / TUI / API / desktop can move to separate repos without rewriting the narrative.
